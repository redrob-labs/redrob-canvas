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

    // A slider drawn from the tokens. The default style tints palette.highlight, so the fill measured
    // srgb(62,98,254) on screen against the token's #2b52ff -- close enough to pass a glance and not the
    // brand's colour. Track, fill and handle are painted here so the pixels are the token's own.
    component TokenSlider: Slider {
        id: tokenSlider
        // The control's own height is what takes the press. The background and handle below set
        // only width/height, which contribute nothing to implicit size, so without this the slider
        // was a few pixels tall and a press on the visible track or handle could miss it.
        implicitHeight: 28
        implicitWidth: 120
        background: Rectangle {
            x: tokenSlider.leftPadding
            y: tokenSlider.topPadding + tokenSlider.availableHeight / 2 - height / 2
            implicitWidth: 120
            implicitHeight: 4
            width: tokenSlider.availableWidth
            height: 4
            radius: 2
            color: window.tokens.borderSubtle
            Rectangle {
                width: tokenSlider.visualPosition * parent.width
                height: parent.height
                radius: 2
                color: tokenSlider.enabled ? window.tokens.actionPrimary : window.tokens.inkMuted
            }
        }
        handle: Rectangle {
            x: tokenSlider.leftPadding + tokenSlider.visualPosition * (tokenSlider.availableWidth - width)
            y: tokenSlider.topPadding + tokenSlider.availableHeight / 2 - height / 2
            implicitWidth: 14
            implicitHeight: 14
            width: 14
            height: 14
            radius: 7
            color: window.tokens.surfaceBase
            border.width: 2
            border.color: tokenSlider.visualFocus ? window.tokens.focusRing
                                                  : (tokenSlider.enabled ? window.tokens.actionPrimary : window.tokens.inkMuted)
        }
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

    property string activeTool: "brush"
    // The bucket tool's Lab tolerance, 0 to 255; 15 is the core's default.
    property int fillTolerance: 15
    // The frame strip is for animation; a still image does not need 90px of it. Closed until there
    // is more than one frame, and adding or duplicating a frame opens it.
    property bool timelineOpen: editor.frameCount > 1
    property real canvasZoom: 1.0
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
                                      vectorFill.text, vectorStrokeColor.text,
                                      Number(vectorStroke.text));
            return;
        }
        editor.addShapeFromBox(shapeKind, shapeKind === "ellipse" ? "Ellipse"
                                        : shapeKind === "line" ? "Line" : "Rectangle",
                               start.x, start.y, end.x, end.y, shapeCornerRadius,
                               vectorFill.text, vectorStrokeColor.text,
                               Number(vectorStroke.text));
    }
    property color gradientStartColor: "#f4f6ff"
    property color gradientEndColor: "#4267c9"
    property string samplingMode: "bilinear"
    property string exportFormat: "png"
    property bool exportAllowLoss: false
    property int exportJpegQuality: 90
    property color exportMatte: "#ffffff"

    function maskNodeFromSelection(nodeId) {
        editor.rasterMaskFromSelection(nodeId)
    }

    onActiveToolChanged: canvasPointer.cancelGesture()

    component CommandButton: ToolButton {
        id: commandButton
        // Optional design-system glyph (icons/ui/<name>.svg); iconOnly drops the label beside it.
        property string iconName: ""
        property bool iconOnly: false
        // The one emphasised action in a bar, filled with the primary action colour.
        property bool primary: false
        implicitHeight: 34
        leftPadding: iconOnly ? 8 : 10
        rightPadding: iconOnly ? 8 : 12
        spacing: 6
        font.pixelSize: 13
        display: iconName.length === 0 ? AbstractButton.TextOnly
                 : iconOnly ? AbstractButton.IconOnly : AbstractButton.TextBesideIcon
        icon.source: iconName.length > 0 ? "qrc:/icons/ui/" + iconName + ".svg" : ""
        icon.width: 18
        icon.height: 18
        icon.color: !enabled ? window.tokens.inkMuted
                    : primary ? window.tokens.inkOnBrand : window.tokens.inkSecondary
        palette.buttonText: primary ? window.tokens.inkOnBrand : window.tokens.inkPrimary
        ToolTip.visible: hovered
        ToolTip.delay: 500
        Accessible.name: ToolTip.text.length > 0 ? ToolTip.text : text
        background: Rectangle {
            radius: 7
            color: commandButton.primary
                   ? (commandButton.hovered ? window.tokens.actionPrimaryHover : window.tokens.actionPrimary)
                   : commandButton.down || commandButton.hovered ? window.tokens.borderSubtle : "transparent"
            FocusOutline { shown: commandButton.visualFocus; innerRadius: 7 }
        }
    }

    // redrob-ui :focus-visible: a 2px focusRing outline 2px outside the control, on keyboard focus only
    // (visualFocus is false after a mouse click). Placed inside a background so it follows its shape.
    component FocusOutline: Rectangle {
        required property bool shown
        property real ringWidth: 2
        visible: shown
        anchors.fill: parent
        anchors.margins: -2 * ringWidth
        property real innerRadius: 0
        radius: innerRadius + 2 * ringWidth
        color: "transparent"
        border.width: ringWidth
        border.color: window.tokens.focusRing
    }

    component ToolRailButton: ToolButton {
        id: toolButton
        required property string toolId
        // Design-system glyph name under icons/ui/ (third_party/redrob-ui/icons, pinned).
        required property string iconName
        required property string toolName
        // One key picks the tool. Typing into a field does not trigger it: a focused text input
        // takes printable keys before window shortcuts see them.
        property string shortcut: ""
        // Said instead of the name while the tool cannot be used, so the tooltip explains why.
        property string disabledHint: ""
        checkable: true
        checked: window.activeTool === toolId
        implicitWidth: 40
        implicitHeight: 40
        display: AbstractButton.IconOnly
        icon.source: "qrc:/icons/ui/" + iconName + ".svg"
        icon.width: 20
        icon.height: 20
        // 45-icons.md: colour from the token, never the icon. The SVG strokes currentColor.
        Layout.alignment: Qt.AlignHCenter
        ToolTip.visible: hovered
        ToolTip.delay: 450
        ToolTip.text: !enabled && disabledHint.length > 0 ? disabledHint
                      : shortcut.length > 0 ? toolName + "   " + shortcut : toolName
        Accessible.name: toolName
        onClicked: window.activeTool = toolId
        Shortcut {
            sequence: toolButton.shortcut
            enabled: toolButton.shortcut.length > 0 && toolButton.enabled
            onActivated: window.activeTool = toolButton.toolId
        }
        // Selected reads like the system's pressed chip (brand-subtle fill, brand ink); the focus ring is
        // kept for keyboard focus alone, as in redrob-ui's :focus-visible, so the two never look alike.
        icon.color: !enabled ? window.tokens.inkMuted
                             : checked ? window.tokens.inkBrand : window.tokens.inkSecondary
        background: Rectangle {
            radius: 8
            color: toolButton.checked ? window.tokens.surfaceBrandSubtle
                   : toolButton.hovered ? window.tokens.surfaceSunken : "transparent"
            FocusOutline { shown: toolButton.visualFocus; innerRadius: 8 }
        }
    }

    component SectionTitle: Label {
        Layout.fillWidth: true
        topPadding: 8
        text: "SECTION"
        color: window.tokens.inkSecondary
        font.pixelSize: 10
        font.weight: Font.DemiBold
    }

    // A named group inside an option section. Follows Krita's brush editor, which keeps the tip
    // (shape, size, hardness, ratio) apart from how paint lands (opacity) and how the stroke is
    // drawn (smoothing, mirror). Sentence case, so it reads below the section's capital heading.
    component SubsectionTitle: Label {
        Layout.fillWidth: true
        topPadding: 6
        color: window.tokens.inkMuted
        font.pixelSize: 11
        font.weight: Font.DemiBold
    }

    // One group of options. Tool groups show only while their tool is active, so the panel holds
    // what the current tool needs instead of every operation at once. Image-wide groups collapse.
    component OptionSection: ColumnLayout {
        id: section
        property string title
        property bool shown: true
        property bool collapsible: false
        property bool expanded: true
        // Opens the group when it becomes true (e.g. its tool is picked); the user can still close it.
        property bool autoExpand: false
        default property alias content: sectionBody.data
        onAutoExpandChanged: if (autoExpand) expanded = true
        Layout.fillWidth: true
        spacing: 6
        visible: shown

        AbstractButton {
            id: sectionHeader
            Layout.fillWidth: true
            Layout.topMargin: 6
            implicitHeight: 28
            enabled: section.collapsible
            hoverEnabled: true
            focusPolicy: section.collapsible ? Qt.StrongFocus : Qt.NoFocus
            Accessible.role: section.collapsible ? Accessible.Button : Accessible.Heading
            Accessible.name: section.title + (section.collapsible ? (section.expanded ? ", expanded" : ", collapsed") : "")
            onClicked: section.expanded = !section.expanded
            contentItem: RowLayout {
                spacing: 6
                Label {
                    Layout.fillWidth: true
                    text: section.title
                    color: window.tokens.inkSecondary
                    font.pixelSize: 10
                    font.weight: Font.DemiBold
                }
                ToolButton {
                    visible: section.collapsible
                    implicitWidth: 20
                    implicitHeight: 20
                    padding: 0
                    display: AbstractButton.IconOnly
                    focusPolicy: Qt.NoFocus
                    icon.source: "qrc:/icons/ui/" + (section.expanded ? "chevronDown" : "chevronRight") + ".svg"
                    icon.width: 16
                    icon.height: 16
                    icon.color: window.tokens.inkSecondary
                    Accessible.ignored: true
                    background: null
                    onClicked: section.expanded = !section.expanded
                }
            }
            background: Rectangle {
                radius: 6
                color: sectionHeader.hovered ? window.tokens.surfaceSunken : "transparent"
                border.color: sectionHeader.visualFocus ? window.tokens.focusRing : "transparent"
            }
        }
        ColumnLayout {
            id: sectionBody
            Layout.fillWidth: true
            spacing: 6
            visible: section.expanded
        }
    }

    component NumericField: TextField {
        Layout.fillWidth: true
        selectByMouse: true
        horizontalAlignment: TextInput.AlignRight
        Accessible.name: placeholderText
    }

    // Thin separator between groups of tools on the rail (paint, transform, fill, select, view).
    component RailDivider: Rectangle {
        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: 24
        Layout.preferredHeight: 1
        Layout.topMargin: 3
        Layout.bottomMargin: 3
        color: window.tokens.borderSubtle
    }

    // Thin separator between groups of timeline controls.
    component TimelineDivider: Rectangle {
        Layout.preferredWidth: 1
        Layout.preferredHeight: 20
        Layout.leftMargin: 6
        Layout.rightMargin: 6
        color: window.tokens.borderSubtle
    }

    // Label column for one-value-per-row parameters, so every field in a group starts at the same x.
    component ParamLabel: Label {
        Layout.preferredWidth: 84
        elide: Text.ElideRight
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
        title: "Open Redrob Project"
        fileMode: FileDialog.OpenFile
        nameFilters: ["Redrob projects (*.rrg)"]
        onAccepted: editor.openProject(selectedFile)
    }
    FileDialog {
        id: importDialog
        title: "Import Interchange File"
        fileMode: FileDialog.OpenFile
        nameFilters: ["Supported interchange (*.png *.jpg *.jpeg *.webp *.ora *.svg)",
                      "PNG images (*.png)", "JPEG images (*.jpg *.jpeg)",
                      "Lossless WebP images (*.webp)", "OpenRaster documents (*.ora)",
                      "Limited SVG documents (*.svg)"]
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
    Shortcut {
        sequence: "E"
        enabled: editor.activeNodeCanEditRaster
        onActivated: {
            editor.brushErase = !editor.brushErase;
            window.activeTool = "brush";
        }
    }
    Shortcut {
        sequence: "Escape"
        onActivated: canvasPointer.cancelGesture()
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
                    visible: window.activeTool === "brush" && window.width >= 1120
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
                Layout.preferredWidth: 58
                Layout.fillHeight: true
                color: window.tokens.surfaceRaised
                border.color: window.tokens.borderSubtle
                ScrollView {
                    anchors.fill: parent
                    anchors.topMargin: 6
                    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
                    ColumnLayout {
                        width: 56
                        spacing: 3
                        ToolRailButton {
                            objectName: "brushToolAction"
                            iconName: "brush"
                            toolId: "brush"
                            toolName: "Brush"
                            shortcut: "B"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Brush requires a raster node"
                        }
                        ToolRailButton {
                            objectName: "shapeToolAction"
                            iconName: "shape"
                            toolId: "shape"
                            toolName: "Shape"
                            shortcut: "U"
                        }
                        RailDivider {}
                        ToolRailButton {
                            objectName: "transformToolAction"
                            iconName: "transform"
                            toolId: "transform"
                            toolName: "Move layer"
                            shortcut: "T"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Move requires a raster node"
                        }
                        ToolRailButton {
                            iconName: "crop"
                            toolId: "crop"
                            toolName: "Crop canvas"
                            shortcut: "C"
                        }
                        RailDivider {}
                        ToolRailButton {
                            objectName: "fillToolAction"
                            iconName: "fill"
                            toolId: "fill"
                            toolName: "Fill (bucket)"
                            shortcut: "F"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Fill requires a raster node"
                        }
                        ToolRailButton {
                            objectName: "gradientToolAction"
                            iconName: "gradient"
                            toolId: "gradient"
                            toolName: "Gradient"
                            shortcut: "G"
                            enabled: editor.activeNodeCanEditRaster
                            disabledHint: "Gradient requires a raster node"
                        }
                        ToolRailButton {
                            objectName: "pickerToolAction"
                            iconName: "eyedropper"
                            toolId: "picker"
                            toolName: "Pick colour"
                            shortcut: "P"
                        }
                        RailDivider {}
                        ToolRailButton {
                            iconName: "rectangle"
                            toolId: "rectangle"
                            toolName: "Rectangle selection"
                            shortcut: "R"
                        }
                        ToolRailButton {
                            iconName: "ellipse"
                            toolId: "ellipse"
                            toolName: "Ellipse selection"
                            shortcut: "J"
                        }
                        RailDivider {}
                        ToolRailButton {
                            iconName: "eye"
                            toolId: "inspect"
                            toolName: "Inspect (view only)"
                            shortcut: "I"
                        }
                        Rectangle {
                            Layout.alignment: Qt.AlignHCenter
                            Layout.preferredWidth: 32
                            Layout.preferredHeight: 1
                            color: window.tokens.borderSubtle
                        }
                        ToolButton {
                            Layout.alignment: Qt.AlignHCenter
                            implicitWidth: 38
                            implicitHeight: 38
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
                        Label {
                            text: Math.round(editor.brushSize)
                            color: window.tokens.inkSecondary
                            Layout.alignment: Qt.AlignHCenter
                            font.pixelSize: 10
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

                CanvasItem {
                    id: canvas
                    anchors.top: parent.top
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: timelinePanel.top
                    image: editor.renderImage
                    selectionMask: editor.selectionMask
                    selectionActive: editor.selectionActive
                    zoom: window.canvasZoom
                    Accessible.name: "Document canvas with selection overlay"

                    PointHandler {
                        id: canvasPointer
                        objectName: "canvasPointer"
                        target: null
                        acceptedButtons: Qt.LeftButton
                        property bool gestureActive: false
                        property point startCanvas: Qt.point(0, 0)
                        property point endCanvas: Qt.point(0, 0)

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
                            return window.activeTool === "brush" || window.activeTool === "fill"
                                || window.activeTool === "gradient" || window.activeTool === "transform";
                        }
                        function cancelGesture() {
                            if (gestureActive && window.activeTool === "brush")
                                editor.cancelStroke();
                            gestureActive = false;
                            canvas.clearPreview();
                        }
                        function commitGesture() {
                            if (!gestureActive)
                                return;
                            gestureActive = false;
                            const dx = endCanvas.x - startCanvas.x;
                            const dy = endCanvas.y - startCanvas.y;
                            if (window.activeTool === "brush") {
                                editor.endStroke();
                            } else if (window.activeTool === "fill") {
                                editor.floodFill(endCanvas.x, endCanvas.y, editor.brushColor, window.fillTolerance);
                            } else if (window.activeTool === "rectangle") {
                                editor.selectRectangle(startCanvas.x, startCanvas.y, dx, dy, window.selectionMode);
                            } else if (window.activeTool === "ellipse") {
                                editor.selectEllipse(startCanvas.x, startCanvas.y, dx, dy, window.selectionMode);
                            } else if (window.activeTool === "gradient") {
                                if (window.gradientKind === "linear")
                                    editor.linearGradient(startCanvas.x, startCanvas.y, endCanvas.x, endCanvas.y, window.gradientStartColor, window.gradientEndColor);
                                else
                                    editor.radialGradient(startCanvas.x, startCanvas.y, Math.hypot(dx, dy), window.gradientStartColor, window.gradientEndColor);
                            } else if (window.activeTool === "shape") {
                                window.commitShape(startCanvas, endCanvas);
                            } else if (window.activeTool === "crop") {
                                editor.cropCanvas(startCanvas.x, startCanvas.y, dx, dy);
                            } else if (window.activeTool === "transform") {
                                editor.transformActive(1, 0, 0, 1, dx, dy, window.samplingMode);
                            }
                            canvas.clearPreview();
                        }
                        onActiveChanged: {
                            if (active) {
                                const position = point.position;
                                if (!canvas.containsCanvasPoint(position) || window.activeTool === "inspect"
                                        || (activeToolNeedsRaster() && !editor.activeNodeCanEditRaster))
                                    return;
                                startCanvas = boundedCanvasPoint(position);
                                endCanvas = startCanvas;
                                gestureActive = true;
                                if (window.activeTool === "brush") {
                                    editor.beginStroke(startCanvas.x, startCanvas.y, pointPressure(point));
                                } else if (window.activeTool === "picker") {
                                    window.pickColorAt(startCanvas);
                                } else if (window.activeTool !== "fill") {
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
                            if (window.activeTool === "brush") {
                                if (canvas.containsCanvasPoint(position))
                                    editor.addStrokePoint(endCanvas.x, endCanvas.y, pointPressure(point));
                            } else if (window.activeTool === "picker") {
                                window.pickColorAt(endCanvas);
                            } else {
                                canvas.previewEnd = endCanvas;
                            }
                        }
                        onCanceled: point => cancelGesture()
                    }

                    HoverHandler {
                        cursorShape: window.activeTool === "inspect" ? Qt.ArrowCursor : Qt.CrossCursor
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
                Layout.preferredWidth: Math.min(370, Math.max(270, window.width * 0.25))
                Layout.fillHeight: true
                color: window.tokens.surfaceRaised
                border.color: window.tokens.borderSubtle
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0
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

                        Item {
                            Dialog {
                                id: textSemanticDialog
                                objectName: "textSemanticEditor"
                                property string nodeId: ""
                                property string sourceFontId: "font8x8-basic-0.3.1"
                                property string sourceFontFamily: "font8x8 Basic Latin"
                                title: nodeId.length > 0 ? "Edit deterministic text" : "Add deterministic text"
                                modal: true
                                anchors.centerIn: parent
                                standardButtons: Dialog.Ok | Dialog.Cancel
                                onAccepted: {
                                    if (nodeId.length > 0)
                                        editor.setTextContent(nodeId, semanticText.text, Number(textX.text),
                                                              Number(textY.text), Number(textSize.text), textColor.text,
                                                              sourceFontFamily, sourceFontId)
                                    else
                                        editor.addTextNode(textName.text, semanticText.text, Number(textX.text),
                                                           Number(textY.text), Number(textSize.text), textColor.text)
                                }
                                ColumnLayout {
                                    width: 360
                                    Label { text: "Printable ASCII + newline only · embedded font8x8"; wrapMode: Text.Wrap }
                                    TextField {
                                        id: textName
                                        Layout.fillWidth: true
                                        placeholderText: "Node name"
                                        visible: textSemanticDialog.nodeId.length === 0
                                    }
                                    TextArea {
                                        id: semanticText
                                        objectName: "semanticTextInput"
                                        Layout.fillWidth: true
                                        Layout.preferredHeight: 110
                                        wrapMode: TextEdit.NoWrap
                                        onTextChanged: if (length > 262144) text = text.slice(0, 262144)
                                        Accessible.name: "Bounded semantic text content"
                                    }
                                    RowLayout {
                                        Label { text: "X" }
                                        TextField { id: textX; text: "24"; validator: DoubleValidator {} }
                                        Label { text: "Y" }
                                        TextField { id: textY; text: "24"; validator: DoubleValidator {} }
                                        Label { text: "Size" }
                                        TextField { id: textSize; text: "32"; validator: DoubleValidator { bottom: 0.00390625; top: 4096 } }
                                    }
                                    RowLayout {
                                        Label { text: "Color" }
                                        TextField {
                                            id: textColor
                                            Layout.fillWidth: true
                                            text: "#ff000000"
                                        }
                                    }
                                    Label {
                                        text: "Font: " + textSemanticDialog.sourceFontFamily + " (" + textSemanticDialog.sourceFontId + ")"
                                        wrapMode: Text.Wrap
                                    }
                                }
                            }
                            Dialog {
                                id: vectorSemanticDialog
                                objectName: "vectorRectangleEditor"
                                property string nodeId: ""
                                title: nodeId.length > 0 ? "Edit recognized vector rectangle" : "Add vector rectangle"
                                modal: true
                                anchors.centerIn: parent
                                standardButtons: Dialog.Ok | Dialog.Cancel
                                onAccepted: {
                                    if (nodeId.length > 0)
                                        editor.setVectorRectangle(nodeId, Number(vectorX.text), Number(vectorY.text),
                                                                  Number(vectorW.text), Number(vectorH.text),
                                                                  vectorFill.text, vectorStrokeColor.text,
                                                                  Number(vectorStroke.text))
                                    else
                                        editor.addVectorRectangle(vectorName.text, Number(vectorX.text), Number(vectorY.text),
                                                                  Number(vectorW.text), Number(vectorH.text),
                                                                  vectorFill.text, vectorStrokeColor.text,
                                                                  Number(vectorStroke.text))
                                }
                                ColumnLayout {
                                    width: 380
                                    Label { text: "Core-rendered solid rectangle (no SVG/platform painter)"; wrapMode: Text.Wrap }
                                    TextField {
                                        id: vectorName
                                        Layout.fillWidth: true
                                        placeholderText: "Node name"
                                        visible: vectorSemanticDialog.nodeId.length === 0
                                    }
                                    GridLayout {
                                        columns: 4
                                        Label { text: "X" }
                                        TextField { id: vectorX; text: "48"; validator: DoubleValidator {} }
                                        Label { text: "Y" }
                                        TextField { id: vectorY; text: "48"; validator: DoubleValidator {} }
                                        Label { text: "Width" }
                                        TextField { id: vectorW; text: "180"; validator: DoubleValidator { bottom: 0.00390625 } }
                                        Label { text: "Height" }
                                        TextField { id: vectorH; text: "120"; validator: DoubleValidator { bottom: 0.00390625 } }
                                        Label { text: "Stroke" }
                                        TextField { id: vectorStroke; text: "2"; validator: DoubleValidator { bottom: 0.00390625; top: 4096 } }
                                    }
                                    RowLayout {
                                        Label { text: "Fill" }
                                        TextField { id: vectorFill; text: "#ff5378dc" }
                                        Label { text: "Stroke color" }
                                        TextField { id: vectorStrokeColor; text: "#ff20242a" }
                                    }
                                }
                            }
                            Dialog {
                                id: rasterizeSemanticWarning
                                objectName: "rasterizeSemanticWarning"
                                property string nodeId: ""
                                title: "Rasterize semantic node?"
                                modal: true
                                anchors.centerIn: parent
                                standardButtons: Dialog.Ok | Dialog.Cancel
                                onAccepted: editor.rasterizeSemanticNode(nodeId)
                                Label {
                                    width: 360
                                    wrapMode: Text.Wrap
                                    text: "This replaces editable text/vector content with a raster cel on the current frame only. Other frames are blank. Node ID, name, hierarchy, visibility, opacity, and blend are preserved."
                                }
                            }
                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 10
                                spacing: 8
                                Menu {
                                    id: addNodeMenu
                                    MenuItem {
                                        text: "Add raster layer"
                                        Accessible.name: "Add raster layer at document root"
                                        onTriggered: editor.addLayer()
                                    }
                                    MenuItem {
                                        text: "Add group"
                                        Accessible.name: "Add layer group at document root"
                                        onTriggered: editor.addGroup()
                                    }
                                    MenuItem {
                                        objectName: "addTextNodeAction"
                                        text: "Add text"
                                        Accessible.name: "Add deterministic text node"
                                        onTriggered: {
                                            textSemanticDialog.nodeId = ""
                                            textName.text = "New text"
                                            semanticText.text = "Text"
                                            textX.text = "24"
                                            textY.text = "24"
                                            textSize.text = "32"
                                            textColor.text = editor.brushColor.toString()
                                            textSemanticDialog.sourceFontId = "font8x8-basic-0.3.1"
                                            textSemanticDialog.sourceFontFamily = "font8x8 Basic Latin"
                                            textSemanticDialog.open()
                                        }
                                    }
                                    MenuItem {
                                        objectName: "addVectorNodeAction"
                                        text: "Add vector rectangle"
                                        Accessible.name: "Add deterministic vector rectangle"
                                        onTriggered: {
                                            vectorSemanticDialog.nodeId = ""
                                            vectorName.text = "New vector"
                                            vectorX.text = "48"
                                            vectorY.text = "48"
                                            vectorW.text = "180"
                                            vectorH.text = "120"
                                            vectorStroke.text = "2"
                                            vectorFill.text = editor.brushColor.toString()
                                            vectorStrokeColor.text = "#ff20242a"
                                            vectorSemanticDialog.open()
                                        }
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "LAYER STACK"
                                        color: window.tokens.inkSecondary
                                        font.pixelSize: 11
                                        font.weight: Font.DemiBold
                                    }
                                    Item {
                                        Layout.fillWidth: true
                                    }
                                    CommandButton {
                                        text: "Add node"
                                        iconName: "plus"
                                        iconOnly: true
                                        ToolTip.text: "Add raster, group, text, or vector node"
                                        onClicked: addNodeMenu.open()
                                    }
                                    CommandButton {
                                        text: "Delete layer"
                                        iconName: "minus"
                                        iconOnly: true
                                        enabled: layerList.count > 1
                                        ToolTip.text: "Delete active layer"
                                        onClicked: editor.deleteLayer(editor.activeLayerId)
                                    }
                                }
                                ListView {
                                    id: layerList
                                    Layout.fillWidth: true
                                    Layout.fillHeight: true
                                    spacing: 5
                                    clip: true
                                    model: editor.layers
                                    delegate: Rectangle {
                                        required property string layerId
                                        required property string layerName
                                        required property bool layerVisible
                                        required property real layerOpacity
                                        required property string blendMode
                                        required property bool activeLayer
                                        required property string nodeKind
                                        required property string parentId
                                        required property int nodeDepth
                                        required property int siblingIndex
                                        required property int siblingCount
                                        required property bool isTopSibling
                                        required property bool isBottomSibling
                                        required property bool hasChildren
                                        required property bool hasMask
                                        required property bool maskEnabled
                                        required property bool canEditRaster
                                        required property bool canEditText
                                        required property bool canEditVector
                                        required property bool canRasterize
                                        required property bool isSemantic
                                        required property string semanticPreview
                                        required property string semanticTextSource
                                        required property bool semanticPreviewTruncated
                                        required property int semanticPathCount
                                        required property int semanticCommandCount
                                        required property string semanticFontId
                                        required property string semanticFontFamily
                                        required property real semanticFontSize
                                        required property real semanticOriginX
                                        required property real semanticOriginY
                                        required property color semanticColor
                                        required property bool semanticRectangleRecognized
                                        required property real semanticRectangleX
                                        required property real semanticRectangleY
                                        required property real semanticRectangleWidth
                                        required property real semanticRectangleHeight
                                        required property color semanticRectangleFill
                                        required property color semanticRectangleStroke
                                        required property real semanticRectangleStrokeWidth
                                        width: layerList.width
                                        height: 74
                                        radius: 8
                                        color: activeLayer ? window.tokens.borderSubtle : window.tokens.surfaceSunken
                                        border.color: activeLayer ? window.tokens.focusRing : window.tokens.borderSubtle
                                        Menu {
                                            id: layerActions
                                            MenuItem {
                                                text: "Move to document root"
                                                enabled: parentId.length > 0
                                                Accessible.name: "Move " + layerName + " to document root"
                                                onTriggered: editor.moveNode(layerId, "", 0)
                                            }
                                            MenuItem {
                                                text: "Move active node into this group"
                                                enabled: nodeKind === "group" && !activeLayer
                                                Accessible.name: "Move active node into group " + layerName
                                                onTriggered: editor.moveNode(editor.activeLayerId, layerId, 0)
                                            }
                                            MenuItem {
                                                objectName: "moveTowardTopAction-" + layerId
                                                text: "Move toward top"
                                                enabled: !isTopSibling
                                                Accessible.name: "Move " + layerName + " toward top among siblings"
                                                onTriggered: editor.moveNode(layerId, parentId, siblingIndex + 1)
                                            }
                                            MenuItem {
                                                objectName: "moveTowardBottomAction-" + layerId
                                                text: "Move toward bottom"
                                                enabled: !isBottomSibling
                                                Accessible.name: "Move " + layerName + " toward bottom among siblings"
                                                onTriggered: editor.moveNode(layerId, parentId, siblingIndex - 1)
                                            }
                                            MenuSeparator {}
                                            MenuItem {
                                                objectName: "editSemanticTextAction-" + layerId
                                                text: "Edit text content"
                                                enabled: canEditText
                                                Accessible.name: "Edit bounded text content for " + layerName
                                                onTriggered: {
                                                    textSemanticDialog.nodeId = layerId
                                                    semanticText.text = semanticTextSource
                                                    textX.text = String(semanticOriginX)
                                                    textY.text = String(semanticOriginY)
                                                    textSize.text = String(semanticFontSize)
                                                    textColor.text = semanticColor.toString()
                                                    textSemanticDialog.sourceFontId = semanticFontId
                                                    textSemanticDialog.sourceFontFamily = semanticFontFamily
                                                    textSemanticDialog.open()
                                                }
                                            }
                                            MenuItem {
                                                objectName: "editSemanticVectorAction-" + layerId
                                                text: semanticRectangleRecognized ? "Edit vector rectangle" : "Edit unavailable (not a rectangle)"
                                                enabled: canEditVector && semanticRectangleRecognized
                                                Accessible.name: semanticRectangleRecognized
                                                    ? "Edit recognized rectangle " + layerName
                                                    : "Arbitrary vectors cannot be edited as rectangles"
                                                onTriggered: {
                                                    vectorSemanticDialog.nodeId = layerId
                                                    vectorX.text = String(semanticRectangleX)
                                                    vectorY.text = String(semanticRectangleY)
                                                    vectorW.text = String(semanticRectangleWidth)
                                                    vectorH.text = String(semanticRectangleHeight)
                                                    vectorFill.text = semanticRectangleFill.toString()
                                                    vectorStrokeColor.text = semanticRectangleStroke.toString()
                                                    vectorStroke.text = String(semanticRectangleStrokeWidth)
                                                    vectorSemanticDialog.open()
                                                }
                                            }
                                            MenuItem {
                                                objectName: "rasterizeSemanticAction-" + layerId
                                                text: "Rasterize on current frame…"
                                                enabled: canRasterize
                                                Accessible.name: "Rasterize semantic node " + layerName + " on current frame only"
                                                onTriggered: {
                                                    rasterizeSemanticWarning.nodeId = layerId
                                                    rasterizeSemanticWarning.open()
                                                }
                                            }
                                            MenuSeparator {}
                                            MenuItem {
                                                text: "Add raster mask"
                                                enabled: !hasMask && (nodeKind === "raster" || nodeKind === "group")
                                                Accessible.name: "Add raster mask to " + layerName
                                                onTriggered: editor.addRasterMask(layerId)
                                            }
                                            MenuItem {
                                                text: "Mask from selection"
                                                enabled: editor.selectionActive && (nodeKind === "raster" || nodeKind === "group")
                                                Accessible.name: "Copy selection into raster mask for " + layerName
                                                onTriggered: window.maskNodeFromSelection(layerId)
                                            }
                                            MenuItem {
                                                text: maskEnabled ? "Disable raster mask" : "Enable raster mask"
                                                enabled: hasMask
                                                Accessible.name: text + " for " + layerName
                                                onTriggered: editor.setRasterMaskEnabled(layerId, !maskEnabled)
                                            }
                                            MenuItem {
                                                text: "Remove raster mask"
                                                enabled: hasMask
                                                Accessible.name: "Remove raster mask from " + layerName
                                                onTriggered: editor.removeRasterMask(layerId)
                                            }
                                        }
                                        TapHandler {
                                            acceptedButtons: Qt.LeftButton
                                            onTapped: editor.setActiveLayer(layerId)
                                        }
                                        TapHandler {
                                            acceptedButtons: Qt.RightButton
                                            onTapped: layerActions.open()
                                        }
                                        ColumnLayout {
                                            anchors.fill: parent
                                            anchors.margins: 8
                                            RowLayout {
                                                Layout.fillWidth: true
                                                Layout.leftMargin: nodeDepth * 14
                                                Label {
                                                    visible: nodeKind === "group"
                                                    text: hasChildren ? "▾" : "▹"
                                                    color: window.tokens.inkSecondary
                                                    Accessible.name: hasChildren ? "Expanded group" : "Empty group"
                                                }
                                                CheckBox {
                                                    checked: layerVisible
                                                    Accessible.name: "Toggle visibility for " + layerName
                                                    onToggled: editor.setLayerVisibility(layerId, checked)
                                                }
                                                TextField {
                                                    Layout.fillWidth: true
                                                    text: layerName
                                                    selectByMouse: true
                                                    Accessible.name: nodeKind + " node name"
                                                    onEditingFinished: if (text !== layerName)
                                                        editor.renameLayer(layerId, text)
                                                }
                                                Label {
                                                    visible: hasMask
                                                    text: maskEnabled ? "MASK" : "MASK OFF"
                                                    color: maskEnabled ? window.tokens.statusInfo : window.tokens.inkMuted
                                                    font.pixelSize: 9
                                                    Accessible.name: maskEnabled ? "Raster mask enabled" : "Raster mask disabled"
                                                }
                                                Label {
                                                    text: nodeKind === "group" ? "group"
                                                          : nodeKind === "text" ? (semanticPreviewTruncated ? "text · 256+ chars" : "text · " + semanticPreview.length + " chars")
                                                          : nodeKind === "vector" ? "vector · " + semanticPathCount + " paths / " + semanticCommandCount + " commands"
                                                          : blendMode
                                                    color: window.tokens.inkSecondary
                                                    font.pixelSize: 10
                                                }
                                            }
                                            TokenSlider {
                                                Layout.fillWidth: true
                                                Layout.leftMargin: nodeDepth * 14
                                                from: 0
                                                to: 1
                                                value: layerOpacity
                                                Accessible.name: "Opacity for " + layerName
                                                onPressedChanged: if (!pressed && Math.abs(value - layerOpacity) > 0.001)
                                                    editor.setLayerOpacity(layerId, value)
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        ScrollView {
                            id: optionsScroll
                            clip: true
                            ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
                            ColumnLayout {
                                width: Math.max(240, optionsScroll.availableWidth - 14)
                                x: 7
                                spacing: 6

                                Label {
                                    Layout.fillWidth: true
                                    topPadding: 8
                                    visible: window.activeTool === "transform" || window.activeTool === "inspect"
                                    text: window.activeTool === "transform" ? "Drag on the canvas to move the active layer."
                                                                              : "View only: clicks on the canvas do not edit."
                                    color: window.tokens.inkSecondary
                                    wrapMode: Text.Wrap
                                }
                                OptionSection {
                                    title: "FILL"
                                    shown: window.activeTool === "fill"
                                RowLayout {
                                    Layout.fillWidth: true
                                    visible: window.activeTool === "fill"
                                    Label {
                                        text: "Tolerance"
                                        Layout.preferredWidth: 72
                                    }
                                    TokenSlider {
                                        Layout.fillWidth: true
                                        from: 0
                                        to: 255
                                        stepSize: 1
                                        value: window.fillTolerance
                                        Accessible.name: "Fill tolerance 0 to 255"
                                        onMoved: window.fillTolerance = Math.round(value)
                                    }
                                    Label {
                                        text: window.fillTolerance
                                        Layout.preferredWidth: 36
                                    }
                                }
                                }
                                OptionSection {
                                    title: "BRUSH"
                                    shown: window.activeTool === "brush"
                                SubsectionTitle { text: "Tip" }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Shape"
                                        Layout.preferredWidth: 72
                                    }
                                    ComboBox {
                                        objectName: "brushTipControl"
                                        Layout.fillWidth: true
                                        model: ["Round"].concat(editor.brushTipNames)
                                        currentIndex: editor.brushTipIndex + 1
                                        Accessible.name: "Brush tip"
                                        onActivated: editor.brushTipIndex = currentIndex - 1
                                    }
                                    CommandButton {
                                        objectName: "loadBrushTipsAction"
                                        text: "Load"
                                        iconName: "folderOpen"
                                        ToolTip.text: "Load brush tips from a GIMP .gbr or Photoshop .abr file"
                                        onClicked: brushTipDialog.open()
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Edge"
                                        Layout.preferredWidth: 72
                                    }
                                    // Pencil = GIMP's hard, aliased edge (vs the paintbrush's soft one).
                                    CheckBox {
                                        objectName: "brushPencilControl"
                                        text: "Pencil (hard edge)"
                                        leftPadding: 0
                                        Layout.fillWidth: true
                                        checked: editor.brushPencil
                                        onToggled: editor.brushPencil = checked
                                        Accessible.name: "Pencil hard edge"
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Size"
                                        Layout.preferredWidth: 72
                                    }
                                    TokenSlider {
                                        Layout.fillWidth: true
                                        from: 0
                                        to: 1
                                        value: window.brushSizeToSlider(editor.brushSize)
                                        Accessible.name: "Brush size 1 to 1000"
                                        onMoved: editor.brushSize = window.sliderToBrushSize(value)
                                    }
                                    Label {
                                        text: Math.round(editor.brushSize)
                                        Layout.preferredWidth: 44
                                        horizontalAlignment: Text.AlignRight
                                        font.features: { "tnum": 1 }
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    // An image tip replaces the generated dab, so its shape controls do nothing.
                                    enabled: editor.brushTipIndex < 0
                                    Label {
                                        text: "Hardness"
                                        Layout.preferredWidth: 72
                                    }
                                    TokenSlider {
                                        objectName: "brushHardnessControl"
                                        Layout.fillWidth: true
                                        from: 0
                                        to: 1
                                        value: editor.brushHardness
                                        Accessible.name: "Brush hardness"
                                        onMoved: editor.brushHardness = value
                                    }
                                    Label {
                                        text: Math.round(editor.brushHardness * 100) + "%"
                                        Layout.preferredWidth: 44
                                        horizontalAlignment: Text.AlignRight
                                        font.features: { "tnum": 1 }
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    enabled: editor.brushTipIndex < 0
                                    Label {
                                        text: "Roundness"
                                        Layout.preferredWidth: 72
                                    }
                                    TokenSlider {
                                        objectName: "brushAspectControl"
                                        Layout.fillWidth: true
                                        from: 0.05
                                        to: 1
                                        value: editor.brushAspect
                                        Accessible.name: "Brush roundness, height as a fraction of width"
                                        onMoved: editor.brushAspect = value
                                    }
                                    Label {
                                        text: Math.round(editor.brushAspect * 100) + "%"
                                        Layout.preferredWidth: 44
                                        horizontalAlignment: Text.AlignRight
                                        font.features: { "tnum": 1 }
                                    }
                                }
                                SubsectionTitle { text: "Paint" }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Mode"
                                        Layout.preferredWidth: 72
                                    }
                                    // Krita's eraser is a mode of the current brush, toggled with E,
                                    // not a separate tool: same tip, size and smoothing, removing paint.
                                    CheckBox {
                                        objectName: "brushEraseControl"
                                        text: "Erase"
                                        ToolTip.visible: hovered
                                        ToolTip.delay: 450
                                        ToolTip.text: "Eraser mode   E"
                                        leftPadding: 0
                                        Layout.fillWidth: true
                                        checked: editor.brushErase
                                        onToggled: editor.brushErase = checked
                                        Accessible.name: "Eraser mode"
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Opacity"
                                        Layout.preferredWidth: 72
                                    }
                                    TokenSlider {
                                        Layout.fillWidth: true
                                        from: 0
                                        to: 1
                                        value: editor.brushOpacity
                                        Accessible.name: "Brush opacity"
                                        onMoved: editor.brushOpacity = value
                                    }
                                    Label {
                                        text: Math.round(editor.brushOpacity * 100) + "%"
                                        Layout.preferredWidth: 44
                                        horizontalAlignment: Text.AlignRight
                                        font.features: { "tnum": 1 }
                                    }
                                }
                                SubsectionTitle { text: "Stroke" }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Smoothing"
                                        Layout.preferredWidth: 72
                                    }
                                    ComboBox {
                                        Layout.fillWidth: true
                                        textRole: "text"
                                        valueRole: "value"
                                        model: [
                                            { text: "Off", value: "none" },
                                            { text: "Moving average", value: "moving_average" }
                                        ]
                                        currentIndex: editor.brushSmoothingKind === "moving_average" ? 1 : 0
                                        Accessible.name: "Brush smoothing kind"
                                        onActivated: editor.brushSmoothingKind = currentValue
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    visible: editor.brushSmoothingKind === "moving_average"
                                    Label {
                                        text: "Window"
                                        Layout.preferredWidth: 72
                                    }
                                    SpinBox {
                                        Layout.fillWidth: true
                                        from: 2
                                        to: 64
                                        value: editor.brushSmoothingWindow
                                        enabled: editor.brushSmoothingKind === "moving_average"
                                        Accessible.name: "Moving average smoothing window"
                                        onValueModified: editor.brushSmoothingWindow = value
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    CheckBox {
                                        text: "Mirror X"
                                        leftPadding: 0
                                        Layout.fillWidth: true
                                        checked: editor.mirrorXEnabled
                                        onToggled: editor.mirrorXEnabled = checked
                                        Accessible.name: "Mirror brush across X axis"
                                    }
                                    SpinBox {
                                        Layout.preferredWidth: 120
                                        from: 0
                                        to: Math.max(1, editor.documentWidth)
                                        value: Math.round(editor.mirrorXAxis)
                                        onValueModified: editor.mirrorXAxis = value
                                        Accessible.name: "Mirror X axis position"
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    CheckBox {
                                        text: "Mirror Y"
                                        leftPadding: 0
                                        Layout.fillWidth: true
                                        checked: editor.mirrorYEnabled
                                        onToggled: editor.mirrorYEnabled = checked
                                        Accessible.name: "Mirror brush across Y axis"
                                    }
                                    SpinBox {
                                        Layout.preferredWidth: 120
                                        from: 0
                                        to: Math.max(1, editor.documentHeight)
                                        value: Math.round(editor.mirrorYAxis)
                                        onValueModified: editor.mirrorYAxis = value
                                        Accessible.name: "Mirror Y axis position"
                                    }
                                }
                                }
                                OptionSection {
                                    title: "SELECTION"
                                    shown: window.activeTool === "rectangle" || window.activeTool === "ellipse"
                                ComboBox {
                                    Layout.fillWidth: true
                                    model: ["replace", "add", "subtract", "intersect"]
                                    currentIndex: model.indexOf(window.selectionMode)
                                    Accessible.name: "Selection combination mode"
                                    onActivated: window.selectionMode = currentText
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Button {
                                        text: "All"
                                        Layout.fillWidth: true
                                        onClicked: editor.selectAll()
                                        Accessible.name: "Select all"
                                    }
                                    Button {
                                        text: "Invert"
                                        Layout.fillWidth: true
                                        onClicked: editor.invertSelection()
                                        Accessible.name: "Invert selection"
                                    }
                                    Button {
                                        text: "Clear"
                                        Layout.fillWidth: true
                                        onClicked: editor.clearSelection()
                                        Accessible.name: "Clear selection"
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    SpinBox {
                                        id: morphologyRadius
                                        from: 0
                                        to: 4096
                                        value: 4
                                        Layout.fillWidth: true
                                        Accessible.name: "Selection morphology radius"
                                    }
                                    Button {
                                        text: "Feather"
                                        onClicked: editor.featherSelection(morphologyRadius.value)
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Button {
                                        text: "Grow"
                                        Layout.fillWidth: true
                                        onClicked: editor.growSelection(morphologyRadius.value)
                                    }
                                    Button {
                                        text: "Shrink"
                                        Layout.fillWidth: true
                                        onClicked: editor.shrinkSelection(morphologyRadius.value)
                                    }
                                }

                                }
                                OptionSection {
                                    title: "SHAPE"
                                    shown: window.activeTool === "shape"
                                ComboBox {
                                    objectName: "shapeKindControl"
                                    Layout.fillWidth: true
                                    model: ["rectangle", "rounded_rectangle", "ellipse", "line", "regular_polygon", "star"]
                                    currentIndex: model.indexOf(window.shapeKind)
                                    Accessible.name: "Shape kind"
                                    onActivated: window.shapeKind = currentText
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    visible: window.shapeKind === "regular_polygon" || window.shapeKind === "star"
                                    Label {
                                        text: "Sides"
                                        color: window.tokens.inkSecondary
                                    }
                                    SpinBox {
                                        objectName: "shapeSidesControl"
                                        Layout.fillWidth: true
                                        from: 3
                                        to: 512
                                        value: window.shapeSides
                                        onValueModified: window.shapeSides = value
                                        Accessible.name: "Shape side count"
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    visible: window.shapeKind === "star"
                                    Label {
                                        text: "Inner"
                                        color: window.tokens.inkSecondary
                                    }
                                    TokenSlider {
                                        Layout.fillWidth: true
                                        from: 0.05
                                        to: 0.95
                                        value: window.shapeInnerRatio
                                        onMoved: window.shapeInnerRatio = value
                                        Accessible.name: "Star inner radius ratio"
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    visible: window.shapeKind === "rounded_rectangle"
                                    Label {
                                        text: "Corner"
                                        color: window.tokens.inkSecondary
                                    }
                                    SpinBox {
                                        Layout.fillWidth: true
                                        from: 0
                                        to: 512
                                        value: Math.round(window.shapeCornerRadius)
                                        onValueModified: window.shapeCornerRadius = value
                                        Accessible.name: "Rounded rectangle corner radius"
                                    }
                                }
                                Label {
                                    Layout.fillWidth: true
                                    text: "Drag on the canvas. Shapes use the vector fill and stroke colours."
                                    wrapMode: Text.Wrap
                                    color: window.tokens.inkSecondary
                                    font.pixelSize: 11
                                }

                                }
                                OptionSection {
                                    title: "GRADIENT"
                                    shown: window.activeTool === "gradient"
                                ComboBox {
                                    Layout.fillWidth: true
                                    model: ["linear", "radial"]
                                    currentIndex: window.gradientKind === "radial" ? 1 : 0
                                    Accessible.name: "Gradient kind"
                                    onActivated: window.gradientKind = currentText
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Button {
                                        text: "Start"
                                        Layout.fillWidth: true
                                        onClicked: gradientStartDialog.open()
                                        Accessible.name: "Choose gradient start color"
                                        background: Rectangle {
                                            radius: 5
                                            color: window.gradientStartColor
                                            border.color: window.tokens.borderStrong
                                        }
                                    }
                                    Button {
                                        text: "End"
                                        Layout.fillWidth: true
                                        onClicked: gradientEndDialog.open()
                                        Accessible.name: "Choose gradient end color"
                                        background: Rectangle {
                                            radius: 5
                                            color: window.gradientEndColor
                                            border.color: window.tokens.borderStrong
                                        }
                                    }
                                }

                                }
                                OptionSection {
                                    title: "IMAGE"
                                    collapsible: true
                                    expanded: false
                                    autoExpand: window.activeTool === "crop" || window.activeTool === "transform"
                                ComboBox {
                                    id: samplingCombo
                                    Layout.fillWidth: true
                                    model: ["nearest", "bilinear"]
                                    currentIndex: window.samplingMode === "bilinear" ? 1 : 0
                                    Accessible.name: "Transform sampling mode"
                                    onActivated: window.samplingMode = currentText
                                }
                                GridLayout {
                                    columns: 4
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Crop"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: cropX
                                        text: "0"
                                        placeholderText: "Crop X"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: cropY
                                        text: "0"
                                        placeholderText: "Crop Y"
                                    }
                                    Button {
                                        text: "Apply"
                                        onClicked: editor.cropCanvas(Number(cropX.text), Number(cropY.text), Number(cropW.text), Number(cropH.text))
                                        Accessible.name: "Apply numeric crop"
                                    }
                                    Label {
                                        text: "W × H"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: cropW
                                        text: String(editor.documentWidth)
                                        placeholderText: "Crop width"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: cropH
                                        text: String(editor.documentHeight)
                                        placeholderText: "Crop height"
                                    }
                                    Item {
                                        Layout.preferredWidth: 1
                                        Layout.preferredHeight: 1
                                    }
                                    Label {
                                        text: "Pad L/T/R/B"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: padLeft
                                        text: "0"
                                        placeholderText: "Pad left"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: padTop
                                        text: "0"
                                        placeholderText: "Pad top"
                                    }
                                    Button {
                                        text: "Apply"
                                        onClicked: editor.padCanvas(Number(padLeft.text), Number(padTop.text), Number(padRight.text), Number(padBottom.text))
                                        Accessible.name: "Pad canvas"
                                    }
                                    Item {
                                        Layout.preferredWidth: 1
                                        Layout.preferredHeight: 1
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: padRight
                                        text: "0"
                                        placeholderText: "Pad right"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: padBottom
                                        text: "0"
                                        placeholderText: "Pad bottom"
                                    }
                                    Item {
                                        Layout.preferredWidth: 1
                                        Layout.preferredHeight: 1
                                    }
                                    Label {
                                        text: "Resize"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: resizeW
                                        text: String(editor.documentWidth)
                                        placeholderText: "Resize width"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: resizeH
                                        text: String(editor.documentHeight)
                                        placeholderText: "Resize height"
                                    }
                                    Button {
                                        text: "Apply"
                                        onClicked: editor.resizeCanvas(Number(resizeW.text), Number(resizeH.text), window.samplingMode)
                                        Accessible.name: "Resize canvas"
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Button {
                                        objectName: "flipHorizontalAction"
                                        text: "Flip H"
                                        enabled: editor.activeNodeCanEditRaster
                                        Layout.fillWidth: true
                                        onClicked: editor.flipActive(true, false)
                                    }
                                    Button {
                                        objectName: "flipVerticalAction"
                                        text: "Flip V"
                                        enabled: editor.activeNodeCanEditRaster
                                        Layout.fillWidth: true
                                        onClicked: editor.flipActive(false, true)
                                    }
                                    Button {
                                        objectName: "rotateCounterclockwiseAction"
                                        text: "↶ 90"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.rotateActive90(false)
                                        Accessible.name: "Rotate counterclockwise 90 degrees"
                                    }
                                    Button {
                                        objectName: "rotateClockwiseAction"
                                        text: "↷ 90"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.rotateActive90(true)
                                        Accessible.name: "Rotate clockwise 90 degrees"
                                    }
                                }
                                GridLayout {
                                    columns: 4
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Affine"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: m11
                                        text: "1"
                                        placeholderText: "m11"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: m12
                                        text: "0"
                                        placeholderText: "m12"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: tx
                                        text: "0"
                                        placeholderText: "translate X"
                                    }
                                    Item {
                                        Layout.preferredWidth: 1
                                        Layout.preferredHeight: 1
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: m21
                                        text: "0"
                                        placeholderText: "m21"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: m22
                                        text: "1"
                                        placeholderText: "m22"
                                    }
                                    NumericField {
                                        Layout.fillWidth: true
                                        Layout.minimumWidth: 44
                                        Layout.preferredWidth: 64
                                        id: ty
                                        text: "0"
                                        placeholderText: "translate Y"
                                    }
                                }
                                Button {
                                    objectName: "affineTransformAction"
                                    Layout.fillWidth: true
                                    enabled: editor.activeNodeCanEditRaster
                                    text: "Apply affine transform"
                                    Accessible.name: "Apply affine transform to active layer"
                                    onClicked: editor.transformActive(Number(m11.text), Number(m12.text), Number(m21.text), Number(m22.text), Number(tx.text), Number(ty.text), window.samplingMode)
                                }

                                }
                                OptionSection {
                                    title: "ADJUSTMENTS"
                                    collapsible: true
                                    expanded: false
                                // One adjustment at a time: pick it, set its values, Apply. Listing all nine
                                // with their own Apply rows made the panel a wall of fields.
                                // Every Apply keeps its objectName: main.cpp's smoke test finds each
                                // one and checks it follows activeNodeCanEditRaster.
                                RowLayout {
                                    Layout.fillWidth: true
                                    Button {
                                        objectName: "invertFilterAction"
                                        text: "Invert"
                                        enabled: editor.activeNodeCanEditRaster
                                        Layout.fillWidth: true
                                        onClicked: editor.applyFilter("invert")
                                    }
                                    Button {
                                        objectName: "grayscaleFilterAction"
                                        text: "Grayscale"
                                        enabled: editor.activeNodeCanEditRaster
                                        Layout.fillWidth: true
                                        onClicked: editor.applyFilter("grayscale")
                                    }
                                }
                                ComboBox {
                                    id: adjustmentPicker
                                    Layout.fillWidth: true
                                    Layout.topMargin: 4
                                    Accessible.name: "Adjustment"
                                    model: ["Brightness / contrast", "Levels", "Curves", "Hue / saturation",
                                        "Gaussian blur", "Box blur", "Sharpen", "Threshold", "Posterize"]
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6
                                    visible: adjustmentPicker.currentIndex === 0
                                    RowLayout {
                                        ParamLabel { text: "Brightness" }
                                        SpinBox {
                                            id: brightness
                                            from: -255
                                            to: 255
                                            value: 0
                                            Layout.fillWidth: true
                                            Accessible.name: "Brightness minus 255 to 255"
                                        }
                                    }
                                    RowLayout {
                                        ParamLabel { text: "Contrast" }
                                        SpinBox {
                                            id: contrast
                                            from: -100
                                            to: 100
                                            value: 0
                                            Layout.fillWidth: true
                                            Accessible.name: "Contrast minus 100 to 100"
                                        }
                                    }
                                    Button {
                                        objectName: "brightnessContrastAction"
                                        Layout.fillWidth: true
                                        text: "Apply"
                                        Accessible.name: "Apply brightness / contrast"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyBrightnessContrast(brightness.value, contrast.value)
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6
                                    visible: adjustmentPicker.currentIndex === 1
                                    RowLayout {
                                        ParamLabel { text: "Input black" }
                                        SpinBox {
                                            id: inputBlack
                                            Layout.fillWidth: true
                                            from: 0
                                            to: 255
                                            value: 0
                                            Accessible.name: "Levels input black"
                                        }
                                    }
                                    RowLayout {
                                        ParamLabel { text: "Input white" }
                                        SpinBox {
                                            id: inputWhite
                                            Layout.fillWidth: true
                                            from: 0
                                            to: 255
                                            value: 255
                                            Accessible.name: "Levels input white"
                                        }
                                    }
                                    RowLayout {
                                        ParamLabel { text: "Gamma" }
                                        NumericField {
                                            id: gamma
                                            text: "1"
                                            placeholderText: "Gamma 0.01–100"
                                        }
                                    }
                                    RowLayout {
                                        ParamLabel { text: "Output black" }
                                        SpinBox {
                                            id: outputBlack
                                            Layout.fillWidth: true
                                            from: 0
                                            to: 255
                                            value: 0
                                            Accessible.name: "Levels output black"
                                        }
                                    }
                                    RowLayout {
                                        ParamLabel { text: "Output white" }
                                        SpinBox {
                                            id: outputWhite
                                            Layout.fillWidth: true
                                            from: 0
                                            to: 255
                                            value: 255
                                            Accessible.name: "Levels output white"
                                        }
                                    }
                                    Button {
                                        objectName: "levelsAction"
                                        Layout.fillWidth: true
                                        text: "Apply"
                                        Accessible.name: "Apply levels"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyLevels(inputBlack.value, inputWhite.value, Number(gamma.text), outputBlack.value, outputWhite.value)
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6
                                    visible: adjustmentPicker.currentIndex === 2
                                    RowLayout {
                                        ParamLabel { text: "Shadows" }
                                        SpinBox {
                                            id: curveQuarter
                                            Layout.fillWidth: true
                                            from: 0
                                            to: 255
                                            value: 64
                                            Accessible.name: "Curves output at 25% input"
                                            ToolTip.visible: hovered
                                            ToolTip.text: "Output at 25% input"
                                        }
                                    }
                                    RowLayout {
                                        ParamLabel { text: "Midtones" }
                                        SpinBox {
                                            id: curveMiddle
                                            Layout.fillWidth: true
                                            from: 0
                                            to: 255
                                            value: 128
                                            Accessible.name: "Curves output at 50% input"
                                            ToolTip.visible: hovered
                                            ToolTip.text: "Output at 50% input"
                                        }
                                    }
                                    RowLayout {
                                        ParamLabel { text: "Highlights" }
                                        SpinBox {
                                            id: curveThreeQuarter
                                            Layout.fillWidth: true
                                            from: 0
                                            to: 255
                                            value: 191
                                            Accessible.name: "Curves output at 75% input"
                                            ToolTip.visible: hovered
                                            ToolTip.text: "Output at 75% input"
                                        }
                                    }
                                    Button {
                                        objectName: "curvesAction"
                                        Layout.fillWidth: true
                                        text: "Apply"
                                        Accessible.name: "Apply curves"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyCurves(curveQuarter.value, curveMiddle.value, curveThreeQuarter.value)
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6
                                    visible: adjustmentPicker.currentIndex === 3
                                    RowLayout {
                                        ParamLabel { text: "Hue" }
                                        SpinBox {
                                            id: hue
                                            Layout.fillWidth: true
                                            from: -180
                                            to: 180
                                            value: 0
                                            Accessible.name: "Hue degrees"
                                        }
                                    }
                                    RowLayout {
                                        ParamLabel { text: "Saturation" }
                                        SpinBox {
                                            id: saturation
                                            Layout.fillWidth: true
                                            from: -100
                                            to: 100
                                            value: 0
                                            Accessible.name: "Saturation"
                                        }
                                    }
                                    RowLayout {
                                        ParamLabel { text: "Lightness" }
                                        SpinBox {
                                            id: lightness
                                            Layout.fillWidth: true
                                            from: -100
                                            to: 100
                                            value: 0
                                            Accessible.name: "Lightness"
                                        }
                                    }
                                    Button {
                                        objectName: "hueSaturationAction"
                                        Layout.fillWidth: true
                                        text: "Apply"
                                        Accessible.name: "Apply hue / saturation / lightness"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyHueSaturation(hue.value, saturation.value, lightness.value)
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6
                                    visible: adjustmentPicker.currentIndex === 4
                                    RowLayout {
                                        ParamLabel { text: "Radius (σ)" }
                                        NumericField {
                                            id: sigma
                                            text: "4"
                                            placeholderText: "Gaussian sigma 0–1024"
                                        }
                                    }
                                    Button {
                                        objectName: "gaussianBlurAction"
                                        Layout.fillWidth: true
                                        text: "Apply"
                                        Accessible.name: "Apply Gaussian blur"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyGaussianBlur(Number(sigma.text))
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6
                                    visible: adjustmentPicker.currentIndex === 5
                                    RowLayout {
                                        ParamLabel { text: "Radius" }
                                        SpinBox {
                                            id: boxRadius
                                            from: 1
                                            to: 4096
                                            value: 3
                                            Layout.fillWidth: true
                                            Accessible.name: "Box blur radius 1 to 4096"
                                        }
                                    }
                                    Button {
                                        objectName: "boxBlurAction"
                                        Layout.fillWidth: true
                                        text: "Apply"
                                        Accessible.name: "Apply box blur"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyBoxBlur(boxRadius.value)
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6
                                    visible: adjustmentPicker.currentIndex === 6
                                    RowLayout {
                                        ParamLabel { text: "Amount" }
                                        NumericField {
                                            id: sharpenAmount
                                            text: "1"
                                            placeholderText: "Sharpen amount 0–10"
                                        }
                                    }
                                    Button {
                                        objectName: "sharpenAction"
                                        Layout.fillWidth: true
                                        text: "Apply"
                                        Accessible.name: "Apply sharpen"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applySharpen(Number(sharpenAmount.text))
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6
                                    visible: adjustmentPicker.currentIndex === 7
                                    RowLayout {
                                        ParamLabel { text: "Level" }
                                        SpinBox {
                                            id: threshold
                                            from: 0
                                            to: 255
                                            value: 128
                                            Layout.fillWidth: true
                                            Accessible.name: "Threshold 0 to 255"
                                        }
                                    }
                                    Button {
                                        objectName: "thresholdAction"
                                        Layout.fillWidth: true
                                        text: "Apply"
                                        Accessible.name: "Apply threshold"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyThreshold(threshold.value)
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6
                                    visible: adjustmentPicker.currentIndex === 8
                                    RowLayout {
                                        ParamLabel { text: "Levels" }
                                        SpinBox {
                                            id: posterize
                                            from: 2
                                            to: 256
                                            value: 8
                                            Layout.fillWidth: true
                                            Accessible.name: "Posterize levels 2 to 256"
                                        }
                                    }
                                    Button {
                                        objectName: "posterizeAction"
                                        Layout.fillWidth: true
                                        text: "Apply"
                                        Accessible.name: "Apply posterize"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyPosterize(posterize.value)
                                    }
                                }
                                }
                                OptionSection {
                                    title: "ACTIVE LAYER"
                                    collapsible: true
                                    expanded: false
                                ComboBox {
                                    id: blendMode
                                    Layout.fillWidth: true
                                    model: ["normal", "multiply", "screen", "overlay", "add", "darken_only", "lighten_only", "luma_darken_only", "luma_lighten_only", "dodge", "burn", "linear_burn", "linear_light", "vivid_light", "pin_light", "hard_mix", "hard_light", "soft_light", "grain_extract", "grain_merge", "difference", "exclusion", "subtract", "divide", "hsv_hue", "hsv_saturation", "hsv_value", "hsl_color", "lch_hue", "lch_chroma", "lch_color", "lch_lightness", "luminance", "dissolve", "behind", "erase", "anti_erase", "color_erase", "replace", "overwrite", "pass_through"]
                                    Accessible.name: "Active layer blend mode"
                                }
                                Button {
                                    Layout.fillWidth: true
                                    text: "Set blend mode"
                                    onClicked: editor.setLayerBlendMode(editor.activeLayerId, blendMode.currentText)
                                }
                                Button {
                                    // Moved here from Adjustments: it empties the layer, it does not adjust it.
                                    objectName: "clearLayerAction"
                                    Layout.fillWidth: true
                                    text: "Clear layer"
                                    enabled: editor.activeNodeCanEditRaster
                                    onClicked: editor.clearActiveLayer()
                                }
                                }
                                Item {
                                    Layout.preferredHeight: 12
                                }
                            }
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
                    text: editor.statusMessage
                    color: window.tokens.inkSecondary
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
                    text: (window.activeTool === "brush" && editor.brushErase ? "eraser" : window.activeTool) + " · " + Math.round(editor.brushSize) + " px"
                    color: window.tokens.inkSecondary
                    font.pixelSize: 11
                }
            }
        }
    }
}
