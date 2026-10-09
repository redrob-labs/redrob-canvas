# SPDX-License-Identifier: GPL-3.0-or-later
cmake_minimum_required(VERSION 3.21)
# The version comes from the workspace Cargo.toml, the one the release workflow checks against
# the tag. It was a literal 0.3.0 here, so the 0.4.0 and 0.5.0 macOS bundles told Finder and the
# About box they were 0.3 while the binary inside reported the crate version.
file(READ "${CMAKE_CURRENT_LIST_DIR}/../Cargo.toml" REDROB_CARGO_TOML)
string(REGEX MATCH "\nversion = \"([0-9]+\\.[0-9]+\\.[0-9]+)\"" _redrob_version_line "\n${REDROB_CARGO_TOML}")
if(NOT CMAKE_MATCH_1)
    message(FATAL_ERROR "No workspace version found in Cargo.toml")
endif()
set(REDROB_VERSION "${CMAKE_MATCH_1}")
project(redrob_canvas_native VERSION ${REDROB_VERSION} LANGUAGES C CXX)

include(CTest)

option(REDROB_ENABLE_BABL "Build the optional pinned-boundary babl C adapter" OFF)
option(REDROB_ENABLE_LCMS "Build the optional pinned-boundary Little-CMS 2 C adapter" OFF)
option(REDROB_BUILD_QT "Build the Qt 6.12 desktop shell" ON)

add_subdirectory(adapters)

if(BUILD_TESTING)
    find_package(Python3 REQUIRED COMPONENTS Interpreter)
    add_test(NAME redrob_distribution_artifacts_python
        COMMAND "${Python3_EXECUTABLE}" -m unittest discover
                -s "${CMAKE_CURRENT_SOURCE_DIR}/../tools/tests"
                -p "test_*.py")
    set_tests_properties(redrob_distribution_artifacts_python PROPERTIES
        WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/..")
endif()

if(REDROB_BUILD_QT)
    add_subdirectory(qt)
endif()
