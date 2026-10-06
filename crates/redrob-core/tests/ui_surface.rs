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
const MENU_BAR_QML: &str = include_str!("../../../qml/MainMenuBar.qml");
const LAYER_PANEL_QML: &str = include_str!("../../../qml/LayerPanel.qml");
const OPTIONS_PANEL_QML: &str = include_str!("../../../qml/OptionsPanel.qml");
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
    // In the Options tab, which is its own file since P12.
    let start = OPTIONS_PANEL_QML
        .find("id: blendMode")
        .expect("blend-mode combo");
    let rest = &OPTIONS_PANEL_QML[start..];
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
    // S2 gave the digit keys to opacity, as Photoshop does; the view turns with the Rotate View
    // tool (R) and resets from the View menu.
    for marker in [
        "window.rotateView(",
        "onDoubleTapped: canvas.viewRotation = 0",
    ] {
        assert!(MAIN_QML.contains(marker), "view rotation lost: {marker}");
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
        ("paint", vec!["brush", "lazybrush", "mixer"]),
        ("stamp", vec!["clone", "heal"]),
        ("fill", vec!["gradient", "fill", "enclose"]),
        ("focus", vec!["blur", "sharpen", "smudge"]),
        ("tone", vec!["dodge", "burn"]),
        ("view", vec!["hand", "rotateview"]),
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
        ("mixer", "brushMixer"),
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
    for (file, body) in [
        ("Main.qml", MAIN_QML),
        ("OptionsPanel.qml", OPTIONS_PANEL_QML),
    ] {
        assert!(
            !body.contains("activeTool === \"brush\""),
            "{file} tests for the brush by name"
        );
        assert!(
            !body.contains("activeTool !== \"brush\""),
            "{file} tests for the brush by name"
        );
    }
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
            "mixer",
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
        &["inspect", "hand", "rotateview", "zoom"],
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
    assert!(
        FILTER_BROWSER_QML.contains("objectName: \"filterAddAdjustment\""),
        "no adjustment button"
    );
    assert!(
        FILTER_BROWSER_QML.contains("editor.addAdjustmentNode(selectedKind, params)"),
        "the adjustment button does not send the selected filter"
    );
}

