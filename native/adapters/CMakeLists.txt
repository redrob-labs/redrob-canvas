# SPDX-License-Identifier: GPL-3.0-or-later

add_library(redrob_adapter_capabilities STATIC redrob_adapter_capabilities.c)
target_include_directories(redrob_adapter_capabilities PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}")
target_compile_features(redrob_adapter_capabilities PUBLIC c_std_11)
if(REDROB_ENABLE_GEGL)
    target_compile_definitions(redrob_adapter_capabilities PUBLIC REDROB_GEGL_ADAPTER_COMPILED=1)
else()
    target_compile_definitions(redrob_adapter_capabilities PUBLIC REDROB_GEGL_ADAPTER_COMPILED=0)
endif()
if(REDROB_ENABLE_KRITA)
    target_compile_definitions(redrob_adapter_capabilities PUBLIC REDROB_KRITA_SCAFFOLD_COMPILED=1)
else()
    target_compile_definitions(redrob_adapter_capabilities PUBLIC REDROB_KRITA_SCAFFOLD_COMPILED=0)
endif()

if(REDROB_ENABLE_GEGL)
    find_package(PkgConfig QUIET)
    if(NOT PkgConfig_FOUND)
        message(FATAL_ERROR
            "REDROB_ENABLE_GEGL=ON requires pkg-config and GEGL >= 0.4.66; pkg-config was not found")
    endif()
    # GIMP's pinned source requires GEGL 0.4.66 or newer.
    pkg_check_modules(GEGL QUIET IMPORTED_TARGET "gegl-0.4>=0.4.66")
    if(NOT GEGL_FOUND)
        message(FATAL_ERROR
            "REDROB_ENABLE_GEGL=ON requires the system package gegl-0.4 >= 0.4.66; no compatible pkg-config module was found")
    endif()
    add_library(redrob_gegl_adapter STATIC gegl/redrob_gegl_adapter.c)
    target_include_directories(redrob_gegl_adapter PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/gegl")
    target_link_libraries(redrob_gegl_adapter PRIVATE PkgConfig::GEGL)
    target_compile_features(redrob_gegl_adapter PUBLIC c_std_11)
    target_compile_definitions(redrob_gegl_adapter PUBLIC REDROB_GEGL_ADAPTER_COMPILED=1)
endif()

if(REDROB_ENABLE_KRITA)
    set(REDROB_KRITA_SOURCE_DIR "" CACHE PATH
        "Krita source tree at the commit pinned in docs/upstream-sources.toml")
    if(NOT EXISTS "${REDROB_KRITA_SOURCE_DIR}/CMakeLists.txt")
        message(FATAL_ERROR
            "REDROB_ENABLE_KRITA=ON requires REDROB_KRITA_SOURCE_DIR at pinned commit fdbf33b2146735465bb8aa59928fbc1890ceb160")
    endif()
    find_package(Git QUIET)
    if(NOT Git_FOUND)
        message(FATAL_ERROR
            "REDROB_ENABLE_KRITA=ON requires Git to verify the pinned Krita source commit")
    endif()
    execute_process(
        COMMAND "${GIT_EXECUTABLE}" -C "${REDROB_KRITA_SOURCE_DIR}" rev-parse HEAD
        RESULT_VARIABLE REDROB_KRITA_REVISION_STATUS
        OUTPUT_VARIABLE REDROB_KRITA_REVISION
        OUTPUT_STRIP_TRAILING_WHITESPACE
        ERROR_QUIET)
    if(NOT REDROB_KRITA_REVISION_STATUS EQUAL 0 OR
       NOT REDROB_KRITA_REVISION STREQUAL "fdbf33b2146735465bb8aa59928fbc1890ceb160")
        message(FATAL_ERROR
            "REDROB_ENABLE_KRITA=ON requires REDROB_KRITA_SOURCE_DIR exactly at fdbf33b2146735465bb8aa59928fbc1890ceb160")
    endif()
    add_library(redrob_krita_adapter STATIC krita/RedrobKritaAdapter.cpp)
    target_include_directories(redrob_krita_adapter PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/krita")
    target_compile_features(redrob_krita_adapter PUBLIC cxx_std_20)
    target_compile_definitions(redrob_krita_adapter PUBLIC
        REDROB_KRITA_SCAFFOLD_COMPILED=1
        REDROB_KRITA_SOURCE_DIR="${REDROB_KRITA_SOURCE_DIR}")
    # This remains a pointer-only, non-operational scaffold. It deliberately
    # does not link Krita targets or advertise KRA/paint operations.
endif()

if(BUILD_TESTING)
    add_executable(redrob_adapter_capabilities_test redrob_adapter_capabilities_test.c)
    target_link_libraries(redrob_adapter_capabilities_test PRIVATE redrob_adapter_capabilities)
    add_test(NAME redrob_adapter_capabilities_test COMMAND redrob_adapter_capabilities_test)

    if(REDROB_ENABLE_GEGL)
        add_executable(redrob_gegl_adapter_test gegl/redrob_gegl_adapter_test.c)
        target_link_libraries(redrob_gegl_adapter_test PRIVATE redrob_gegl_adapter)
        add_test(NAME redrob_gegl_adapter_test COMMAND redrob_gegl_adapter_test)
    endif()

    if(REDROB_ENABLE_KRITA)
        add_executable(redrob_krita_adapter_test krita/RedrobKritaAdapterTest.cpp)
        target_link_libraries(redrob_krita_adapter_test PRIVATE redrob_krita_adapter)
        add_test(NAME redrob_krita_adapter_test COMMAND redrob_krita_adapter_test)
    endif()
endif()
