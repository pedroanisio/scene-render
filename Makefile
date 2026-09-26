CC ?= cc
AR ?= ar
CFLAGS ?= -O2 -g
CPPFLAGS += -D_POSIX_C_SOURCE=200809L -Iinclude
SR_WERROR ?= 1
SR_SANITIZE ?= 0
SR_COVERAGE ?= 0
SR_PROFILE ?= 0
# Output directory; e.g. `make SR_PROFILE=1 BUILD=build/profile` keeps an
# instrumented build beside the normal one.
BUILD ?= build
SR_WARNINGS := -Wall -Wextra -Wpedantic -Wshadow -Wstrict-prototypes \
	-Wmissing-prototypes -Wformat=2
ifeq ($(SR_WERROR),1)
SR_WARNINGS += -Werror
endif
# FMA contraction changes rounding between builds; keep arithmetic exact
# as written so frames stay bit-identical for identical inputs.
SR_DETERMINISM := -ffp-contract=off -fno-fast-math
CFLAGS += -std=c17 $(SR_WARNINGS) $(SR_DETERMINISM)
ifeq ($(SR_SANITIZE),1)
CFLAGS += -fsanitize=address,undefined,float-cast-overflow -fno-omit-frame-pointer
LDFLAGS += -fsanitize=address,undefined,float-cast-overflow
endif
ifeq ($(SR_PROFILE),1)
# override: -pg must reach compile and link even when CFLAGS/LDFLAGS are
# given on the command line (e.g. extra -I/-L paths).
override CFLAGS += -pg -fno-omit-frame-pointer
override LDFLAGS += -pg
endif
ifeq ($(SR_COVERAGE),1)
CFLAGS += --coverage -O0
LDFLAGS += --coverage
endif
PKG_CONFIG ?= pkg-config
LIBAV_MODULES := 'libavformat >= 60' 'libavcodec >= 60' 'libavutil >= 58' \
	'libswscale >= 7' 'libswresample >= 4'
LIBAV_CFLAGS := $(shell $(PKG_CONFIG) --cflags $(LIBAV_MODULES))
LIBAV_LIBS := $(shell $(PKG_CONFIG) --libs $(LIBAV_MODULES))
ifeq ($(LIBAV_LIBS),)
$(error libav development files not found: need $(LIBAV_MODULES))
endif
TEXT_MODULES := freetype2 'harfbuzz >= 2.8.2' 'fribidi >= 1.0' 'fontconfig >= 2.13'
TEXT_CFLAGS := $(shell $(PKG_CONFIG) --cflags $(TEXT_MODULES))
TEXT_LIBS := $(shell $(PKG_CONFIG) --libs $(TEXT_MODULES))
ifeq ($(TEXT_LIBS),)
$(error text development files not found: need $(TEXT_MODULES))
endif
# Runtime XSD validation of scene documents (src/xml_schema.c).
XML2_MODULES := 'libxml-2.0 >= 2.9.14'
XML2_CFLAGS := $(shell $(PKG_CONFIG) --cflags $(XML2_MODULES))
XML2_LIBS := $(shell $(PKG_CONFIG) --libs $(XML2_MODULES))
ifeq ($(XML2_LIBS),)
$(error libxml2 development files not found: need $(XML2_MODULES))
endif
CPPFLAGS += $(LIBAV_CFLAGS) $(TEXT_CFLAGS) $(XML2_CFLAGS)
LDLIBS += $(LIBAV_LIBS) $(TEXT_LIBS) $(XML2_LIBS) -lexpat -lm -pthread -ldl
# libav fault injection for tests/unit/test_encode_faults.c.
TEST_WRAPS := avformat_alloc_output_context2 avcodec_find_encoder_by_name \
	avcodec_alloc_context3 avcodec_open2 avformat_new_stream \
	avcodec_parameters_from_context avio_open avformat_write_header av_dict_set av_opt_set \
	sws_getContext av_frame_alloc av_packet_alloc av_frame_get_buffer \
	av_frame_make_writable avcodec_send_frame avcodec_receive_packet \
	av_interleaved_write_frame av_write_trailer avformat_open_input \
	avformat_find_stream_info av_find_best_stream avcodec_find_decoder \
	avcodec_parameters_to_context av_read_frame avformat_seek_file \
	avcodec_send_packet avcodec_receive_frame av_frame_ref \
	swr_alloc_set_opts2 swr_init swr_convert swr_get_out_samples \
	sws_setColorspaceDetails sws_scale
TEST_LDFLAGS := $(foreach fn,$(TEST_WRAPS),-Wl,--wrap=$(fn))
# --- Verification depth (P7): golden images and allocation-failure
# injection (tests/unit/test_golden.c, tests/unit/test_oom.c).
TEST_LDFLAGS += -Wl,--wrap=malloc,--wrap=calloc,--wrap=realloc,--wrap=free
P7_UNIT_SUITES := golden oom
# ----------------------------------------------------------------------

CORE_SOURCES := src/common.c src/card.c src/parallel.c src/color.c src/raster.c \
	src/vector_path.c src/mesh.c src/gpu.c src/spatial.c src/diagnostics.c \
	src/timeline.c src/length.c src/length_frame.c src/curves.c src/property.c src/scene.c \
	src/assets.c src/text.c src/procedural.c src/audio.c src/compositor.c src/camera.c \
	src/lighting.c src/effects.c src/particles.c src/deform.c src/physics.c \
	src/encoder.c src/video.c src/renderer.c \
	src/resume.c src/xml.c \
	src/xml_elements.c src/xml_lengths.c src/xml_animation.c src/xml_nodes.c \
	src/xml_resolve.c src/xml_audio.c \
	src/xml_camera.c \
	src/xml_visual.c \
	src/xml_physics.c src/xml_styles.c src/xml_metadata.c src/metadata.c \
	src/xml_schema.c src/xml_capabilities.c src/cli_args.c
