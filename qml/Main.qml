// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Redrob.Graphics 1.0

ApplicationWindow {
    id: window

    // Every colour in this file resolves through here. RedrobTokens.qml is generated from the
    // vendored design tokens by tools/generate_design_tokens.py -- see DESIGN_SYSTEM_PIN.json for why
    // this repository vendors them instead of installing @redrob-labs/ui like the other products.
    //
    // A plain object, not a singleton: a singleton needs a qmldir and an import path, and
    // native/qt/CMakeLists.txt registers QML through qt_add_resources.
    //
    // Referenced as `window.tokens.…` everywhere, never bare. A file-scope id reads as qualified at the
    // top level but NOT inside a delegate, so the bare form added one `[unqualified]` warning per
    // colour used in a delegate. The window qualifier resolves from every scope.
    // Light or dark. RedrobTokens carries both sets, but `dark: true` was fixed, so the light theme
    // was unreachable. It now follows the system unless the user picks one in the header, and the
    // pick is this session's: "" = follow the system, "light", "dark".
    property string themeChoice: ""
    readonly property bool systemPrefersLight: Qt.styleHints.colorScheme === Qt.ColorScheme.Light
    readonly property RedrobTokens tokens: RedrobTokens {
        dark: window.themeChoice === "" ? !window.systemPrefersLight : window.themeChoice === "dark"
    }


    // 1440x900 was a fixed size, taller than a 1920x904 display once the title bar and panels
    // are counted, so the timeline and the bottom of the options panel opened off screen.
    // Fit the preferred size inside the screen's available area instead.
    width: Screen.desktopAvailableWidth > 0 ? Math.min(1440, Math.round(Screen.desktopAvailableWidth * 0.92)) : 1440
    height: Screen.desktopAvailableHeight > 0 ? Math.min(900, Math.round(Screen.desktopAvailableHeight * 0.88)) : 900
    minimumWidth: 760
    minimumHeight: 540
    visible: true
    title: editor.currentFile.length > 0 ? "Redrob Canvas — " + editor.currentFile : "Redrob Canvas"
    color: window.tokens.surfaceBase
    // Tooltips draw from these palette roles; without them they were Qt's default yellow box in both
    // themes.
    palette.toolTipBase: window.tokens.surfaceRaised
    palette.toolTipText: window.tokens.inkPrimary
    palette.window: window.tokens.surfaceBase
    palette.windowText: window.tokens.inkPrimary
    palette.base: window.tokens.surfaceRaised
    palette.alternateBase: window.tokens.surfaceSunken
    palette.text: window.tokens.inkPrimary
    palette.button: window.tokens.surfaceRaised
    palette.buttonText: window.tokens.inkPrimary
    palette.highlight: window.tokens.actionPrimary
    palette.highlightedText: window.tokens.inkOnBrand

    // UI-3b: the menu bar. Every item calls the same function as the toolbar button, shortcut or
    // panel control it mirrors -- the menu adds a way to reach an action, never a second
    // implementation of it. Colours come through the window palette above, like every other
    // default control. Filters stays a pointer to the panel until UI-1 lists them here.
    menuBar: MainMenuBar {
        app: window
        canvasView: canvas
        sideTabs: tabs
        saveDialog: saveProjectDialog
        openDialog: openProjectDialog
        importFileDialog: importDialog
        exportDialog: exportOptionsDialog
        filters: filterBrowser
        textDialog: textSemanticDialog
        actionSaveDialog: saveActionDialog
        actionPlayDialog: playActionDialog
        shortcutsList: shortcutsDialog
        newDocument: newDocumentDialog
        proofDialog: proofProfileDialog
        cmykExport: cmykExportDialog
        artboardExport: artboardExportDialog
        sizeDialog: imageSizeDialog
        strokeDialog: strokeSelectionDialog
        colorRangeDialog: selectColorRangeDialog
    }

    property string activeTool: "brush"
    // Eraser, Clone and Smudge are the brush engine in a mode, so they paint exactly as the brush
    // does; picking one sets that mode, and leaving them returns the brush to plain painting.
    readonly property bool brushLike: ["brush", "mixer", "eraser", "clone", "heal", "smudge", "blur", "sharpen",
                                       "dodge", "burn"].indexOf(activeTool) >= 0
    // Tool groups: which member each rail cell shows, and every member's name and glyph for the
    // group menu. Both are reassigned rather than mutated, so bindings that read them update.
    // Each group starts on its first tool, as Photoshop's do. Spelled out because QML completes
    // children in no promised order, so "first to register" picked the LAST tool of every group.
    property var groupCurrent: ({ paint: "brush", stamp: "clone", fill: "gradient", focus: "blur", tone: "dodge", view: "hand" })
    property var groupTools: ({})
    function registerGroupTool(group, toolId, toolName, iconName) {
        const tools = Object.assign({}, groupTools);
        tools[group] = (tools[group] || []).concat([{ toolId: toolId, toolName: toolName, iconName: iconName }]);
        groupTools = tools;
    }
    function groupOf(toolId) {
        for (const group in groupTools)
            if (groupTools[group].some(tool => tool.toolId === toolId))
                return group;
        return "";
    }
    function openToolGroup(group, anchor) {
        // Listed in rail order. Spelled out because registration runs in completion order, which
        // QML does not promise (it came out reversed); a test holds this to the rail's file order.
        const order = toolGroupOrder[group] || [];
        toolGroupMenu.tools = (groupTools[group] || []).slice().sort(
            (a, b) => order.indexOf(a.toolId) - order.indexOf(b.toolId));
        toolGroupMenu.popup(anchor, anchor.width, 0);
    }
    readonly property var toolGroupOrder: ({
        paint: ["brush", "lazybrush", "mixer"], stamp: ["clone", "heal"], fill: ["gradient", "fill", "enclose"],
        focus: ["blur", "sharpen", "smudge"], tone: ["dodge", "burn"], view: ["hand", "rotateview"]
    })
    Menu {
        id: toolGroupMenu
        objectName: "toolGroupMenu"
        property var tools: []
        Instantiator {
            model: toolGroupMenu.tools
            delegate: MenuItem {
                required property var modelData
                text: modelData.toolName
                icon.source: "qrc:/icons/ui/" + modelData.iconName + ".svg"
                checkable: true
                checked: window.activeTool === modelData.toolId
                onTriggered: window.activeTool = modelData.toolId
            }
            onObjectAdded: (index, object) => toolGroupMenu.insertItem(index, object)
            onObjectRemoved: (index, object) => toolGroupMenu.removeItem(object)
        }
    }
    // The bucket tool's Lab tolerance, 0 to 255; 15 is the core's default.
    property int fillTolerance: 15
    // Live read-out of the measure tool: "<distance> px  <angle>°" while dragging, else empty.
    property string measureText: ""
    // Magic wand: flood only the connected region (true) or every matching pixel (false).
    property bool wandContiguous: true
    // The frame strip is for animation; a still image does not need 90px of it. Closed until there
    // is more than one frame, and adding or duplicating a frame opens it.
    property bool timelineOpen: editor.frameCount > 1
    property real canvasZoom: 1.0
    // L7: the n-point tool bends rigidly, as Photoshop's Puppet Warp (Edit menu toggle).
    property bool puppetRigid: true
    // L2: rulers (Ctrl+R) and guides (Ctrl+;), as Photoshop's View menu.
    property bool rulersVisible: false
    property bool guidesVisible: true
    // M9: the Navigator panel above the side tabs.
    property bool navigatorVisible: true
    property string selectionMode: "replace"
    property string gradientKind: "linear"
    // Shape tool. The kind is chosen before the drag, the way a gradient's kind is, because the
    // gesture that draws a star and the one that draws a rectangle are the same drag.
    property string shapeKind: "rectangle"
    property int shapeSides: 5
    property real shapeInnerRatio: 0.5
    property real shapeCornerRadius: 12

    // Which of the canvas's four existing preview kinds stands in for the shape being drawn.
    // Krita's colour sampler: press or drag on the canvas and the brush takes the colour under the
    // pointer. An empty pixel leaves the brush colour as it was.
    // Brush size runs 1..1000 px but almost all painting happens under 100. A linear slider gave
    // the first 100 px a tenth of the track (in the header's 110 px, ten pixels). Krita's size slider
    // is exponential for the same reason; this one is cubic, so 18 px sits a quarter along and
    // 100 px close to half.
    function sliderToBrushSize(position) {
        return Math.max(1, Math.round(1 + 999 * Math.pow(position, 3)));
    }
    function brushSizeToSlider(size) {
        return Math.cbrt(Math.max(0, (size - 1) / 999));
    }

    function pickColorAt(point) {
        const sampled = editor.sampleColor(point.x, point.y);
        if (sampled.valid)
            editor.brushColor = sampled;
    }

    function shapePreviewKind() {
        if (shapeKind === "ellipse")
            return "ellipse";
        if (shapeKind === "line")
            return "linear";
        if (shapeKind === "regular_polygon" || shapeKind === "star")
            return "radial";
        return "rectangle";
    }

    // Turn one drag into one shape. A two-corner shape reads the drag as opposite corners; a
    // polygon or star reads it as a centre and a radius, which is why the two bridge calls exist.
    function commitShape(start, end) {
        if (shapeKind === "regular_polygon" || shapeKind === "star") {
            const radius = Math.hypot(end.x - start.x, end.y - start.y);
            editor.addShapeFromRadius(shapeKind, shapeKind === "star" ? "Star" : "Polygon",
                                      start.x, start.y, radius, shapeSides, shapeInnerRatio,
                                      vectorSemanticDialog.fillColor, vectorSemanticDialog.strokeColor,
                                      vectorSemanticDialog.strokeWidth);
            return;
        }
        editor.addShapeFromBox(shapeKind, shapeKind === "ellipse" ? "Ellipse"
                                        : shapeKind === "line" ? "Line" : "Rectangle",
                               start.x, start.y, end.x, end.y, shapeCornerRadius,
                               vectorSemanticDialog.fillColor, vectorSemanticDialog.strokeColor,
                               vectorSemanticDialog.strokeWidth);
    }
    property color gradientStartColor: "#f4f6ff"
    property color gradientEndColor: "#4267c9"
    // Gamut mask (F.6).
    property bool gamutMaskOn: false
    property real gamutStart: 20
    property real gamutSpan: 120
    // Digital colour mixer (F.6): two source colours + a mix amount.
    property color mixerColorA: "#e03131"
    property color mixerColorB: "#1971c2"
    property string samplingMode: "bilinear"
    // Warp / liquify brush options.
    property string warpMode: "move"
    property real warpRadius: 40
    property real warpStrength: 0.5
    property string exportFormat: "png"
    property bool exportAllowLoss: false
    property int exportJpegQuality: 90
    property color exportMatte: "#ffffff"

    function maskNodeFromSelection(nodeId) {
        editor.rasterMaskFromSelection(nodeId)
    }

    onActiveToolChanged: {
        canvasPointer.cancelGesture();
        if (brushLike) {
            editor.brushErase = activeTool === "eraser";
            // Healing is the clone brush matched to its destination, so it needs the clone source.
            editor.brushClone = activeTool === "clone" || activeTool === "heal";
            editor.brushHeal = activeTool === "heal";
            editor.brushSmudge = activeTool === "smudge";
            editor.brushMixer = activeTool === "mixer";
            editor.brushConvolveMode = activeTool === "blur" || activeTool === "sharpen" ? activeTool : "off";
            editor.brushDodgeBurnMode = activeTool === "dodge" || activeTool === "burn" ? activeTool : "off";
        }
        // A tool picked by shortcut or menu becomes the one its group cell shows.
        const group = groupOf(activeTool);
        if (group.length > 0 && groupCurrent[group] !== activeTool) {
            const current = Object.assign({}, groupCurrent);
            current[group] = activeTool;
            groupCurrent = current;
        }
        // Selecting the perspective tool arms its frame; leaving it hands the overlay back. Done here
        // rather than in the tool button so a keyboard shortcut behaves the same as a click.
        if (window.activeTool === "perspective")
            canvasPointer.resetPerspectiveCorners();
        canvasPointer.syncHandles();
    }











    FileDialog {
        id: brushTipDialog
        title: "Load brush tips"
        fileMode: FileDialog.OpenFile
        nameFilters: ["Brushes (*.gbr *.abr)", "GIMP brush (*.gbr)", "Photoshop brushes (*.abr)"]
        onAccepted: editor.loadBrushTips(selectedFile)
    }

    FileDialog {
        id: openProjectDialog
        title: "Open"
        fileMode: FileDialog.OpenFile
        // Open takes any file the engine reads; a project opens as a project, the rest import.
        nameFilters: ["All supported (*.rrg *.psd *.kra *.xcf *.ora *.png *.jpg *.jpeg *.webp *.tif *.tiff *.svg)",
                      "Redrob projects (*.rrg)", "Photoshop documents (*.psd)",
                      "Krita documents (*.kra)", "GIMP images (*.xcf)"]
        onAccepted: editor.openFile(selectedFile)
    }
    FileDialog {
        id: importDialog
        title: "Import Interchange File"
        fileMode: FileDialog.OpenFile
        nameFilters: ["Supported (*.png *.jpg *.jpeg *.webp *.tif *.tiff *.ora *.svg *.psd *.kra *.xcf)",
                      "PNG images (*.png)", "JPEG images (*.jpg *.jpeg)",
                      "Lossless WebP images (*.webp)", "TIFF images (*.tif *.tiff)",
                      "OpenRaster documents (*.ora)", "Limited SVG documents (*.svg)",
                      "Photoshop documents (*.psd)", "Krita documents (*.kra)",
                      "GIMP images (*.xcf)"]
        onAccepted: editor.importFile(selectedFile)
    }
    FileDialog {
        id: saveProjectDialog
        title: "Save Redrob Project As"
        fileMode: FileDialog.SaveFile
        defaultSuffix: "rrg"
        nameFilters: ["Redrob projects (*.rrg)"]
        onAccepted: editor.saveProject(selectedFile)
    }
    // P14: actions are plain JSON files of recorded edits.
    FileDialog {
        id: saveActionDialog
        title: "Save Action"
        fileMode: FileDialog.SaveFile
        defaultSuffix: "rraction"
        nameFilters: ["Redrob actions (*.rraction)", "JSON (*.json)"]
        onAccepted: {
            const path = selectedFile.toString()
            const base = path.substring(path.lastIndexOf("/") + 1).replace(/\.[^.]*$/, "")
            editor.saveAction(selectedFile, decodeURIComponent(base))
        }
    }
    FileDialog {
        id: playActionDialog
        title: "Play Action"
        fileMode: FileDialog.OpenFile
        nameFilters: ["Redrob actions (*.rraction)", "JSON (*.json)"]
        onAccepted: editor.playActionFile(selectedFile)
    }
    FileDialog {
        id: exportFileDialog
        title: "Export Current Frame"
        fileMode: FileDialog.SaveFile
        defaultSuffix: window.exportFormat === "jpeg" ? "jpg" : window.exportFormat
        nameFilters: window.exportFormat === "png" ? ["PNG images (*.png)"]
            : window.exportFormat === "jpeg" ? ["JPEG images (*.jpg *.jpeg)"]
            : window.exportFormat === "webp" ? ["Lossless WebP images (*.webp)"]
            : window.exportFormat === "ora" ? ["OpenRaster documents (*.ora)"]
            : ["Limited SVG documents (*.svg)"]
        onAccepted: editor.exportFile(selectedFile, window.exportFormat,
                                      window.exportAllowLoss, editor.currentFrame,
                                      window.exportJpegQuality, window.exportMatte)
    }
    Dialog {
        id: exportOptionsDialog
        objectName: "exportOptionsDialog"
        anchors.centerIn: parent
        modal: true
        title: "Export Current Frame"
        standardButtons: Dialog.Ok | Dialog.Cancel
        onAccepted: exportFileDialog.open()
        ColumnLayout {
            width: 390
            spacing: 10
            Label {
                Layout.fillWidth: true
                text: "Choose an interchange format. Export never changes the project path or current frame."
                wrapMode: Text.Wrap
                color: window.tokens.inkSecondary
            }
            RowLayout {
                Layout.fillWidth: true
                Label { text: "Format"; Layout.preferredWidth: 90 }
                ComboBox {
                    id: exportFormatControl
                    objectName: "exportFormatControl"
                    Layout.fillWidth: true
                    model: ["png", "jpeg", "webp", "ora", "svg"]
                    currentIndex: model.indexOf(window.exportFormat)
                    onActivated: window.exportFormat = currentText
                }
            }
            CheckBox {
                id: allowLossControl
                objectName: "allowLossControl"
                Layout.fillWidth: true
                text: "Allow documented representational loss"
                checked: window.exportAllowLoss
                onToggled: window.exportAllowLoss = checked
            }
            Label {
                Layout.fillWidth: true
                text: window.exportAllowLoss
                    ? "Allowed loss is returned as machine-readable warnings in the status bar."
                    : "Strict mode rejects omitted frames, selection, metadata, flattening, and rasterization."
                color: window.exportAllowLoss ? window.tokens.statusWarning : window.tokens.inkSecondary
                wrapMode: Text.Wrap
                font.pixelSize: 11
            }
            RowLayout {
                Layout.fillWidth: true
                visible: window.exportFormat === "jpeg"
                Label { text: "JPEG quality"; Layout.preferredWidth: 90 }
                SpinBox {
                    objectName: "jpegQualityControl"
                    from: 1
                    to: 100
                    value: window.exportJpegQuality
                    editable: true
                    onValueModified: window.exportJpegQuality = value
                }
            }
            RowLayout {
                Layout.fillWidth: true
                visible: window.exportFormat === "jpeg"
                Label { text: "Opaque matte"; Layout.preferredWidth: 90 }
                Button {
                    text: "Choose…"
                    onClicked: jpegMatteDialog.open()
                }
                Rectangle {
                    width: 28
                    height: 22
                    radius: 4
                    color: window.exportMatte
                    border.color: window.tokens.borderStrong
                }
            }
            Label {
                Layout.fillWidth: true
                visible: window.exportFormat === "jpeg"
                text: "JPEG has no alpha. Transparent pixels are explicitly composited over this opaque matte; alpha is never silently dropped."
                color: window.tokens.statusWarning
                wrapMode: Text.Wrap
                font.pixelSize: 11
            }
        }
    }
    // The searchable filter list (UI-1); lives in FilterBrowser.qml since P12.
    ShortcutsDialog {
        id: shortcutsDialog
        tokens: window.tokens
    }
    NewDocumentDialog {
        id: newDocumentDialog
        tokens: window.tokens
        backgroundColor: window.backgroundColor
    }
    ColorRangeDialog {
        id: selectColorRangeDialog
        tokens: window.tokens
        selectionMode: window.selectionMode
    }
    BlendIfDialog {
        id: blendIfDialog
        tokens: window.tokens
    }
    StrokeDialog {
        id: strokeSelectionDialog
        tokens: window.tokens
    }
    SizeDialog {
        id: imageSizeDialog
        tokens: window.tokens
        samplingMode: window.samplingMode
    }
    FilterBrowser {
        id: filterBrowser
        tokens: window.tokens
    }
    // The node dialogs (P12). At window level: the layer panel, the text tool and the menu bar
    // all open them.
    TextNodeDialog {
        id: textSemanticDialog
    }
    VectorRectDialog {
        id: vectorSemanticDialog
    }
    ColorDialog {
        id: jpegMatteDialog
        title: "Choose opaque JPEG matte"
        selectedColor: window.exportMatte
        onAccepted: window.exportMatte = Qt.rgba(selectedColor.r, selectedColor.g,
                                                selectedColor.b, 1)
    }
    ColorDialog {
        id: brushColorDialog
        title: "Choose brush color"
        selectedColor: editor.brushColor
        onAccepted: editor.brushColor = selectedColor
    }
    ColorDialog {
        id: gradientStartDialog
        title: "Choose gradient start color"
        selectedColor: window.gradientStartColor
        onAccepted: window.gradientStartColor = selectedColor
    }
    ColorDialog {
        id: gradientEndDialog
        title: "Choose gradient end color"
        selectedColor: window.gradientEndColor
        onAccepted: window.gradientEndColor = selectedColor
    }

    Shortcut {
        sequences: [StandardKey.Undo]
        enabled: editor.canUndo
        onActivated: editor.undo()
    }
    Shortcut {
        sequences: [StandardKey.Redo]
        enabled: editor.canRedo
        onActivated: editor.redo()
    }
    Shortcut {
        sequences: [StandardKey.Open]
        onActivated: openProjectDialog.open()
    }
    Shortcut {
        sequences: [StandardKey.Save]
        onActivated: editor.currentFile.length > 0 ? editor.saveProject() : saveProjectDialog.open()
    }

    // ---- Photoshop keyboard layout (S2) ----
    // Tool keys, Photoshop's: a letter picks the tool last used in that letter's group, Shift+letter
    // steps to the next tool in it (Shift+M: rectangle -> ellipse). Only groups with more than one
    // tool get a Shift binding, so Shift+E stays free for the erase-mode toggle below. Tools with no
    // Photoshop key (blur, sharpen, smudge, cage, warp, n-point, align, inspect, perspective) are
    // reached from the rail; Ctrl+T opens the perspective/transform handles.
    readonly property var psToolKeys: ({
        "V": ["transform"],
        "M": ["rectangle", "ellipse"],
        "L": ["lasso", "polygon", "scissors"],
        "W": ["wand", "fgselect"],
        "C": ["crop"],
        "I": ["picker", "measure"],
        "J": ["heal"],
        "B": ["brush", "lazybrush", "mixer"],
        "S": ["clone"],
        "E": ["eraser"],
        "G": ["gradient", "fill", "enclose"],
        "O": ["dodge", "burn"],
        "P": ["pen"],
        "T": ["text"],
        "U": ["shape"],
        "H": ["hand"],
        "R": ["rotateview"],
        "Z": ["zoom"]
    })
    // The tool each letter currently selects (the last one picked in its group).
    property var psKeyCurrent: ({})
    function psKeyHint(toolId) {
        for (const key in psToolKeys) {
            const tools = psToolKeys[key];
            const index = tools.indexOf(toolId);
            if (index === 0)
                return tools.length > 1 ? key + "  (Shift+" + key + ")" : key;
            if (index > 0)
                return "Shift+" + key;
        }
        return "";
    }
    function selectPsTool(key, cycle) {
        const tools = psToolKeys[key];
        const current = psKeyCurrent[key] || tools[0];
        let next = current;
        if (cycle)
            next = tools[(tools.indexOf(current) + 1) % tools.length];
        else if (tools.indexOf(activeTool) >= 0)
            next = activeTool;
        const remembered = Object.assign({}, psKeyCurrent);
        remembered[key] = next;
        psKeyCurrent = remembered;
        activeTool = next;
    }
    // Repeaters, not Instantiators: a Shortcut finds its window through its parent item.
    Item {
        visible: false
        Repeater {
            model: Object.keys(window.psToolKeys)
            delegate: Item {
                required property string modelData
                Shortcut {
                    sequence: modelData
                    onActivated: window.selectPsTool(modelData, false)
                }
                Shortcut {
                    sequence: "Shift+" + modelData
                    enabled: window.psToolKeys[modelData].length > 1
                    onActivated: window.selectPsTool(modelData, true)
                }
            }
        }
    }

    // Brush size with [ and ], in Photoshop's steps (finer when small). Shift+[ / Shift+] change
    // hardness by 25%.
    function brushSizeStep(size) {
        return size < 10 ? 1 : size < 50 ? 5 : size < 100 ? 10 : size < 300 ? 25 : 50;
    }
    Shortcut {
        sequence: "]"
        onActivated: editor.brushSize = Math.min(1000, editor.brushSize + window.brushSizeStep(editor.brushSize))
    }
    Shortcut {
        sequence: "["
        onActivated: editor.brushSize = Math.max(1, editor.brushSize - window.brushSizeStep(editor.brushSize - 1))
    }
    Shortcut {
        sequences: ["Shift+]", "}"]
        onActivated: editor.brushHardness = Math.min(1, editor.brushHardness + 0.25)
    }
    Shortcut {
        sequences: ["Shift+[", "{"]
        onActivated: editor.brushHardness = Math.max(0, editor.brushHardness - 0.25)
    }
    // Digits set opacity: 1 = 10% ... 9 = 90%, 0 = 100%. The brush's while a painting tool is
    // active, the active layer's otherwise -- Photoshop's rule.
    Item {
        visible: false
        Repeater {
            model: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"]
            delegate: Item {
                required property string modelData
                Shortcut {
                    sequence: modelData
                    onActivated: {
                        const value = modelData === "0" ? 1.0 : Number(modelData) / 10;
                        if (window.brushLike)
                            editor.brushOpacity = value;
                        else if (editor.activeLayerId.length > 0)
                            editor.setLayerOpacity(editor.activeLayerId, value);
                    }
                }
                // M3: Shift+digit sets the brush flow. On a US layout Shift+1 arrives as "!", so
                // both spellings are bound (index lines up with the digit row).
                Shortcut {
                    readonly property string shifted: "!@#$%^&*()"["1234567890".indexOf(modelData)]
                    sequences: ["Shift+" + modelData, shifted]
                    onActivated: editor.brushFlow = modelData === "0" ? 1.0 : Number(modelData) / 10
                }
            }
        }
    }
    // Foreground/background colours. The background colour feeds Ctrl+Backspace and X.
    property color backgroundColor: "#ffffffff"
    Shortcut {
        sequence: "X"
        onActivated: {
            const foreground = editor.brushColor;
            editor.brushColor = window.backgroundColor;
            window.backgroundColor = foreground;
        }
    }
    Shortcut {
        sequence: "D"
        onActivated: {
            editor.brushColor = "#ff000000";
            window.backgroundColor = "#ffffffff";
        }
    }
    // Selection.
    Shortcut { sequence: "Ctrl+A"; onActivated: editor.selectAll() }
    Shortcut { sequence: "Ctrl+D"; onActivated: editor.clearSelection() }
    Shortcut { sequence: "Ctrl+Shift+I"; onActivated: editor.invertSelection() }
    // Layers. Photoshop's Ctrl+G groups the SELECTED layers; this adds an empty group, since the
    // engine has no multi-layer selection yet.
    Shortcut { sequence: "Ctrl+Shift+N"; onActivated: editor.addLayer() }
    Shortcut { sequence: "Ctrl+G"; onActivated: editor.groupSelectedLayers() }
    Shortcut { sequence: "Ctrl+Alt+G"; enabled: editor.activeLayerId.length > 0; onActivated: editor.toggleClippingMask() }
    Shortcut {
        sequence: "Ctrl+J"
        enabled: editor.activeLayerId.length > 0
        onActivated: editor.duplicateLayer(editor.activeLayerId)
    }
    Shortcut {
        sequence: "Ctrl+E"
        enabled: editor.activeLayerId.length > 0
        onActivated: editor.mergeDown(editor.activeLayerId)
    }
    Shortcut { sequence: "Ctrl+Shift+E"; onActivated: editor.mergeVisible() }
    Shortcut { sequence: "Ctrl+T"; onActivated: window.activeTool = "perspective" }
    // File.
    Shortcut { sequence: "Ctrl+Shift+S"; onActivated: saveProjectDialog.open() }
    Shortcut { sequences: [StandardKey.New]; onActivated: newDocumentDialog.openNew() }
    Shortcut { sequence: "Ctrl+Alt+I"; onActivated: imageSizeDialog.openFor("image") }
    Shortcut { sequence: "Ctrl+Alt+C"; onActivated: imageSizeDialog.openFor("canvas") }
    Shortcut { sequence: "Ctrl+Y"; onActivated: editor.proofColors = !editor.proofColors }
    Shortcut { sequence: "Ctrl+Shift+Y"; onActivated: editor.proofGamutWarning = !editor.proofGamutWarning }
    FolderDialog {
        id: artboardExportDialog
        title: "Export artboards to folder"
        onAccepted: editor.exportArtboards(selectedFolder)
    }
    FileDialog {
        id: cmykExportDialog
        title: "Export CMYK TIFF"
        fileMode: FileDialog.SaveFile
        defaultSuffix: "tif"
        nameFilters: ["CMYK TIFF (*.tif *.tiff)"]
        onAccepted: editor.exportCmykTiff(selectedFile)
    }
    FileDialog {
        id: proofProfileDialog
        title: "Proof setup: choose a CMYK profile"
        nameFilters: ["ICC profiles (*.icc *.icm)", "All files (*)"]
        onAccepted: editor.loadProofProfile(selectedFile, 1)
    }
    Shortcut { sequence: "Ctrl+R"; onActivated: window.rulersVisible = !window.rulersVisible }
    Shortcut { sequence: "Ctrl+;"; onActivated: window.guidesVisible = !window.guidesVisible }
    Shortcut { sequence: "Shift+F5"; enabled: editor.activeNodeCanEditRaster; onActivated: editor.contentAwareFill() }
    // Edit > Copy / Cut / Paste (H5). Text fields keep their own Ctrl+C/X/V: a focused input
    // takes the key first.
    Shortcut { sequences: [StandardKey.Copy]; onActivated: editor.copySelection() }
    Shortcut { sequences: [StandardKey.Cut]; enabled: editor.activeNodeCanEditRaster; onActivated: editor.cutSelection() }
    Shortcut { sequences: [StandardKey.Paste]; onActivated: editor.pasteClipboard() }
    // View.
    function fitCanvasToView() {
        if (editor.documentWidth <= 0 || editor.documentHeight <= 0)
            return;
        window.canvasZoom = Math.max(0.05, Math.min(32, 0.95 * Math.min(canvas.width / editor.documentWidth,
                                                                          canvas.height / editor.documentHeight)));
        canvas.pan = Qt.point(0, 0);
    }
    Shortcut { sequence: "Ctrl+0"; onActivated: window.fitCanvasToView() }
    Shortcut {
        sequence: "Ctrl+1"
        onActivated: { window.canvasZoom = 1; canvas.pan = Qt.point(0, 0) }
    }
    Shortcut {
        sequences: ["Ctrl+=", "Ctrl++", StandardKey.ZoomIn]
        onActivated: window.canvasZoom = Math.min(32, window.canvasZoom * 1.2)
    }
    Shortcut {
        sequences: ["Ctrl+-", StandardKey.ZoomOut]
        onActivated: window.canvasZoom = Math.max(0.05, window.canvasZoom / 1.2)
    }
    property bool panelsHidden: false
    Shortcut { sequence: "Tab"; onActivated: window.panelsHidden = !window.panelsHidden }
    // Image > Adjustments. Ctrl+I and Ctrl+Shift+U apply at once; the others open the filter
    // window on that adjustment with its defaults, as Photoshop opens its dialog.
    Shortcut { sequence: "Ctrl+I"; enabled: editor.activeNodeCanEditRaster; onActivated: editor.applyFilter("invert") }
    Shortcut { sequence: "Ctrl+Shift+U"; enabled: editor.activeNodeCanEditRaster; onActivated: editor.applyFilter("grayscale") }
    Shortcut { sequence: "Ctrl+L"; onActivated: filterBrowser.openFor("levels") }
    Shortcut { sequence: "Ctrl+M"; onActivated: filterBrowser.openFor("curves") }
    Shortcut { sequence: "Ctrl+U"; onActivated: filterBrowser.openFor("hue_saturation") }
    Shortcut { sequence: "Ctrl+B"; onActivated: filterBrowser.openFor("color_balance") }
    // Delete clears (the selection, when there is one, as the clear command does); Alt+Backspace
    // fills with the foreground colour and Ctrl+Backspace with the background colour.
    Shortcut { sequences: ["Delete", "Backspace"]; enabled: editor.activeNodeCanEditRaster; onActivated: editor.clearActiveLayer() }
    Shortcut { sequence: "Alt+Backspace"; enabled: editor.activeNodeCanEditRaster; onActivated: editor.fill(editor.brushColor) }
    Shortcut { sequence: "Ctrl+Backspace"; enabled: editor.activeNodeCanEditRaster; onActivated: editor.fill(window.backgroundColor) }
    Shortcut { sequence: "F1"; onActivated: shortcutsDialog.open() }

    // Space held: the hand tool; Alt held while painting: the eyedropper. The bridge reads the
    // key state from raw key events (keys typed into a text field are left alone) and the tool
    // returns when the key is released, as in Photoshop.
    property string heldToolReturn: ""
    function holdTool(tool, held) {
        if (held) {
            if (heldToolReturn.length === 0 && activeTool !== tool && !canvasPointer.gestureActive) {
                heldToolReturn = activeTool;
                activeTool = tool;
            }
        } else if (heldToolReturn.length > 0 && activeTool === tool) {
            activeTool = heldToolReturn;
            heldToolReturn = "";
        }
    }
    Connections {
        target: editor
        function onHeldKeysChanged() {
            window.holdTool("hand", editor.spaceHeld);
            if (!editor.altHeld || window.brushLike || window.activeTool === "fill"
                    || window.activeTool === "gradient")
                window.holdTool("picker", editor.altHeld);
        }
    }
    // Turn the view about the middle of the canvas area, so the part the user is looking at stays.
    function rotateView(degrees) {
        const middle = Qt.point(canvas.width / 2, canvas.height / 2);
        const held = canvas.canvasPoint(middle);
        canvas.viewRotation = canvas.viewRotation + degrees;
        canvas.anchorCanvasPoint(held, middle);
    }
    Shortcut {
        // Shift+E flips erase mode while painting; E itself picks the Eraser tool on the rail.
        sequence: "Shift+E"
        enabled: editor.activeNodeCanEditRaster
        onActivated: {
            window.activeTool = "brush";
            editor.brushErase = !editor.brushErase;
        }
    }
    Shortcut {
        sequence: "Escape"
        onActivated: {
            canvasPointer.polyPoints = [];
            canvasPointer.penHandles = [];
            canvasPointer.penDragging = false;
            canvasPointer.fgMarks = [];
            canvasPointer.bgMarks = [];
            canvasPointer.cageSrc = [];
            canvasPointer.cageDst = [];
            canvasPointer.cageGrab = -1;
            canvasPointer.npSrc = [];
            canvasPointer.npDst = [];
            canvasPointer.npGrab = -1;
            canvasPointer.lazyMarks = [];
            // Escape resets the perspective frame to the image's own corners rather than clearing it:
            // the tool is still selected, and an empty frame would leave nothing to grab.
            if (window.activeTool === "perspective")
                canvasPointer.resetPerspectiveCorners();
            canvasPointer.syncHandles();
            window.measureText = "";
            canvas.clearPreview();
            canvasPointer.cancelGesture();
        }
    }
    Shortcut {
        // Close an in-progress polygon/scissors selection, or apply the foreground scribbles.
        sequences: ["Return", "Enter"]
        enabled: ((window.activeTool === "polygon" || window.activeTool === "scissors" || window.activeTool === "pen")
                      && canvasPointer.polyPoints.length >= 4)
                 || (window.activeTool === "cage" && canvasPointer.cageSrc.length === 0
                      && canvasPointer.polyPoints.length >= 4)
                 || (window.activeTool === "npoint" && canvasPointer.npSrc.length === 0
                      && canvasPointer.polyPoints.length >= 4)
                 || (window.activeTool === "fgselect" && canvasPointer.fgMarks.length >= 2)
                 || (window.activeTool === "lazybrush" && canvasPointer.lazyMarks.length >= 6)
        onActivated: {
            if (window.activeTool === "fgselect")
                canvasPointer.applyForeground();
            else if (window.activeTool === "lazybrush")
                canvasPointer.applyLazybrush();
            else
                canvasPointer.closePolygon();
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 52
            color: window.tokens.surfaceRaised
            border.color: window.tokens.borderSubtle
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 12
                anchors.rightMargin: 12
                spacing: 3
                Image {
                    source: "qrc:/icons/redrob-canvas.svg"
                    sourceSize: Qt.size(28, 28)
                    Layout.rightMargin: 7
                    Accessible.name: "Redrob Canvas"
                }
                Label {
                    // 10-logo.md: "Redrob Desk" at first mention on every surface, then "Desk"; and the
                    // name is never in the lockup's letterforms, boxed, its own colour or abbreviated.
                    // A tracked-out all-caps "REDROB" was both the wrong name and the wrong treatment.
                    text: "Redrob Canvas"
                    font.pixelSize: 14
                    font.weight: Font.DemiBold
                    color: window.tokens.inkPrimary
                    Layout.rightMargin: 14
                }
                CommandButton {
                    objectName: "openProjectAction"
                    text: "Open"
                    iconName: "folderOpen"
                    ToolTip.text: "Open an editable RRG project (Ctrl+O)"
                    onClicked: openProjectDialog.open()
                }
                CommandButton {
                    objectName: "importFileAction"
                    text: "Import"
                    iconName: "image"
                    ToolTip.text: "Import PNG, JPEG, lossless WebP, ORA, or limited SVG"
                    onClicked: importDialog.open()
                }
                CommandButton {
                    objectName: "saveProjectAction"
                    text: "Save"
                    iconName: "save"
                    ToolTip.text: "Save the current RRG project (Ctrl+S)"
                    onClicked: editor.currentFile.length > 0 ? editor.saveProject() : saveProjectDialog.open()
                }
                CommandButton {
                    objectName: "saveProjectAsAction"
                    text: "Save As"
                    ToolTip.text: "Choose a new RRG project path"
                    onClicked: saveProjectDialog.open()
                }
                Rectangle {
                    Layout.preferredWidth: 1
                    Layout.preferredHeight: 20
                    Layout.leftMargin: 6
                    Layout.rightMargin: 6
                    color: window.tokens.borderSubtle
                }
                CommandButton {
                    text: "Undo"
                    iconName: "undo"
                    iconOnly: true
                    enabled: editor.canUndo
                    ToolTip.text: "Undo (Ctrl+Z)"
                    onClicked: editor.undo()
                }
                CommandButton {
                    text: "Redo"
                    iconName: "redo"
                    iconOnly: true
                    enabled: editor.canRedo
                    ToolTip.text: "Redo (Ctrl+Shift+Z)"
                    onClicked: editor.redo()
                }
                // Krita keeps the brush's size and opacity in the top tool bar, so they can change
                // mid-painting without opening the options panel. Shown for the brush only (they mean
                // nothing to the other tools) and only when the window is wide enough to keep the
                // header on one line; the Options panel has the same two controls either way.
                RowLayout {
                    id: headerBrushStrip
                    objectName: "headerBrushStrip"
                    visible: window.brushLike && window.width >= 1120
                    spacing: 6
                    Rectangle {
                        Layout.preferredWidth: 1
                        Layout.preferredHeight: 20
                        Layout.leftMargin: 6
                        Layout.rightMargin: 6
                        color: window.tokens.borderSubtle
                    }
                    Label {
                        text: editor.brushErase ? "Eraser" : "Size"
                        color: window.tokens.inkSecondary
                        font.pixelSize: 12
                    }
                    TokenSlider {
                        objectName: "headerBrushSize"
                        Layout.preferredWidth: 110
                        from: 0
                        to: 1
                        value: window.brushSizeToSlider(editor.brushSize)
                        Accessible.name: "Brush size"
                        onMoved: editor.brushSize = window.sliderToBrushSize(value)
                    }
                    Label {
                        text: Math.round(editor.brushSize) + " px"
                        Layout.preferredWidth: 44
                        horizontalAlignment: Text.AlignRight
                        font.features: { "tnum": 1 }
                        font.pixelSize: 12
                    }
                    Label {
                        text: "Opacity"
                        Layout.leftMargin: 10
                        color: window.tokens.inkSecondary
                        font.pixelSize: 12
                    }
                    TokenSlider {
                        objectName: "headerBrushOpacity"
                        Layout.preferredWidth: 90
                        from: 0
                        to: 1
                        value: editor.brushOpacity
                        Accessible.name: "Brush opacity"
                        onMoved: editor.brushOpacity = value
                    }
                    Label {
                        text: Math.round(editor.brushOpacity * 100) + "%"
                        Layout.preferredWidth: 40
                        horizontalAlignment: Text.AlignRight
                        font.features: { "tnum": 1 }
                        font.pixelSize: 12
                    }
                }
                Item {
                    Layout.fillWidth: true
                }
                Label {
                    text: editor.documentWidth + " × " + editor.documentHeight + " px"
                    color: window.tokens.inkSecondary
                    font.pixelSize: 12
                    Layout.rightMargin: 8
                    Accessible.name: "Document size " + editor.documentWidth + " by " + editor.documentHeight + " pixels"
                }
                CommandButton {
                    objectName: "themeToggleAction"
                    text: window.tokens.dark ? "Light" : "Dark"
                    iconName: window.tokens.dark ? "sun" : "moon"
                    iconOnly: true
                    ToolTip.text: window.tokens.dark ? "Switch to the light theme" : "Switch to the dark theme"
                    onClicked: window.themeChoice = window.tokens.dark ? "light" : "dark"
                }
                CommandButton {
                    objectName: "exportCurrentFrameAction"
                    text: "Export"
                    iconName: "download"
                    primary: true
                    Layout.leftMargin: 6
                    ToolTip.text: "Export the current frame with explicit loss and JPEG options"
                    onClicked: exportOptionsDialog.open()
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            Rectangle {
                Layout.preferredWidth: 96
                Layout.fillHeight: true
                // Tab hides the panels, as in Photoshop (S2).
                visible: !window.panelsHidden
                color: window.tokens.surfaceRaised
                border.color: window.tokens.borderSubtle
                ScrollView {
                    anchors.fill: parent
                    anchors.topMargin: 6
                    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
                    // Two columns, grouped and ordered as Photoshop's toolbar: move and select,
                    // measure, paint, draw and type, distort, view. A divider spans both columns.
                    GridLayout {
                        width: 88
                        columns: 2
                        rowSpacing: 0
                        columnSpacing: 2
                        ToolRailButton {
                            objectName: "transformToolAction"
                            iconName: "transform"
                            toolId: "transform"
                            toolName: "Move layer"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Move requires a raster node"
                        }
                        ToolRailButton {
                            // Align: buttons in the Options panel align the active layer.
                            iconName: "align"
                            toolId: "align"
                            toolName: "Align layer"
                        }
                        ToolRailButton {
                            iconName: "rectangle"
                            toolId: "rectangle"
                            toolName: "Rectangle selection"
                        }
                        ToolRailButton {
                            iconName: "ellipse"
                            toolId: "ellipse"
                            toolName: "Ellipse selection"
                        }
                        ToolRailButton {
                            iconName: "lasso"
                            toolId: "lasso"
                            toolName: "Free selection (lasso)"
                        }
                        ToolRailButton {
                            // Click to drop vertices; Enter or a click near the start closes and selects.
                            iconName: "polygon"
                            toolId: "polygon"
                            toolName: "Polygon selection"
                        }
                        ToolRailButton {
                            // Intelligent scissors: click anchors, the boundary snaps to edges.
                            iconName: "scissors"
                            toolId: "scissors"
                            toolName: "Intelligent scissors"
                        }
                        ToolRailButton {
                            // Foreground select: scribble over the subject (drag) and the background
                            // (Shift-drag), then Enter. The glyph says so: a plus inside the subject, a
                            // minus outside it.
                            iconName: "fgselect"
                            toolId: "fgselect"
                            toolName: "Foreground select"
                        }
                        ToolRailButton {
                            // Magic wand: flood-select by colour from the click.
                            iconName: "wand"
                            toolId: "wand"
                            toolName: "Select by colour (wand)"
                        }
                        ToolRailButton {
                            iconName: "crop"
                            toolId: "crop"
                            toolName: "Crop canvas"
                        }
                        RailDivider { Layout.columnSpan: 2; Layout.preferredWidth: 64 }
                        ToolRailButton {
                            objectName: "pickerToolAction"
                            iconName: "eyedropper"
                            toolId: "picker"
                            toolName: "Pick colour"
                        }
                        ToolRailButton {
                            // Measure: drag to read distance and angle in the status bar. Read-only.
                            iconName: "measure"
                            toolId: "measure"
                            toolName: "Measure (distance and angle)"
                        }
                        RailDivider { Layout.columnSpan: 2; Layout.preferredWidth: 64 }
                        ToolRailButton {
                            objectName: "brushToolAction"
                            iconName: "brush"
                            toolId: "brush"
                            toolName: "Brush"
                            group: "paint"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Brush requires a raster node"
                        }
                        ToolRailButton {
                            // Lazybrush: scribble colours, press Enter; regions colour to the nearest
                            // scribble, stopping at line art.
                            iconName: "lazybrush"
                            toolId: "lazybrush"
                            toolName: "Lazybrush (colourize regions)"
                            group: "paint"
                        }
                        ToolRailButton {
                            // Mixer brush: wet paint that picks up and blends the canvas colour
                            // (wet / load / mix in the brush options).
                            objectName: "mixerToolAction"
                            iconName: "mixer"
                            toolId: "mixer"
                            toolName: "Mixer brush"
                            group: "paint"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Mixer brush requires a raster node"
                        }
                        ToolRailButton {
                            // Clone: Ctrl-click sets the source, then paint copies from it. The brush
                            // engine's clone mode, picked as a tool as in Photoshop and GIMP.
                            objectName: "cloneToolAction"
                            iconName: "clone"
                            toolId: "clone"
                            toolName: "Clone (Ctrl-click sets the source)"
                            group: "stamp"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Clone requires a raster node"
                        }
                        ToolRailButton {
                            // Healing: clone whose copy takes on the destination's colour, so a
                            // blemish is covered with surrounding tone. Ctrl-click sets the source.
                            objectName: "healToolAction"
                            iconName: "heal"
                            toolId: "heal"
                            toolName: "Healing (Ctrl-click sets the source)"
                            group: "stamp"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Healing requires a raster node"
                        }
                        ToolRailButton {
                            // Eraser: the brush in erase mode. E, as in Photoshop and Krita.
                            objectName: "eraserToolAction"
                            iconName: "eraser"
                            toolId: "eraser"
                            toolName: "Eraser"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Eraser requires a raster node"
                        }
                        ToolRailButton {
                            objectName: "gradientToolAction"
                            iconName: "gradient"
                            toolId: "gradient"
                            toolName: "Gradient"
                            group: "fill"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Gradient requires a raster node"
                        }
                        ToolRailButton {
                            objectName: "fillToolAction"
                            iconName: "fill"
                            toolId: "fill"
                            toolName: "Fill (bucket)"
                            group: "fill"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Fill requires a raster node"
                        }
                        ToolRailButton {
                            // Enclose & fill: drag a rectangle; regions closed off inside it fill with
                            // the brush colour.
                            iconName: "enclose"
                            toolId: "enclose"
                            toolName: "Enclose and fill"
                            group: "fill"
                        }
                        ToolRailButton {
                            // Blur: the convolve brush pulling each pixel toward its neighbours.
                            objectName: "blurToolAction"
                            iconName: "blur"
                            toolId: "blur"
                            toolName: "Blur"
                            group: "focus"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Blur requires a raster node"
                        }
                        ToolRailButton {
                            // Sharpen: the convolve brush pushing each pixel away from its neighbours.
                            objectName: "sharpenToolAction"
                            iconName: "sharpen"
                            toolId: "sharpen"
                            toolName: "Sharpen"
                            group: "focus"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Sharpen requires a raster node"
                        }
                        ToolRailButton {
                            // Smudge: drag the colour already on the layer (the brush's smudge mode).
                            objectName: "smudgeToolAction"
                            iconName: "smudge"
                            toolId: "smudge"
                            toolName: "Smudge"
                            group: "focus"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Smudge requires a raster node"
                        }
                        ToolRailButton {
                            // Dodge: lightens the tones under the brush (range in the brush options).
                            objectName: "dodgeToolAction"
                            iconName: "dodge"
                            toolId: "dodge"
                            toolName: "Dodge"
                            group: "tone"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Dodge requires a raster node"
                        }
                        ToolRailButton {
                            // Burn: darkens the tones under the brush.
                            objectName: "burnToolAction"
                            iconName: "burn"
                            toolId: "burn"
                            toolName: "Burn"
                            group: "tone"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Burn requires a raster node"
                        }
                        RailDivider { Layout.columnSpan: 2; Layout.preferredWidth: 64 }
                        ToolRailButton {
                            // Pen: click anchors to build a vector path; DRAG an anchor to pull its
                            // bezier handle out (a plain click stays a corner). Enter/near-start closes.
                            iconName: "pen"
                            toolId: "pen"
                            toolName: "Pen (click for corners, drag for curves)"
                        }
                        ToolRailButton {
                            // Text: click the canvas to place a new text node there. T is taken by
                            objectName: "textToolAction"
                            iconName: "text"
                            toolId: "text"
                            toolName: "Text"
                        }
                        ToolRailButton {
                            objectName: "shapeToolAction"
                            iconName: "shape"
                            toolId: "shape"
                            toolName: "Shape"
                        }
                        Item { Layout.preferredWidth: 36; Layout.preferredHeight: 36 }
                        RailDivider { Layout.columnSpan: 2; Layout.preferredWidth: 64 }
                        ToolRailButton {
                            // Perspective: the four corner handles start on the image's own corners;
                            // drag one to warp. Shift+P, GIMP's key; E belongs to the eraser.
                            iconName: "perspective"
                            toolId: "perspective"
                            toolName: "Perspective (drag the corners)"
                        }
                        ToolRailButton {
                            // Cage: click to lay a source cage, close with Enter, then drag its
                            // vertices to warp.
                            iconName: "cage"
                            toolId: "cage"
                            toolName: "Cage transform"
                        }
                        ToolRailButton {
                            // Warp / liquify: drag to push, grow, shrink or swirl pixels.
                            iconName: "warp"
                            toolId: "warp"
                            toolName: "Warp (liquify)"
                        }
                        ToolRailButton {
                            // N-point: click control points, close with Enter, then drag them to warp
                            // (thin-plate spline).
                            iconName: "npoint"
                            toolId: "npoint"
                            toolName: "N-point deformation"
                        }
                        RailDivider { Layout.columnSpan: 2; Layout.preferredWidth: 64 }
                        ToolRailButton {
                            iconName: "eye"
                            toolId: "inspect"
                            toolName: "Inspect (view only)"
                        }
                        ToolRailButton {
                            // Hand: drag to move the view. H, as in Photoshop, GIMP and Krita.
                            objectName: "handToolAction"
                            iconName: "hand"
                            toolId: "hand"
                            toolName: "Hand (drag to move the view)"
                            group: "view"
                        }
                        ToolRailButton {
                            // L1: Photoshop's Rotate View (R): drag around the middle to turn the
                            // view; double-click resets it. The image is not changed.
                            objectName: "rotateViewToolAction"
                            iconName: "rotateview"
                            toolId: "rotateview"
                            toolName: "Rotate view (drag; double-click resets)"
                            group: "view"
                        }
                        ToolRailButton {
                            // Zoom: click to zoom in on that point, Alt-click to zoom out.
                            objectName: "zoomToolAction"
                            iconName: "zoomIn"
                            toolId: "zoom"
                            toolName: "Zoom (click in, Alt-click out)"
                        }
                        // The brush colour fills the cell beside Zoom, so the rail needs no extra row.
                        // The brush size it used to show under it is already in the top bar.
                        ToolButton {
                            objectName: "brushColorSwatch"
                            Layout.alignment: Qt.AlignHCenter
                            implicitWidth: 36
                            implicitHeight: 36
                            ToolTip.visible: hovered
                            ToolTip.text: "Brush color"
                            Accessible.name: "Brush color"
                            onClicked: brushColorDialog.open()
                            background: Rectangle {
                                anchors.centerIn: parent
                                width: 24
                                height: 24
                                radius: 6
                                color: editor.brushColor
                                border.color: window.tokens.borderStrong
                            }
                        }
                    }
                }
            }

            Rectangle {
                id: workspace
                Layout.fillWidth: true
                Layout.fillHeight: true
                color: window.tokens.surfaceBase
                clip: true

                // L2: rulers along the top and left edge and the guide lines over the canvas.
                RulersOverlay {
                    anchors.fill: canvas
                    z: 5
                    app: window
                    view: canvas
                }
                CanvasItem {
                    id: canvas
                    anchors.top: parent.top
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: timelinePanel.top
                    // While the filter browser previews, the canvas shows that result instead.
                    image: editor.hasFilterPreview ? editor.filterPreview : editor.renderImage
                    selectionMask: editor.selectionMask
                    selectionActive: editor.selectionActive
                    zoom: window.canvasZoom
                    Accessible.name: "Document canvas with selection overlay"

                    // The committed-path overlay is the DOCUMENT's geometry, so it has to be re-read
                    // whenever the document moves: a new path, an undo, a different layer selected.
                    // Tool events alone would leave an undone path's handles on screen. Placed on the
                    // canvas item rather than inside the PointHandler, which takes no children.
                    Connections {
                        target: editor
                        function onDocumentChanged() {
                            canvasPointer.syncHandles();
                        }
                    }

                    // Right-click on the canvas opens the common image commands, as in GIMP and
                    // Photoshop. Drawing stays on the left button (canvasPointer below).
                    TapHandler {
                        objectName: "canvasContextTap"
                        acceptedButtons: Qt.RightButton
                        // Alt+right-drag is the brush-resize gesture, not the menu (S2).
                        acceptedModifiers: Qt.NoModifier
                        onTapped: canvasMenu.popup()
                    }
                    // Photoshop's brush resize: Alt+right-drag, left/right = size, up/down =
                    // hardness. Ctrl+Alt+left-drag does the same, for desktops (XFCE) that take
                    // Alt+right-drag to resize windows.
                    property point resizeStart: Qt.point(0, 0)
                    property real resizeStartSize: 0
                    property real resizeStartHardness: 0
                    function beginBrushResize(position) {
                        resizeStart = position;
                        resizeStartSize = editor.brushSize;
                        resizeStartHardness = editor.brushHardness;
                    }
                    function updateBrushResize(position) {
                        editor.brushSize = Math.max(1, Math.min(1000, resizeStartSize + (position.x - resizeStart.x)));
                        editor.brushHardness = Math.max(0, Math.min(1, resizeStartHardness - (position.y - resizeStart.y) / 200));
                    }
                    PointHandler {
                        objectName: "brushResizeAltRight"
                        target: null
                        acceptedButtons: Qt.RightButton
                        acceptedModifiers: Qt.AltModifier
                        onActiveChanged: if (active) parent.beginBrushResize(point.position)
                        onPointChanged: if (active) parent.updateBrushResize(point.position)
                    }
                    PointHandler {
                        objectName: "brushResizeCtrlAlt"
                        target: null
                        acceptedButtons: Qt.LeftButton
                        acceptedModifiers: Qt.ControlModifier | Qt.AltModifier
                        onActiveChanged: if (active) parent.beginBrushResize(point.position)
                        onPointChanged: if (active) parent.updateBrushResize(point.position)
                    }
                    Menu {
                        id: canvasMenu
                        objectName: "canvasMenu"
                        Action { text: qsTr("&Undo"); enabled: editor.canUndo; onTriggered: editor.undo() }
                        Action { text: qsTr("&Redo"); enabled: editor.canRedo; onTriggered: editor.redo() }
                        MenuSeparator {}
                        Action { text: qsTr("Select &all"); onTriggered: editor.selectAll() }
                        Action { text: qsTr("Select &none"); onTriggered: editor.clearSelection() }
                        Action { text: qsTr("&Invert selection"); onTriggered: editor.invertSelection() }
                        MenuSeparator {}
                        Action { text: qsTr("New raster &layer"); onTriggered: editor.addLayer() }
                        Action {
                            text: qsTr("&Delete layer")
                            enabled: editor.activeLayerId.length > 0
                            onTriggered: editor.deleteLayer(editor.activeLayerId)
                        }
                        Action {
                            text: qsTr("Flip &horizontally")
                            enabled: editor.activeNodeCanEditRaster
                            onTriggered: editor.flipActive(true, false)
                        }
                        Action {
                            text: qsTr("Flip &vertically")
                            enabled: editor.activeNodeCanEditRaster
                            onTriggered: editor.flipActive(false, true)
                        }
                        MenuSeparator {}
                        Action { text: qsTr("&Filters…"); onTriggered: filterBrowser.open() }
                        MenuSeparator {}
                        Action { text: qsTr("Zoom &in"); onTriggered: window.canvasZoom = Math.min(32, window.canvasZoom * 1.2) }
                        Action { text: qsTr("Zoom &out"); onTriggered: window.canvasZoom = Math.max(0.05, window.canvasZoom / 1.2) }
                        Action { text: qsTr("Actual &pixels"); onTriggered: { window.canvasZoom = 1; canvas.pan = Qt.point(0, 0) } }
                    }

                    PointHandler {
                        id: canvasPointer
                        objectName: "canvasPointer"
                        target: null
                        acceptedButtons: Qt.LeftButton
                        property bool gestureActive: false
                        property point startCanvas: Qt.point(0, 0)
                        property point endCanvas: Qt.point(0, 0)
                        // Flat [x0, y0, x1, y1, ...] path collected while dragging the lasso.
                        property var lassoPoints: []
                        // Polygon tool: vertices accumulated across clicks until the shape is closed.
                        // Every one of these point lists is REASSIGNED rather than pushed into, and that
                        // is load-bearing: a `property var` holding a JS array emits no change signal
                        // when the array is mutated in place, so every binding that reads its `length`
                        // keeps the value it had at the last assignment. The Enter-to-close shortcut is
                        // bound to exactly that, so `polyPoints.push(...)` left the shortcut permanently
                        // disabled — "press Enter to close the shape" did nothing, for the polygon,
                        // scissors, pen, cage, n-point, foreground-select and lazybrush tools alike.
                        // Found by driving the real app (I.5); no build or test sees it.
                        property var polyPoints: []
                        // Cage tool: the committed source cage (flat [x0,y0,...]) and the editable
                        // destination cage. Empty until the user closes the source cage; while
                        // non-empty, drags move the nearest destination vertex. cageGrab is the index
                        // of the vertex being dragged, or -1.
                        property var cageSrc: []
                        property var cageDst: []
                        property int cageGrab: -1
                        // Perspective tool (H.21): four destination corners, dragged on the canvas.
                        // Unlike the cage there is no placing phase — the corners START as the canvas
                        // corners, because a perspective transform is defined by where the IMAGE's own
                        // four corners go, and asking the user to place them first would just have them
                        // click the corners they already have.
                        property var perspCorners: []
                        property int perspGrab: -1
                        // Pen handles (I.2): one OUTGOING control point per anchor, flat [x,y,...] and
                        // always the same length as the pen's anchor list. A handle left on its own
                        // anchor is a corner, so a plain click and "no handle" are the same thing and
                        // there is no null to encode. `penDragging` is true while a press is still down
                        // and its handle is following the pointer.
                        property var penHandles: []
                        property bool penDragging: false
                        // N-point deformation: committed source control points, their editable
                        // destinations, and the index being dragged (-1 = none).
                        property var npSrc: []
                        property var npDst: []
                        property int npGrab: -1
                        // Foreground-select scribbles: foreground and background sample marks.
                        property var fgMarks: []
                        property var bgMarks: []
                        // Lazybrush scribbles: flat [x,y,r,g,b,a, ...] seeds, each in the current
                        // brush colour, applied on Enter.
                        property var lazyMarks: []
                        function applyForeground() {
                            if (fgMarks.length >= 2)
                                editor.selectForeground(fgMarks, bgMarks, window.selectionMode);
                            fgMarks = [];
                            bgMarks = [];
                            canvas.clearPreview();
                        }
                        function pushLazyMark(x, y) {
                            const c = editor.brushColor;
                            lazyMarks = lazyMarks.concat([x, y, c.r * 255, c.g * 255, c.b * 255, c.a * 255]);
                        }
                        function applyLazybrush() {
                            if (lazyMarks.length >= 6)
                                editor.lazybrush(lazyMarks);
                            lazyMarks = [];
                            canvas.clearPreview();
                        }
                        function closePolygon() {
                            if (polyPoints.length >= 6) {
                                if (window.activeTool === "scissors")
                                    editor.selectScissors(polyPoints, window.selectionMode);
                                else if (window.activeTool === "pen")
                                    editor.addVectorPathBezier(polyPoints, penHandles, true, "Path");
                                else if (window.activeTool === "cage") {
                                    // Commit the source cage; the same points become the editable
                                    // destination cage, which the user then drags.
                                    cageSrc = polyPoints.slice();
                                    cageDst = polyPoints.slice();
                                } else if (window.activeTool === "npoint") {
                                    // Commit the source control points; copies become draggable.
                                    npSrc = polyPoints.slice();
                                    npDst = polyPoints.slice();
                                } else
                                    editor.selectPolygon(polyPoints, window.selectionMode);
                            } else if (window.activeTool === "pen" && polyPoints.length >= 4) {
                                // A pen path needs only 2 points (an open line) to be worth keeping.
                                editor.addVectorPathBezier(polyPoints, penHandles, false, "Path");
                            }
                            polyPoints = [];
                            penHandles = [];
                            penDragging = false;
                            syncHandles();
                            canvas.clearPreview();
                        }
                        // Index of the destination cage vertex within `radius` px of (x,y), or -1.
                        function cageVertexAt(x, y, radius) {
                            for (var i = 0; i < cageDst.length; i += 2) {
                                if (Math.abs(cageDst[i] - x) <= radius && Math.abs(cageDst[i + 1] - y) <= radius)
                                    return i;
                            }
                            return -1;
                        }
                        // The perspective corners, reset to the image's own. Called when the tool is
                        // selected and after each applied drag: the pixels have already moved, so
                        // leaving the handles where they were dragged would re-warp an image that is
                        // already warped — each gesture is a fresh perspective on the current pixels.
                        function resetPerspectiveCorners() {
                            const w = editor.documentWidth;
                            const h = editor.documentHeight;
                            perspCorners = [0, 0, w, 0, w, h, 0, h];
                            perspGrab = -1;
                        }
                        // Index of the perspective corner within `radius` px of (x,y), in LIST slots.
                        function perspCornerAt(x, y, radius) {
                            for (var i = 0; i < perspCorners.length; i += 2) {
                                if (Math.abs(perspCorners[i] - x) <= radius && Math.abs(perspCorners[i + 1] - y) <= radius)
                                    return i;
                            }
                            return -1;
                        }
                        // What the canvas draws as handles: whichever transform tool is live owns them.
                        // Bound in one place so two tools can never both claim the overlay.
                        function syncHandles() {
                            if (window.activeTool === "perspective") {
                                canvas.handlePoints = perspCorners;
                                canvas.controlPoints = [];
                                canvas.activeHandle = perspGrab >= 0 ? perspGrab / 2 : -1;
                            } else if (window.activeTool === "pen" && polyPoints.length > 0) {
                                // The anchors placed so far with their outgoing control points, plus
                                // the anchor being pressed and the handle currently pulled out of it.
                                // The dragged handle rides in the CONTROL list, not the anchor list:
                                // as an anchor it drew a second square away from the path, which is
                                // the one thing a handle must not look like.
                                var pts = polyPoints.slice();
                                var ctl = penHandles.slice();
                                if (penDragging) {
                                    pts.push(startCanvas.x, startCanvas.y);
                                    ctl.push(endCanvas.x, endCanvas.y);
                                    canvas.activeHandle = pts.length / 2 - 1;
                                } else {
                                    canvas.activeHandle = -1;
                                }
                                canvas.handlePoints = pts;
                                canvas.controlPoints = ctl;
                            } else if (window.activeTool === "pen") {
                                // Nothing in progress: show the ACTIVE vector node's own geometry,
                                // read back out of the document. Committing a path clears the tool
                                // state, so without this the handles vanished the moment the path
                                // existed -- and because this is the document's answer, an undo takes
                                // the overlay with the node instead of leaving a ghost behind.
                                canvas.handlePoints = editor.activeVectorAnchors;
                                canvas.controlPoints = editor.activeVectorHandles;
                                canvas.activeHandle = -1;
                            } else if (window.activeTool === "cage" && cageDst.length > 0) {
                                canvas.handlePoints = cageDst;
                                canvas.controlPoints = [];
                                canvas.activeHandle = cageGrab >= 0 ? cageGrab / 2 : -1;
                            } else if (window.activeTool === "npoint" && npDst.length > 0) {
                                canvas.handlePoints = npDst;
                                canvas.controlPoints = [];
                                canvas.activeHandle = npGrab >= 0 ? npGrab / 2 : -1;
                            } else {
                                canvas.handlePoints = [];
                                canvas.controlPoints = [];
                                canvas.activeHandle = -1;
                            }
                        }

                        function boundedCanvasPoint(position) {
                            const raw = canvas.canvasPoint(position);
                            return Qt.point(Math.max(0, Math.min(editor.documentWidth, raw.x)), Math.max(0, Math.min(editor.documentHeight, raw.y)));
                        }
                        function normalizedPressure(measuredPressure, deviceType) {
                            const measured = Number(measuredPressure);
                            if (deviceType === PointerDevice.Mouse || deviceType === PointerDevice.TouchPad)
                                return 1;
                            if (isFinite(measured))
                                return Math.max(0, Math.min(1, measured));
                            return 1;
                        }
                        function pointPressure(handlerPoint) {
                            return normalizedPressure(handlerPoint.pressure, handlerPoint.device.deviceType);
                        }
                        function activeToolNeedsRaster() {
                            return window.brushLike || window.activeTool === "fill"
                                || window.activeTool === "gradient" || window.activeTool === "transform"
                                || window.activeTool === "warp" || window.activeTool === "enclose"
                                || window.activeTool === "perspective"
                                || window.activeTool === "lazybrush";
                        }
                        function cancelGesture() {
                            airbrushTimer.stop();
                            if (gestureActive && window.brushLike)
                                editor.cancelStroke();
                            gestureActive = false;
                            // A cancelled pen press drops the handle it was pulling but KEEPS the
                            // anchors already placed — losing a half-finished path to a stray cancel
                            // would be worse than losing one handle.
                            penDragging = false;
                            canvas.clearPreview();
                        }
                        function commitGesture() {
                            airbrushTimer.stop();
                            if (!gestureActive)
                                return;
                            gestureActive = false;
                            const dx = endCanvas.x - startCanvas.x;
                            const dy = endCanvas.y - startCanvas.y;
                            if (window.activeTool === "perspective") {
                                if (perspGrab >= 0) {
                                    perspCorners[perspGrab] = endCanvas.x;
                                    perspCorners[perspGrab + 1] = endCanvas.y;
                                    editor.perspectiveActive(perspCorners, window.samplingMode);
                                    // The pixels have moved to where the corners were dragged, so the
                                    // frame restarts from the image's own corners. Keeping the dragged
                                    // positions would warp an already-warped image on the next drag.
                                    resetPerspectiveCorners();
                                }
                                syncHandles();
                                canvas.clearPreview();
                                return;
                            }
                            if (window.activeTool === "cage") {
                                if (cageSrc.length === 0) {
                                    // Placing the source cage: each click drops a vertex; a click near
                                    // the first (>=3 vertices) closes it.
                                    if (polyPoints.length >= 6
                                            && Math.abs(polyPoints[0] - endCanvas.x) <= 6
                                            && Math.abs(polyPoints[1] - endCanvas.y) <= 6) {
                                        closePolygon();
                                    } else {
                                        polyPoints = polyPoints.concat([endCanvas.x, endCanvas.y]);
                                    }
                                } else if (cageGrab >= 0) {
                                    // Moved a dst vertex: update it and warp the layer.
                                    cageDst[cageGrab] = endCanvas.x;
                                    cageDst[cageGrab + 1] = endCanvas.y;
                                    editor.cageTransform(cageSrc, cageDst, window.samplingMode);
                                    // The destination becomes the new source so further drags compose.
                                    cageSrc = cageDst.slice();
                                    cageGrab = -1;
                                }
                                syncHandles();
                                canvas.clearPreview();
                                return;
                            }
                            if (window.activeTool === "npoint") {
                                if (npSrc.length === 0) {
                                    // Placing control points: each click drops one; a click near the
                                    // first (>=3) closes the set.
                                    if (polyPoints.length >= 6
                                            && Math.abs(polyPoints[0] - endCanvas.x) <= 6
                                            && Math.abs(polyPoints[1] - endCanvas.y) <= 6) {
                                        closePolygon();
                                    } else {
                                        polyPoints = polyPoints.concat([endCanvas.x, endCanvas.y]);
                                    }
                                } else if (npGrab >= 0) {
                                    npDst[npGrab] = endCanvas.x;
                                    npDst[npGrab + 1] = endCanvas.y;
                                    editor.nPointTransform(npSrc, npDst, window.samplingMode, window.puppetRigid);
                                    // Compose: the dragged destination becomes the new source.
                                    npSrc = npDst.slice();
                                    npGrab = -1;
                                }
                                syncHandles();
                                canvas.clearPreview();
                                return;
                            }
                            if (window.brushLike) {
                                editor.endStroke();
                            } else if (window.activeTool === "fill") {
                                editor.floodFill(endCanvas.x, endCanvas.y, editor.brushColor, window.fillTolerance);
                            } else if (window.activeTool === "wand") {
                                editor.selectByColor(endCanvas.x, endCanvas.y, window.fillTolerance, window.wandContiguous, window.selectionMode);
                            } else if (window.activeTool === "rectangle") {
                                editor.selectRectangle(startCanvas.x, startCanvas.y, dx, dy, window.selectionMode);
                            } else if (window.activeTool === "ellipse") {
                                editor.selectEllipse(startCanvas.x, startCanvas.y, dx, dy, window.selectionMode);
                            } else if (window.activeTool === "lasso") {
                                if (lassoPoints.length >= 6)
                                    editor.selectPolygon(lassoPoints, window.selectionMode);
                                lassoPoints = [];
                            } else if (window.activeTool === "warp") {
                                if (lassoPoints.length >= 2)
                                    editor.warpBrush(lassoPoints, window.warpMode, window.warpRadius, window.warpStrength, window.samplingMode);
                                lassoPoints = [];
                            } else if (window.activeTool === "pen") {
                                // The pen's anchor is where the press BEGAN; the drag end is that
                                // anchor's outgoing handle. Using the release point as the anchor (as
                                // polygon does) would move the anchor to wherever the handle was
                                // dragged, so the curve could never be shaped without also moving the
                                // point it passes through.
                                penDragging = false;
                                if (polyPoints.length >= 6
                                        && Math.abs(polyPoints[0] - startCanvas.x) <= 6
                                        && Math.abs(polyPoints[1] - startCanvas.y) <= 6) {
                                    closePolygon();
                                } else {
                                    polyPoints = polyPoints.concat([startCanvas.x, startCanvas.y]);
                                    penHandles = penHandles.concat([endCanvas.x, endCanvas.y]);
                                    syncHandles();
                                }
                            } else if (window.activeTool === "polygon" || window.activeTool === "scissors") {
                                // Each click drops a vertex. A click within 6px of the first (with >=3
                                // so far) closes the shape and selects it.
                                if (polyPoints.length >= 6
                                        && Math.abs(polyPoints[0] - endCanvas.x) <= 6
                                        && Math.abs(polyPoints[1] - endCanvas.y) <= 6) {
                                    closePolygon();
                                } else {
                                    polyPoints = polyPoints.concat([endCanvas.x, endCanvas.y]);
                                }
                            } else if (window.activeTool === "gradient") {
                                if (window.gradientKind === "linear")
                                    editor.linearGradient(startCanvas.x, startCanvas.y, endCanvas.x, endCanvas.y, window.gradientStartColor, window.gradientEndColor);
                                else
                                    editor.radialGradient(startCanvas.x, startCanvas.y, Math.hypot(dx, dy), window.gradientStartColor, window.gradientEndColor);
                            } else if (window.activeTool === "shape") {
                                window.commitShape(startCanvas, endCanvas);
                            } else if (window.activeTool === "crop") {
                                editor.cropCanvas(startCanvas.x, startCanvas.y, dx, dy);
                            } else if (window.activeTool === "enclose") {
                                editor.encloseAndFill(startCanvas.x, startCanvas.y, dx, dy, editor.brushColor, 8);
                            } else if (window.activeTool === "transform") {
                                editor.transformActive(1, 0, 0, 1, dx, dy, window.samplingMode);
                            }
                            canvas.clearPreview();
                        }
                        onActiveChanged: {
                            if (active) {
                                const position = point.position;
                                // Ctrl+Alt+left-drag resizes the brush (brushResizeCtrlAlt); it
                                // must not also paint.
                                if ((point.modifiers & (Qt.ControlModifier | Qt.AltModifier))
                                        === (Qt.ControlModifier | Qt.AltModifier))
                                    return;
                                if (!canvas.containsCanvasPoint(position) || window.activeTool === "inspect"
                                        || window.activeTool === "hand" || window.activeTool === "zoom"
                                        || window.activeTool === "rotateview"
                                        || window.activeTool === "align"
                                        || (activeToolNeedsRaster() && !editor.activeNodeCanEditRaster))
                                    return;
                                startCanvas = boundedCanvasPoint(position);
                                endCanvas = startCanvas;
                                if (window.activeTool === "text") {
                                    // Text tool: a click places a new text node at that point. The
                                    // dialog does the editing; there is no drag to follow.
                                    textSemanticDialog.openNew(startCanvas.x, startCanvas.y);
                                    return;
                                }
                                gestureActive = true;
                                if (window.activeTool === "perspective") {
                                    // Grab the nearest corner. A press that hits none leaves grab at
                                    // -1 and the gesture does nothing, which is what the user means by
                                    // clicking the middle of the frame — dragging the whole image is
                                    // the move tool's job, not this one's.
                                    perspGrab = perspCornerAt(startCanvas.x, startCanvas.y, 10);
                                    syncHandles();
                                    return;
                                }
                                if (window.activeTool === "pen") {
                                    // A press starts an anchor whose handle follows the pointer until
                                    // release. Nothing is committed here: a press that never moves ends
                                    // as a corner, and one that drags ends as a curve.
                                    penDragging = true;
                                    return;
                                }
                                if (window.activeTool === "cage" && cageSrc.length > 0) {
                                    // Source cage is set: this press grabs the nearest dst vertex.
                                    cageGrab = cageVertexAt(startCanvas.x, startCanvas.y, 8);
                                    return;
                                }
                                if (window.activeTool === "npoint" && npSrc.length > 0) {
                                    // Control points are set: grab the nearest destination point.
                                    npGrab = -1;
                                    for (var ni = 0; ni < npDst.length; ni += 2) {
                                        if (Math.abs(npDst[ni] - startCanvas.x) <= 8 && Math.abs(npDst[ni + 1] - startCanvas.y) <= 8) {
                                            npGrab = ni;
                                            break;
                                        }
                                    }
                                    return;
                                }
                                if (window.brushLike) {
                                    // Clone: Ctrl-click sets the source anchor instead of painting.
                                    if (editor.brushClone && (point.modifiers & Qt.ControlModifier)) {
                                        editor.setCloneSource(startCanvas.x, startCanvas.y);
                                        gestureActive = false;
                                        return;
                                    }
                                    editor.beginStroke(startCanvas.x, startCanvas.y, pointPressure(point));
                                    if (editor.brushAirbrush)
                                        airbrushTimer.start();
                                } else if (window.activeTool === "picker") {
                                    window.pickColorAt(startCanvas);
                                } else if (window.activeTool === "lasso") {
                                    lassoPoints = [startCanvas.x, startCanvas.y];
                                } else if (window.activeTool === "warp") {
                                    // Collect the drag path; apply the liquify on release.
                                    lassoPoints = [startCanvas.x, startCanvas.y];
                                } else if (window.activeTool === "fgselect") {
                                    // Shift-drag marks background, plain drag marks foreground.
                                    if (point.modifiers & Qt.ShiftModifier)
                                        bgMarks = bgMarks.concat([startCanvas.x, startCanvas.y]);
                                    else
                                        fgMarks = fgMarks.concat([startCanvas.x, startCanvas.y]);
                                } else if (window.activeTool === "lazybrush") {
                                    pushLazyMark(startCanvas.x, startCanvas.y);
                                } else if (window.activeTool === "measure") {
                                    window.measureText = "0 px   0°";
                                } else if (window.activeTool !== "fill" && window.activeTool !== "wand"
                                           && window.activeTool !== "polygon" && window.activeTool !== "scissors"
                                           && window.activeTool !== "fgselect" && window.activeTool !== "pen"
                                           && window.activeTool !== "cage"
                                           && window.activeTool !== "npoint") {
                                    canvas.previewStart = startCanvas;
                                    canvas.previewEnd = endCanvas;
                                    canvas.previewKind = window.activeTool === "rectangle" ? "rectangle" : window.activeTool === "ellipse" ? "ellipse" : window.activeTool === "gradient" ? window.gradientKind : window.activeTool === "shape" ? window.shapePreviewKind() : window.activeTool;
                                    canvas.previewVisible = true;
                                }
                            } else {
                                commitGesture();
                            }
                        }
                        onPointChanged: {
                            if (!active || !gestureActive)
                                return;
                            const position = point.position;
                            endCanvas = boundedCanvasPoint(position);
                            if (window.activeTool === "perspective" && perspGrab >= 0) {
                                // Move the grabbed corner as the pointer moves, so the frame follows the
                                // hand. The warp itself is only applied on release — running it per move
                                // event would stack dozens of transforms into the undo history for one
                                // gesture.
                                perspCorners[perspGrab] = endCanvas.x;
                                perspCorners[perspGrab + 1] = endCanvas.y;
                                syncHandles();
                                return;
                            }
                            if (window.activeTool === "pen" && penDragging) {
                                // Show the handle being pulled out of the anchor. The anchor itself
                                // does not move, which is what tells the user the drag is shaping a
                                // curve rather than placing a point.
                                syncHandles();
                                return;
                            }
                            if (window.activeTool === "cage" && cageGrab >= 0) {
                                // Same reason: show the vertex moving, warp once on release.
                                cageDst[cageGrab] = endCanvas.x;
                                cageDst[cageGrab + 1] = endCanvas.y;
                                syncHandles();
                                return;
                            }
                            if (window.activeTool === "npoint" && npGrab >= 0) {
                                npDst[npGrab] = endCanvas.x;
                                npDst[npGrab + 1] = endCanvas.y;
                                syncHandles();
                                return;
                            }
                            if (window.brushLike) {
                                if (canvas.containsCanvasPoint(position))
                                    editor.addStrokePoint(endCanvas.x, endCanvas.y, pointPressure(point));
                            } else if (window.activeTool === "picker") {
                                window.pickColorAt(endCanvas);
                            } else if (window.activeTool === "lasso") {
                                // Append each move, skipping sub-pixel jitter so the path stays small.
                                const n = lassoPoints.length;
                                if (n < 2 || Math.abs(lassoPoints[n - 2] - endCanvas.x) >= 1
                                        || Math.abs(lassoPoints[n - 1] - endCanvas.y) >= 1) {
                                    lassoPoints = lassoPoints.concat([endCanvas.x, endCanvas.y]);
                                }
                            } else if (window.activeTool === "warp") {
                                // Sample the drag path sparsely (every >=2px) to keep the field cheap.
                                const wn = lassoPoints.length;
                                if (wn < 2 || Math.abs(lassoPoints[wn - 2] - endCanvas.x) >= 2
                                        || Math.abs(lassoPoints[wn - 1] - endCanvas.y) >= 2) {
                                    lassoPoints = lassoPoints.concat([endCanvas.x, endCanvas.y]);
                                }
                            } else if (window.activeTool === "fgselect") {
                                // Collect sample marks sparsely as the scribble moves.
                                if (point.modifiers & Qt.ShiftModifier)
                                    bgMarks = bgMarks.concat([endCanvas.x, endCanvas.y]);
                                else
                                    fgMarks = fgMarks.concat([endCanvas.x, endCanvas.y]);
                            } else if (window.activeTool === "lazybrush") {
                                // Drop a colour seed every few pixels as the scribble moves.
                                const ln = lazyMarks.length;
                                if (ln < 6 || Math.abs(lazyMarks[ln - 6] - endCanvas.x) >= 4
                                        || Math.abs(lazyMarks[ln - 5] - endCanvas.y) >= 4) {
                                    pushLazyMark(endCanvas.x, endCanvas.y);
                                }
                            } else if (window.activeTool === "measure") {
                                const mdx = endCanvas.x - startCanvas.x;
                                const mdy = endCanvas.y - startCanvas.y;
                                const dist = Math.hypot(mdx, mdy);
                                // Angle in degrees, 0 along +x, measured like GIMP's measure tool.
                                const ang = Math.atan2(-mdy, mdx) * 180 / Math.PI;
                                window.measureText = dist.toFixed(1) + " px   " + ang.toFixed(1) + "°";
                            } else {
                                canvas.previewEnd = endCanvas;
                            }
                        }
                        onCanceled: point => cancelGesture()
                    }

                    HoverHandler {
                        cursorShape: window.activeTool === "inspect" ? Qt.ArrowCursor : Qt.CrossCursor
                    }

                    // Airbrush: while a brush stroke is held, keep depositing at the current point even
                    // when it is not moving, so paint builds up (GIMP's airbrush rate). A sub-pixel
                    // jitter each tick keeps the point past addStrokePoint's duplicate filter; the low
                    // per-dab flow (BrushSettings.flow) makes the build-up gradual rather than a jump.
                    Timer {
                        id: airbrushTimer
                        interval: 40
                        repeat: true
                        running: false
                        property real phase: 0
                        onTriggered: {
                            if (!canvasPointer.gestureActive || !window.brushLike
                                    || !editor.brushAirbrush) {
                                stop();
                                return;
                            }
                            phase += 1;
                            const jx = (phase % 2 === 0 ? 0.2 : -0.2);
                            editor.addStrokePoint(canvasPointer.endCanvas.x + jx, canvasPointer.endCanvas.y, 1.0);
                        }
                    }

                    // Hand tool: drag moves the view. The pointer handler above ignores this tool.
                    DragHandler {
                        objectName: "handDrag"
                        target: null
                        enabled: window.activeTool === "hand"
                        property point startPan: Qt.point(0, 0)
                        onActiveChanged: if (active) startPan = canvas.pan
                        onTranslationChanged: canvas.pan = Qt.point(startPan.x + translation.x,
                                                                    startPan.y + translation.y)
                    }
                    // L1: Rotate View drags the view round the middle of the canvas area; Shift snaps
                    // to 15 degrees, as in Photoshop. A double-click resets the rotation.
                    DragHandler {
                        objectName: "rotateViewDrag"
                        target: null
                        enabled: window.activeTool === "rotateview"
                        property real startRotation: 0
                        property real startAngle: 0
                        function angleAt(p) {
                            return Math.atan2(p.y - canvas.height / 2, p.x - canvas.width / 2) * 180 / Math.PI;
                        }
                        onActiveChanged: if (active) {
                            startRotation = canvas.viewRotation;
                            startAngle = angleAt(centroid.pressPosition);
                        }
                        onCentroidChanged: if (active) {
                            let delta = angleAt(centroid.position) - startAngle;
                            if (centroid.modifiers & Qt.ShiftModifier)
                                delta = Math.round((startRotation + delta) / 15) * 15 - startRotation;
                            window.rotateView(startRotation + delta - canvas.viewRotation);
                        }
                    }
                    TapHandler {
                        objectName: "rotateViewReset"
                        enabled: window.activeTool === "rotateview"
                        onDoubleTapped: canvas.viewRotation = 0
                    }
                    // Zoom tool: click zooms in on that point, Alt-click zooms out. The clicked image
                    // pixel stays under the pointer, as in Photoshop and GIMP.
                    TapHandler {
                        objectName: "zoomTap"
                        enabled: window.activeTool === "zoom"
                        onTapped: (eventPoint, button) => {
                            const out = (eventPoint.modifiers & Qt.AltModifier) !== 0;
                            const before = canvas.zoom;
                            const after = Math.max(0.05, Math.min(32, before * (out ? 1 / 1.5 : 1.5)));
                            const at = eventPoint.position;
                            const image = canvas.canvasPoint(at);
                            window.canvasZoom = after;
                            canvas.anchorCanvasPoint(image, at);
                        }
                    }

                    WheelHandler {
                        target: null
                        acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
                        onWheel: event => {
                            const factor = event.angleDelta.y > 0 ? 1.12 : 1 / 1.12;
                            window.canvasZoom = Math.max(0.05, Math.min(32, window.canvasZoom * factor));
                            event.accepted = true;
                        }
                    }
                }

                Rectangle {
                    anchors.right: parent.right
                    anchors.bottom: timelinePanel.top
                    anchors.margins: 14
                    width: zoomRow.implicitWidth + 16
                    height: 40
                    radius: 9
                    color: window.tokens.surfaceRaised
                    border.color: window.tokens.borderSubtle
                    RowLayout {
                        id: zoomRow
                        anchors.centerIn: parent
                        spacing: 2
                        CommandButton {
                            text: "Zoom out"
                            iconName: "zoomOut"
                            iconOnly: true
                            ToolTip.text: "Zoom out"
                            onClicked: window.canvasZoom = Math.max(0.05, window.canvasZoom / 1.2)
                        }
                        Label {
                            text: Math.round(window.canvasZoom * 100) + "%"
                            color: window.tokens.inkPrimary
                            Layout.preferredWidth: 50
                            horizontalAlignment: Text.AlignHCenter
                        }
                        CommandButton {
                            text: "Zoom in"
                            iconName: "zoomIn"
                            iconOnly: true
                            ToolTip.text: "Zoom in"
                            onClicked: window.canvasZoom = Math.min(32, window.canvasZoom * 1.2)
                        }
                        CommandButton {
                            text: "1:1"
                            ToolTip.text: "Actual pixels"
                            onClicked: window.canvasZoom = 1
                        }
                    }
                }

                Rectangle {
                    id: timelinePanel
                    objectName: "timelinePanel"
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    // Controls row plus margins when closed; the frame strip adds 90px when open.
                    height: timelineControls.implicitHeight + 16 + (window.timelineOpen ? 96 : 0)
                    color: window.tokens.surfaceRaised
                    border.color: window.tokens.borderSubtle

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 8
                        spacing: 6

                        RowLayout {
                            id: timelineControls
                            Layout.fillWidth: true
                            spacing: 2
                            CommandButton {
                                objectName: "timelineToggleAction"
                                text: "Timeline"
                                iconName: window.timelineOpen ? "chevronDown" : "chevronRight"
                                ToolTip.text: window.timelineOpen ? "Hide frames" : "Show frames"
                                onClicked: window.timelineOpen = !window.timelineOpen
                            }
                            Label {
                                text: editor.frameCount === 1 ? "1 frame" : editor.frameCount + " frames"
                                color: window.tokens.inkSecondary
                                font.pixelSize: 12
                                Layout.rightMargin: 8
                            }
                            TimelineDivider {}
                            CommandButton {
                                objectName: "previousFrameAction"
                                text: "Previous frame"
                                iconName: "skipBack"
                                iconOnly: true
                                enabled: editor.currentFrameIndex > 0
                                ToolTip.text: "Previous frame"
                                onClicked: editor.setCurrentFrame(editor.frames.frameIdAt(editor.currentFrameIndex - 1))
                            }
                            CommandButton {
                                objectName: "playFrameAction"
                                text: editor.playing ? "Stop" : "Play"
                                iconName: editor.playing ? "pause" : "play"
                                iconOnly: true
                                ToolTip.text: editor.playing ? "Stop playback" : "Play range"
                                onClicked: editor.setPlaying(!editor.playing)
                            }
                            CommandButton {
                                objectName: "nextFrameAction"
                                text: "Next frame"
                                iconName: "skipForward"
                                iconOnly: true
                                enabled: editor.currentFrameIndex + 1 < editor.frameCount
                                ToolTip.text: "Next frame"
                                onClicked: editor.setCurrentFrame(editor.frames.frameIdAt(editor.currentFrameIndex + 1))
                            }
                            TimelineDivider {}
                            CommandButton {
                                objectName: "addFrameAction"
                                text: "Frame"
                                iconName: "plus"
                                ToolTip.text: "Add blank sparse frame"
                                onClicked: {
                                    editor.addFrame(editor.currentFrameIndex + 1);
                                    window.timelineOpen = true;
                                }
                            }
                            CommandButton {
                                objectName: "duplicateFrameAction"
                                text: "Duplicate frame"
                                iconName: "duplicate"
                                iconOnly: true
                                ToolTip.text: "Duplicate current frame cels"
                                onClicked: {
                                    editor.duplicateFrame(editor.currentFrame, editor.currentFrameIndex + 1);
                                    window.timelineOpen = true;
                                }
                            }
                            CommandButton {
                                objectName: "moveFrameLeftAction"
                                text: "Move frame left"
                                iconName: "arrowLeft"
                                iconOnly: true
                                enabled: editor.currentFrameIndex > 0
                                ToolTip.text: "Move current frame left"
                                onClicked: editor.moveFrame(editor.currentFrame, editor.currentFrameIndex - 1)
                            }
                            CommandButton {
                                objectName: "moveFrameRightAction"
                                text: "Move frame right"
                                iconName: "arrowRight"
                                iconOnly: true
                                enabled: editor.currentFrameIndex + 1 < editor.frameCount
                                ToolTip.text: "Move current frame right"
                                onClicked: editor.moveFrame(editor.currentFrame, editor.currentFrameIndex + 1)
                            }
                            CommandButton {
                                objectName: "deleteFrameAction"
                                text: "Delete frame"
                                iconName: "trash"
                                iconOnly: true
                                enabled: editor.frameCount > 1
                                ToolTip.text: "Delete current frame"
                                onClicked: editor.removeFrame(editor.currentFrame)
                            }
                            Item { Layout.fillWidth: true }
                            Label { text: "FPS"; color: window.tokens.inkSecondary; Layout.rightMargin: 4 }
                            SpinBox {
                                objectName: "timelineFpsControl"
                                from: 1
                                to: 240
                                value: Math.round(editor.fps)
                                editable: true
                                Layout.preferredWidth: 96
                                onValueModified: editor.setTimelineFps(value)
                                Accessible.name: "Timeline frames per second"
                            }
                            CheckBox {
                                objectName: "timelineLoopControl"
                                text: "Loop"
                                checked: editor.looping
                                onToggled: if (checked !== editor.looping) editor.setLooping(checked)
                            }
                            Label { text: "Range"; color: window.tokens.inkSecondary; Layout.leftMargin: 6; Layout.rightMargin: 4 }
                            // Frames are numbered from 1 everywhere a person reads them ("Frame 1"
                            // below); the model's index is 0-based, so the range shows index + 1.
                            ComboBox {
                                id: rangeStartControl
                                objectName: "rangeStartControl"
                                model: editor.frames
                                textRole: "index"
                                valueRole: "frameId"
                                Layout.preferredWidth: 72
                                displayText: String(currentIndex + 1)
                                Accessible.name: "Playback range first frame"
                                delegate: ItemDelegate {
                                    required property int index
                                    width: ListView.view ? ListView.view.width : implicitWidth
                                    text: String(index + 1)
                                }
                                currentIndex: editor.frames.indexOf(editor.rangeStart)
                                onActivated: if (currentIndex <= rangeEndControl.currentIndex)
                                    editor.setPlaybackRange(currentValue, rangeEndControl.currentValue)
                            }
                            Label { text: "–"; color: window.tokens.inkSecondary }
                            ComboBox {
                                id: rangeEndControl
                                objectName: "rangeEndControl"
                                model: editor.frames
                                textRole: "index"
                                valueRole: "frameId"
                                Layout.preferredWidth: 72
                                displayText: String(currentIndex + 1)
                                Accessible.name: "Playback range last frame"
                                delegate: ItemDelegate {
                                    required property int index
                                    width: ListView.view ? ListView.view.width : implicitWidth
                                    text: String(index + 1)
                                }
                                currentIndex: editor.frames.indexOf(editor.rangeEnd)
                                onActivated: if (currentIndex >= rangeStartControl.currentIndex)
                                    editor.setPlaybackRange(rangeStartControl.currentValue, currentValue)
                            }
                        }

                        ListView {
                            id: timelineList
                            objectName: "timelineFrameList"
                            visible: window.timelineOpen
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            orientation: ListView.Horizontal
                            spacing: 6
                            clip: true
                            model: editor.frames
                            delegate: Rectangle {
                                required property var frameId
                                required property int index
                                required property int duration
                                required property bool current
                                required property bool inRange
                                required property bool isFirst
                                required property bool isLast
                                width: 82
                                height: timelineList.height
                                radius: 7
                                color: current ? window.tokens.borderSubtle : inRange ? window.tokens.surfaceSunken : window.tokens.surfaceRaised
                                border.width: current ? 2 : 1
                                border.color: current ? window.tokens.focusRing : inRange ? window.tokens.borderStrong : window.tokens.borderSubtle
                                TapHandler { onTapped: editor.setCurrentFrame(frameId) }
                                Column {
                                    anchors.centerIn: parent
                                    spacing: 2
                                    Label {
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        text: "Frame " + (index + 1)
                                        color: window.tokens.inkPrimary
                                        font.weight: current ? Font.DemiBold : Font.Normal
                                    }
                                    Label {
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        text: duration + " ms"
                                        color: window.tokens.inkSecondary
                                        font.pixelSize: 10
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Rectangle {
                id: inspector
                visible: !window.panelsHidden
                Layout.preferredWidth: Math.min(370, Math.max(270, window.width * 0.25))
                Layout.fillHeight: true
                color: window.tokens.surfaceRaised
                border.color: window.tokens.borderSubtle
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0
                    // M9: Photoshop's Navigator, toggled from View > Navigator.
                    NavigatorPanel {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 170
                        visible: window.navigatorVisible
                        app: window
                        mainCanvas: canvas
                    }
                    TabBar {
                        id: tabs
                        Layout.fillWidth: true
                        TabButton {
                            text: "Layers"
                            Accessible.name: "Layers inspector"
                        }
                        TabButton {
                            text: "Options"
                            Accessible.name: "Tool and operation options"
                        }
                        TabButton {
                            text: "Agent"
                            Accessible.name: "Agent proposals"
                        }
                    }
                    StackLayout {
                        currentIndex: tabs.currentIndex
                        Layout.fillWidth: true
                        Layout.fillHeight: true

                        LayerPanel {
                            app: window
                            textDialog: textSemanticDialog
                            vectorDialog: vectorSemanticDialog
                            filterWindow: filterBrowser
                            blendIfWindow: blendIfDialog
                        }

                        OptionsPanel {
                            app: window
                            tipDialog: brushTipDialog
                            gradientStartPicker: gradientStartDialog
                            gradientEndPicker: gradientEndDialog
                        }

                        Item {
                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 12
                                spacing: 10
                                Rectangle {
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: agentStatusRow.implicitHeight + 18
                                    radius: 8
                                    color: window.tokens.surfaceSunken
                                    border.color: window.tokens.borderSubtle
                                    RowLayout {
                                        id: agentStatusRow
                                        anchors.fill: parent
                                        anchors.margins: 9
                                        Rectangle {
                                            Layout.preferredWidth: 8
                                            Layout.preferredHeight: 8
                                            radius: 4
                                            color: editor.agentBusy ? window.tokens.statusWarning : editor.liveAgentConfigured ? window.tokens.statusSuccess : window.tokens.borderSubtle
                                        }
                                        Label {
                                            Layout.fillWidth: true
                                            text: editor.agentStatus
                                            color: window.tokens.inkPrimary
                                            wrapMode: Text.Wrap
                                            font.pixelSize: 11
                                        }
                                        BusyIndicator {
                                            visible: editor.agentBusy
                                            running: editor.agentBusy
                                            Layout.preferredWidth: 24
                                            Layout.preferredHeight: 24
                                            Accessible.name: "Redrob request in progress"
                                        }
                                    }
                                }
                                Label {
                                    text: editor.liveAgentConfigured ? "Ask Redrob for a safe edit proposal" : "Create an explicitly no-network local proposal"
                                    font.weight: Font.DemiBold
                                    wrapMode: Text.Wrap
                                    Layout.fillWidth: true
                                }
                                TextArea {
                                    id: prompt
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 90
                                    placeholderText: editor.liveAgentConfigured ? "Describe an edit for review." : "Local subset: selection, gradient, threshold, blur, flip, rotate, clear, fill, layers, undo/redo"
                                    wrapMode: TextEdit.Wrap
                                    Accessible.name: "Agent prompt"
                                }
                                Button {
                                    Layout.fillWidth: true
                                    text: editor.agentBusy ? "Contacting Redrob…" : editor.liveAgentConfigured ? "Create live proposal" : "Create local no-network proposal"
                                    enabled: prompt.text.trim().length > 0 && !editor.agentBusy
                                    Accessible.name: "Create a proposal without applying it"
                                    onClicked: if (editor.proposePrompt(prompt.text))
                                        prompt.clear()
                                }
                                Label {
                                    Layout.fillWidth: true
                                    visible: editor.assistantText.length > 0
                                    text: editor.assistantText
                                    color: window.tokens.inkPrimary
                                    wrapMode: Text.Wrap
                                    font.pixelSize: 12
                                }
                                // P13. Let redrob-code drive the graphics tools over a loopback MCP
                                // endpoint. Off at every launch; its edits land in the list below as
                                // proposals, exactly like the hosted agent's.
                                Switch {
                                    objectName: "mcpEnableSwitch"
                                    Layout.fillWidth: true
                                    text: "Allow redrob-code to connect (this computer only)"
                                    checked: editor.mcpEnabled
                                    onToggled: editor.mcpEnabled = checked
                                    Accessible.name: "Allow redrob-code to propose edits through a local MCP connection"
                                }
                                Label {
                                    Layout.fillWidth: true
                                    text: editor.mcpStatus
                                    color: window.tokens.inkSecondary
                                    wrapMode: Text.Wrap
                                    font.pixelSize: 11
                                }
                                TextArea {
                                    id: mcpSnippet
                                    objectName: "mcpConfigSnippet"
                                    Layout.fillWidth: true
                                    visible: editor.mcpEnabled
                                    readOnly: true
                                    selectByMouse: true
                                    wrapMode: TextEdit.WrapAnywhere
                                    font.family: "monospace"
                                    font.pixelSize: 11
                                    text: editor.mcpConfigSnippet
                                    Accessible.name: "redrob-code configuration for this session, including its access token"
                                }
                                Button {
                                    objectName: "mcpCopySnippet"
                                    Layout.fillWidth: true
                                    visible: editor.mcpEnabled
                                    text: "Copy redrob-code config"
                                    onClicked: {
                                        mcpSnippet.selectAll()
                                        mcpSnippet.copy()
                                        mcpSnippet.deselect()
                                    }
                                }
                                // A2. Hand redrob-code a task; it works through the canvas tools
                                // and every edit still waits in PENDING PROPOSALS for approval.
                                Label {
                                    visible: editor.mcpEnabled
                                    text: "AUTOMATE WITH REDROB-CODE"
                                    color: window.tokens.inkSecondary
                                    font.pixelSize: 11
                                    font.weight: Font.DemiBold
                                }
                                TextArea {
                                    id: codeTask
                                    objectName: "codeTaskInput"
                                    Layout.fillWidth: true
                                    visible: editor.mcpEnabled
                                    enabled: !editor.codeRunner.running
                                    placeholderText: editor.codeRunner.available
                                        ? "e.g. Add a title layer and a soft vignette"
                                        : "Install redrob-code (the redrob command) to use this"
                                    wrapMode: TextEdit.Wrap
                                    font.pixelSize: 12
                                    Accessible.name: "Task for redrob-code"
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    visible: editor.mcpEnabled
                                    Button {
                                        objectName: "codeTaskRun"
                                        Layout.fillWidth: true
                                        text: "Run task"
                                        enabled: editor.codeRunner.available && !editor.codeRunner.running
                                            && codeTask.text.trim().length > 0
                                        onClicked: editor.runRedrobCodeTask(codeTask.text)
                                    }
                                    Button {
                                        objectName: "codeTaskStop"
                                        text: "Stop"
                                        visible: editor.codeRunner.running
                                        onClicked: editor.codeRunner.stop()
                                    }
                                }
                                Label {
                                    Layout.fillWidth: true
                                    visible: editor.mcpEnabled && text.length > 0
                                    text: editor.codeRunner.status
                                    color: window.tokens.inkSecondary
                                    wrapMode: Text.Wrap
                                    font.pixelSize: 11
                                }
                                Label {
                                    objectName: "codeTaskLog"
                                    Layout.fillWidth: true
                                    visible: editor.mcpEnabled && editor.codeRunner.log.length > 0
                                    text: editor.codeRunner.log.slice(-8).join("\n")
                                    color: window.tokens.inkSecondary
                                    wrapMode: Text.Wrap
                                    font.family: "monospace"
                                    font.pixelSize: 11
                                }
                                Label {
                                    text: "PENDING PROPOSALS"
                                    color: window.tokens.inkSecondary
                                    font.pixelSize: 11
                                    font.weight: Font.DemiBold
                                }
                                ListView {
                                    id: proposalList
                                    Layout.fillWidth: true
                                    Layout.fillHeight: true
                                    spacing: 8
                                    clip: true
                                    model: editor.proposals
                                    delegate: Rectangle {
                                        required property string proposalId
                                        required property string proposalTitle
                                        required property string proposalSummary
                                        required property string proposalAction
                                        width: proposalList.width
                                        height: cardContent.implicitHeight + 20
                                        radius: 9
                                        color: window.tokens.surfaceSunken
                                        border.color: window.tokens.borderSubtle
                                        ColumnLayout {
                                            id: cardContent
                                            anchors.left: parent.left
                                            anchors.right: parent.right
                                            anchors.top: parent.top
                                            anchors.margins: 10
                                            Label {
                                                Layout.fillWidth: true
                                                text: proposalTitle
                                                font.weight: Font.DemiBold
                                                wrapMode: Text.Wrap
                                            }
                                            Label {
                                                Layout.fillWidth: true
                                                text: proposalSummary
                                                color: window.tokens.inkSecondary
                                                wrapMode: Text.Wrap
                                                font.pixelSize: 12
                                            }
                                            RowLayout {
                                                Layout.fillWidth: true
                                                Button {
                                                    Layout.fillWidth: true
                                                    text: "Reject"
                                                    Accessible.name: "Reject " + proposalTitle
                                                    onClicked: editor.rejectProposal(proposalId)
                                                }
                                                Button {
                                                    Layout.fillWidth: true
                                                    text: "Apply"
                                                    highlighted: true
                                                    Accessible.name: "Apply " + proposalTitle
                                                    onClicked: editor.applyProposal(proposalId)
                                                }
                                            }
                                        }
                                    }
                                    Label {
                                        anchors.centerIn: parent
                                        visible: proposalList.count === 0
                                        text: "No pending proposals\nEdits always wait for Apply."
                                        horizontalAlignment: Text.AlignHCenter
                                        color: window.tokens.inkMuted
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            color: window.tokens.surfaceRaised
            border.color: window.tokens.borderSubtle
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 10
                anchors.rightMargin: 10
                Label {
                    // The measure tool's live read-out takes over the status text while measuring.
                    text: window.measureText.length > 0 ? window.measureText : editor.statusMessage
                    color: window.measureText.length > 0 ? window.tokens.inkPrimary : window.tokens.inkSecondary
                    font.pixelSize: 11
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Label {
                    text: editor.selectionActive ? "Selection active" : "No selection"
                    color: editor.selectionActive ? window.tokens.statusInfo : window.tokens.inkSecondary
                    font.pixelSize: 11
                }
                Label {
                    text: "Gen " + editor.generation
                    color: window.tokens.inkSecondary
                    font.pixelSize: 11
                }
                Label {
                    text: (window.brushLike && editor.brushErase ? "eraser" : window.activeTool) + " · " + Math.round(editor.brushSize) + " px"
                          + " · " + editor.colorMode + " " + editor.precision
                    color: window.tokens.inkSecondary
                    font.pixelSize: 11
                }
            }
        }
    }
}