#[test]
fn a_running_filter_can_be_cancelled_from_the_browser() {
    // P8b. The button shows only while a filter runs, and the bridge call it makes uses the one
    // engine entry that does not wait for the lock the running filter holds.
    assert!(
        FILTER_BROWSER_QML.contains("objectName: \"filterCancel\""),
        "no cancel button"
    );
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
fn pen_tilt_reaches_the_stroke_and_the_sensor_menus() {
    // P7. Qt Quick's handlers carry no tilt, so the bridge reads it from raw tablet events through
    // an application event filter and attaches it to each stroke point; the three sensor combos
    // offer it. A real mouse resets it, or a mouse stroke would inherit the pen's last lean.
    let main_cpp = include_str!("../../../native/qt/main.cpp");
    assert!(
        main_cpp.contains("application.installEventFilter(&editor);"),
        "no tilt filter"
    );
    let filter = bridge_fn("eventFilter");
    assert!(filter.contains("QEvent::TabletMove") && filter.contains("xTilt()"));
    assert!(
        filter.contains("QInputDevice::DeviceType::Stylus"),
        "a mouse would keep the tilt"
    );
    assert!(
        filter.contains("return QObject::eventFilter(watched, event);"),
        "must not eat events"
    );
    assert!(bridge_fn("addStrokePoint").contains("QStringLiteral(\"tilt_x\")"));
    assert_eq!(
        OPTIONS_PANEL_QML
            .matches("model: [\"off\", \"pressure\", \"speed\", \"random\", \"tilt\"]")
            .count(),
        3,
        "size, opacity and flow each offer the tilt sensor"
    );
    assert!(EDITOR_BRIDGE_CPP.contains("sensor == QStringLiteral(\"tilt\")"));
}

#[test]
fn the_mcp_endpoint_is_loopback_tokened_off_by_default_and_proposal_only() {
    // P13. The runtime checks are in the native smoke (mcpServerIsValid); these pin the shape so
    // a refactor cannot quietly drop one of the guards.
    let server = include_str!("../../../native/qt/McpServer.cpp");
    assert!(
        server.contains("m_server.listen(QHostAddress::LocalHost, 0)"),
        "must bind 127.0.0.1 only"
    );
    assert!(
        !server.contains("QHostAddress::Any"),
        "must never bind every interface"
    );
    assert!(
        server.contains("tokenMatches(authorization.mid("),
        "bearer token not checked"
    );
    assert!(
        server.contains("headers.contains(\"origin\")"),
        "browser origins not refused"
    );
    assert!(
        server.contains("unexpected Host header"),
        "no DNS-rebinding guard"
    );
    assert!(
        server.contains("QRandomGenerator::system()"),
        "token must come from the OS generator"
    );
    // Off by default and never persisted: nothing turns it on at startup.
    let main_cpp = include_str!("../../../native/qt/main.cpp");
    assert!(!main_cpp.contains("setMcpEnabled(true)"));
    assert!(
        !EDITOR_BRIDGE_CPP.contains("setValue(QStringLiteral(\"mcp"),
        "the switch must not be remembered"
    );
    // tools/call only ever queues a proposal; it never executes a command.
    let handler = bridge_fn("handleMcpToolCall");
    assert!(handler.contains("m_proposals.enqueue("));
    assert!(
        !handler.contains("redrob_editor_execute_json") && !handler.contains("executeCommand(")
    );
    assert!(MAIN_QML.contains("objectName: \"mcpEnableSwitch\""));
    assert!(MAIN_QML.contains("onToggled: editor.mcpEnabled = checked"));
}

#[test]
fn actions_record_successful_edits_and_play_back_through_the_engine() {
    // P14. Recording hooks the two places an edit succeeds -- the synchronous command path and the
    // filter worker's finish -- so a rejected or cancelled edit is never a step. Playback goes to
    // the engine as one call, which makes it one undo step and all-or-nothing.
    let execute = bridge_fn("executeCommand(const QJsonObject &command)");
    let rejected = execute.find("Edit rejected").unwrap();
    let recorded = execute
        .find("recordActionStep(command)")
        .expect("commands are not recorded");
    assert!(
        recorded > rejected,
        "a step must be recorded only after the engine accepted it"
    );
    let finish = bridge_fn("finishFilterRun");
    assert!(
        finish
            .find("recordActionStep(m_pendingFilterCommand)")
            .unwrap()
            > finish.find("Filter cancelled").unwrap(),
        "a cancelled filter must not be recorded"
    );
    assert!(bridge_fn("playActionFile").contains("redrob_editor_play_action_json("));
    // The bridge writes the envelope the engine reads.
    assert!(bridge_fn("saveAction").contains(&format!(
        "{{QStringLiteral(\"format\"), QStringLiteral(\"{}\")}}",
        redrob_core::ACTION_FORMAT
    )));
    let menu = menu_bar_block();
    for call in [
        "editor.startActionRecording()",
        "editor.stopActionRecording()",
        "root.actionSaveDialog.open()",
        "root.actionPlayDialog.open()",
    ] {
        assert!(menu.contains(call), "the Actions menu lost {call}");
    }
}

#[test]
fn strokes_paint_while_the_pointer_is_down() {
    // S1. Measured before this: dragging changed 0 pixels on screen; the stroke appeared on
    // release. Now each move is queued, a 16 ms timer paints the queue through the engine's live
    // stroke and redraws, and release commits through the same engine stroke.
    let begin = bridge_fn("beginStroke");
    assert!(
        begin.contains("redrob_editor_live_stroke_begin("),
        "the stroke does not start live"
    );
    let add = bridge_fn("addStrokePoint");
    assert!(
        add.contains("m_livePending.append(point)") && add.contains("m_liveStrokeTimer.start()")
    );
    let flush = bridge_fn("flushLiveStroke");
    assert!(
        flush.contains("redrob_editor_live_stroke_extend(")
            && flush.contains("refreshLiveRender()")
    );
    let end = bridge_fn("endStroke");
    let flushed = end
        .find("flushLiveStroke()")
        .expect("queued points are dropped on release");
    let committed = end
        .find("redrob_editor_live_stroke_end(")
        .expect("live stroke not committed");
    assert!(
        flushed < committed,
        "the last moves must be painted before the commit"
    );
    assert!(
        end.contains("recordActionStep(command)"),
        "a live stroke is missing from actions"
    );
    // Falls back to the old commit when the engine cannot draw live.
    assert!(end.contains("executeCommand(command)"));
    assert!(bridge_fn("cancelStroke").contains("redrob_editor_live_stroke_cancel("));
    assert!(EDITOR_BRIDGE_CPP.contains("m_liveStrokeTimer.setInterval(16);"));
}

#[test]
fn the_filter_browser_lives_in_its_own_file_and_is_shipped() {
    // P12. Moved out of Main.qml. A QML file the resource list leaves out loads nothing and fails
    // only at runtime ("FilterBrowser is not a type"), so the embedding and the lint are pinned.
    assert!(
        MAIN_QML
            .contains("FilterBrowser {\n        id: filterBrowser\n        tokens: window.tokens"),
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
    // Shipping is checked for every split file at once, by every_split_qml_file_is_shipped.
    // applyFilterParams and addAdjustmentNode; the rest of its editor.* uses are properties.
    assert!(assert_bridge_calls_exist(FILTER_BROWSER_QML, "filter browser") >= 2);
}

#[test]
fn every_split_qml_file_is_shipped() {
    // P12. A QML file the resource list leaves out fails only at runtime ("X is not a type").
    // Every file in qml/ besides the three that predate the split must be in REDROB_SPLIT_QML,
    // and that list must feed the alias, the resources and qmllint.
    let cmake = include_str!("../../../native/qt/CMakeLists.txt");
    let list_line = cmake
        .lines()
        .find(|l| l.starts_with("set(REDROB_SPLIT_QML "))
        .expect("REDROB_SPLIT_QML list");
    let listed: BTreeSet<&str> = list_line
        .trim_start_matches("set(REDROB_SPLIT_QML ")
        .trim_end_matches(')')
        .split_whitespace()
        .collect();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qml");
    for entry in std::fs::read_dir(&dir).expect("qml dir") {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let Some(stem) = name.strip_suffix(".qml") else {
            continue;
        };
        if ["Main", "RedrobTokens", "ColorWheel"].contains(&stem) {
            continue;
        }
        assert!(
            listed.contains(stem),
            "qml/{name} is not in REDROB_SPLIT_QML"
        );
    }
    assert!(
        cmake.contains("QT_RESOURCE_ALIAS \"qml/${panel}.qml\""),
        "split files are not aliased"
    );
    assert!(
        cmake.matches("${REDROB_SPLIT_QML_FILES}").count() >= 2,
        "the split files must be both embedded and linted"
    );
}

#[test]
fn the_text_dialog_sends_paragraph_width_and_alignment_on_both_paths() {
    // P10. setTextContent replaces the whole text content, so an edit that forgot the paragraph
    // fields would silently turn paragraph text back into point text. Pin both calls.
    assert!(
        TEXT_DIALOG_QML.contains("objectName: \"textBoxWidthInput\""),
        "no box width field"
    );
    assert!(
        TEXT_DIALOG_QML.contains("objectName: \"textAlignCombo\""),
        "no alignment control"
    );
    assert!(
        TEXT_DIALOG_QML
            .contains("sourceFontFamily, sourceFontId, boxWidth, textAlign.currentText)"),
        "editing text drops the paragraph fields"
    );
    assert!(
        TEXT_DIALOG_QML
            .contains("\"\", -1, boxWidth, textAlign.currentText, sourceFontFamily, sourceFontId)"),
        "adding text drops the paragraph fields"
    );
    assert!(
        TEXT_DIALOG_QML.contains("textBoxWidth.text = boxWidth > 0"),
        "the edit dialog does not load the node's current box width"
    );
    // P12: the layer menu hands the node's paragraph fields to the dialog's openEdit.
    assert!(
        LAYER_PANEL_QML.contains("semanticFontFamily, semanticBoxWidth, semanticAlign)"),
        "the layer menu does not pass the paragraph fields to the text dialog"
    );
}

#[test]
fn the_node_dialogs_live_in_their_own_files_and_main_does_not_reach_inside() {
    // P12. Main.qml used to set the dialogs' fields by id. From another file that silently fails
    // at runtime ("textX is not defined"), so pin that no inner id is used from Main.qml.
    for inner in [
        "semanticText.",
        "textName.",
        "textX.",
        "textY.",
        "textSize.",
        "textColor.",
        "textBoxWidth.",
        "textAlign.",
        "vectorName.",
        "vectorX.",
        "vectorY.",
        "vectorW.",
        "vectorH.",
        "vectorFill.",
        "vectorStrokeColor.",
        "vectorStroke.",
    ] {
        let used = MAIN_QML.lines().any(|line| {
            line.find(inner)
                .is_some_and(|i| i == 0 || !line.as_bytes()[i - 1].is_ascii_alphanumeric())
        });
        assert!(!used, "Main.qml reaches into a node dialog through {inner}");
    }
    // At window level since the layer panel moved out: the text tool and the menu open them too.
    assert!(MAIN_QML.contains("    TextNodeDialog {\n        id: textSemanticDialog"));
    assert!(MAIN_QML.contains("    VectorRectDialog {\n        id: vectorSemanticDialog"));
    assert!(VECTOR_DIALOG_QML.contains("readonly property string fillColor: vectorFill.text"));
    assert!(assert_bridge_calls_exist(TEXT_DIALOG_QML, "text dialog") >= 2);
    assert!(assert_bridge_calls_exist(VECTOR_DIALOG_QML, "vector dialog") >= 2);
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

/// The `psToolKeys` table in Main.qml: `(key, [toolId...])`, in file order (S2).
fn ps_tool_keys() -> Vec<(String, Vec<String>)> {
    let start = MAIN_QML
        .find("readonly property var psToolKeys: ({")
        .expect("psToolKeys");
    let body = &MAIN_QML[start..start + MAIN_QML[start..].find("})").unwrap()];
    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (key, tools) = line.split_once(": [")?;
            let key = key.trim_matches('"').to_string();
            let tools = tools
                .trim_end_matches(',')
                .trim_end_matches(']')
                .split(',')
                .map(|t| t.trim().trim_matches('"').to_string())
                .collect();
            Some((key, tools))
        })
        .collect()
}

/// Every key sequence bound by a window Shortcut in Main.qml (string literals only).
fn main_sequences() -> Vec<String> {
    let mut out = Vec::new();
    for line in MAIN_QML.lines() {
        let line = line.trim();
        let rest = if let Some(i) = line.find("sequence: \"") {
            &line[i + "sequence: ".len()..]
        } else if let Some(i) = line.find("sequences: [") {
            &line[i + "sequences: [".len()..]
        } else {
            continue;
        };
        // A Repeater's template sequence ("Shift+" + modelData) is not a literal key.
        if rest.contains("modelData") {
            continue;
        }
        let single = line.contains("sequence: \"");
        for piece in rest
            .split('"')
            .skip(1)
            .step_by(2)
            .take(if single { 1 } else { usize::MAX })
        {
            out.push(piece.to_string());
        }
    }
    out
}

#[test]
fn tool_keys_are_photoshops() {
    // S2. The user asked for Photoshop as the reference. The letters are Photoshop's; within a
    // letter the order is Photoshop's group order, and Shift+letter steps through it.
    let keys = ps_tool_keys();
    let first = |key: &str| {
        keys.iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("no {key} key"))
            .1[0]
            .clone()
    };
    for (key, tool) in [
        ("V", "transform"),
        ("M", "rectangle"),
        ("L", "lasso"),
        ("W", "wand"),
        ("C", "crop"),
        ("I", "picker"),
        ("J", "heal"),
        ("B", "brush"),
        ("S", "clone"),
        ("E", "eraser"),
        ("G", "gradient"),
        ("O", "dodge"),
        ("P", "pen"),
        ("T", "text"),
        ("U", "shape"),
        ("H", "hand"),
        ("Z", "zoom"),
        ("R", "rotateview"),
    ] {
        assert_eq!(first(key), tool, "{key} must pick {tool}, as in Photoshop");
    }
    // Every tool named exists on the rail, and none answers to two letters.
    let rail: BTreeSet<String> = rail_tools().into_iter().map(|t| t.0).collect();
    let mut seen = BTreeSet::new();
    for (key, tools) in &keys {
        for tool in tools {
            assert!(
                rail.contains(tool),
                "{key} names {tool}, which is not a rail tool"
            );
            assert!(seen.insert(tool.clone()), "{tool} answers to two keys");
        }
    }
    // The old per-button keys are gone, or they would fight the table.
    assert!(
        !MAIN_QML.contains("shortcut: \""),
        "a rail button still binds its own key"
    );
}

