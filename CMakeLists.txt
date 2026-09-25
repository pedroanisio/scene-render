cmake_minimum_required(VERSION 3.20)
project(scene_render VERSION 1.1.0 LANGUAGES C)

set(CMAKE_C_STANDARD 17)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

option(SR_WERROR "Treat warnings as errors" ON)
option(SR_SANITIZE "Build with AddressSanitizer and UBSan" OFF)
option(SR_COVERAGE "Instrument for gcov coverage" OFF)
option(SR_PROFILE "Instrument for gprof (-pg, frame pointers kept)" OFF)

set(SR_WARNINGS -Wall -Wextra -Wpedantic -Wshadow -Wstrict-prototypes
                -Wmissing-prototypes -Wformat=2)
if(SR_WERROR)
    list(APPEND SR_WARNINGS -Werror)
endif()
# FMA contraction changes rounding between builds; keep arithmetic exact
# as written so frames stay bit-identical for identical inputs.
set(SR_DETERMINISM -ffp-contract=off -fno-fast-math)

# Applies warnings, determinism flags and optional instrumentation to a target.
function(sr_configure_target target)
    target_compile_options(${target} PRIVATE
        $<$<COMPILE_LANG_AND_ID:C,GNU,Clang>:${SR_WARNINGS};${SR_DETERMINISM}>)
    if(SR_COVERAGE)
        target_compile_options(${target} PRIVATE --coverage -O0)
        target_link_options(${target} PRIVATE --coverage)
    endif()
    if(SR_PROFILE)
        target_compile_options(${target} PRIVATE -pg -fno-omit-frame-pointer)
        target_link_options(${target} PRIVATE -pg)
    endif()
    if(SR_SANITIZE)
        target_compile_options(${target} PRIVATE
            -fsanitize=address,undefined -fno-omit-frame-pointer)
        target_link_options(${target} PRIVATE -fsanitize=address,undefined)
    endif()
endfunction()

find_package(EXPAT 2.5 REQUIRED)
# Encoding and decoding run in-process through libav* (FFmpeg libraries).
find_package(PkgConfig REQUIRED)
pkg_check_modules(LIBAV REQUIRED IMPORTED_TARGET
    libavformat>=60 libavcodec>=60 libavutil>=58 libswscale>=7 libswresample>=4)

set(SCENE_RENDER_SOURCES
    src/common.c
    src/parallel.c
    src/color.c
    src/raster.c
    src/vector_path.c
    src/mesh.c
    src/gpu.c
    src/spatial.c
    src/diagnostics.c
    src/timeline.c
    src/scene.c
    src/assets.c
    src/procedural.c
    src/audio.c
    src/compositor.c
    src/camera.c
    src/lighting.c
    src/effects.c
    src/physics.c
    src/encoder.c
    src/video.c
    src/renderer.c
    src/resume.c
    src/xml.c
    src/xml_elements.c
    src/xml_nodes.c
    src/xml_resolve.c
    src/xml_audio.c
    src/xml_camera.c
    src/xml_visual.c
    src/xml_physics.c)

add_library(scene_render_core STATIC ${SCENE_RENDER_SOURCES})
target_include_directories(scene_render_core PUBLIC include)
target_compile_definitions(scene_render_core PUBLIC _POSIX_C_SOURCE=200809L)
sr_configure_target(scene_render_core)
find_package(Threads REQUIRED)
target_link_libraries(scene_render_core PUBLIC EXPAT::EXPAT PkgConfig::LIBAV m
                      Threads::Threads ${CMAKE_DL_LIBS})

add_executable(scene-render src/main.c)
target_link_libraries(scene-render PRIVATE scene_render_core)
sr_configure_target(scene-render)

include(CTest)
if(BUILD_TESTING)
    set(SR_UNIT_SUITES timeline geometry compositor color vector mesh scene xml
                       camera physics blend group raster mask path image
                       encode encode_faults audio video)
    file(GLOB SR_UNIT_SOURCES CONFIGURE_DEPENDS
         ${CMAKE_CURRENT_SOURCE_DIR}/tests/unit/*.c)
    add_executable(sr-unit-tests ${SR_UNIT_SOURCES})
    target_compile_definitions(sr-unit-tests PRIVATE
        SR_TEST_DATA_DIR="${CMAKE_CURRENT_SOURCE_DIR}"
        SR_TEST_TMP_DIR="${CMAKE_BINARY_DIR}/test_tmp")
    target_link_libraries(sr-unit-tests PRIVATE scene_render_core)
    sr_configure_target(sr-unit-tests)
    # libav fault injection (tests/unit/test_encode_faults.c): every libav
    # call made by the statically linked core goes through a wrapper that can
    # be told to fail.
    foreach(fn avformat_alloc_output_context2 avcodec_find_encoder_by_name
               avcodec_alloc_context3 avcodec_open2 avformat_new_stream
               avcodec_parameters_from_context avio_open avformat_write_header
               sws_getContext av_frame_alloc av_packet_alloc av_frame_get_buffer
               av_frame_make_writable avcodec_send_frame avcodec_receive_packet
               av_interleaved_write_frame av_write_trailer avformat_open_input
               avformat_find_stream_info av_find_best_stream avcodec_find_decoder
               avcodec_parameters_to_context av_read_frame avformat_seek_file
               avcodec_send_packet avcodec_receive_frame av_frame_ref
               swr_alloc_set_opts2 swr_init swr_convert swr_get_out_samples)
        target_link_options(sr-unit-tests PRIVATE -Wl,--wrap=${fn})
    endforeach()
    # Stream inspection for tests/run-integration.sh (replaces ffprobe).
    add_executable(sr-probe tests/tools/sr-probe.c)
    target_link_libraries(sr-probe PRIVATE PkgConfig::LIBAV)
    sr_configure_target(sr-probe)
    foreach(suite IN LISTS SR_UNIT_SUITES)
        add_test(NAME unit.${suite} COMMAND sr-unit-tests ${suite})
        # UBSan recovers and keeps going by default; treat its reports as
        # failures too so SR_SANITIZE builds cannot pass silently.
        set_tests_properties(unit.${suite} PROPERTIES
            FAIL_REGULAR_EXPRESSION "\\[FAIL\\];runtime error:;ERROR: AddressSanitizer")
    endforeach()
    add_test(NAME integration
             COMMAND sh ${CMAKE_CURRENT_SOURCE_DIR}/tests/run-integration.sh
                        ${CMAKE_CURRENT_SOURCE_DIR} $<TARGET_FILE:scene-render>
                        ${CMAKE_BINARY_DIR}/test-artifacts
                        $<TARGET_FILE:sr-probe>)
endif()

install(TARGETS scene-render scene_render_core
        RUNTIME DESTINATION bin
        ARCHIVE DESTINATION lib)
install(DIRECTORY include/ DESTINATION include)
install(DIRECTORY schema/ DESTINATION share/scene-render/schema)
install(DIRECTORY examples/ DESTINATION share/scene-render/examples)
