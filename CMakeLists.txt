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
            -fsanitize=address,undefined,float-cast-overflow -fno-omit-frame-pointer)
        target_link_options(${target} PRIVATE -fsanitize=address,undefined,float-cast-overflow)
    endif()
endfunction()

find_package(EXPAT 2.5 REQUIRED)
# Encoding and decoding run in-process through libav* (FFmpeg libraries).
find_package(PkgConfig REQUIRED)
pkg_check_modules(LIBAV REQUIRED IMPORTED_TARGET
    libavformat>=60 libavcodec>=60 libavutil>=58 libswscale>=7 libswresample>=4)
# Text: Fontconfig lookup, FriBidi reordering, HarfBuzz shaping, FreeType
# rasterization, all in-process (src/text.c).
pkg_check_modules(TEXT REQUIRED IMPORTED_TARGET
    freetype2 harfbuzz>=2.8.2 fribidi>=1.0 fontconfig>=2.13)
# Runtime XSD validation of scene documents (src/xml_schema.c). 2.9.14 is
# the first release that never loads external parameter entities while
# entity substitution is off.
pkg_check_modules(LIBXML2 REQUIRED IMPORTED_TARGET libxml-2.0>=2.9.14)

# Embed the XSD so the schema the loader enforces is the schema we ship.
set(SR_XSD ${CMAKE_CURRENT_SOURCE_DIR}/schema/scene-render-1.1.xsd)
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS ${SR_XSD})
file(READ ${SR_XSD} SR_XSD_HEX HEX)
string(REGEX REPLACE "([0-9a-f][0-9a-f])" "0x\\1," SR_XSD_BYTES "${SR_XSD_HEX}")
configure_file(cmake/schema_data.c.in
               ${CMAKE_CURRENT_BINARY_DIR}/generated/schema_data.c @ONLY)

set(SCENE_RENDER_SOURCES
    src/common.c
    src/card.c
    src/parallel.c
    src/random.c
    src/color.c
    src/raster.c
    src/vector_path.c
    src/mesh.c
    src/gpu.c
    src/spatial.c
    src/diagnostics.c
    src/timeline.c
    src/length.c
    src/length_frame.c
    src/curves.c
    src/property.c
    src/scene.c
    src/assets.c
    src/text.c
    src/procedural.c
    src/audio.c
    src/compositor.c
    src/camera.c
    src/lighting.c
    src/effects.c
    src/particles.c
    src/deform.c
    src/physics.c
    src/encoder.c
    src/video.c
    src/renderer.c
    src/resume.c
    src/xml.c
    src/xml_elements.c
    src/xml_lengths.c
    src/xml_animation.c
    src/xml_nodes.c
    src/xml_resolve.c
    src/xml_audio.c
    src/xml_camera.c
    src/xml_visual.c
    src/xml_physics.c
    src/xml_styles.c
    src/xml_metadata.c
    src/metadata.c
    src/xml_schema.c
    src/xml_capabilities.c
    src/cli_args.c
    ${CMAKE_CURRENT_BINARY_DIR}/generated/schema_data.c)

add_library(scene_render_core STATIC ${SCENE_RENDER_SOURCES})
target_include_directories(scene_render_core PUBLIC include PRIVATE src)
target_compile_definitions(scene_render_core PUBLIC _POSIX_C_SOURCE=200809L)
sr_configure_target(scene_render_core)
find_package(Threads REQUIRED)
target_link_libraries(scene_render_core PUBLIC EXPAT::EXPAT PkgConfig::LIBAV
                      PkgConfig::TEXT PkgConfig::LIBXML2 m
                      Threads::Threads ${CMAKE_DL_LIBS})

add_executable(scene-render src/main.c)
target_link_libraries(scene-render PRIVATE scene_render_core)
sr_configure_target(scene-render)

