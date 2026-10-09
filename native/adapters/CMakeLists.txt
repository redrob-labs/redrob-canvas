# SPDX-License-Identifier: GPL-3.0-or-later

add_library(redrob_adapter_capabilities STATIC redrob_adapter_capabilities.c)
target_include_directories(redrob_adapter_capabilities PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}")
target_compile_features(redrob_adapter_capabilities PUBLIC c_std_11)
if(REDROB_ENABLE_BABL)
    target_compile_definitions(redrob_adapter_capabilities PUBLIC REDROB_BABL_ADAPTER_COMPILED=1)
else()
    target_compile_definitions(redrob_adapter_capabilities PUBLIC REDROB_BABL_ADAPTER_COMPILED=0)
endif()
if(REDROB_ENABLE_LCMS)
    target_compile_definitions(redrob_adapter_capabilities PUBLIC REDROB_LCMS_ADAPTER_COMPILED=1)
else()
    target_compile_definitions(redrob_adapter_capabilities PUBLIC REDROB_LCMS_ADAPTER_COMPILED=0)
endif()


if(REDROB_ENABLE_BABL)
    find_package(PkgConfig QUIET)
    if(NOT PkgConfig_FOUND)
        message(FATAL_ERROR
            "REDROB_ENABLE_BABL=ON requires pkg-config and babl >= 0.1.108; pkg-config was not found")
    endif()
    # The floor is the version recorded in docs/upstream-sources.toml. It sits deliberately below the
    # 0.1.124 currently packaged on Ubuntu: the format names this adapter resolves have been stable
    # for far longer, and pinning to the newest available would refuse builds for no measured reason.
    pkg_check_modules(BABL QUIET IMPORTED_TARGET "babl-0.1>=0.1.108")
    if(NOT BABL_FOUND)
        message(FATAL_ERROR
            "REDROB_ENABLE_BABL=ON requires the system package babl-0.1 >= 0.1.108; no compatible pkg-config module was found")
    endif()
    find_package(Threads REQUIRED)
    add_library(redrob_babl_adapter STATIC babl/redrob_babl_adapter.c)
    target_include_directories(redrob_babl_adapter PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/babl")
    target_link_libraries(redrob_babl_adapter PRIVATE PkgConfig::BABL Threads::Threads)
    target_compile_features(redrob_babl_adapter PUBLIC c_std_11)
    target_compile_definitions(redrob_babl_adapter PUBLIC REDROB_BABL_ADAPTER_COMPILED=1)
endif()

if(REDROB_ENABLE_LCMS)
    find_package(PkgConfig QUIET)
    if(NOT PkgConfig_FOUND)
        message(FATAL_ERROR
            "REDROB_ENABLE_LCMS=ON requires pkg-config and lcms2 >= 2.16; pkg-config was not found")
    endif()
    # The floor is the version recorded in docs/upstream-sources.toml. cmsFLAGS_COPY_ALPHA, which
    # this adapter depends on absolutely, has been present far longer, so the floor is about having a
    # version anyone can obtain rather than about a feature gate.
    pkg_check_modules(LCMS2 QUIET IMPORTED_TARGET "lcms2>=2.16")
    if(NOT LCMS2_FOUND)
        message(FATAL_ERROR
            "REDROB_ENABLE_LCMS=ON requires the system package lcms2 >= 2.16; no compatible pkg-config module was found")
    endif()
    find_package(Threads REQUIRED)
    add_library(redrob_lcms_adapter STATIC lcms/redrob_lcms_adapter.c)
    target_include_directories(redrob_lcms_adapter PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/lcms")
    target_link_libraries(redrob_lcms_adapter PRIVATE PkgConfig::LCMS2 Threads::Threads)
    target_compile_features(redrob_lcms_adapter PUBLIC c_std_11)
    target_compile_definitions(redrob_lcms_adapter PUBLIC REDROB_LCMS_ADAPTER_COMPILED=1)
endif()


if(BUILD_TESTING)
    add_executable(redrob_adapter_capabilities_test redrob_adapter_capabilities_test.c)
    target_link_libraries(redrob_adapter_capabilities_test PRIVATE redrob_adapter_capabilities)
    add_test(NAME redrob_adapter_capabilities_test COMMAND redrob_adapter_capabilities_test)


    if(REDROB_ENABLE_BABL)
        add_executable(redrob_babl_adapter_test babl/redrob_babl_adapter_test.c)
        # libm for fabsf in the tolerance checks.
        target_link_libraries(redrob_babl_adapter_test PRIVATE redrob_babl_adapter m)
        add_test(NAME redrob_babl_adapter_test COMMAND redrob_babl_adapter_test)
    endif()

    if(REDROB_ENABLE_LCMS)
        add_executable(redrob_lcms_adapter_test lcms/redrob_lcms_adapter_test.c)
        # The test builds its own ICC fixture with lcms2, so it links the library
        # directly as well as the adapter. libm for fabsf in the tolerance checks.
        target_link_libraries(redrob_lcms_adapter_test PRIVATE redrob_lcms_adapter PkgConfig::LCMS2 m)
        add_test(NAME redrob_lcms_adapter_test COMMAND redrob_lcms_adapter_test)
    endif()
endif()
