# SPDX-License-Identifier: GPL-3.0-or-later
cmake_minimum_required(VERSION 3.21)
project(redrob_canvas_native VERSION 0.1.0 LANGUAGES C CXX)

include(CTest)

option(REDROB_ENABLE_GEGL "Build the optional pinned-boundary GEGL C adapter" OFF)
option(REDROB_ENABLE_KRITA "Build the optional pinned-boundary Krita C++ adapter" OFF)
option(REDROB_BUILD_QT "Build the Qt 6.8 desktop shell" ON)

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