#[test]
fn ctrl_j_duplicates_the_active_layer() {
    // Batch 4 H1.
    assert!(bridge_fn("duplicateLayer").contains("\"duplicate_layer\""));
    let at = MAIN_QML.find("sequence: \"Ctrl+J\"").expect("Ctrl+J");
    assert!(MAIN_QML[at..at + 200].contains("editor.duplicateLayer(editor.activeLayerId)"));
    assert!(menu_bar_block().contains("editor.duplicateLayer(editor.activeLayerId)"));
}

#[test]
fn ctrl_e_merges_down() {
    // Batch 4 H2.
    assert!(bridge_fn("mergeDown").contains("\"merge_down\""));
    let at = MAIN_QML.find("sequence: \"Ctrl+E\"").expect("Ctrl+E");
    assert!(MAIN_QML[at..at + 200].contains("editor.mergeDown(editor.activeLayerId)"));
    assert!(menu_bar_block().contains("editor.mergeDown(editor.activeLayerId)"));
}

#[test]
fn slow_renders_move_to_a_worker() {
    // Batch 4 L11.
    let start = bridge_fn("startAsyncRender");
    assert!(
        start.contains("QtConcurrent::run")
            && start.contains("redrob_editor_render_rgba_detached(editor.get()")
    );
    let refresh = bridge_fn("refresh");
    assert!(
        refresh.contains("if (!detachedRender)\n            redrob_buffer_free(render.rgba);"),
        "the borrowed picture is never freed"
    );
    assert!(refresh.contains("renderClock.elapsed() > kAsyncRenderMs"));
    assert!(
        bridge_fn("finishAsyncRender").contains("result.generation >= m_renderGeneration"),
        "an older frame never replaces a newer one"
    );
}

