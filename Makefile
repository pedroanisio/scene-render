CC ?= cc
AR ?= ar
CFLAGS ?= -O2 -g
CPPFLAGS += -D_POSIX_C_SOURCE=200809L -Iinclude
CFLAGS += -std=c17 -Wall -Wextra -Wpedantic -Werror
LDLIBS += -lexpat -lm -pthread

CORE_SOURCES := src/common.c src/diagnostics.c src/timeline.c src/scene.c \
	src/assets.c src/procedural.c src/audio.c src/compositor.c src/camera.c \
	src/lighting.c src/effects.c src/physics.c src/encoder.c src/renderer.c \
	src/resume.c src/xml.c \
	src/xml_elements.c src/xml_nodes.c src/xml_resolve.c src/xml_audio.c \
	src/xml_camera.c \
	src/xml_visual.c \
	src/xml_physics.c
CORE_OBJECTS := $(CORE_SOURCES:src/%.c=build/%.o)
APP_OBJECT := build/main.o
TEST_OBJECT := build/test_main.o
DEPS := $(CORE_OBJECTS:.o=.d) $(APP_OBJECT:.o=.d) $(TEST_OBJECT:.o=.d)

.PHONY: all clean test unit integration
all: build/scene-render

build/scene-render: $(CORE_OBJECTS) $(APP_OBJECT)
	$(CC) $(LDFLAGS) -o $@ $^ $(LDLIBS)

build/sr-unit-tests: $(CORE_OBJECTS) $(TEST_OBJECT)
	$(CC) $(LDFLAGS) -o $@ $^ $(LDLIBS)

build/%.o: src/%.c
	@mkdir -p build
	$(CC) $(CPPFLAGS) $(CFLAGS) -MMD -MP -c $< -o $@

build/test_main.o: tests/test_main.c
	@mkdir -p build
	$(CC) $(CPPFLAGS) $(CFLAGS) -MMD -MP -c $< -o $@

unit: build/sr-unit-tests
	./build/sr-unit-tests "$(CURDIR)"

integration: build/scene-render
	sh tests/run-integration.sh "$(CURDIR)" "$(CURDIR)/build/scene-render"

test: unit integration

clean:
	rm -f build/*.o build/*.d build/scene-render build/sr-unit-tests
	rm -rf build/test-artifacts

-include $(DEPS)
