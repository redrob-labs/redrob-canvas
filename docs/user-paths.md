# redrob-canvas — user-visible action inventory (read-only)

Audited at `develop` `cb47d2a` (2026-10-01) by reading the code; line numbers refer to that commit. Status is from the code and existing tests, not from running the app.


Repo: `` @ `qml/Main.qml` (2169 lines), `native/qt/` (Qt 6 / QML),
`crates/redrob-{core,ffi,agent}` (Rust). Nothing was edited, built, run or committed.

## How to read this

**Architecture.** Almost every action funnels through one seam: a QML handler calls a
`Q_INVOKABLE` on `EditorBridge`, which builds a canonical JSON command and hands it to
`EditorBridge::executeCommand` (`native/qt/EditorBridge.cpp:454`) → `redrob_editor_execute_json`
(`crates/redrob-ffi/src/abi.rs:948`) → `Command` dispatch (`crates/redrob-core/src/command.rs:217`).
Only history, timeline navigation, file I/O, agent proposals and projection reads use their own FFI
entry points. Where the "Backend call" column says *execute_json → `<type>`*, read it as that chain
with the named command `type`.

**Status rule applied, strictly.**

- `works` — real work happens **and** a named existing test exercises the effect of that action:
  either the ctest smoke (`redrob_native_smoke`, `native/qt/CMakeLists.txt:236`, body at
  `native/qt/main.cpp:985`) driving the same bridge method/QML object, or a `cargo test` over the
  command the handler emits.
- `code-only` — real code on the path, nothing on the path is tested (typically pure view state).
- `none` — handler empty / visual-only / not wired.
- `broken` — the code shows it cannot work.

**Caveat that applies to every `works` row.** No QtTest/QML test exists in this repo. The ctest
smoke drives exactly **six** QML entry points (`addFrameAction`, `duplicateFrameAction`,
`deleteFrameAction`, `moveFrameLeftAction`, `moveFrameRightAction` via
`QMetaObject::invokeMethod(..., "clicked")`, plus `maskNodeFromSelection` invoked on the root at
`native/qt/main.cpp:113`) and asserts the `enabled` property of 20 more
(`rasterActionsMatch`, `native/qt/main.cpp:71`). Every other `works` row is tested **below** the
QML line: the bridge method and/or the core command are covered, the `onClicked`/`onTriggered`
binding itself is not. Defect D15 records this.

## Actions