#[test]
fn artboards_are_reachable() {
    // Batch 4 L8.
    assert!(bridge_fn("newArtboard").contains("set_artboard"));
    assert!(bridge_fn("exportArtboards").contains("m_layers.artboards()"));
    assert!(menu_bar_block().contains("editor.newArtboard("));
    assert!(menu_bar_block().contains("root.artboardExport.open()"));
}

#[test]
fn proof_colors_are_reachable() {
    // Batch 4 L5 step 1.
    let load = bridge_fn("loadProofProfile");
    assert!(load.contains("redrob_cmyk_proof_create"));
    assert!(bridge_fn("updateProofImage").contains("redrob_cmyk_proof_apply"));
    assert!(MAIN_QML.contains(
        "Shortcut { sequence: \"Ctrl+Y\"; onActivated: editor.proofColors = !editor.proofColors }"
    ));
    assert!(menu_bar_block().contains("root.proofDialog.open()"));
    // L5c.
    assert!(bridge_fn("convertColorMode").contains("QStringLiteral(\"cmyk_profile\")"));
    assert!(menu_bar_block().contains("editor.convertColorMode(\"cmyk\")"));
    // L5b.
    assert!(bridge_fn("exportCmykTiff").contains("redrob_cmyk_export_tiff"));
    assert!(menu_bar_block().contains("root.cmykExport.open()"));
}

#[test]
fn blend_if_is_reachable_from_the_layers_panel() {
    // Batch 4 L6.
    assert!(LAYER_PANEL_QML.contains("root.blendIfWindow.openFor(layerId, blendIf)"));
    assert!(bridge_fn("setLayerBlendIf").contains("\"set_layer_blend_if\""));
    assert!(
        include_str!("../../../qml/BlendIfDialog.qml")
            .contains("editor.setLayerBlendIf(nodeId, range(thisRow), range(underRow))")
    );
}

#[test]
fn smart_objects_are_reachable() {
    // Batch 4 L4.
    assert!(bridge_fn("convertToSmartObject").contains("\"convert_to_smart_object\""));
    assert!(bridge_fn("rasterizeSmartObject").contains("\"rasterize_smart_object\""));
    assert!(menu_bar_block().contains("editor.convertToSmartObject(editor.activeLayerId)"));
    assert!(LAYER_PANEL_QML.contains("required property bool isSmartObject"));
}

#[test]
fn brush_angle_controls_are_wired() {
    // Batch 4 L3.
    assert!(OPTIONS_PANEL_QML.contains("onMoved: editor.brushAngle = value"));
    assert!(OPTIONS_PANEL_QML.contains("onToggled: editor.brushAngleFromTilt = checked"));
    let settings = bridge_fn("brushSettingsObject");
    assert!(
        settings.contains("QStringLiteral(\"angle\")")
            && settings.contains("QStringLiteral(\"angle_from_tilt\")")
    );
}

#[test]
fn rulers_place_move_and_remove_guides() {
    // Batch 4 L2.
    let rulers = include_str!("../../../qml/RulersOverlay.qml");
    for call in [
        "editor.addGuide(parent.isVertical,",
        "editor.moveGuide(parent.modelData.id,",
        "editor.removeGuide(parent.modelData.id)",
    ] {
        assert!(rulers.contains(call), "{call}");
    }
    assert!(bridge_fn("addGuide").contains("\"add_guide\""));
    assert!(ABI_RS.contains("\"guides\": document.guides()"));
    assert!(MAIN_QML.contains("Shortcut { sequence: \"Ctrl+R\"; onActivated: window.rulersVisible = !window.rulersVisible }"));
}

#[test]
fn rotate_view_tool_turns_the_view() {
    // Batch 4 L1.
    assert!(MAIN_QML.contains("objectName: \"rotateViewDrag\""));
    assert!(MAIN_QML.contains("onDoubleTapped: canvas.viewRotation = 0"));
    assert!(MAIN_QML.contains("view: [\"hand\", \"rotateview\"]"));
    assert!(
        include_str!("../../../native/qt/CMakeLists.txt").contains("dodge burn rotateview mixer)")
    );
}

#[test]
fn selected_layers_can_be_linked() {
    // Batch 4 M11.
    assert!(bridge_fn("linkSelectedLayers").contains("\"link_layers\""));
    assert!(menu_bar_block().contains("editor.linkSelectedLayers(true)"));
    assert!(LAYER_PANEL_QML.contains("required property int linkGroup"));
}

#[test]
fn content_aware_fill_runs_on_the_worker() {
    // Batch 4 M10.
    assert!(bridge_fn("contentAwareFill").contains("\"content_aware_fill\""));
    let execute = bridge_fn("executeCommand(const QJsonObject &command)");
    assert!(execute.contains("commandType == QStringLiteral(\"content_aware_fill\")"));
    assert!(MAIN_QML.contains("sequence: \"Shift+F5\""));
    assert!(menu_bar_block().contains("editor.contentAwareFill()"));
}

#[test]
fn the_navigator_shows_and_moves_the_view() {
    // Batch 4 M9.
    let nav = include_str!("../../../qml/NavigatorPanel.qml");
    assert!(nav.contains("image: editor.renderImage"));
    assert!(nav.contains("mainCanvas.anchorCanvasPoint(Qt.point(cx, cy)"));
    assert!(nav.contains("onMoved: root.app.canvasZoom = Math.exp(value)"));
    assert!(MAIN_QML.contains("mainCanvas: canvas"));
    assert!(menu_bar_block().contains("root.app.navigatorVisible = !root.app.navigatorVisible"));
}