include(CTest)
if(BUILD_TESTING)
    find_package(Python3 REQUIRED COMPONENTS Interpreter)
    add_test(NAME schema.capabilities
        COMMAND ${Python3_EXECUTABLE}
                ${CMAKE_CURRENT_SOURCE_DIR}/tools/feature-matrix.py --check)
    add_test(NAME schema.compatibility
        COMMAND ${Python3_EXECUTABLE}
                ${CMAKE_CURRENT_SOURCE_DIR}/tools/schema-compatibility.py)
    set(SR_UNIT_SUITES timeline length length_frame length_physics
                       curves geometry compositor color vector random
                       mesh scene property xml xml_lengths profile styles metadata
                       camera physics blend group raster mask path image
                       encode encode_faults audio video
                       fx anim_color particles deform shadow text args resume
                       depth)
    file(GLOB SR_UNIT_SOURCES CONFIGURE_DEPENDS
         ${CMAKE_CURRENT_SOURCE_DIR}/tests/unit/*.c)
    add_executable(sr-unit-tests ${SR_UNIT_SOURCES})
    target_compile_definitions(sr-unit-tests PRIVATE
        SR_TEST_DATA_DIR="${CMAKE_CURRENT_SOURCE_DIR}"
        SR_TEST_TMP_DIR="${CMAKE_BINARY_DIR}/test_tmp")
    # Internal headers too (src/xml_schema.h: refusing-loader counter).
    target_include_directories(sr-unit-tests PRIVATE src)
    target_link_libraries(sr-unit-tests PRIVATE scene_render_core)
    sr_configure_target(sr-unit-tests)
    # The interruption test's program: the same sources with the
    # SR_TEST_HOOKS abort hook compiled in. Only cli.resume runs it; the
    # shipped scene-render has no hook (cli.no_test_hooks checks).
    add_executable(scene-render-testhooks src/main.c ${SCENE_RENDER_SOURCES})
    target_include_directories(scene-render-testhooks PRIVATE include src)
    target_compile_definitions(scene-render-testhooks PRIVATE
        _POSIX_C_SOURCE=200809L SR_TEST_HOOKS)
    target_link_libraries(scene-render-testhooks PRIVATE EXPAT::EXPAT
        PkgConfig::LIBAV PkgConfig::TEXT PkgConfig::LIBXML2 m Threads::Threads
        ${CMAKE_DL_LIBS})
    sr_configure_target(scene-render-testhooks)
    add_test(NAME tools.verification
        COMMAND ${Python3_EXECUTABLE}
                ${CMAKE_CURRENT_SOURCE_DIR}/tests/test_verification_tools.py)
    add_test(NAME frame_order
        COMMAND sh ${CMAKE_CURRENT_SOURCE_DIR}/tools/frame-order-check.sh
                $<TARGET_FILE:scene-render> $<TARGET_FILE:scene-render-testhooks>)
    # --- Verification depth (P7) ---------------------------------------
    # unit.golden: reference images in tests/golden/expected/
    # (SR_UPDATE_GOLDEN=1 regenerates them). unit.oom: allocation-failure
    # injection (tests/unit/test_oom.c) reroutes every malloc/calloc/realloc
    # (and free, for its leak check) made by the statically linked core
    # through a failing counter.
    list(APPEND SR_UNIT_SUITES golden oom)
    target_link_options(sr-unit-tests PRIVATE
        -Wl,--wrap=malloc,--wrap=calloc,--wrap=realloc,--wrap=free)
    # --------------------------------------------------------------------
    # libav fault injection (tests/unit/test_encode_faults.c): every libav
    # call made by the statically linked core goes through a wrapper that can
    # be told to fail.
    foreach(fn avformat_alloc_output_context2 avcodec_find_encoder_by_name
               avcodec_alloc_context3 avcodec_open2 avformat_new_stream
               avcodec_parameters_from_context avio_open avformat_write_header av_dict_set
               av_opt_set
               sws_getContext av_frame_alloc av_packet_alloc av_frame_get_buffer
               av_frame_make_writable avcodec_send_frame avcodec_receive_packet
               av_interleaved_write_frame av_write_trailer avformat_open_input
               avformat_find_stream_info av_find_best_stream avcodec_find_decoder
               avcodec_parameters_to_context av_read_frame avformat_seek_file
               avcodec_send_packet avcodec_receive_frame av_frame_ref
               swr_alloc_set_opts2 swr_init swr_convert swr_get_out_samples
               sws_setColorspaceDetails sws_scale)
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

    # Command-line contract: exit codes, output formats, --hash, --resume and
    # --physics-cache, driven through cmake -P scripts in tests/cli/.
    set(SR_CLI $<TARGET_FILE:scene-render>)
    set(SR_CLI_DIR ${CMAKE_CURRENT_SOURCE_DIR}/tests/cli)
    set(SR_CLI_TMP ${CMAKE_BINARY_DIR}/test_tmp/cli)
    set(SR_MEDIA ${CMAKE_CURRENT_SOURCE_DIR}/tests/data-media.xml)
    file(MAKE_DIRECTORY ${SR_CLI_TMP})
    function(sr_cli_exit name expect regex)
        string(REPLACE ";" "|" args "${ARGN}")
        add_test(NAME cli.${name} COMMAND ${CMAKE_COMMAND} -DCMD=${SR_CLI}
                 "-DARGS=${args}" -DEXPECT=${expect} "-DOUTPUT_REGEX=${regex}"
                 -P ${SR_CLI_DIR}/expect_exit.cmake)
    endfunction()
    add_test(NAME cli.help COMMAND scene-render --help)
    set_tests_properties(cli.help PROPERTIES PASS_REGULAR_EXPRESSION
        "--scene.*--print-schema.*--hash.*--resume.*--segment-frames.*--keep-parts.*--physics-cache")
    add_test(NAME cli.version COMMAND scene-render --version)
    set_tests_properties(cli.version PROPERTIES PASS_REGULAR_EXPRESSION
        "^scene-render ${PROJECT_VERSION}\n$")
    add_test(NAME cli.schema_matches
        COMMAND ${CMAKE_COMMAND} -DCMD=${SR_CLI}
                -DSCHEMA=${CMAKE_CURRENT_SOURCE_DIR}/schema/scene-render-1.1.xsd
                -P ${SR_CLI_DIR}/schema_matches.cmake)
    add_test(NAME cli.print_schema COMMAND scene-render --print-schema)
    set_tests_properties(cli.print_schema PROPERTIES PASS_REGULAR_EXPRESSION
        "<xs:schema .*</xs:schema>")
    sr_cli_exit(report_unsupported_requires_validate 2
        "--report-unsupported requires --validate"
        --scene ${SR_MEDIA} --report-unsupported)
    sr_cli_exit(report_unsupported_duplicate 2
        "--report-unsupported given more than once"
        --scene ${SR_MEDIA} --validate --report-unsupported --report-unsupported)
    sr_cli_exit(report_unsupported 3
        "unsupported in this build: <shape condition=.*unsupported in this build: <effect type="
        --scene ${SR_CLI_DIR}/unsupported.xml --validate --report-unsupported)
    sr_cli_exit(validate_positional_profile 0 "valid:"
        --validate ${CMAKE_CURRENT_SOURCE_DIR}/tests/data-profile.xml)
    sr_cli_exit(validate_profile 0 "valid:"
        --scene ${CMAKE_CURRENT_SOURCE_DIR}/tests/data-profile.xml --validate)
    sr_cli_exit(unknown_option 2 "unknown option '--frobnicate'" --frobnicate)
    sr_cli_exit(validate_ok 0 "valid: [^\n]*data-media.xml \\(2 assets, 2 top-level layers\\)"
                --validate --scene ${SR_MEDIA})
    sr_cli_exit(validate_error 3
                "schema-error.xml:5: error: <shape> @bogus: [^\n]*not allowed.*schema-error.xml:6: error: <shape> @opacity: "
                --validate --scene ${SR_CLI_DIR}/schema-error.xml)
    sr_cli_exit(invalid_scene_exit 3 "<project> @width"
                --scene ${CMAKE_CURRENT_SOURCE_DIR}/tests/data-invalid.xml --hash)
    sr_cli_exit(missing_asset_exit 4 "does-not-exist.ppm"
                --scene ${SR_CLI_DIR}/missing-asset.xml --hash)
    sr_cli_exit(frame_out_of_range 2 "preview frame 99 is outside \\[0, 12\\)"
                --scene ${SR_MEDIA} --frame 99 --preview-out ${SR_CLI_TMP}/x.png)
    sr_cli_exit(hash_with_resume 2 "--hash cannot be combined with --resume"
                --scene ${SR_MEDIA} --hash --resume)
    add_test(NAME cli.preview_png COMMAND scene-render --scene ${SR_MEDIA}
             --frame 6 --preview-out ${SR_CLI_TMP}/preview.png)
    set_tests_properties(cli.preview_png PROPERTIES PASS_REGULAR_EXPRESSION
        "preview.png [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]\n")
    add_test(NAME cli.preview_default_name COMMAND scene-render --scene ${SR_MEDIA}
             --frame 1)
    set_tests_properties(cli.preview_default_name PROPERTIES
        WORKING_DIRECTORY ${SR_CLI_TMP}
        PASS_REGULAR_EXPRESSION "^frame-000001.png [0-9a-f]+\n$")
    add_test(NAME cli.hash_threads COMMAND ${CMAKE_COMMAND} -DCMD=${SR_CLI}
             -DSCENE=${SR_MEDIA} -P ${SR_CLI_DIR}/hash_threads.cmake)
    add_test(NAME cli.resume COMMAND ${CMAKE_COMMAND} -DCMD=${SR_CLI}
             -DHOOKS=$<TARGET_FILE:scene-render-testhooks>
             -DSCENE=${SR_MEDIA} -DROOT=${CMAKE_CURRENT_SOURCE_DIR}
             -DWORK=${SR_CLI_TMP}/resume -P ${SR_CLI_DIR}/resume_check.cmake)
    add_test(NAME cli.no_test_hooks COMMAND ${CMAKE_COMMAND} -DCMD=${SR_CLI}
             -DHOOKS=$<TARGET_FILE:scene-render-testhooks>
             -P ${SR_CLI_DIR}/no_test_hooks.cmake)
    # A second render on the same OUTPUT while the lock is held: exit 7.
    find_program(SR_FLOCK flock)
    if(SR_FLOCK)
        add_test(NAME cli.resume_locked COMMAND ${CMAKE_COMMAND} -DCMD=${SR_CLI}
                 -DFLOCK=${SR_FLOCK} -DSCENE=${SR_MEDIA}
                 -DWORK=${SR_CLI_TMP}/locked -P ${SR_CLI_DIR}/resume_locked.cmake)
    endif()
    # Standard output that cannot be written is an I/O error (exit 7).
    add_test(NAME cli.stdout_full COMMAND ${CMAKE_COMMAND} -DCMD=${SR_CLI}
             -DSCENE=${SR_MEDIA} -DWORK=${SR_CLI_TMP}/full
             -P ${SR_CLI_DIR}/stdout_full.cmake)
    sr_cli_exit(missing_value 2 "--scene requires a value" --scene --validate)
    sr_cli_exit(duplicate_option 2 "--scene given more than once"
                --scene ${SR_MEDIA} --scene ${SR_MEDIA} --validate)
    sr_cli_exit(signed_range 2 "--frame-range expects"
                --scene ${SR_MEDIA} --hash --frame-range "0:-1")
    add_test(NAME cli.physics_cache COMMAND ${CMAKE_COMMAND} -DCMD=${SR_CLI}
             -DSCENE=${CMAKE_CURRENT_SOURCE_DIR}/tests/data-advanced.xml
             -DWORK=${SR_CLI_TMP}/physics -P ${SR_CLI_DIR}/physics_cache.cmake)
    get_property(sr_all_tests DIRECTORY PROPERTY TESTS)
    foreach(test IN LISTS sr_all_tests)
        if(test MATCHES "^cli\\.")
            set_tests_properties(${test} PROPERTIES FAIL_REGULAR_EXPRESSION
                "runtime error:;ERROR: AddressSanitizer")
        endif()
    endforeach()
endif()

install(TARGETS scene-render scene_render_core
        RUNTIME DESTINATION bin
        ARCHIVE DESTINATION lib)
install(DIRECTORY include/ DESTINATION include)
install(DIRECTORY schema/ DESTINATION share/scene-render/schema)
install(DIRECTORY examples/ DESTINATION share/scene-render/examples)
