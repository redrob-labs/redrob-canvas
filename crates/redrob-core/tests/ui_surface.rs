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