#[test]
fn swatches_persist_and_load_from_files() {
    // Batch 4 M7.
    let load = bridge_fn("loadSwatches");
    assert!(load.contains("redrob_swatches_parse"));
    assert!(bridge_fn("saveSwatches").contains("GIMP Palette"));
    assert!(EDITOR_BRIDGE_CPP.contains("QSettings().setValue(QStringLiteral(\"swatches/colors\")"));
    assert!(OPTIONS_PANEL_QML.contains("editor.loadSwatches(selectedFile, false)"));
    assert!(OPTIONS_PANEL_QML.contains("editor.saveSwatches(selectedFile)"));
}

#[test]
fn select_color_range_is_reachable() {
    // Batch 4 M6.
    assert!(bridge_fn("selectColorRange").contains("\"select_color_range\""));
    let dialog = include_str!("../../../qml/ColorRangeDialog.qml");
    assert!(dialog.contains("editor.selectColorRange(editor.brushColor,"));
    assert!(menu_bar_block().contains("root.colorRangeDialog.open()"));
}

#[test]
fn edit_stroke_strokes_the_selection() {
    // Batch 4 M5.
    assert!(bridge_fn("strokeSelection").contains("\"stroke_selection\""));
    let dialog = include_str!("../../../qml/StrokeDialog.qml");
    assert!(dialog.contains("editor.strokeSelection(widthField.value, editor.brushColor,"));
    assert!(menu_bar_block().contains("root.strokeDialog.open()"));
}

#[test]
fn an_adjustment_layer_can_be_edited() {
    // Batch 4 M4.
    assert!(
        LAYER_PANEL_QML.contains("root.filterWindow.openForAdjustment(layerId, adjustmentFilter)")
    );
    assert!(
        FILTER_BROWSER_QML
            .contains("editor.setAdjustmentFilter(editingNodeId, selectedKind, params)")
    );
    assert!(FILTER_BROWSER_QML.contains("objectName: \"filterUpdateAdjustment\""));
    assert!(ABI_RS.contains("\"adjustment\": layer.content().adjustment_filter()"));
    assert!(MAIN_QML.contains("filterWindow: filterBrowser"));
}

#[test]
fn brush_flow_has_a_slider_and_shift_digits() {
    // Batch 4 M3.
    assert!(OPTIONS_PANEL_QML.contains("objectName: \"brushFlowControl\""));
    assert!(OPTIONS_PANEL_QML.contains("onMoved: editor.brushFlow = value"));
    assert!(MAIN_QML.contains("sequences: [\"Shift+\" + modelData, shifted]"));
    // 100% flow sends nothing, so a default stroke's command is byte-identical to before.
    let settings = bridge_fn("brushSettingsObject");
    assert!(
        settings.contains("m_brushFlowSetting < 0.999"),
        "{settings}"
    );
}

#[test]
fn text_can_use_an_installed_font_by_name() {
    // Batch 4 H7.
    assert!(TEXT_DIALOG_QML.contains("objectName: \"textFontPicker\""));
    assert!(TEXT_DIALOG_QML.contains("editor.fontFamilies"));
    assert!(TEXT_DIALOG_QML.contains("sourceFontFamily, sourceFontId)"));
    assert!(bridge_fn("startFontScan").contains("redrob_font_names"));
    assert!(
        bridge_fn("startFontScan").contains("QtConcurrent::run"),
        "the scan stays off the GUI thread"
    );
    let ensure = bridge_fn("ensureFont");
    assert!(
        ensure.contains("redrob_editor_register_font") && ensure.contains("refuseWhileFilterRuns")
    );
}

#[test]
fn the_layer_menu_offers_photoshops_locks() {
    // Batch 4 M2.
    assert!(bridge_fn("setLayerLocks").contains("\"set_layer_locks\""));
    for name in [
        "lockTransparentAction-",
        "lockPixelsAction-",
        "lockPositionAction-",
        "lockAllAction-",
    ] {
        assert!(LAYER_PANEL_QML.contains(name), "{name}");
    }
}

#[test]
fn ctrl_alt_g_toggles_a_clipping_mask() {
    // Batch 4 M1.
    assert!(bridge_fn("toggleClippingMask").contains("\"set_layer_clipped\""));
    assert!(MAIN_QML.contains("onActivated: editor.toggleClippingMask()"));
    assert!(menu_bar_block().contains("editor.toggleClippingMask()"));
    assert!(LAYER_PANEL_QML.contains("required property bool isClipped"));
}

#[test]
fn several_layers_can_be_selected_grouped_and_deleted() {
    // Batch 4 H8.
    let layers = include_str!("../../../qml/LayerPanel.qml");
    assert!(layers.contains("editor.selectLayer(layerId,"));
    assert!(layers.contains("editor.selectedLayerIds.indexOf(layerId)"));
    assert!(layers.contains("editor.deleteSelectedLayers()"));
    assert!(
        MAIN_QML.contains(
            "Shortcut { sequence: \"Ctrl+G\"; onActivated: editor.groupSelectedLayers() }"
        )
    );
    let group = bridge_fn("groupSelectedLayers");
    assert!(
        group.contains("\"add_group\"")
            && group.contains("\"move_node\"")
            && group.contains("runAsOneStep")
    );
    assert!(bridge_fn("deleteSelectedLayers").contains("runAsOneStep"));
    let one = bridge_fn("runAsOneStep");
    assert!(
        one.contains("redrob_editor_play_action_json") && one.contains("refuseWhileFilterRuns")
    );
    assert!(bridge_fn("alignActiveLayer").contains("selectedRoots()"));
}