| Action | Location | Entry (file:line) | Backend call (file:line) | Status | Evidence/notes |
|---|---|---|---|---|---|
| Open project… | Top toolbar | `qml/Main.qml:350` → dialog `qml/Main.qml:148` | `EditorBridge::openProject` `native/qt/EditorBridge.cpp:1629` → `replaceFromGenericBytes` `:1575` → `redrob_editor_import_file` `crates/redrob-ffi/src/abi.rs:1237` | works | ctest `redrob_native_smoke` → `genericFormatBridgeIsValid` `native/qt/main.cpp:755` (project open/identity at `:923`); `cargo test -p redrob-ffi --test ffi_api:336 generic_format_routes_roundtrip_with_structured_results_and_truthful_capabilities` |
| Import interchange file… | Top toolbar | `qml/Main.qml:356` → dialog `qml/Main.qml:158` | `EditorBridge::importFile` `:1655` → same import FFI `abi.rs:1237` | works | `native/qt/main.cpp:808` (SVG import); `cargo test --test formats:416 svg_subset_accepts_shapes_and_normalizes_relative_smooth_paths`, `:256 hand_built_ora_imports_top_first_hierarchy_blend_and_offsets` |
| Save project | Top toolbar | `qml/Main.qml:362` | `EditorBridge::saveProject` `:1758` → `exportGenericBytes` `:1682` → `redrob_editor_export_file` `abi.rs:1274` | works | `native/qt/main.cpp:828` save-identity assert; `ffi_api.rs:259 execute_render_project_and_png_roundtrip_end_to_end` |
| Save project as… | Top toolbar | `qml/Main.qml:368` → dialog `qml/Main.qml:166` | same as above | works | same |
| Undo (button) | Top toolbar | `qml/Main.qml:379` | `EditorBridge::undo` `:519` → `executeHistoryAction` `:487` → `redrob_editor_undo` `abi.rs:979` | works | `main.cpp:443`; `editor_core.rs:327 grouped_undo_redo_is_atomic_and_history_is_bounded`; `ffi_api.rs:137` |
| Redo (button) | Top toolbar | `qml/Main.qml:385` | `EditorBridge::redo` `:520` → `redrob_editor_redo` `abi.rs:1000` | works | `main.cpp:448`; `editor_core.rs:327`; `ffi_api.rs:137` |
| Export… (open options dialog) | Top toolbar | `qml/Main.qml:399` | none — opens `exportOptionsDialog` `qml/Main.qml:182` | code-only | smoke only asserts the object EXISTS (`main.cpp:759-768`), never opens it |
| Export format | Export dialog | `qml/Main.qml:208` | none — sets `window.exportFormat` `qml/Main.qml:81` | code-only | control existence asserted only (`main.cpp:760`) |
| Allow documented loss | Export dialog | `qml/Main.qml:217` | none — sets `window.exportAllowLoss` `qml/Main.qml:82` | code-only | existence only (`main.cpp:761`) |
| JPEG quality | Export dialog | `qml/Main.qml:238` | none — sets `window.exportJpegQuality` `qml/Main.qml:83` | code-only | existence only (`main.cpp:762`) |
| Choose opaque JPEG matte | Export dialog | `qml/Main.qml:247` → `ColorDialog` `qml/Main.qml:271` | none — sets `window.exportMatte` | code-only | no test |
| Export dialog OK → write file | Export dialog | `qml/Main.qml:189` → file dialog `qml/Main.qml:178` | `EditorBridge::exportFile` `:1792` → `exportGenericBytes` `:1682` → `redrob_editor_export_file` `abi.rs:1274` | works | `main.cpp:845-860` (png/webp/jpeg/ora/svg + non-mutation contract); `formats.rs:128 explicit_frame_render_and_export_do_not_mutate_editor_state`; `ffi_api.rs:657` |
| Shortcut StandardKey.Undo | Window | `qml/Main.qml:296` | `undo` `:519` → `abi.rs:979` | works | as Undo button |
| Shortcut StandardKey.Redo | Window | `qml/Main.qml:301` | `redo` `:520` → `abi.rs:1000` | works | as Redo button |
| Shortcut StandardKey.Open | Window | `qml/Main.qml:305` | none — opens dialog | code-only | no test |
| Shortcut StandardKey.Save | Window | `qml/Main.qml:309` | `saveProject` `:1758` (or dialog if no path) | works | `main.cpp:828` |
| Shortcut Escape | Window | `qml/Main.qml:313` → `cancelGesture` `qml/Main.qml:556` | `EditorBridge::cancelStroke` `:718` — clears local point buffer, no FFI | code-only | no test |
| Tool rail **B** brush | Tool rail | `qml/Main.qml:422`, handler `qml/Main.qml:119` | none — sets `window.activeTool` | code-only | `enabled` projection asserted by `rasterActionsMatch` `main.cpp:72`; the click is untested |
| Tool rail **F** fill | Tool rail | `qml/Main.qml:429` → `:119` | none — mode only | code-only | `enabled` asserted `main.cpp:73`. Fills the whole layer/selection, not a bucket — see D8 |
| Tool rail **R** rectangle select | Tool rail | `qml/Main.qml:435` → `:119` | none — mode only | code-only | no test |
| Tool rail **E** ellipse select | Tool rail | `qml/Main.qml:440` → `:119` | none — mode only | code-only | no test |
| Tool rail **S** shape | Tool rail | `qml/Main.qml:446` → `:119` | none — mode only | code-only | no test |
| Tool rail **G** gradient | Tool rail | `qml/Main.qml:452` → `:119` | none — mode only | code-only | `enabled` asserted `main.cpp:74` |
| Tool rail **C** crop | Tool rail | `qml/Main.qml:458` → `:119` | none — mode only | code-only | no test |
| Tool rail **T** transform | Tool rail | `qml/Main.qml:464` → `:119` | none — mode only | code-only | `enabled` asserted `main.cpp:75` |
| Tool rail **I** inspect | Tool rail | `qml/Main.qml:470` → `:119` | none | none | selecting it makes the canvas gesture return early (`qml/Main.qml:600`); only effect is the cursor shape (`qml/Main.qml:625`). See D14 |
| Brush colour swatch | Tool rail | `qml/Main.qml:487` → `ColorDialog` `qml/Main.qml:278` | `EditorBridge::setBrushColor` `:257` (property; consumed by `endStroke` `:698`) | works | colour field of the stroke command covered by `editor_core.rs:185 brush_interpolates_and_uses_pressure_and_selection`; the bridge setter itself untested |
| Canvas drag — brush stroke | Canvas | `qml/Main.qml:595` / `:612` / commit `qml/Main.qml:570` | `beginStroke` `:665`, `addStrokePoint` `:677`, `endStroke` `:698` → execute_json → `brush_stroke` | works | `pressureNormalizationIsValid` `main.cpp:931` (pressure, via QML); `editor_core.rs:185`, `:214 brush_point_limit_...`, `:990 brush_smoothing_and_mirror_settings_...` |
| Canvas drag — fill | Canvas | `qml/Main.qml:572` | `EditorBridge::fill` `:725` → execute_json → `fill` | works | `main.cpp:109` (smoke calls `editor.fill`); `editor_core.rs:137 selection_boolean_modes_limit_fill_clear_and_filters` |
| Canvas drag — rectangle selection | Canvas | `qml/Main.qml:574` | `selectRectangle` `:1150` → `select_rectangle` | works | `main.cpp:110`; `editor_core.rs:137`, `:567` |
| Canvas drag — ellipse selection | Canvas | `qml/Main.qml:576` | `selectEllipse` `:1166` → `select_ellipse` | works | `main.cpp:1000` (smoke selection assert); `editor_core.rs:567 ellipse_select_all_invert_feather_grow_and_shrink_are_grayscale_and_bounded` |
| Canvas drag — linear gradient | Canvas | `qml/Main.qml:579` | `linearGradient` `:1201` → `gradient_fill` | works | `editor_core.rs:664 linear_and_radial_gradients_clamp_interpolate_and_respect_selection` |
| Canvas drag — radial gradient | Canvas | `qml/Main.qml:581` | `radialGradient` `:1215` → `gradient_fill` | works | `editor_core.rs:664` |
| Canvas drag — draw shape | Canvas | `qml/Main.qml:583` → `commitShape` `qml/Main.qml:70` | `addShapeFromBox` `:921` / `addShapeFromRadius` `:975` → `add_shape_node` | works | `main.cpp:310-375` (ellipse, star, polygon pixel assert, 4 rejections, zero-stroke); `shape_commands.rs:30`, `:167 every_shape_variant_reaches_the_document`. Colour source is a defect — D13 |
| Canvas drag — crop | Canvas | `qml/Main.qml:585` | `cropCanvas` `:1230` → `crop_canvas` | works | `editor_core.rs:762 crop_extends_transparently_updates_all_layers_selection_and_undo` |
| Canvas drag — translate active layer | Canvas | `qml/Main.qml:587` | `transformActive` `:1300` → `transform_active` | works | `editor_core.rs:811 resize_flip_rotate_and_affine_transform_have_deterministic_mapping` |
| Canvas wheel zoom | Canvas | `qml/Main.qml:631` | `CanvasItem::setZoom` `native/qt/CanvasItem.cpp:86` (view only) | code-only | no test |
| Canvas hover cursor | Canvas | `qml/Main.qml:624` | none | none | visual only |
| Zoom out / in / 1:1 | Canvas overlay | `qml/Main.qml:655`, `:666`, `:671` | `CanvasItem::setZoom` `CanvasItem.cpp:86` | code-only | no test |
| Previous frame | Timeline | `qml/Main.qml:699` | `setCurrentFrame` `:637` → `executeNavigation` `:611` → `redrob_editor_set_current_frame` `abi.rs:1116` | works | `main.cpp:527`; `timeline.rs:305 navigation_advances_generation_without_touching_history_...`; `ffi_api.rs:1647` |
| Play / stop | Timeline | `qml/Main.qml:705` | `setPlaying` `:639` → `redrob_editor_set_playing` `abi.rs:1135` | works | `main.cpp:586`, `:740`; `timeline.rs:347 loop_end_wraps_to_range_start_and_keeps_playing_history_neutral` |
| Next frame | Timeline | `qml/Main.qml:712` | `setCurrentFrame` `:637` → `abi.rs:1116` | works | same as Previous frame |
| + Frame | Timeline | `qml/Main.qml:719` | `addFrame` `:550` (+ `allocateFrameId` `:531`) → `add_frame` | works | **click driven** by `main.cpp:518`; `timeline.rs:18 frame_crud_is_ordered_typed_and_transactional`; `frameIdAllocatorIsValid` `main.cpp:26` |
| Duplicate frame | Timeline | `qml/Main.qml:725` | `duplicateFrame` `:561` → `duplicate_frame` | works | **click driven** by `main.cpp:534`; `timeline.rs:224 duplicate_uses_cow_and_edit_detaches_only_the_destination` |
| Delete frame | Timeline | `qml/Main.qml:732` | `removeFrame` `:573` → `remove_frame` | works | **click driven** by `main.cpp:547`; `timeline.rs:536` |
| Move frame left | Timeline | `qml/Main.qml:739` | `moveFrame` `:581` → `move_frame` | works | **click driven** by `main.cpp:567`; `timeline.rs:271 range_order_move_and_remove_current_fallback_are_deterministic` |
| Move frame right | Timeline | `qml/Main.qml:746` | `moveFrame` `:581` → `move_frame` | works | **click driven** by `main.cpp:571` |
| Timeline FPS | Timeline | `qml/Main.qml:756` | `setTimelineFps` `:590` → `set_timeline_fps` | works | `main.cpp:556`; `timeline.rs:133 fps_is_authoritative_for_existing_added_and_duplicated_frames` |
| Loop | Timeline | `qml/Main.qml:763` | `setLooping` `:605` → `set_looping` | works | `main.cpp:557`; `timeline.rs:347` |
| Playback range start | Timeline | `qml/Main.qml:773` | `setPlaybackRange` `:598` → `set_playback_range` | works | `timeline.rs:271`, `:437` |
| Playback range end | Timeline | `qml/Main.qml:784` | `setPlaybackRange` `:598` | works | same |
| Frame thumbnail tap | Timeline | `qml/Main.qml:812` | `setCurrentFrame` `:637` → `abi.rs:1116` | works | `timeline.rs:305` |
| "+" open add-node menu | Layers panel | `qml/Main.qml:1055` | none — opens `addNodeMenu` `qml/Main.qml:994` | code-only | no test |
| Add raster layer | Layers menu | `qml/Main.qml:999` | `addLayer` `:736` → `add_layer` | works | `editor_core.rs:25 typed_commands_serialize_and_layer_commands_preserve_identity` |
| Add group | Layers menu | `qml/Main.qml:1004` | `addGroup` `:745` → `add_group` | works | `main.cpp:107`; `editor_core.rs:1934 group_raster_editing_is_typed_and_group_compositing_masks_exactly_once`; `ffi_api.rs:1520` |
| Add text (opens dialog) | Layers menu | `qml/Main.qml:1010` | none — resets + opens `textSemanticEditor` `qml/Main.qml:868` | code-only | object existence asserted `main.cpp:173-177` |
| Add vector rectangle (opens dialog) | Layers menu | `qml/Main.qml:1027` | none — resets + opens `vectorRectangleEditor` `qml/Main.qml:927` | code-only | existence only `main.cpp:176` |
| Text dialog OK (add) | Text dialog | `qml/Main.qml:881` (buttons `:875`) | `addTextNode` `:758` → `add_text_node` | works | `main.cpp:217`; `semantic_path.rs:50 exact_text_and_vector_goldens_are_stable`; `ffi_api.rs:1396` |
| Text dialog OK (edit) | Text dialog | `qml/Main.qml:878` | `setTextContent` `:795` → `set_text_content` | works | `main.cpp:262`; `semantic_path.rs:988 add_and_set_semantic_commands_preflight_before_commit` |
| Text dialog fields (name/body/X/Y/size/colour) | Text dialog | `qml/Main.qml:889`, `:894`, `:905`, `:907`, `:909`, `:913` | values consumed by `addTextNode`/`setTextContent` above | works | same tests; the 256 KiB clamp at `qml/Main.qml:900` is itself untested |
| Vector dialog OK (add) | Vector dialog | `qml/Main.qml:940` | `addVectorRectangle` `:847` → `add_vector_node` | works | `main.cpp:280`; `semantic_path.rs:50`; `ffi_api.rs:1749 ffi_projects_authoritative_rectangle_source_...` |
| Vector dialog OK (edit) | Vector dialog | `qml/Main.qml:935` | `setVectorRectangle` `:870` → `set_vector_content` | works | `semantic_path.rs:988`; `ffi_api.rs:1749` |
| Vector dialog fields (X/Y/W/H/stroke/fill/stroke colour) | Vector dialog | `qml/Main.qml:957`–`:971` | consumed by the two calls above | works | same. Also read by the shape tool — D13 |
| "−" delete active layer | Layers panel | `qml/Main.qml:1061` | `deleteLayer` `:1099` → `remove_layer` | works | `main.cpp:382`; `editor_core.rs:1760 hierarchy_moves_cycles_deletion_and_active_fallback_are_transactional` |
| Layer row tap (make active) | Layer row | `qml/Main.qml:1220` | `setActiveLayer` `:1105` → `set_active_layer` | works | `main.cpp:156`; `editor_core.rs:1760` |
| Layer row right-click | Layer row | `qml/Main.qml:1223` | none — opens `layerActions` `qml/Main.qml:1117` | code-only | no test |
| Layer visibility toggle | Layer row | `qml/Main.qml:1241` | `setLayerVisibility` `:1124` → `set_layer_visibility` | works | `editor_core.rs:66 rendering_composites_order_opacity_visibility_and_blend_modes` |
| Layer rename | Layer row | `qml/Main.qml:1248` | `renameLayer` `:1111` → `rename_layer` | works | `editor_core.rs:25` |
| Layer opacity slider | Layer row | `qml/Main.qml:1274` | `setLayerOpacity` `:1117` → `set_layer_opacity` | works | `editor_core.rs:66`. Commits on release only — D16 |
| Move to document root | Layer menu | `qml/Main.qml:1123` | `moveNode` `:1037` → `move_node` | works | `main.cpp:110`; `editor_core.rs:1760`, `:1829` |
| Move active node into this group | Layer menu | `qml/Main.qml:1129` | `moveNode` `:1037` | works | same |
| Move toward top | Layer menu | `qml/Main.qml:1136` | `moveNode` `:1037` | works | `editor_core.rs:1829 same_parent_moves_accept_exact_top_and_bottom_boundaries_and_reject_one_over` |
| Move toward bottom | Layer menu | `qml/Main.qml:1143` | `moveNode` `:1037` | works | same. Passes `siblingIndex - 1`, clamped at `EditorBridge.cpp:1043` — D17 |
| Edit text content… | Layer menu | `qml/Main.qml:1151` | opens dialog → `setTextContent` `:795` | works | `main.cpp:262`; `semantic_path.rs:988` |
| Edit vector rectangle… | Layer menu | `qml/Main.qml:1170` | opens dialog → `setVectorRectangle` `:870` | works | `ffi_api.rs:1749`; `main.cpp:415` asserts a non-rectangle path is NOT advertised as editable |
| Rasterize on current frame… | Layer menu | `qml/Main.qml:1187` → warning dialog OK `qml/Main.qml:983` | `rasterizeSemanticNode` `:1029` → `rasterize_semantic_node` | works | `main.cpp:480`; `semantic_path.rs:86 rasterize_preserves_identity_properties_and_exact_visual_then_undo_redo` |
| Add raster mask | Layer menu | `qml/Main.qml:1197` | `addRasterMask` `:1046` → `add_raster_mask` | works | `editor_core.rs:2105 raster_mask_toggle_crop_resize_roundtrip_and_history_restore_exact_state`; `ffi_api.rs:1520` |
| Mask from selection | Layer menu | `qml/Main.qml:1203` → `window.maskNodeFromSelection` `qml/Main.qml:87` | `rasterMaskFromSelection` `:1052` → `raster_mask_from_selection` | works | **invoked through QML** by `main.cpp:113`; `editor_core.rs:2040 raster_mask_from_selection_requires_active_selection_...` |
| Enable / disable raster mask | Layer menu | `qml/Main.qml:1209` | `setRasterMaskEnabled` `:1064` → `set_raster_mask_enabled` | works | `main.cpp:151`; `editor_core.rs:2105` |
| Remove raster mask | Layer menu | `qml/Main.qml:1215` | `removeRasterMask` `:1058` → `remove_raster_mask` | works | `editor_core.rs:2105` |
| Brush size slider | Options · BRUSH | `qml/Main.qml:1307` | `setBrushSize` `:246` (property) → `size` on `brush_stroke` | works | `editor_core.rs:246 brush_size_limit_accepts_boundary_and_rejects_one_over_transactionally`; setter untested |
| Brush opacity slider | Options · BRUSH | `qml/Main.qml:1326` | `setBrushOpacity` `:265` → `opacity` on `brush_stroke` | works | `editor_core.rs:185`; setter untested |
| Brush smoothing kind | Options · BRUSH | `qml/Main.qml:1338` | `setBrushSmoothingKind` `:276` → `settings.smoothing` | works | `editor_core.rs:990 brush_smoothing_and_mirror_settings_are_deterministic_and_validated` |
| Smoothing window | Options · BRUSH | `qml/Main.qml:1352` | `setBrushSmoothingWindow` `:286` | works | `editor_core.rs:990` |
| Mirror X on/off | Options · BRUSH | `qml/Main.qml:1360` | `setMirrorXEnabled` `:295` → `settings.mirror_x` | works | `editor_core.rs:990` |
| Mirror X axis | Options · BRUSH | `qml/Main.qml:1368` | `setMirrorXAxis` `:311` | works | `editor_core.rs:990` |
| Mirror Y on/off | Options · BRUSH | `qml/Main.qml:1377` | `setMirrorYEnabled` `:303` → `settings.mirror_y` | works | `editor_core.rs:990` |
| Mirror Y axis | Options · BRUSH | `qml/Main.qml:1385` | `setMirrorYAxis` `:319` | works | `editor_core.rs:990` |
| Selection combination mode | Options · SELECTION | `qml/Main.qml:1398` | `window.selectionMode` → `mode` on select commands, validated `EditorBridge.cpp:419` | works | `editor_core.rs:137` |
| Select All | Options · SELECTION | `qml/Main.qml:1405` | `selectAll` `:1182` → `select_all` | works | `editor_core.rs:567` |
| Invert selection | Options · SELECTION | `qml/Main.qml:1411` | `invertSelection` `:1183` → `invert_selection` | works | `editor_core.rs:567` |
| Clear selection | Options · SELECTION | `qml/Main.qml:1417` | `clearSelection` `:1184` → `clear_selection` | works | `editor_core.rs:567` |
| Morphology radius | Options · SELECTION | `qml/Main.qml:1423` | local value only | code-only | no test |
| Feather | Options · SELECTION | `qml/Main.qml:1433` | `featherSelection` `:1185` → `feather_selection` | works | `editor_core.rs:567` |
| Grow | Options · SELECTION | `qml/Main.qml:1441` | `growSelection` `:1190` → `grow_selection` | works | `editor_core.rs:567` |
| Shrink | Options · SELECTION | `qml/Main.qml:1446` | `shrinkSelection` `:1195` → `shrink_selection` | works | `editor_core.rs:567` |
| Shape kind | Options · SHAPE | `qml/Main.qml:1459` | `window.shapeKind` → `addShapeFromBox` `:921` / `addShapeFromRadius` `:975` | works | `shape_commands.rs:167` covers all six variants; `main.cpp:310` |
| Shape sides | Options · SHAPE | `qml/Main.qml:1474` | `sides` on `add_shape_node`; bound-checked `EditorBridge.cpp:994` | works | `shape_commands.rs:167`; `main.cpp:356` rejects 2 sides |
| Star inner ratio | Options · SHAPE | `qml/Main.qml:1490` | `inner_radius = radius × ratio` `EditorBridge.cpp:1013` | works | `shape_commands.rs:167`; `main.cpp:359` rejects ratio 1.0 |
| Rounded-rect corner radius | Options · SHAPE | `qml/Main.qml:1506` | clamped at `EditorBridge.cpp:959` | works | `shape_commands.rs:167`; `main.cpp:369` |
| Gradient kind | Options · GRADIENT | `qml/Main.qml:1526` | selects `linearGradient` `:1201` / `radialGradient` `:1215` | works | `editor_core.rs:664` |
| Gradient start colour | Options · GRADIENT | `qml/Main.qml:1533` → `ColorDialog` `qml/Main.qml:284` | `stops[0]` on `gradient_fill` | works | `editor_core.rs:664` |
| Gradient end colour | Options · GRADIENT | `qml/Main.qml:1544` → `ColorDialog` `qml/Main.qml:290` | `stops[1]` on `gradient_fill` | works | `editor_core.rs:664` |
| Sampling mode | Options · CANVAS | `qml/Main.qml:1563` | validated `EditorBridge.cpp:425`, used by `resizeCanvas` `:1270` / `transformActive` `:1300` | works | `editor_core.rs:811` |
| Crop X/Y/W/H + Apply | Options · CANVAS | `qml/Main.qml:1583` (fields `:1572`, `:1577`, `:1590`, `:1595`) | `cropCanvas` `:1230` → `crop_canvas` | works | `editor_core.rs:762`, `:1063` |
| Pad L/T/R/B + Apply | Options · CANVAS | `qml/Main.qml:1618` (fields `:1607`, `:1612`, `:1626`, `:1631`) | `padCanvas` `:1247` → **`crop_canvas` with a negative origin** (no `pad` command exists) | works | `editor_core.rs:762` covers the transparent-extend behaviour padding relies on |
| Resize W/H + Apply | Options · CANVAS | `qml/Main.qml:1654` (fields `:1643`, `:1648`) | `resizeCanvas` `:1270` → `resize_canvas` | works | `editor_core.rs:811`, `:1703 resize_preflights_aggregate_cel_bytes_before_allocating` |
| Flip H | Options · CANVAS | `qml/Main.qml:1665` | `flipActive` `:1287` → `flip_active` | works | `editor_core.rs:811` |
| Flip V | Options · CANVAS | `qml/Main.qml:1672` | `flipActive` `:1287` | works | `editor_core.rs:811` |
| Rotate 90° counter-clockwise | Options · CANVAS | `qml/Main.qml:1678` | `rotateActive90` `:1294` → `rotate_active90` | works | `editor_core.rs:811` |
| Rotate 90° clockwise | Options · CANVAS | `qml/Main.qml:1685` | `rotateActive90` `:1294` | works | `editor_core.rs:811` |
| Affine m11/m12/m21/m22/tx/ty + Apply | Options · CANVAS | `qml/Main.qml:1736` (fields `:1696`, `:1701`, `:1715`, `:1720`, `:1706`, `:1725`) | `transformActive` `:1300` → `transform_active` | works | `editor_core.rs:811` |
| Invert filter | Options · FILTERS | `qml/Main.qml:1749` | `applyFilter` `:1315` → `apply_filter{invert}` | works | `editor_core.rs:907 all_new_filters_execute_respect_selection_and_validate_strictly` |
| Grayscale filter | Options · FILTERS | `qml/Main.qml:1756` | `applyFilter` `:1315` → `apply_filter{grayscale}` | works | `editor_core.rs:463 grayscale_brightness_contrast_and_blur_execute` |
| Clear layer | Options · FILTERS | `qml/Main.qml:1763` | `clearActiveLayer` `:731` → `clear` | works | `editor_core.rs:137` |
| Brightness / contrast + Apply | Options · FILTERS | `qml/Main.qml:1796` (spinboxes `:1771`, `:1782`) | `applyBrightnessContrast` `:1325` | works | `editor_core.rs:463`, `:1158 new_filter_channel_math_has_expected_reference_outputs` |
| Gaussian blur σ + Apply | Options · FILTERS | `qml/Main.qml:1812` (field `:1804`) | `applyGaussianBlur` `:1334` | works | `editor_core.rs:463`, `:907` |
| Threshold + Apply | Options · FILTERS | `qml/Main.qml:1832` (spinbox `:1820`) | `applyThreshold` `:1342` | works | `editor_core.rs:907`, `:1158` |
| Posterize + Apply | Options · FILTERS | `qml/Main.qml:1852` (spinbox `:1840`) | `applyPosterize` `:1350` | works | `editor_core.rs:907`, `:1158` |
| Levels (5 inputs) + Apply | Options · FILTERS | `qml/Main.qml:1897` (inputs `:1861`–`:1887`) | `applyLevels` `:1358` | works | `editor_core.rs:907`, `:1158` |
| Hue / saturation / lightness + Apply | Options · FILTERS | `qml/Main.qml:1933` (spinboxes `:1906`–`:1920`) | `applyHueSaturation` `:1371` | works | `editor_core.rs:907`, `:1255 every_new_filter_preserves_pixels_outside_selection` |
| Box blur + Apply | Options · FILTERS | `qml/Main.qml:1952` (spinbox `:1940`) | `applyBoxBlur` `:1381` | works | `editor_core.rs:907` |
| Sharpen + Apply | Options · FILTERS | `qml/Main.qml:1969` (field `:1961`) | `applySharpen` `:1389` | works | `editor_core.rs:907` |
| Blend mode selector | Options · ACTIVE LAYER | `qml/Main.qml:1976` | none — read by the button below | code-only | no `currentIndex` binding, so it never reflects the active layer — D1 |
| Set blend mode | Options · ACTIVE LAYER | `qml/Main.qml:1985` | `setLayerBlendMode` `:1130` → `set_layer_blend_mode` | works | `editor_core.rs:66`. No `enabled` guard — D2 |
| Agent prompt field | Agent tab | `qml/Main.qml:2036` | local text only | code-only | no test |
| Create proposal | Agent tab | `qml/Main.qml:2049` | `proposePrompt` `:1997` → offline `proposeLocally` `:1883`, or `redrob_agent_propose` `abi.rs:891` on a `QtConcurrent` thread (`EditorBridge.cpp:2024`) | works | `ffi_api.rs:811 agent_propose_rejects_null_malformed_empty_and_oversized_inputs_offline`; local path exercised by `main.cpp:600-700` through `enqueueProposal` |
| Reject proposal | Agent tab | `qml/Main.qml:2108` | `rejectProposal` `:2158` → `ProposalModel::reject` `native/qt/ProposalModel.cpp` | works | `main.cpp:640-660` (stale proposal stays inert) |
| Apply proposal | Agent tab | `qml/Main.qml:2115` | `applyProposal` `:2121` → `executeCommand` `:454` / `executeHistoryAction` `:487` with generation+epoch guard `EditorBridge.cpp:2136` | works | `main.cpp:600-730` (current / committed / stale / invalid / rejected / unavailable-redo cases) |

