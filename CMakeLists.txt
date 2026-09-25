cmake_minimum_required(VERSION 3.20)
project(scene_render VERSION 1.1.0 LANGUAGES C)

set(CMAKE_C_STANDARD 17)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

find_package(EXPAT 2.5 REQUIRED)

set(SCENE_RENDER_SOURCES
    src/common.c
    src/parallel.c
    src/color.c
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
target_compile_options(scene_render_core PRIVATE
    $<$<COMPILE_LANG_AND_ID:C,GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Werror>)
find_package(Threads REQUIRED)
target_link_libraries(scene_render_core PUBLIC EXPAT::EXPAT m Threads::Threads ${CMAKE_DL_LIBS})

add_executable(scene-render src/main.c)
target_link_libraries(scene-render PRIVATE scene_render_core)

include(CTest)
if(BUILD_TESTING)
    add_executable(sr-unit-tests tests/test_main.c)
    target_link_libraries(sr-unit-tests PRIVATE scene_render_core)
    add_test(NAME unit COMMAND sr-unit-tests ${CMAKE_CURRENT_SOURCE_DIR})
    add_test(NAME integration
             COMMAND sh ${CMAKE_CURRENT_SOURCE_DIR}/tests/run-integration.sh
                        ${CMAKE_CURRENT_SOURCE_DIR} $<TARGET_FILE:scene-render>)
endif()

install(TARGETS scene-render scene_render_core
        RUNTIME DESTINATION bin
        ARCHIVE DESTINATION lib)
install(DIRECTORY include/ DESTINATION include)
install(DIRECTORY schema/ DESTINATION share/scene-render/schema)
install(DIRECTORY examples/ DESTINATION share/scene-render/examples)