#[test]
fn image_size_and_canvas_size_dialogs() {
    // Batch 4 H6.
    let dialog = include_str!("../../../qml/SizeDialog.qml");
    assert!(dialog.contains("editor.resizeCanvas(w, h, root.samplingMode)"));
    assert!(dialog.contains("editor.cropCanvas(x, y, w, h)"));
    assert!(
        MAIN_QML
            .contains("sequence: \"Ctrl+Alt+I\"; onActivated: imageSizeDialog.openFor(\"image\")")
    );
    assert!(
        MAIN_QML
            .contains("sequence: \"Ctrl+Alt+C\"; onActivated: imageSizeDialog.openFor(\"canvas\")")
    );
    assert!(menu_bar_block().contains("root.sizeDialog.openFor(\"canvas\")"));
}

#[test]
fn copy_cut_paste_use_the_clipboard() {
    // Batch 4 H5.
    for (name, ffi) in [
        ("copySelection", "redrob_editor_copy_rgba"),
        ("pasteClipboard", "redrob_editor_paste_rgba"),
    ] {
        let body = bridge_fn(name);
        assert!(
            body.contains(ffi) && body.contains("refuseWhileFilterRuns"),
            "{name}"
        );
        assert!(body.contains("QGuiApplication::clipboard()"), "{name}");
    }
    assert!(bridge_fn("cutSelection").contains("clearActiveLayer()"));
    for call in [
        "editor.copySelection()",
        "editor.cutSelection()",
        "editor.pasteClipboard()",
    ] {
        assert!(
            MAIN_QML.contains(call) && menu_bar_block().contains(call),
            "{call}"
        );
    }
}

#[test]
fn file_new_opens_the_new_document_dialog() {
    // Batch 4 H4.
    let dialog = include_str!("../../../qml/NewDocumentDialog.qml");
    assert!(dialog.contains("editor.newDocument(widthField.value, heightField.value, fill)"));
    assert!(MAIN_QML.contains(
        "Shortcut { sequences: [StandardKey.New]; onActivated: newDocumentDialog.openNew() }"
    ));
    assert!(menu_bar_block().contains("root.newDocument.openNew()"));
    let bridge = bridge_fn("newDocument");
    assert!(
        bridge.contains("redrob_editor_new_document") && bridge.contains("refuseWhileFilterRuns")
    );
}

#[test]
fn merge_visible_and_flatten_are_reachable() {
    // Batch 4 H3.
    assert!(bridge_fn("mergeVisible").contains("\"merge_visible\""));
    assert!(bridge_fn("flattenImage").contains("\"flatten_image\""));
    assert!(
        MAIN_QML.contains(
            "Shortcut { sequence: \"Ctrl+Shift+E\"; onActivated: editor.mergeVisible() }"
        )
    );
    assert!(menu_bar_block().contains("editor.flattenImage(root.app.backgroundColor)"));
}

#[test]
fn no_two_shortcuts_share_a_key() {
    // Qt fires NEITHER of two Shortcuts on the same key ("ambiguous"), so a duplicate silently
    // disables both.
    let mut all = main_sequences();
    for (key, tools) in ps_tool_keys() {
        all.push(key.clone());
        if tools.len() > 1 {
            all.push(format!("Shift+{key}"));
        }
    }
    all.extend(["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"].map(String::from));
    // The template sequences in the Repeaters are not literal keys.
    all.retain(|s| !s.is_empty() && !s.contains("modelData"));
    let mut seen = BTreeSet::new();
    for key in &all {
        assert!(seen.insert(key.to_lowercase()), "{key} is bound twice");
    }
}

