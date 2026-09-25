CC ?= cc
AR ?= ar
CFLAGS ?= -O2 -g
CPPFLAGS += -D_POSIX_C_SOURCE=200809L -Iinclude
SR_WERROR ?= 1
SR_SANITIZE ?= 0
SR_COVERAGE ?= 0
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
CFLAGS += -fsanitize=address,undefined -fno-omit-frame-pointer
LDFLAGS += -fsanitize=address,undefined
endif
ifeq ($(SR_COVERAGE),1)
CFLAGS += --coverage -O0
LDFLAGS += --coverage
endif
LDLIBS += -lexpat -lm -pthread -ldl

CORE_SOURCES := src/common.c src/parallel.c src/color.c src/raster.c src/vector_path.c src/mesh.c src/gpu.c src/spatial.c src/diagnostics.c src/timeline.c src/scene.c \
	src/assets.c src/procedural.c src/audio.c src/compositor.c src/camera.c \
	src/lighting.c src/effects.c src/particles.c src/deform.c src/physics.c src/encoder.c src/renderer.c \
	src/resume.c src/xml.c \
	src/xml_elements.c src/xml_nodes.c src/xml_resolve.c src/xml_audio.c \
	src/xml_camera.c \
	src/xml_visual.c \
	src/xml_physics.c
CORE_OBJECTS := $(CORE_SOURCES:src/%.c=build/%.o)
APP_OBJECT := build/main.o
TEST_SOURCES := $(sort $(wildcard tests/unit/*.c))
TEST_OBJECTS := $(TEST_SOURCES:tests/unit/%.c=build/unit/%.o)
UNIT_SUITES := timeline geometry compositor color vector mesh scene xml \
	camera physics blend group raster mask path image \
	fx anim_color particles deform shadow
TEST_CPPFLAGS := -DSR_TEST_DATA_DIR='"$(CURDIR)"' \
	-DSR_TEST_TMP_DIR='"$(CURDIR)/build/test_tmp"'
DEPS := $(CORE_OBJECTS:.o=.d) $(APP_OBJECT:.o=.d) $(TEST_OBJECTS:.o=.d)

.PHONY: all clean test unit integration
all: build/scene-render

build/scene-render: $(CORE_OBJECTS) $(APP_OBJECT)
	$(CC) $(LDFLAGS) -o $@ $^ $(LDLIBS)

build/sr-unit-tests: $(CORE_OBJECTS) $(TEST_OBJECTS)
	$(CC) $(LDFLAGS) -o $@ $^ $(LDLIBS)

build/%.o: src/%.c
	@mkdir -p build
	$(CC) $(CPPFLAGS) $(CFLAGS) -MMD -MP -c $< -o $@

build/unit/%.o: tests/unit/%.c
	@mkdir -p build/unit
	$(CC) $(CPPFLAGS) $(TEST_CPPFLAGS) $(CFLAGS) -MMD -MP -c $< -o $@

# One run per suite, mirroring the unit.<suite> ctest entries.
unit: build/sr-unit-tests
	@status=0; for suite in $(UNIT_SUITES); do \
		out=$$(./build/sr-unit-tests $$suite 2>&1); rc=$$?; \
		printf '%s\n' "$$out"; \
		if [ $$rc -ne 0 ] || printf '%s\n' "$$out" | grep -q '\[FAIL\]'; then \
			echo "unit.$$suite FAILED"; status=1; fi; \
	done; exit $$status

integration: build/scene-render
	sh tests/run-integration.sh "$(CURDIR)" "$(CURDIR)/build/scene-render" \
		"$(CURDIR)/build/test-artifacts"

test: unit integration

clean:
	rm -f build/*.o build/*.d build/scene-render build/sr-unit-tests
	rm -rf build/unit build/test-artifacts build/test_tmp

-include $(DEPS)
