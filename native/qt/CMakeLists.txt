# SPDX-License-Identifier: GPL-3.0-or-later

# 6.11 is the floor. It was briefly raised to 6.12 because redrob_qml_restricted_lint passed
# --only-explicit-categories unconditionally, which only Qt 6.12's qmllint accepts -- but
# pinning the release to Qt's newest branch is its own failure mode: 6.12.0's archives were not
# mirrored for macOS or Windows days after release, and every platform leg died installing it.
# The lint test now probes qmllint for that flag instead, so the strict mode is used where it
# exists without making it a build requirement.
find_package(Qt6 6.11 REQUIRED COMPONENTS Core Concurrent Gui Qml Quick QuickControls2 Svg)
qt_standard_project_setup(REQUIRES 6.8)

find_program(CARGO_EXECUTABLE cargo REQUIRED)
find_package(Python3 REQUIRED COMPONENTS Interpreter)
get_filename_component(REDROB_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
set(REDROB_DISTRIBUTION_TOOL "${REDROB_ROOT}/tools/generate_distribution_artifacts.py")
set(REDROB_NOTICE_FILE "${REDROB_ROOT}/THIRD_PARTY_NOTICES.md")
set(REDROB_SOURCE_MANIFEST "${REDROB_ROOT}/SOURCE_OFFER.md")
set(REDROB_SOURCE_BUNDLE "${CMAKE_BINARY_DIR}/redrob-canvas-corresponding-source.tar.gz")

execute_process(
    COMMAND "${Python3_EXECUTABLE}" "${REDROB_DISTRIBUTION_TOOL}" --check
            --gegl "${REDROB_ENABLE_GEGL}" --krita "${REDROB_ENABLE_KRITA}" --babl "${REDROB_ENABLE_BABL}" --lcms "${REDROB_ENABLE_LCMS}"
    WORKING_DIRECTORY "${REDROB_ROOT}"
    RESULT_VARIABLE REDROB_DISTRIBUTION_CHECK_STATUS
    OUTPUT_VARIABLE REDROB_DISTRIBUTION_CHECK_OUTPUT
    ERROR_VARIABLE REDROB_DISTRIBUTION_CHECK_ERROR)
if(NOT REDROB_DISTRIBUTION_CHECK_STATUS EQUAL 0)
    message(FATAL_ERROR
        "Distribution artifacts are missing, stale, or do not match the native adapter selection. "
        "Run tools/generate_distribution_artifacts.py --generate --gegl ${REDROB_ENABLE_GEGL} "
        "--krita ${REDROB_ENABLE_KRITA} --babl ${REDROB_ENABLE_BABL} --lcms ${REDROB_ENABLE_LCMS}.\n${REDROB_DISTRIBUTION_CHECK_OUTPUT}${REDROB_DISTRIBUTION_CHECK_ERROR}")
endif()

add_custom_target(redrob_distribution_check
    COMMAND "${Python3_EXECUTABLE}" "${REDROB_DISTRIBUTION_TOOL}" --check
            --gegl "${REDROB_ENABLE_GEGL}" --krita "${REDROB_ENABLE_KRITA}" --babl "${REDROB_ENABLE_BABL}" --lcms "${REDROB_ENABLE_LCMS}"
    DEPENDS
        "${REDROB_DISTRIBUTION_TOOL}"
        "${REDROB_ROOT}/Cargo.lock"
        "${REDROB_NOTICE_FILE}"
        "${REDROB_SOURCE_MANIFEST}"
    WORKING_DIRECTORY "${REDROB_ROOT}"
    COMMENT "Checking complete offline distribution notices and source offer"
    VERBATIM)

add_custom_target(redrob_source_bundle ALL
    COMMAND "${Python3_EXECUTABLE}" "${REDROB_DISTRIBUTION_TOOL}" --check
            --gegl "${REDROB_ENABLE_GEGL}" --krita "${REDROB_ENABLE_KRITA}" --babl "${REDROB_ENABLE_BABL}" --lcms "${REDROB_ENABLE_LCMS}"
    COMMAND "${Python3_EXECUTABLE}" "${REDROB_DISTRIBUTION_TOOL}"
            --source-bundle "${REDROB_SOURCE_BUNDLE}"
            --gegl "${REDROB_ENABLE_GEGL}" --krita "${REDROB_ENABLE_KRITA}" --babl "${REDROB_ENABLE_BABL}" --lcms "${REDROB_ENABLE_LCMS}"
    BYPRODUCTS "${REDROB_SOURCE_BUNDLE}"
    DEPENDS
        "${REDROB_DISTRIBUTION_TOOL}"
        "${REDROB_ROOT}/Cargo.lock"
        "${REDROB_NOTICE_FILE}"
        "${REDROB_SOURCE_MANIFEST}"
    WORKING_DIRECTORY "${REDROB_ROOT}"
    COMMENT "Checking notices and creating deterministic corresponding source bundle"
    VERBATIM)

set(REDROB_CARGO_TARGET_DIR "${CMAKE_BINARY_DIR}/cargo-target")
set(REDROB_FFI_LIBRARY "${REDROB_CARGO_TARGET_DIR}/release/${CMAKE_STATIC_LIBRARY_PREFIX}redrob_ffi${CMAKE_STATIC_LIBRARY_SUFFIX}")

file(GLOB_RECURSE REDROB_RUST_SOURCES CONFIGURE_DEPENDS
    "${REDROB_ROOT}/crates/redrob-core/src/*.rs"
    "${REDROB_ROOT}/crates/redrob-agent/src/*.rs"
    "${REDROB_ROOT}/crates/redrob-ffi/src/*.rs")

add_custom_command(
    OUTPUT "${REDROB_FFI_LIBRARY}"
    COMMAND "${CMAKE_COMMAND}" -E env "CARGO_TARGET_DIR=${REDROB_CARGO_TARGET_DIR}"
            "${CARGO_EXECUTABLE}" build --manifest-path "${REDROB_ROOT}/Cargo.toml"
            --package redrob-ffi --release --locked
    DEPENDS
        "${REDROB_ROOT}/Cargo.toml"
        "${REDROB_ROOT}/Cargo.lock"
        "${REDROB_ROOT}/crates/redrob-core/Cargo.toml"
        "${REDROB_ROOT}/crates/redrob-agent/Cargo.toml"
        "${REDROB_ROOT}/crates/redrob-ffi/Cargo.toml"
        "${REDROB_ROOT}/crates/redrob-ffi/include/redrob_ffi.h"
        ${REDROB_RUST_SOURCES}
    WORKING_DIRECTORY "${REDROB_ROOT}"
    COMMENT "Building redrob-ffi static library with Cargo"
    VERBATIM)
add_custom_target(redrob_ffi_cargo DEPENDS "${REDROB_FFI_LIBRARY}")

add_library(redrob_ffi STATIC IMPORTED GLOBAL)
set_target_properties(redrob_ffi PROPERTIES
    IMPORTED_LOCATION "${REDROB_FFI_LIBRARY}"
    INTERFACE_INCLUDE_DIRECTORIES "${REDROB_ROOT}/crates/redrob-ffi/include")
add_dependencies(redrob_ffi redrob_ffi_cargo)

qt_add_executable(redrob-canvas
    main.cpp
    EditorBridge.cpp EditorBridge.h
    FrameIdAllocator.cpp FrameIdAllocator.h
    FrameModel.cpp FrameModel.h
    LayerModel.cpp LayerModel.h
    ProposalModel.cpp ProposalModel.h
    CanvasItem.cpp CanvasItem.h)
add_dependencies(redrob-canvas redrob_source_bundle)

set_source_files_properties("${REDROB_ROOT}/qml/Main.qml" PROPERTIES QT_RESOURCE_ALIAS "qml/Main.qml")
# Generated from the vendored design tokens by tools/generate_design_tokens.py. It must sit beside
# Main.qml in the resource tree, because Main.qml refers to it by type name and the QML engine resolves
# that against its own directory.
set_source_files_properties("${REDROB_ROOT}/qml/RedrobTokens.qml" PROPERTIES QT_RESOURCE_ALIAS "qml/RedrobTokens.qml")
set_source_files_properties("${REDROB_ROOT}/resources/icons/redrob-canvas.svg" PROPERTIES QT_RESOURCE_ALIAS "icons/redrob-canvas.svg")
# assets/Products/README.md: at 32px and below the doorway is two or three pixels wide, so the small
# variant carries tile and symbol only. Qt picks by size through the QIcon, so both are embedded.
set_source_files_properties("${REDROB_ROOT}/resources/icons/redrob-canvas-small.svg" PROPERTIES QT_RESOURCE_ALIAS "icons/redrob-canvas-small.svg")
# Design-system glyphs (tool rail, section headers), vendored from the design system and pinned in DESIGN_SYSTEM_PIN.json.
set(REDROB_TOOL_ICONS brush fill rectangle ellipse shape gradient crop transform eye chevronDown chevronRight
    folderOpen image save undo redo sun moon download
    play pause skipBack skipForward plus duplicate trash arrowLeft arrowRight
    zoomIn zoomOut minus eyedropper)
set(REDROB_TOOL_ICON_FILES)
foreach(icon IN LISTS REDROB_TOOL_ICONS)
    set(icon_file "${REDROB_ROOT}/third_party/redrob-ui/icons/${icon}.svg")
    set_source_files_properties("${icon_file}" PROPERTIES QT_RESOURCE_ALIAS "icons/ui/${icon}.svg")
    list(APPEND REDROB_TOOL_ICON_FILES "${icon_file}")
endforeach()
qt_add_resources(redrob-canvas redrob_embedded_resources
    PREFIX "/"
    FILES
        "${REDROB_ROOT}/qml/Main.qml"
        "${REDROB_ROOT}/qml/RedrobTokens.qml"
        "${REDROB_ROOT}/resources/icons/redrob-canvas.svg"
        "${REDROB_ROOT}/resources/icons/redrob-canvas-small.svg"
        ${REDROB_TOOL_ICON_FILES})

target_compile_features(redrob-canvas PRIVATE cxx_std_20)
target_compile_definitions(redrob-canvas PRIVATE QT_NO_CAST_FROM_ASCII QT_NO_CAST_TO_ASCII)
target_link_libraries(redrob-canvas PRIVATE
    redrob_ffi
    Qt6::Core Qt6::Concurrent Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg)

if(UNIX AND NOT APPLE)
    target_link_libraries(redrob-canvas PRIVATE dl pthread m)
endif()

# A Rust STATICLIB does not carry its own native dependencies. cargo passes them when IT links;
# here CMake links, so the libraries Rust's std and its crates import have to be named. Without
# them the first Windows build to get this far failed at the link step, not the compile:
#
#   LNK2019: unresolved external __imp_NtCreateNamedPipeFile  (std::sys::pal::windows::pipe)
#   LNK2019: unresolved external __imp_BCryptGenRandom        (getrandom)
#
# The list is not from memory. rustc reports it for this exact target:
#
#   rustc --target x86_64-pc-windows-msvc --crate-type staticlib --print native-static-libs
#   -> kernel32.lib ntdll.lib userenv.lib ws2_32.lib dbghelp.lib /defaultlib:msvcrt
#
# bcrypt is NOT in that list, and that is not an omission to be surprised by: the probe above is
# an empty crate, so it reports only what STD needs. BCryptGenRandom comes from the getrandom
# dependency's own link directive, which is why the linker named it separately. Run the same
# command against this workspace's ffi crate if a future dependency adds another one.
if(WIN32)
    target_link_libraries(redrob-canvas PRIVATE
        kernel32 ntdll userenv ws2_32 dbghelp bcrypt)
endif()

set_target_properties(redrob-canvas PROPERTIES
    WIN32_EXECUTABLE TRUE
    MACOSX_BUNDLE TRUE
    MACOSX_BUNDLE_BUNDLE_NAME "Redrob Canvas"
    MACOSX_BUNDLE_GUI_IDENTIFIER "ai.redrob.canvas")

if(NOT EXISTS "${REDROB_ROOT}/LICENSE"
   OR NOT EXISTS "${REDROB_NOTICE_FILE}"
   OR NOT EXISTS "${REDROB_SOURCE_MANIFEST}")
    message(FATAL_ERROR "Complete distribution license and source artifacts are required")
endif()

install(CODE "
    execute_process(
        COMMAND \"${Python3_EXECUTABLE}\" \"${REDROB_DISTRIBUTION_TOOL}\" --check
                --gegl \"${REDROB_ENABLE_GEGL}\" --krita \"${REDROB_ENABLE_KRITA}\" --babl \"${REDROB_ENABLE_BABL}\" --lcms \"${REDROB_ENABLE_LCMS}\"
        WORKING_DIRECTORY \"${REDROB_ROOT}\"
        RESULT_VARIABLE REDROB_INSTALL_CHECK_STATUS
        OUTPUT_VARIABLE REDROB_INSTALL_CHECK_OUTPUT
        ERROR_VARIABLE REDROB_INSTALL_CHECK_ERROR)
    if(NOT REDROB_INSTALL_CHECK_STATUS EQUAL 0)
        message(FATAL_ERROR
            \"Install blocked: distribution artifacts are stale or incomplete.\\n\${REDROB_INSTALL_CHECK_OUTPUT}\${REDROB_INSTALL_CHECK_ERROR}\")
    endif()
    execute_process(
        COMMAND \"${Python3_EXECUTABLE}\" \"${REDROB_DISTRIBUTION_TOOL}\"
                --check-source-bundle \"${REDROB_SOURCE_BUNDLE}\"
                --gegl \"${REDROB_ENABLE_GEGL}\" --krita \"${REDROB_ENABLE_KRITA}\" --babl \"${REDROB_ENABLE_BABL}\" --lcms \"${REDROB_ENABLE_LCMS}\"
        WORKING_DIRECTORY \"${REDROB_ROOT}\"
        RESULT_VARIABLE REDROB_INSTALL_BUNDLE_STATUS
        OUTPUT_VARIABLE REDROB_INSTALL_BUNDLE_OUTPUT
        ERROR_VARIABLE REDROB_INSTALL_BUNDLE_ERROR)
    if(NOT REDROB_INSTALL_BUNDLE_STATUS EQUAL 0)
        message(FATAL_ERROR
            \"Install blocked: build and verify target redrob_source_bundle first.\\n\${REDROB_INSTALL_BUNDLE_OUTPUT}\${REDROB_INSTALL_BUNDLE_ERROR}\")
    endif()
")

install(TARGETS redrob-canvas
    BUNDLE DESTINATION .
    RUNTIME DESTINATION bin)
install(FILES
    "${REDROB_ROOT}/LICENSE"
    "${REDROB_NOTICE_FILE}"
    "${REDROB_SOURCE_MANIFEST}"
    DESTINATION "share/redrob-canvas/licenses")
install(FILES "${REDROB_SOURCE_BUNDLE}"
    DESTINATION "share/redrob-canvas/source")


if(BUILD_TESTING)
    get_filename_component(REDROB_QT_PREFIX "${Qt6_DIR}/../../.." ABSOLUTE)
    find_program(REDROB_QMLLINT_EXECUTABLE NAMES qmllint HINTS "${REDROB_QT_PREFIX}/bin")
    if(REDROB_QMLLINT_EXECUTABLE)
        # --only-explicit-categories exists only in Qt 6.12 and later; Qt 6.8 through 6.11
        # reject it outright ("Unknown option 'only-explicit-categories'") and qmllint exits
        # non-zero before linting anything, which failed this test for a reason that had
        # nothing to do with the QML. Rather than requiring the newest Qt branch -- whose
        # archives are not reliably mirrored for the first weeks after release -- ask the
        # qmllint we actually have what it supports, and use the stricter mode where it exists.
        execute_process(
            COMMAND "${REDROB_QMLLINT_EXECUTABLE}" --help
            OUTPUT_VARIABLE REDROB_QMLLINT_HELP
            ERROR_VARIABLE REDROB_QMLLINT_HELP_ERR
            RESULT_VARIABLE REDROB_QMLLINT_HELP_STATUS)
        string(APPEND REDROB_QMLLINT_HELP "${REDROB_QMLLINT_HELP_ERR}")
        set(REDROB_QMLLINT_CATEGORY_ARGS "")
        if(REDROB_QMLLINT_HELP_STATUS EQUAL 0
           AND REDROB_QMLLINT_HELP MATCHES "only-explicit-categories")
            set(REDROB_QMLLINT_CATEGORY_ARGS "--only-explicit-categories")
            message(STATUS "qmllint supports --only-explicit-categories; linting in restricted mode")
        else()
            message(STATUS "qmllint does not support --only-explicit-categories (Qt < 6.12); linting with its default categories")
        endif()
        add_test(NAME redrob_qml_restricted_lint
            COMMAND "${REDROB_QMLLINT_EXECUTABLE}"
                    --ignore-settings
                    -I "${REDROB_QT_PREFIX}/qml"
                    # This repository's own QML directory, so RedrobTokens.qml -- generated from the
                    # vendored design tokens -- resolves as a type instead of being reported missing.
                    -I "${REDROB_ROOT}/qml"
                    ${REDROB_QMLLINT_CATEGORY_ARGS}
                    "${REDROB_ROOT}/qml/Main.qml")
    endif()
    add_test(NAME redrob_native_smoke COMMAND $<TARGET_FILE:redrob-canvas>)
    set_tests_properties(redrob_native_smoke PROPERTIES
        TIMEOUT 20
        ENVIRONMENT "QT_QPA_PLATFORM=offscreen;QT_QUICK_BACKEND=software;REDROB_SMOKE_TEST=1;REDROB_SCREENSHOT_PATH=${CMAKE_CURRENT_BINARY_DIR}/redrob-native-smoke.png")
    add_test(NAME redrob_minimal_platform_smoke COMMAND $<TARGET_FILE:redrob-canvas>)
    set_tests_properties(redrob_minimal_platform_smoke PROPERTIES
        TIMEOUT 20
        ENVIRONMENT "QT_QPA_PLATFORM=minimal;QT_QUICK_BACKEND=software;REDROB_SMOKE_TEST=1")
endif()