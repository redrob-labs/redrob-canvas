//! The Qt front end must expose what the engine can do.
//!
//! These tests read `qml/Main.qml` as text and compare it with the engine's own declarations. They
//! exist because the UI drifted behind the engine silently: the blend-mode combo listed 41 of the
//! engine's 43 modes (`merge` and `split`, added for J.6, were unreachable from the UI) and nothing
//! failed. Each check runs in BOTH directions -- a UI name the engine does not know is as much a
//! defect as an engine capability the UI hides.

use redrob_core::BlendMode;
use std::collections::BTreeSet;

const MAIN_QML: &str = include_str!("../../../qml/Main.qml");
/// P12: the filter browser moved out of Main.qml. Tests about the browser read this file.
const FILTER_BROWSER_QML: &str = include_str!("../../../qml/FilterBrowser.qml");
const TEXT_DIALOG_QML: &str = include_str!("../../../qml/TextNodeDialog.qml");
const VECTOR_DIALOG_QML: &str = include_str!("../../../qml/VectorRectDialog.qml");
const DOCUMENT_RS: &str = include_str!("../src/document.rs");
const EDITOR_BRIDGE_CPP: &str = include_str!("../../../native/qt/EditorBridge.cpp");

/// `HsvHue` -> `hsv_hue`, the `serde(rename_all = "snake_case")` spelling.
fn snake(ident: &str) -> String {
    let mut out = String::new();
    for (i, c) in ident.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Variant names of `pub enum BlendMode`, read from its source so a new variant cannot be missed.
///
/// Only lines that are a bare `Ident,` count; attributes, doc and plain comments are skipped.
fn engine_blend_modes() -> BTreeSet<String> {
    let start = DOCUMENT_RS
        .find("pub enum BlendMode {")
        .expect("BlendMode declaration");
    let body = &DOCUMENT_RS[start..];
    let end = body.find("\n}").expect("end of BlendMode");
    body[..end]
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && !l.starts_with("#["))
        .filter_map(|l| l.strip_suffix(','))
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(snake)
        .collect()
}