#[test]
fn the_shortcut_list_matches_the_bindings() {
    // S2. Help > Keyboard Shortcuts is checked against what is really bound, both ways.
    let dialog = include_str!("../../../qml/ShortcutsDialog.qml");
    let rows: Vec<(String, String)> = dialog
        .lines()
        .filter_map(|l| l.trim().strip_prefix("[\""))
        .map(|l| {
            let cells: Vec<&str> = l.split("\", \"").collect();
            let state = cells
                .last()
                .unwrap()
                .trim_end_matches("\"],")
                .trim_end_matches("\"]");
            (cells[0].to_string(), state.to_string())
        })
        .collect();
    assert!(rows.len() > 40, "the list lost its rows");
    let sequences: BTreeSet<String> = main_sequences().into_iter().collect();
    let tool_keys = ps_tool_keys();
    let gestures = [
        ("Alt+right-drag", "objectName: \"brushResizeAltRight\""),
        ("Ctrl+Alt+drag", "objectName: \"brushResizeCtrlAlt\""),
        (
            "1 … 9, 0",
            "model: [\"1\", \"2\", \"3\", \"4\", \"5\", \"6\", \"7\", \"8\", \"9\", \"0\"]",
        ),
        (
            "Shift+1 … 0",
            "onActivated: editor.brushFlow = modelData === \"0\" ? 1.0 : Number(modelData) / 10",
        ),
        (
            "Space (hold)",
            "window.holdTool(\"hand\", editor.spaceHeld)",
        ),
        ("Alt (hold)", "window.holdTool(\"picker\", editor.altHeld)"),
        // L10: Alt-click sets the clone source; Ctrl held is the temporary Move tool.
        (
            "Alt+click",
            "editor.brushClone && (point.modifiers & Qt.AltModifier)",
        ),
        ("Ctrl (hold)", "window.holdTool(\"move\", editor.ctrlHeld)"),
    ];
    for (keys, state) in &rows {
        let works = !state.starts_with("not yet");
        for key in keys.split(" / ") {
            let bound = if let Some((_, marker)) = gestures.iter().find(|(g, _)| g == keys) {
                MAIN_QML.contains(marker)
            } else if let Some(letter) = key.strip_prefix("Shift+").filter(|k| k.len() == 1) {
                tool_keys.iter().any(|(k, t)| k == letter && t.len() > 1) || sequences.contains(key)
            } else if key.len() == 1 && key.chars().all(|c| c.is_ascii_uppercase()) {
                tool_keys.iter().any(|(k, _)| k == key) || sequences.contains(key)
            } else {
                sequences.contains(key)
                    || (key == "Ctrl+O" && MAIN_QML.contains("StandardKey.Open"))
                    || (key == "Ctrl+N" && MAIN_QML.contains("StandardKey.New]"))
                    || (key == "Ctrl+C" && MAIN_QML.contains("StandardKey.Copy]"))
                    || (key == "Ctrl+X" && MAIN_QML.contains("StandardKey.Cut]"))
                    || (key == "Ctrl+V" && MAIN_QML.contains("StandardKey.Paste]"))
                    || (key == "Ctrl+S" && MAIN_QML.contains("StandardKey.Save"))
                    || (key == "Ctrl+Z" && MAIN_QML.contains("StandardKey.Undo"))
                    || (key == "Ctrl+Shift+Z" && MAIN_QML.contains("StandardKey.Redo"))
            };
            if works {
                assert!(bound, "the list says {key} works, but nothing binds it");
            } else {
                assert!(!bound, "{key} is bound but the list says \"{state}\"");
            }
        }
    }
    // Every literal binding is listed.
    let listed: String = rows.iter().map(|(k, _)| format!("{k} / ")).collect();
    for sequence in &sequences {
        if ["Return", "Enter", "Escape", "Ctrl++", "Backspace", "{", "}"]
            .contains(&sequence.as_str())
        {
            continue; // Aliases of a listed key, or dialog keys.
        }
        assert!(
            listed.contains(&format!("{sequence} /")) || listed.contains(&format!("{sequence} ")),
            "{sequence} is bound but missing from Help > Keyboard Shortcuts"
        );
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
        OPTIONS_PANEL_QML.contains("editor.historyLabels[index - 1]"),
        "rows do not show labels"
    );
    assert!(
        OPTIONS_PANEL_QML.contains("onClicked: editor.jumpToHistory(index)"),
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
    // P12: the menu bar is its own file now; the whole file is the block.
    MENU_BAR_QML
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
fn split_panels_are_wired_and_do_not_reach_back_into_main() {
    // P12. A required property Main.qml forgets to set stops the window from loading; one set as
    // `name: name` binds to itself. Checked for every panel that takes inputs.
    for (file, body, opening) in [
        ("MainMenuBar.qml", MENU_BAR_QML, "menuBar: MainMenuBar {"),
        ("LayerPanel.qml", LAYER_PANEL_QML, "LayerPanel {"),
        ("OptionsPanel.qml", OPTIONS_PANEL_QML, "OptionsPanel {"),
    ] {
        let props: Vec<&str> = body
            .lines()
            // The file's own inputs sit at the root's indent; deeper ones are a delegate's roles.
            .filter_map(|l| l.strip_prefix("    required property var "))
            .collect();
        assert!(!props.is_empty(), "{file} takes no inputs");
        let wiring = qml_block(opening);
        for prop in &props {
            let line = wiring
                .lines()
                .find(|l| l.trim().starts_with(&format!("{prop}:")))
                .unwrap_or_else(|| panic!("Main.qml does not set {file}'s {prop}"));
            assert_ne!(
                line.trim()[prop.len() + 1..].trim(),
                *prop,
                "{file}: {prop} bound to itself"
            );
        }
        assert!(
            !body.contains("window."),
            "{file} reaches Main.qml's window id"
        );
    }
}

#[test]
fn the_menu_bar_file_is_wired_to_every_object_it_drives() {
    // P12. Each required property must be set by Main.qml, or the window fails to load; and none
    // may be set as `name: name`, which binds a property to itself instead of to Main.qml's id.
    let props: Vec<&str> = MENU_BAR_QML
        .lines()
        .filter_map(|l| l.trim().strip_prefix("required property var "))
        .collect();
    assert!(props.len() >= 9, "menu bar lost its inputs: {props:?}");
    let wiring = qml_block("menuBar: MainMenuBar {");
    for prop in &props {
        let line = wiring
            .lines()
            .find(|l| l.trim().starts_with(&format!("{prop}:")))
            .unwrap_or_else(|| panic!("Main.qml does not set the menu bar's {prop}"));
        let value = line.trim()[prop.len() + 1..].trim();
        assert_ne!(value, *prop, "{prop} is bound to itself");
    }
    assert!(
        !MENU_BAR_QML.contains("window."),
        "the menu bar reaches Main.qml's window id"
    );
}

#[test]
fn the_menu_bar_has_the_expected_menus() {
    let block = menu_bar_block();
    for title in [
        "&File", "&Edit", "&Select", "&Layer", "&Image", "&Actions", "Filte&rs", "&View",
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
        "newDocument",
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
        // P13: refuses while a filter runs (m_filterBusy) before it takes the engine lock.
        "handleMcpToolCall",
        // S1: the live-stroke path; each checks m_filterBusy before touching the engine.
        "beginStroke",
        "flushLiveStroke",
        "refreshLiveRender",
        "endStroke",
        "cancelStroke",
        // H4: File > New, refuses while a filter runs.
        "newDocument",
        // H5: each refuses while a filter runs.
        "copySelection",
        "pasteClipboard",
        // H8: the one-step command runner refuses while a filter runs.
        "runAsOneStep",
        // H7: loads a font into the engine; refuses while a filter runs.
        "ensureFont",
        // H7: the font scan worker calls only redrob_font_names, which takes no editor.
        "startFontScan",
        // #109: the preview copies the document under the engine lock and filters outside it.
        "previewFilterParams",
        // L11: the worker render locks only to copy the document and to store the frame.
        "startAsyncRender",
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
fn filter_preview_runs_off_the_lock_and_cancels_by_ticket() {
    // Preview shows Apply's result without committing it; cancelling drops the result.
    let preview = bridge_fn("previewFilterParams");
    assert!(
        preview.contains("QtConcurrent::run"),
        "preview runs on the GUI thread"
    );
    assert!(preview.contains("redrob_editor_preview_filter_rgba(editor.get()"));
    assert!(
        preview.contains("if (ticket != m_filterPreviewTicket)\n            return;"),
        "a cancelled or superseded preview still lands"
    );
    assert!(bridge_fn("clearFilterPreview").contains("++m_filterPreviewTicket"));
    assert!(
        MAIN_QML
            .contains("image: editor.hasFilterPreview ? editor.filterPreview : editor.renderImage")
    );
    // Apply, closing the browser, and choosing another filter all drop the preview.
    assert!(
        FILTER_BROWSER_QML
            .matches("editor.clearFilterPreview()")
            .count()
            >= 3
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
        menu_bar_block().contains("onTriggered: root.filters.open()"),
        "no menu route to the browser"
    );
}

/// A2: a redrob-code task runs with the canvas server injected through the environment (the token
/// never lands in a file), project config off, every file/shell/network permission denied, in a
/// private scratch folder, with no shell between the task text and the process.
#[test]
fn a_redrob_code_task_runs_locked_down_against_the_canvas_only() {
    let runner = include_str!("../../../native/qt/RedrobCodeRunner.cpp");
    assert!(
        runner.contains("\"REDROB_CONFIG_CONTENT\""),
        "config must come from the env"
    );
    assert!(
        runner.contains("\"REDROB_DISABLE_PROJECT_CONFIG\"), QStringLiteral(\"1\")"),
        "project config could re-enable tools"
    );
    for key in [
        "edit",
        "bash",
        "task",
        "webfetch",
        "websearch",
        "external_directory",
        "question",
        "skill",
    ] {
        assert!(
            runner.contains(&format!(
                "{{QStringLiteral(\"{key}\"), QStringLiteral(\"deny\")}}"
            )),
            "{key} is not denied"
        );
    }
    assert!(
        runner.contains("QTemporaryDir"),
        "runs outside a private folder"
    );
    assert!(
        !runner.contains("/bin/sh") && !runner.contains("\"-c\""),
        "no shell"
    );
    assert!(
        !runner.contains("QFile") && !runner.contains("QSaveFile"),
        "the runner must not write the token to a file"
    );
    // The bridge only starts a run while the tokened loopback endpoint is listening, and turning
    // the endpoint off stops the run.
    let bridge = EDITOR_BRIDGE_CPP;
    let run = bridge
        .split("void EditorBridge::runRedrobCodeTask")
        .nth(1)
        .expect("runRedrobCodeTask");
    let run = &run[..run.find("\n}\n").unwrap()];
    assert!(run.contains("if (!m_mcp.isListening())"), "{run}");
    assert!(run.contains("RedrobCodeRunner::lockedDownConfig"), "{run}");
    assert!(bridge.contains("m_codeRunner.stop();\n        m_mcp.stop();"));
}

/// A2: the config entry matches redrob-code's McpRemoteConfig (core/v1/config/mcp.ts): `type`,
/// `url`, `enabled`, `headers`, `oauth` -- and no other key, since redrob hard-fails on unknown ones.
#[test]
fn the_mcp_entry_uses_only_redrob_code_remote_config_keys() {
    let entry = EDITOR_BRIDGE_CPP
        .split("QJsonObject EditorBridge::mcpServerEntry() const")
        .nth(1)
        .expect("mcpServerEntry");
    let entry = &entry[..entry.find("\n}\n").unwrap()];
    let allowed = [
        "type",
        "url",
        "enabled",
        "headers",
        "oauth",
        "Authorization",
    ];
    let mut rest = entry;
    let mut seen = Vec::new();
    while let Some(at) = rest.find("{QStringLiteral(\"") {
        rest = &rest[at + "{QStringLiteral(\"".len()..];
        let key = &rest[..rest.find('"').unwrap()];
        seen.push(key.to_owned());
    }
    for key in &seen {
        assert!(allowed.contains(&key.as_str()), "unknown key {key}");
    }
    for key in ["type", "url", "headers", "oauth"] {
        assert!(seen.iter().any(|k| k == key), "missing {key}");
    }
    assert!(
        entry.contains("http://127.0.0.1:%1/mcp"),
        "must stay loopback"
    );
}

/// A1 menu wiring: both items exist and call the bridge.
#[test]
fn crop_to_selection_and_clear_outside_are_in_the_menus() {
    assert!(MENU_BAR_QML.contains("onTriggered: editor.cropToSelection()"));
    assert!(MENU_BAR_QML.contains("onTriggered: editor.clearOutsideSelection()"));
    assert!(EDITOR_BRIDGE_CPP.contains("QStringLiteral(\"crop_to_selection\")"));
    assert!(EDITOR_BRIDGE_CPP.contains("QStringLiteral(\"clear_outside_selection\")"));
}

/// A3: the console sign-in sends this app's own product id, keeps the device code inside the
/// engine, stores the key owner-only, opens only an https console page, and never replaces a key
/// the person set in the environment.
#[test]
fn console_device_connect_is_scoped_and_keeps_secrets_in() {
    let abi = include_str!("../../redrob-ffi/src/abi.rs");
    assert!(abi.contains("const DEVICE_PRODUCT: &str = \"canvas\";"));
    let start = abi
        .split("pub unsafe extern \"C\" fn redrob_device_flow_start")
        .nth(1)
        .expect("redrob_device_flow_start");
    let shown = &start[start.find("let shown = json!({").unwrap()..];
    let shown = &shown[..shown.find("});").unwrap()];
    assert!(
        !shown.contains("device_code"),
        "the device code must not reach the host"
    );

    let conn = include_str!("../../../native/qt/ConsoleConnection.cpp");
    assert!(conn.contains("QFileDevice::ReadOwner | QFileDevice::WriteOwner"));
    assert!(conn.contains("url.scheme() == QStringLiteral(\"https\")"));
    assert!(conn.contains("if (m_fromEnvironment)"), "env key must win");
    assert!(
        !conn.contains("qDebug") && !conn.contains("qWarning"),
        "nothing here may log the key"
    );
    assert!(EDITOR_BRIDGE_CPP.contains("m_apiKey = m_console.key();"));
}

/// N1: after a crop the navigator frame was drawn against the main view's OLD image size, because
/// its binding did not depend on that image. It must re-evaluate when the main view's image does.
#[test]
fn the_navigator_frame_follows_the_main_view_image() {
    let nav = include_str!("../../../qml/NavigatorPanel.qml");
    let view = nav
        .split("property rect view: {")
        .nth(1)
        .expect("view binding");
    let view = &view[..view.find("return root.viewRect();").unwrap()];
    assert!(view.contains("root.mainCanvas.imageRect;"), "{view}");
}