Status-only surfaces, not actions: the status bar (`qml/Main.qml:2145`), the document-size
label (`qml/Main.qml:391`), the mask badge (`qml/Main.qml:1253`) and the agent busy indicator
(`qml/Main.qml:2021`) are read-only projections of `EditorBridge` properties.

## Compatibility-matrix reachability (`docs/compatibility.md`)

23 rows are `native`, 10 `parity`, 12 `planned`.

| # | Native matrix row (`docs/compatibility.md`) | Reachable from UI | Entry |
|---|---|---|---|
| 1 | Document · layered editable project | yes | Open/Save project `qml/Main.qml:350`/`:362`; document created at `native/qt/EditorBridge.cpp:185` |
| 2 | Layers · raster layers, ordering, opacity, visibility | yes | Add raster layer `qml/Main.qml:999`; opacity `:1274`; visibility `:1241`; order `:1136`/`:1143` |
| 3 | Layers · isolated groups and raster masks | yes | Add group `qml/Main.qml:1004`; mask items `:1197`, `:1203`, `:1209`, `:1215` |
| 4 | Text · printable ASCII + newline, embedded font8x8 | yes | Add text `qml/Main.qml:1010` → `:881`; Edit text `:1151` |
| 5 | Vector · bounded paths, optional solid fill/stroke, non-zero/**even-odd** fill | **partial** | Rectangle `qml/Main.qml:1027`/`:940` and shapes `:583` only. No UI builds an arbitrary path, and `fill_rule` is hardcoded `non_zero` at `EditorBridge.cpp:844` and `:902`, so even-odd is unreachable |
| 6 | Vector · deterministic SVG subset (paths/rect/line/polyline/polygon, groups, namespaced text) | yes | Import `qml/Main.qml:356` (filter `:154`); Export format `svg` `:204` |
| 7 | History · grouped undo/redo with bounded memory | yes | `qml/Main.qml:379`, `:385`, `:296`, `:301` |
| 8 | Paint · pressure-aware round brush stroke | yes | Tool rail `qml/Main.qml:422` + canvas drag `:595` |
| 9 | Selection · grayscale mask; replace/add/subtract/intersect | yes | Tools `qml/Main.qml:435`/`:440`; mode `:1398` |
| 10 | Filters · invert, grayscale, blur | yes | `qml/Main.qml:1749`, `:1756`, `:1812`, `:1952` |
| 11 | Render · dirty-region projection | yes (implicit) | No control of its own; every mutation refreshes through `EditorBridge::refresh` `native/qt/EditorBridge.cpp:1405` → `redrob_editor_render_rgba` `abi.rs:1160` |
| 12 | Filters · parameterless GEGL ops behind `REDROB_ENABLE_GEGL` | **no** | `redrob_gegl_*` is referenced nowhere in `native/qt/`, `crates/` or `qml/`; capabilities JSON hardcodes `gegl ready=false` at `crates/redrob-ffi/src/abi.rs:443`. Only `native/adapters/gegl/redrob_gegl_adapter_test.c` calls it |
| 13 | Composite · normal, multiply, screen, overlay | yes | Blend selector `qml/Main.qml:1976` + `:1985` (also exposes `add`) |
| 14 | Color · embedded ICC profiles via lcms2 behind `REDROB_ENABLE_LCMS` | **no** | `redrob_lcms_*` referenced only by `native/adapters/lcms/redrob_lcms_adapter_test.c`; not even listed in the capabilities JSON (`abi.rs:442`–`:445` carries `gegl` and `krita` only) |
| 15 | Color · sRGB u8 ↔ linear float via babl behind `REDROB_ENABLE_BABL` | **no** | same: `native/adapters/babl/redrob_babl_adapter_test.c` only; absent from capabilities JSON |
| 16 | Files · RRG v1 import / v2 import-export **and exact RRG/PNG compatibility wrappers** | **partial** | v1/v2 project I/O yes (`qml/Main.qml:350`/`:362`); the wrapper routes `openFile`/`saveFile` (`native/qt/EditorBridge.h:241`–`:242`) have no QML caller |
| 17 | Files · strict generic PNG/JPEG/lossless-WebP, explicit-frame export, JPEG matte/quality | yes | Import `qml/Main.qml:356`; export dialog `:184`–`:247` → `:189` |
| 18 | Files · bounded ORA hierarchy/blend/offsets, deterministic ZIP/XML | yes | Import filter `qml/Main.qml:155`; export format `ora` `:204` |
| 19 | Files · limited deterministic SVG subset with loss warnings | yes | Export format `svg` `:204`; warnings surface in the status bar `:2145` |
| 20 | Animation · sparse raster frame timeline, frame CRUD, playback range/loop | yes | Timeline panel `qml/Main.qml:695`–`:785`, `:812` |
| 21 | Automation · typed command/procedure registry | **no** | `Q_INVOKABLE executeCommand` (`native/qt/EditorBridge.h:135`) has no QML caller; the registry is reachable only indirectly by approving an agent proposal (`qml/Main.qml:2115`), and directly only from the smoke harness |
| 22 | Agent · streaming Redrob tool calls | yes (conditional) | `qml/Main.qml:2049`; the live HTTP/SSE path runs only when `REDROB_API_KEY` is set (`native/qt/EditorBridge.cpp:156`), otherwise the offline deterministic path `:1883` runs instead |
| 23 | Agent · epoch-bound preview, per-proposal approval, undo/redo navigation | yes | Apply `qml/Main.qml:2115`, Reject `:2108` |

**Unreachable from the UI: 4** (#12 GEGL, #14 lcms2, #15 babl, #21 typed command registry).
**Partial: 2** (#5 arbitrary vector paths and the even-odd fill rule, #16 RRG/PNG compatibility
wrappers). #22 reachable only with an environment variable set.

## Defects

- **D1** `qml/Main.qml:1976` — the blend-mode `ComboBox` has no `currentIndex` binding to the active layer's `blendMode` role, so it always reads "normal" no matter what the active layer is.
- **D2** `qml/Main.qml:1985` — "Set blend mode" has no `enabled` guard; with D1 it silently rewrites a `multiply` layer to `normal`, and it fires on text/vector/group nodes too (`EditorBridge::setLayerBlendMode` `native/qt/EditorBridge.cpp:1130` validates only the mode string).
- **D3** `native/qt/EditorBridge.h:135` — `Q_INVOKABLE executeCommand` has no QML caller, so native matrix row 21 ("typed command/procedure registry") has no user entry point.
- **D4** `native/qt/EditorBridge.h:241`–`:242` — `openFile`/`saveFile` compatibility routes have no QML caller; the matrix row that advertises them is half-unreachable.
- **D5** `native/qt/EditorBridge.h:199` — `reorderLayer` has no QML caller (the UI reorders via `moveNode`); dead invokable.
- **D6** `native/qt/EditorBridge.h:191` — `replaceRasterMask` has no QML caller, so a mask can be created and toggled but never painted or edited.
- **D7** `native/qt/EditorBridge.h:246` — `enqueueProposal` has no QML caller; only `native/qt/main.cpp` uses it.
- **D8** `crates/redrob-core/src/command.rs:411` — `Command::FloodFill` (the Krita-parity bucket fill with Lab tolerance) has no UI entry: the rail's "F" tool commits `EditorBridge::fill` `native/qt/EditorBridge.cpp:725` at `qml/Main.qml:572`, which fills the whole layer or selection. No tolerance or spread control exists.
- **D9** `crates/redrob-core/src/command.rs:209` — `Filter::Curves` has no UI control; the FILTERS section (`qml/Main.qml:1742`–`:1971`) exposes the other 10 `Filter` variants only.
- **D10** `crates/redrob-core/src/command.rs:89` and `:79` — `BrushSettings::shape` (dab hardness/softness/aspect) and `::spacing` have no UI control; the in-source comment at `command.rs:87` says so outright ("a shape the tool surface cannot set yet").
- **D11** `crates/redrob-core/src/command.rs:397` — `BrushStroke::tip` (GBR/ABR image tips, three `parity` matrix rows) has no UI picker; nothing in `qml/Main.qml` mentions a tip.
- **D12** `crates/redrob-ffi/src/abi.rs:442`–`:445` — the capabilities JSON the UI consumes reports only `gegl` and `krita`, both `ready=false`; the two adapters that `docs/compatibility.md` calls operational (babl, lcms2) are absent, so the UI cannot even display them.
- **D13** `qml/Main.qml:70` — `commitShape` reads a shape's fill and stroke from the vector-rectangle dialog's `TextField`s (`qml/Main.qml:965`, `:969`, `:971`), which the shape tool never shows. Shape colour is therefore whatever the last vector-rectangle edit left behind (`qml/Main.qml:1174`–`:1177` overwrites those fields from the edited node).
- **D14** `qml/Main.qml:591` — the "I" inspect tool makes the gesture handler return before recording anything, so the tool only changes the cursor (`qml/Main.qml:625`); it inspects nothing.
- **D15** `native/qt/main.cpp:985` — the only QML-level test coverage is 6 driven handlers plus 20 `enabled`-property assertions (`native/qt/main.cpp:71`); the remaining ~110 `onClicked`/`onTriggered`/`onActivated` bindings are never executed by any test, so a handler calling a renamed or removed bridge method would still pass `ctest`.
- **D16** `qml/Main.qml:1274` — the layer-opacity `Slider` commits only in `onPressedChanged` (on release), so a keyboard-driven change (arrow keys, no press) never reaches `setLayerOpacity`.
- **D17** `qml/Main.qml:1143` — "Move toward bottom" passes `siblingIndex - 1`, i.e. `-1` at the bottom; harmless only because `EditorBridge::moveNode` clamps with `qMax(0, …)` at `native/qt/EditorBridge.cpp:1043` and the item is disabled at index 0.

## Row counts

| Status | Rows |
|---|---|
| works | 103 |
| code-only | 24 |
| none | 2 |
| broken | 0 |
| **total** | **129** |

Counts measured from this file's own action table (column 5), not carried from drafting.