/// The string list of the `ComboBox { id: blendMode ... model: [...] }`.
fn ui_blend_modes() -> Vec<String> {
    let start = MAIN_QML.find("id: blendMode").expect("blend-mode combo");
    let rest = &MAIN_QML[start..];
    let open = rest.find("model: [").expect("blend-mode model") + "model: [".len();
    let close = rest[open..].find(']').expect("end of model") + open;
    rest[open..close]
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn the_engine_enum_reader_sees_every_variant() {
    // Guard on the reader itself: if it silently returned too few, the comparison below would pass
    // on a shrinking set. 43 is the count at the time of writing; a NEW variant raises it and must
    // update this number deliberately, together with the UI.
    let engine = engine_blend_modes();
    assert_eq!(engine.len(), 43, "read {engine:?}");
    for name in ["normal", "hsv_hue", "pass_through", "merge", "split"] {
        assert!(engine.contains(name), "{name} missing from {engine:?}");
    }
}

#[test]
fn every_engine_blend_mode_is_selectable_in_the_ui() {
    let ui: BTreeSet<String> = ui_blend_modes().into_iter().collect();
    let hidden: Vec<_> = engine_blend_modes().difference(&ui).cloned().collect();
    assert!(
        hidden.is_empty(),
        "engine blend modes the UI hides: {hidden:?}"
    );
}

#[test]
fn the_bridge_accepts_exactly_what_the_engine_declares() {
    // The combo is not the only gate: `EditorBridge::setLayerBlendMode` keeps its own allow-list and
    // refuses anything else with "Unknown blend mode". When the combo first gained `merge`/`split`
    // the GUI showed them and clicking "Set blend mode" silently did nothing -- the bridge list was
    // still 41 long. Only driving the real app found it; this pins it.
    let start = EDITOR_BRIDGE_CPP
        .find("void EditorBridge::setLayerBlendMode")
        .expect("setLayerBlendMode");
    let body = &EDITOR_BRIDGE_CPP[start..];
    let end = body.find("};").expect("end of allow-list");
    let bridge: BTreeSet<String> = body[..end]
        .split("QStringLiteral(\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .map(str::to_string)
        .collect();
    assert_eq!(
        bridge,
        engine_blend_modes(),
        "bridge allow-list differs from the engine"
    );
}

#[test]
fn every_ui_blend_mode_is_one_the_engine_accepts() {
    let ui = ui_blend_modes();
    let unique: BTreeSet<_> = ui.iter().collect();
    assert_eq!(
        unique.len(),
        ui.len(),
        "duplicate entries in the combo: {ui:?}"
    );
    for name in &ui {
        let parsed: Result<BlendMode, _> = serde_json::from_str(&format!("\"{name}\""));
        assert!(
            parsed.is_ok(),
            "UI offers {name:?}, which the engine rejects"
        );
    }
}

const QT_CMAKE: &str = include_str!("../../../native/qt/CMakeLists.txt");

/// `(toolId, iconName, shortcut)` of every `ToolRailButton { ... }` instance in the rail.
fn rail_tools() -> Vec<(String, String, String)> {
    fn field(block: &str, key: &str) -> String {
        block
            .find(&format!("{key}: \""))
            .map(|i| {
                let rest = &block[i + key.len() + 3..];
                rest[..rest.find('"').unwrap()].to_string()
            })
            .unwrap_or_default()
    }
    MAIN_QML
        .split("ToolRailButton {")
        .skip(1)
        .map(|chunk| {
            let block = &chunk[..chunk.find('}').unwrap_or(chunk.len())];
            (
                field(block, "toolId"),
                field(block, "iconName"),
                field(block, "shortcut"),
            )
        })
        .filter(|(id, _, _)| !id.is_empty())
        .collect()
}

#[test]
fn hand_and_zoom_tools_move_and_scale_the_view() {
    // The view could only zoom (wheel, buttons) around the centre; it could not move at all.
    let hand = qml_block("DragHandler {\n                        objectName: \"handDrag\"");
    assert!(hand.contains("activeTool === \"hand\"") && hand.contains("canvas.pan ="));
    let zoom = qml_block("TapHandler {\n                        objectName: \"zoomTap\"");
    assert!(zoom.contains("activeTool === \"zoom\"") && zoom.contains("Qt.AltModifier"));
    // The canvas pointer must ignore both, or a hand drag would also paint.
    let pointer = qml_block("PointHandler {\n                        id: canvasPointer");
    assert!(pointer.contains("window.activeTool === \"hand\" || window.activeTool === \"zoom\""));
    // The pan is part of the view transform, so every overlay and every hit test follow it.
    let canvas_cpp = include_str!("../../../native/qt/CanvasItem.cpp");
    let view = &canvas_cpp[canvas_cpp
        .find("QTransform CanvasItem::viewTransform()")
        .unwrap()..];
    let view = &view[..view.find("\n}\n").unwrap()];
    assert!(view.contains("m_pan.x()") && view.contains("m_pan.y()"));
}

#[test]
fn the_view_rotates_and_mirrors_through_one_transform() {
    // Rotation and mirroring only make sense if painting, overlays and hit tests share one
    // mapping; a single overlay still using its own translate+scale draws in the wrong place.
    let canvas_cpp = include_str!("../../../native/qt/CanvasItem.cpp");
    let view = &canvas_cpp[canvas_cpp
        .find("QTransform CanvasItem::viewTransform()")
        .unwrap()..];
    let view = &view[..view.find("\n}\n").unwrap()];
    assert!(view.contains("rotate(m_rotation)"));
    assert!(view.contains("m_mirrored ? -m_zoom : m_zoom"));
    assert!(
        !canvas_cpp.contains("translate(target.topLeft())"),
        "an overlay maps with its own translate+scale instead of viewTransform()"
    );
    let point = &canvas_cpp[canvas_cpp.find("QPointF CanvasItem::canvasPoint(").unwrap()..];
    let point = &point[..point.find("\n}\n").unwrap()];
    assert!(point.contains("viewTransform().inverted("));
    let contains = &canvas_cpp[canvas_cpp
        .find("bool CanvasItem::containsCanvasPoint(")
        .unwrap()..];
    let contains = &contains[..contains.find("\n}\n").unwrap()];
    assert!(
        contains.contains("canvasPoint(itemPoint)"),
        "hit test ignores rotation"
    );
    for key in ["4", "6", "5"] {
        assert!(
            MAIN_QML.contains(&format!("Shortcut {{ sequence: \"{key}\"")),
            "no view key {key}"
        );
    }
}

#[test]
fn grouped_tools_share_a_cell_and_set_their_brush_mode() {
    // The rail was full at 16 rows; Photoshop-style groups put related tools behind one cell.
    let mut groups: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for chunk in MAIN_QML.split("ToolRailButton {").skip(1) {
        let block = &chunk[..chunk
            .find("\n                        }")
            .unwrap_or(chunk.len())];
        let field = |key: &str| {
            block.find(&format!("{key}: \"")).map(|i| {
                let rest = &block[i + key.len() + 3..];
                rest[..rest.find('"').unwrap()].to_string()
            })
        };
        if let (Some(id), Some(group)) = (field("toolId"), field("group")) {
            groups.entry(group).or_default().push(id);
        }
    }
    let expected: std::collections::BTreeMap<String, Vec<String>> = [
        ("paint", vec!["brush", "lazybrush"]),
        ("stamp", vec!["clone", "heal"]),
        ("fill", vec!["gradient", "fill", "enclose"]),
        ("focus", vec!["blur", "sharpen", "smudge"]),
        ("tone", vec!["dodge", "burn"]),
    ]
    .into_iter()
    .map(|(g, ids)| (g.to_string(), ids.into_iter().map(String::from).collect()))
    .collect();
    assert_eq!(groups, expected);
    // Every group starts on its first tool; a group with no start shows no cell at all.
    let order = qml_block("readonly property var toolGroupOrder: ({");
    for (group, ids) in &expected {
        let quoted: Vec<String> = ids.iter().map(|id| format!("\"{id}\"")).collect();
        assert!(
            order.contains(&format!("{group}: [{}]", quoted.join(", "))),
            "menu order for {group} is not the rail's"
        );
    }
    let start = MAIN_QML
        .lines()
        .find(|l| l.contains("property var groupCurrent:"))
        .expect("groupCurrent");
    for (group, ids) in &expected {
        assert!(
            start.contains(&format!("{group}: \"{}\"", ids[0])),
            "group {group} does not start on {}",
            ids[0]
        );
    }

    let like_start = MAIN_QML.find("readonly property bool brushLike").unwrap();
    let like = &MAIN_QML[like_start..like_start + MAIN_QML[like_start..].find("indexOf").unwrap()];
    let sync = qml_block("onActiveToolChanged: {");
    for (tool, line) in [
        ("heal", "editor.brushHeal = activeTool === \"heal\""),
        (
            "blur",
            "activeTool === \"blur\" || activeTool === \"sharpen\" ? activeTool",
        ),
        ("sharpen", "editor.brushConvolveMode ="),
        (
            "dodge",
            "activeTool === \"dodge\" || activeTool === \"burn\" ? activeTool",
        ),
        ("burn", "editor.brushDodgeBurnMode ="),
    ] {
        assert!(
            like.contains(&format!("\"{tool}\"")),
            "{tool} does not paint"
        );
        assert!(sync.contains(line), "{tool}: missing `{line}`");
    }
    // Healing copies from the clone source, so it must switch clone mode on too.
    assert!(
        sync.contains("editor.brushClone = activeTool === \"clone\" || activeTool === \"heal\"")
    );
}

#[test]
fn eraser_clone_and_smudge_are_brush_modes_picked_as_tools() {
    // They were only checkboxes in the brush options. As rail tools each one must paint like the
    // brush (be in brushLike) and switch the engine mode it stands for.
    let like = MAIN_QML
        .lines()
        .find(|l| l.contains("readonly property bool brushLike"))
        .expect("brushLike");
    let sync = qml_block("onActiveToolChanged: {");
    for (tool, flag) in [
        ("eraser", "brushErase"),
        ("clone", "brushClone"),
        ("smudge", "brushSmudge"),
    ] {
        assert!(
            like.contains(&format!("\"{tool}\"")),
            "{tool} does not paint"
        );
        assert!(
            sync.contains(&format!("editor.{flag} = activeTool === \"{tool}\"")),
            "{tool} does not set {flag}"
        );
    }
    // Painting code asks brushLike, not for the brush by name, or the new tools would not paint.
    assert!(!MAIN_QML.contains("activeTool === \"brush\""));
    assert!(!MAIN_QML.contains("activeTool !== \"brush\""));
}

#[test]
fn the_tool_rail_is_two_columns_in_photoshop_order() {
    // Photoshop's toolbar order, grouped: move and select, measure, paint, draw and type, then
    // our distort tools (Photoshop keeps those under Edit), then view. A new tool must be placed
    // in a group here on purpose, not appended to the end.
    let groups: [&[&str]; 6] = [
        &[
            "transform",
            "align",
            "rectangle",
            "ellipse",
            "lasso",
            "polygon",
            "scissors",
            "fgselect",
            "wand",
            "crop",
        ],
        &["picker", "measure"],
        &[
            "brush",
            "lazybrush",
            "clone",
            "heal",
            "eraser",
            "gradient",
            "fill",
            "enclose",
            "blur",
            "sharpen",
            "smudge",
            "dodge",
            "burn",
        ],
        &["pen", "text", "shape"],
        &["perspective", "cage", "warp", "npoint"],
        &["inspect", "hand", "zoom"],
    ];
    let expected: Vec<&str> = groups.iter().flat_map(|g| g.iter().copied()).collect();
    let actual: Vec<String> = rail_tools().into_iter().map(|t| t.0).collect();
    assert_eq!(actual, expected, "tool rail order");

    let start = MAIN_QML
        .find("GridLayout {\n                        width: 88")
        .expect("rail grid");
    let rail = &MAIN_QML[start
        ..start
            + MAIN_QML[start..]
                .find("objectName: \"brushColorSwatch\"")
                .unwrap()];
    assert!(rail.contains("columns: 2"), "the rail is not two columns");
    // One spanning divider between each pair of groups.
    assert_eq!(
        rail.matches("RailDivider { Layout.columnSpan: 2").count(),
        groups.len() - 1,
        "group dividers"
    );
}

/// Icon names the Qt build embeds under `icons/ui/`, from both `set(...)` lists.
fn embedded_icons() -> BTreeSet<String> {
    ["set(REDROB_TOOL_ICONS", "set(REDROB_LOCAL_TOOL_ICONS"]
        .iter()
        .flat_map(|head| {
            let start = QT_CMAKE.find(head).expect(head) + head.len();
            let end = QT_CMAKE[start..].find(')').unwrap() + start;
            QT_CMAKE[start..end]
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_rail_reader_sees_the_known_tools() {
    let ids: BTreeSet<_> = rail_tools().into_iter().map(|t| t.0).collect();
    assert!(ids.len() >= 25, "read only {ids:?}");
    for id in ["brush", "shape", "picker", "pen", "measure", "align"] {
        assert!(ids.contains(id), "{id} missing from {ids:?}");
    }
}

#[test]
fn the_rail_has_a_text_tool_that_the_canvas_handles() {
    // The engine has had text nodes (addTextNode) since before this test; the only way to make one
    // was a layer-menu item that placed it at a fixed (24, 24). UI-2 adds the tool. Pin both halves:
    // the button, and the canvas branch that turns a click into a placed node.
    let text = rail_tools().into_iter().find(|t| t.0 == "text");
    assert!(text.is_some(), "no text tool in the rail");
    assert!(
        MAIN_QML.contains("window.activeTool === \"text\""),
        "the canvas has no branch for the text tool"
    );
    assert!(
        MAIN_QML.contains("textSemanticDialog.openNew(startCanvas.x, startCanvas.y)"),
        "the text tool does not place the node at the click"
    );
}

#[test]
fn the_filter_browser_can_add_an_adjustment_layer() {
    // P11. The button and the call it makes; the bridge method itself is pinned by the test that
    // checks every QML editor.* call against EditorBridge.h.
    assert!(FILTER_BROWSER_QML.contains("objectName: \"filterAddAdjustment\""), "no adjustment button");
    assert!(
        FILTER_BROWSER_QML.contains("editor.addAdjustmentNode(selectedKind, params)"),
        "the adjustment button does not send the selected filter"
    );
}

#[test]
fn a_running_filter_can_be_cancelled_from_the_browser() {
    // P8b. The button shows only while a filter runs, and the bridge call it makes uses the one
    // engine entry that does not wait for the lock the running filter holds.
    assert!(FILTER_BROWSER_QML.contains("objectName: \"filterCancel\""), "no cancel button");
    assert!(FILTER_BROWSER_QML.contains("onClicked: editor.cancelFilter()"));
    assert!(FILTER_BROWSER_QML.contains("visible: editor.filterBusy"));
    let cancel = bridge_fn("cancelFilter");
    assert!(cancel.contains("redrob_editor_request_cancel(m_editor.get())"));
    assert!(
        !cancel.contains("redrob_editor_execute_json"),
        "cancel must not queue behind the running filter"
    );
    assert!(
        bridge_fn("finishFilterRun").contains("QStringLiteral(\"cancelled\")"),
        "a cancel would be reported as a rejected edit"
    );
}

#[test]
fn the_filter_browser_lives_in_its_own_file_and_is_shipped() {
    // P12. Moved out of Main.qml. A QML file the resource list leaves out loads nothing and fails
    // only at runtime ("FilterBrowser is not a type"), so the embedding and the lint are pinned.
    assert!(
        MAIN_QML.contains("FilterBrowser {\n        id: filterBrowser\n        tokens: window.tokens"),
        "Main.qml does not instantiate the browser"
    );
    assert!(
        !MAIN_QML.contains("objectName: \"filterApply\""),
        "the browser body is still duplicated in Main.qml"
    );
    assert!(
        !FILTER_BROWSER_QML.contains("window."),
        "the browser reaches into Main.qml's window id; pass it as a property"
    );
    let cmake = include_str!("../../../native/qt/CMakeLists.txt");
    assert!(cmake.contains("QT_RESOURCE_ALIAS \"qml/FilterBrowser.qml\""), "not aliased beside Main.qml");
    assert!(
        cmake.matches("\"${REDROB_ROOT}/qml/FilterBrowser.qml\"").count() >= 3,
        "FilterBrowser.qml must be aliased, embedded and linted"
    );
    // applyFilterParams and addAdjustmentNode; the rest of its editor.* uses are properties.
    assert!(assert_bridge_calls_exist(FILTER_BROWSER_QML, "filter browser") >= 2);
}

#[test]
fn the_text_dialog_sends_paragraph_width_and_alignment_on_both_paths() {
    // P10. setTextContent replaces the whole text content, so an edit that forgot the paragraph
    // fields would silently turn paragraph text back into point text. Pin both calls.
    assert!(TEXT_DIALOG_QML.contains("objectName: \"textBoxWidthInput\""), "no box width field");
    assert!(TEXT_DIALOG_QML.contains("objectName: \"textAlignCombo\""), "no alignment control");
    assert!(
        TEXT_DIALOG_QML.contains("sourceFontFamily, sourceFontId, boxWidth, textAlign.currentText)"),
        "editing text drops the paragraph fields"
    );
    assert!(
        TEXT_DIALOG_QML.contains("\"\", -1, boxWidth, textAlign.currentText)"),
        "adding text drops the paragraph fields"
    );
    assert!(
        TEXT_DIALOG_QML.contains("textBoxWidth.text = boxWidth > 0"),
        "the edit dialog does not load the node's current box width"
    );
    // P12: the layer menu hands the node's paragraph fields to the dialog's openEdit.
    assert!(
        MAIN_QML.contains("semanticFontFamily, semanticBoxWidth, semanticAlign)"),
        "the layer menu does not pass the paragraph fields to the text dialog"
    );
}

#[test]
fn the_node_dialogs_live_in_their_own_files_and_main_does_not_reach_inside() {
    // P12. Main.qml used to set the dialogs' fields by id. From another file that silently fails
    // at runtime ("textX is not defined"), so pin that no inner id is used from Main.qml.
    for inner in [
        "semanticText.", "textName.", "textX.", "textY.", "textSize.", "textColor.",
        "textBoxWidth.", "textAlign.", "vectorName.", "vectorX.", "vectorY.", "vectorW.",
        "vectorH.", "vectorFill.", "vectorStrokeColor.", "vectorStroke.",
    ] {
        let used = MAIN_QML.lines().any(|line| {
            line.find(inner)
                .is_some_and(|i| i == 0 || !line.as_bytes()[i - 1].is_ascii_alphanumeric())
        });
        assert!(!used, "Main.qml reaches into a node dialog through {inner}");
    }
    assert!(MAIN_QML.contains("TextNodeDialog {\n                                id: textSemanticDialog"));
    assert!(MAIN_QML.contains("VectorRectDialog {\n                                id: vectorSemanticDialog"));
    assert!(VECTOR_DIALOG_QML.contains("readonly property string fillColor: vectorFill.text"));
    assert!(assert_bridge_calls_exist(TEXT_DIALOG_QML, "text dialog") >= 2);
    assert!(assert_bridge_calls_exist(VECTOR_DIALOG_QML, "vector dialog") >= 2);
    let cmake = include_str!("../../../native/qt/CMakeLists.txt");
    for file in ["TextNodeDialog.qml", "VectorRectDialog.qml"] {
        assert!(
            cmake.matches(&format!("\"${{REDROB_ROOT}}/qml/{file}\"")).count() >= 3,
            "{file} must be aliased, embedded and linted"
        );
    }
}

#[test]
fn every_rail_icon_is_embedded() {
    // A missing icon does not fail the build or the QML load; the button just renders blank.
    let icons = embedded_icons();
    for (id, icon, _) in rail_tools() {
        assert!(
            icons.contains(&icon),
            "tool {id} names icon {icon:?}, which is not embedded"
        );
    }
}

#[test]
fn rail_shortcuts_are_unique() {
    let mut seen = BTreeSet::new();
    for (id, _, key) in rail_tools() {
        if !key.is_empty() {
            assert!(seen.insert(key.clone()), "shortcut {key} reused by {id}");
        }
    }
}

const ABI_RS: &str = include_str!("../../redrob-ffi/src/abi.rs");

#[test]
fn the_history_panel_is_wired_end_to_end() {
    // Four hops from the engine's labels to a clickable row; a break at any one leaves the panel
    // showing "Step N" or rows that do nothing, and nothing else fails.
    for key in [
        "\"undo_labels\": editor.undo_labels()",
        "\"redo_labels\": editor.redo_labels()",
    ] {
        assert!(ABI_RS.contains(key), "FFI document state lacks {key}");
    }
    for key in [
        "QStringLiteral(\"undo_labels\")",
        "QStringLiteral(\"redo_labels\")",
    ] {
        assert!(
            EDITOR_BRIDGE_CPP.contains(key),
            "bridge does not read {key}"
        );
    }
    assert!(
        MAIN_QML.contains("editor.historyLabels[index - 1]"),
        "rows do not show labels"
    );
    assert!(
        MAIN_QML.contains("onClicked: editor.jumpToHistory(index)"),
        "rows are not clickable"
    );
}

const EDITOR_BRIDGE_H: &str = include_str!("../../../native/qt/EditorBridge.h");

#[test]
fn every_open_dialog_suffix_reaches_the_engine() {
    // The engine read PSD/KRA/XCF/TIFF, but the dialog, the bridge's suffix map and the FFI's
    // format names each listed only six. Every suffix the Open dialog offers must be one the
    // bridge maps, or picking that file fails with "unknown file extension".
    let dialog = &MAIN_QML[MAIN_QML.find("id: openProjectDialog").expect("open dialog")..];
    let filter = &dialog[dialog.find("All supported (").unwrap() + "All supported (".len()..];
    let filter = &filter[..filter.find(')').unwrap()];
    let map = &EDITOR_BRIDGE_CPP[EDITOR_BRIDGE_CPP
        .find("QString canonicalFormatForSuffix")
        .unwrap()..];
    let map = &map[..map.find("\n}\n").unwrap()];
    let suffixes: Vec<&str> = filter.split_whitespace().map(|s| &s[2..]).collect();
    for wanted in ["psd", "kra", "xcf", "tiff"] {
        assert!(
            suffixes.contains(&wanted),
            "Open dialog does not offer .{wanted}"
        );
    }
    for suffix in suffixes {
        assert!(
            map.contains(&format!("QStringLiteral(\"{suffix}\")")),
            "Open dialog offers .{suffix}, which the bridge does not map"
        );
    }
}

/// The `menuBar: MenuBar { ... }` block, brace-matched.
fn menu_bar_block() -> &'static str {
    qml_block("menuBar: MenuBar {")
}

/// The QML block opened by `marker` (which ends in `{`), brace-matched.
fn qml_block(marker: &str) -> &'static str {
    let start = MAIN_QML
        .find(marker)
        .unwrap_or_else(|| panic!("{marker} not found"));
    let mut depth = 0_i32;
    for (i, c) in MAIN_QML[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &MAIN_QML[start..start + i + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated {marker}");
}

/// Every `editor.<name>(` call in `block` must be a Q_INVOKABLE; returns how many were checked.
fn assert_bridge_calls_exist(block: &str, what: &str) -> usize {
    let mut checked = 0;
    for piece in block.split("editor.").skip(1) {
        let name: String = piece
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if !piece[name.len()..].starts_with('(') {
            continue;
        }
        let declared = EDITOR_BRIDGE_H
            .lines()
            .any(|l| l.contains("Q_INVOKABLE") && l.contains(&format!(" {name}(")));
        assert!(
            declared,
            "{what} calls editor.{name}(), which the bridge does not declare"
        );
        checked += 1;
    }
    checked
}

#[test]
fn the_canvas_has_a_right_click_menu() {
    // The canvas took only the left button, so a right-click did nothing there. The menu opens
    // from a right-button TapHandler on the canvas, and every item calls a real bridge method.
    let canvas = qml_block("CanvasItem {\n                    id: canvas");
    let tap = qml_block("TapHandler {\n                        objectName: \"canvasContextTap\"");
    assert!(
        canvas.contains(tap),
        "the right-click handler is not on the canvas"
    );
    assert!(tap.contains("Qt.RightButton") && tap.contains("canvasMenu.popup()"));
    let menu = qml_block("Menu {\n                        id: canvasMenu");
    assert!(
        assert_bridge_calls_exist(menu, "canvas menu") >= 8,
        "canvas menu lost its commands"
    );
    // Drawing stays on the left button.
    assert!(
        qml_block("PointHandler {\n                        id: canvasPointer")
            .contains("acceptedButtons: Qt.LeftButton")
    );
}

#[test]
fn the_menu_bar_has_the_expected_menus() {
    let block = menu_bar_block();
    for title in [
        "&File", "&Edit", "&Select", "&Layer", "&Image", "Filte&rs", "&View",
    ] {
        assert!(
            block.contains(&format!("title: qsTr(\"{title}\")")),
            "no {title} menu"
        );
    }
}

#[test]
fn every_menu_action_calls_a_real_bridge_method() {
    // A QML call to a method the bridge does not declare fails only at click time, as a console
    // TypeError; the build and the load both pass. Read every `editor.<name>(` in the menu bar and
    // require a Q_INVOKABLE of that name.
    let block = menu_bar_block();
    let mut checked = 0;
    for piece in block.split("editor.").skip(1) {
        let name: String = piece
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if !piece[name.len()..].starts_with('(') {
            continue; // a property read such as editor.canUndo
        }
        let declared = EDITOR_BRIDGE_H
            .lines()
            .any(|l| l.contains("Q_INVOKABLE") && l.contains(&format!(" {name}(")));
        assert!(
            declared,
            "menu calls editor.{name}(), which the bridge does not declare"
        );
        checked += 1;
    }
    assert!(
        checked >= 15,
        "read only {checked} bridge calls from the menu bar"
    );
}

#[test]
fn menu_items_bind_no_shortcut_the_window_already_owns() {
    // A key bound by both a window Shortcut and a menu Action is ambiguous to Qt and fires neither.
    assert!(
        !menu_bar_block().contains("shortcut:"),
        "a menu Action binds a shortcut; the window Shortcut objects own them"
    );
}

#[test]
fn filter_defaults_are_the_engines_own_and_round_trip() {
    // UI-1 builds its filter form from these, so each must be what applying `{"kind": k}` really
    // does: the kind it was asked for, and a value the engine accepts back unchanged.
    let kinds = redrob_core::filter_wire_tags();
    let mut with = 0;
    for kind in kinds {
        let Some(defaults) = redrob_core::filter_defaults(kind) else {
            continue;
        };
        with += 1;
        assert_eq!(
            defaults["kind"], *kind,
            "defaults for {kind} name another kind"
        );
        let back: redrob_core::Filter = serde_json::from_value(defaults.clone())
            .unwrap_or_else(|e| panic!("defaults for {kind} do not parse back: {e}"));
        assert_eq!(
            serde_json::to_value(back).unwrap(),
            defaults,
            "{kind} is not stable"
        );
    }
    // Every filter: serde defaults for 58, the required-parameter table for the other 73 (23 the
    // panel never exposed, 50 seeded from their panel's own starting values). No filter is left
    // as "needs parameters" in the browser.
    let missing: Vec<_> = kinds
        .iter()
        .filter(|k| redrob_core::filter_defaults(k).is_none())
        .collect();
    assert!(missing.is_empty(), "no starting parameters: {missing:?}");
    assert_eq!(with, kinds.len());
    assert_eq!(redrob_core::filter_defaults("no_such_filter"), None);
}

#[test]
fn every_filters_starting_parameters_actually_apply() {
    // Parsing is not applying: validation runs on execute, and a starting value outside a range
    // would put an Apply button in the browser that always fails. Run each one.
    let mut applied = 0;
    for kind in redrob_core::filter_wire_tags() {
        let Some(defaults) = redrob_core::filter_defaults(kind) else {
            continue;
        };
        let mut editor =
            redrob_core::Editor::new(redrob_core::Document::new(16, 16).unwrap()).unwrap();
        // Content with a real range: tone mappers refuse a flat image (FilterNoDynamicRange),
        // which is a property of the input, not of the parameters under test.
        editor
            .execute(redrob_core::Command::Fill {
                color: redrob_core::Pixel::rgba(40, 80, 160, 255),
            })
            .unwrap();
        editor
            .execute(redrob_core::Command::BrushStroke {
                points: vec![
                    redrob_core::BrushPoint::new(1.0, 1.0, 1.0),
                    redrob_core::BrushPoint::new(14.0, 14.0, 1.0),
                ],
                color: redrob_core::Pixel::rgba(250, 240, 20, 255),
                size: 4.0,
                opacity: 1.0,
                settings: redrob_core::BrushSettings::default(),
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        let filter: redrob_core::Filter = serde_json::from_value(defaults).unwrap();
        let result = editor.execute(redrob_core::Command::ApplyFilter { filter });
        assert!(
            result.is_ok(),
            "{kind}: starting parameters rejected: {result:?}"
        );
        applied += 1;
    }
    assert_eq!(
        applied,
        redrob_core::filter_wire_tags().len(),
        "every filter's starting parameters must apply"
    );
}

#[test]
fn hidden_filters_are_measured() {
    // A filter is reachable when a typed bridge method sends its `kind`, or when the filter browser
    // can apply it -- which needs a full default set (`filter_defaults`). 78 were unreachable before
    // the browser; it reaches every filter with defaults, leaving those with a required parameter
    // and no typed method. The ceiling can only go down.
    let sent: BTreeSet<String> = EDITOR_BRIDGE_CPP
        .split("QStringLiteral(\"kind\"), QStringLiteral(\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .map(str::to_string)
        .chain(["invert".to_string(), "grayscale".to_string()])
        .collect();
    let unreachable: Vec<_> = redrob_core::filter_wire_tags()
        .iter()
        .filter(|k| !sent.contains(**k) && redrob_core::filter_defaults(k).is_none())
        .collect();
    assert!(
        unreachable.is_empty(),
        "unreachable from the UI: {unreachable:?}"
    );
}

/// The body of one `EditorBridge::<name>(` definition, up to the next column-0 `}`.
fn bridge_fn(name: &str) -> &'static str {
    let head = if name.contains('(') {
        format!("EditorBridge::{name}")
    } else {
        format!("EditorBridge::{name}(")
    };
    let start = EDITOR_BRIDGE_CPP
        .match_indices(&head)
        .map(|(i, _)| i)
        .find(|&i| i == 0 || EDITOR_BRIDGE_CPP.as_bytes()[i - 1] == b' ')
        .unwrap_or_else(|| panic!("{head} not defined"));
    let rest = &EDITOR_BRIDGE_CPP[start..];
    &rest[..rest.find("\n}\n").expect("unterminated function")]
}

#[test]
fn filters_run_off_the_gui_thread() {
    // A filter on a large image took ~10 s on the GUI thread and froze the window. apply_filter
    // now goes to a worker, and while it runs no GUI-thread path may enter the engine: each would
    // block on the engine's mutex and freeze the window all the same.
    let execute = bridge_fn("executeCommand(const QJsonObject &command)");
    let routed = execute
        .find("startFilterRun(json)")
        .expect("apply_filter is not routed to the worker");
    let sync_call = execute
        .find("redrob_editor_execute_json")
        .expect("synchronous path missing");
    assert!(
        routed < sync_call,
        "the worker route must come before the synchronous call"
    );
    assert!(
        bridge_fn("startFilterRun").contains("QtConcurrent::run"),
        "startFilterRun does not leave the GUI thread"
    );
    for name in [
        "executeCommand(const QJsonObject &command)",
        "executeHistoryAction",
        "executeNavigation",
        "refresh",
        "replaceFromGenericBytes",
        "exportGenericBytes",
    ] {
        let body = bridge_fn(name);
        assert!(
            body.contains("m_filterBusy") || body.contains("refuseWhileFilterRuns"),
            "{name} can enter the engine while a filter runs"
        );
    }
    // Every function that calls into the editor handle must be on that list or be the worker.
    let guarded = [
        "executeCommand",
        "executeHistoryAction",
        "executeNavigation",
        "refresh",
        "replaceFromGenericBytes",
        "exportGenericBytes",
        "startFilterRun",
        // P8b: calls only redrob_editor_request_cancel, which takes no engine lock -- it is the
        // one call that is MEANT to run while a filter does.
        "cancelFilter",
        "EditorBridge",
        "proposePrompt",
    ];
    let mut current = "";
    for line in EDITOR_BRIDGE_CPP.lines() {
        if !line.starts_with(' ')
            && let Some(i) = line.find("EditorBridge::")
        {
            let name = &line[i + "EditorBridge::".len()..];
            current = name.split('(').next().unwrap_or("");
        }
        if line.contains("redrob_editor_") && line.contains("m_editor.get()") {
            assert!(
                guarded.contains(&current),
                "{current} calls the engine without the filter-run guard: {line}"
            );
        }
    }
    assert!(
        FILTER_BROWSER_QML.contains("!editor.filterBusy"),
        "Apply stays enabled during a run"
    );
}

#[test]
fn the_filter_browser_is_wired() {
    assert!(
        EDITOR_BRIDGE_CPP.contains("value(QStringLiteral(\"filters\"))"),
        "bridge does not read the catalogue"
    );
    assert!(
        FILTER_BROWSER_QML.contains("model: editor.filterCatalog.filter("),
        "browser list is not the catalogue"
    );
    assert!(
        FILTER_BROWSER_QML.contains("editor.applyFilterParams(selectedKind, params)"),
        "browser cannot apply"
    );
    assert!(
        menu_bar_block().contains("onTriggered: filterBrowser.open()"),
        "no menu route to the browser"
    );
}