# The embedded XSD (generated below) is part of the core library.
CORE_OBJECTS := $(CORE_SOURCES:src/%.c=$(BUILD)/%.o) $(BUILD)/schema_data.o
APP_OBJECT := $(BUILD)/main.o
TEST_SOURCES := $(sort $(wildcard tests/unit/*.c))
TEST_OBJECTS := $(TEST_SOURCES:tests/unit/%.c=$(BUILD)/unit/%.o)
UNIT_SUITES := timeline length length_frame length_physics curves geometry \
	compositor color vector mesh \
	scene property xml profile styles metadata \
	camera physics blend group raster mask path image encode encode_faults \
	audio video fx anim_color particles deform shadow text args resume depth \
	$(P7_UNIT_SUITES)
TEST_CPPFLAGS := -Isrc -DSR_TEST_DATA_DIR='"$(CURDIR)"' \
	-DSR_TEST_TMP_DIR='"$(abspath $(BUILD))/test_tmp"'
DEPS := $(CORE_OBJECTS:.o=.d) $(APP_OBJECT:.o=.d) $(TEST_OBJECTS:.o=.d)

.PHONY: all clean test unit integration profile perf-check
all: $(BUILD)/scene-render

$(BUILD)/scene-render: $(CORE_OBJECTS) $(APP_OBJECT)
	$(CC) $(LDFLAGS) -o $@ $^ $(LDLIBS)

$(BUILD)/sr-unit-tests: $(CORE_OBJECTS) $(TEST_OBJECTS)
	$(CC) $(LDFLAGS) $(TEST_LDFLAGS) -o $@ $^ $(LDLIBS)

$(BUILD)/sr-probe: tests/tools/sr-probe.c
	@mkdir -p $(BUILD)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(LDFLAGS) -o $@ $< $(LIBAV_LIBS)

$(BUILD)/%.o: src/%.c
	@mkdir -p $(BUILD)
	$(CC) $(CPPFLAGS) $(CFLAGS) -MMD -MP -c $< -o $@

# schema/scene-render-1.1.xsd as a C byte array (the CMake build does the same with
# cmake/schema_data.c.in); od and sed are POSIX, so no CMake or xxd needed.
$(BUILD)/schema_data.c: schema/scene-render-1.1.xsd
	@mkdir -p $(BUILD)
	{ echo '/* Generated from schema/scene-render-1.1.xsd by the Makefile. Do not edit. */'; \
	  echo '#include "schema_data.h"'; \
	  echo 'const unsigned char sr_schema_xsd[] = {'; \
	  od -An -v -tx1 schema/scene-render-1.1.xsd | sed -e 's/[0-9a-f][0-9a-f]/0x&,/g'; \
	  echo '0x00};'; \
	  echo 'const size_t sr_schema_xsd_len = sizeof sr_schema_xsd - 1;'; \
	} > $@.tmp && mv $@.tmp $@

$(BUILD)/schema_data.o: $(BUILD)/schema_data.c src/schema_data.h
	$(CC) $(CPPFLAGS) -Isrc $(CFLAGS) -c $< -o $@

$(BUILD)/unit/%.o: tests/unit/%.c
	@mkdir -p $(BUILD)/unit
	$(CC) $(CPPFLAGS) $(TEST_CPPFLAGS) $(CFLAGS) -MMD -MP -c $< -o $@

# One run per suite, mirroring the unit.<suite> ctest entries.
unit: $(BUILD)/sr-unit-tests
	@status=0; for suite in $(UNIT_SUITES); do \
		out=$$("$(abspath $(BUILD))/sr-unit-tests" $$suite 2>&1); rc=$$?; \
		printf '%s\n' "$$out"; \
		if [ $$rc -ne 0 ] || printf '%s\n' "$$out" | grep -Eq '\[FAIL\]|runtime error:|ERROR: AddressSanitizer'; then \
			echo "unit.$$suite FAILED"; status=1; fi; \
	done; exit $$status

integration: $(BUILD)/scene-render $(BUILD)/sr-probe
	sh tests/run-integration.sh "$(CURDIR)" "$(abspath $(BUILD))/scene-render" \
		"$(abspath $(BUILD))/test-artifacts" "$(abspath $(BUILD))/sr-probe"

test: unit integration

# gprof flat profile + call graph for a scene slice (see scripts/profile.sh).
profile:
	sh scripts/profile.sh

# Stage-CPU regression guard against benchmarks/baseline.json; not part of
# `make test` because timing depends on the host.
perf-check: $(BUILD)/scene-render
	python3 scripts/perf-check.py --binary $(BUILD)/scene-render

clean:
	rm -f $(BUILD)/*.o $(BUILD)/*.d $(BUILD)/scene-render $(BUILD)/sr-unit-tests \
		$(BUILD)/sr-probe $(BUILD)/schema_data.c
	rm -rf $(BUILD)/unit $(BUILD)/test-artifacts $(BUILD)/test_tmp

-include $(DEPS)
