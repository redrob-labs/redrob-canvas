// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The Layers tab: the node stack, its add/edit menus and the rasterise warning. Moved out of
// Main.qml in P12.
Item {
    id: root
    objectName: "layerPanel"
    // The window (tokens, maskNodeFromSelection) and the two node dialogs, which stay at window
    // level because the canvas tools and the menu bar open them too (P12).
    required property var app
    required property var textDialog
    required property var vectorDialog
    // M4: the filter window, opened on an adjustment layer to edit it.
    required property var filterWindow
    // L6: Blend If.
    required property var blendIfWindow
    // U5: Smart filters, opened on a smart object.
    required property var smartFiltersWindow

    // The active row publishes its values here (Binding below), so the controls at the top of the
    // panel -- blend mode, opacity, locks -- act on the active layer, as in Photoshop.
    property real activeOpacity: 1
    property string activeBlendMode: "normal"
    property bool activeLockTransparent: false
    property bool activeLockPixels: false
    property bool activeLockPosition: false
    readonly property bool hasActive: editor.activeLayerId.length > 0
    readonly property var blendModes: ["normal", "dissolve", "darken_only", "multiply", "burn", "linear_burn",
        "lighten_only", "screen", "dodge", "add", "overlay", "soft_light", "hard_light", "vivid_light",
        "linear_light", "pin_light", "hard_mix", "difference", "exclusion", "subtract", "divide", "hsv_hue",
        "hsv_saturation", "hsl_color", "luminance", "pass_through"]
    // Kind filter: All, Pixel, Adjustment, Type, Shape, Group, Smart object.
    property int kindFilter: 0
    function kindShown(kind, smart) {
        switch (kindFilter) {
        case 1: return kind === "raster" && !smart;
        case 2: return kind === "adjustment";
        case 3: return kind === "text";
        case 4: return kind === "vector";
        case 5: return kind === "group";
        case 6: return smart;
        default: return true;
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
                onTriggered: root.textDialog.openNew(24, 24)
            }
            MenuItem {
                objectName: "addVectorNodeAction"
                text: "Add vector rectangle"
                Accessible.name: "Add deterministic vector rectangle"
                onTriggered: root.vectorDialog.openNew(editor.brushColor.toString())
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            ComboBox {
                objectName: "layerKindFilter"
                Layout.preferredWidth: 96
                model: ["Kind: All", "Pixel", "Adjustment", "Type", "Shape", "Group", "Smart object"]
                currentIndex: root.kindFilter
                Accessible.name: "Show layers of kind"
                onActivated: index => root.kindFilter = index
            }
            // Photoshop's kind buttons: one click shows only that kind, a second click shows all.
            Repeater {
                model: [[1, "image", "Pixel layers"], [2, "adjust", "Adjustment layers"], [3, "text", "Type layers"],
                        [4, "shape", "Shape layers"], [6, "duplicate", "Smart objects"]]
                delegate: CommandButton {
                    required property var modelData
                    objectName: "layerKindButton-" + modelData[0]
                    text: modelData[2]
                    iconName: modelData[1]
                    iconOnly: true
                    implicitHeight: 26
                    leftPadding: 4
                    rightPadding: 4
                    icon.width: 14
                    icon.height: 14
                    highlighted: root.kindFilter === modelData[0]
                    ToolTip.text: "Show " + modelData[2].toLowerCase() + " only"
                    onClicked: root.kindFilter = root.kindFilter === modelData[0] ? 0 : modelData[0]
                }
            }
            Item {
                Layout.fillWidth: true
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            ComboBox {
                objectName: "layerBlendMode"
                Layout.fillWidth: true
                enabled: root.hasActive
                model: root.blendModes.map(mode => mode.split("_").map(w => w.charAt(0).toUpperCase() + w.slice(1)).join(" "))
                currentIndex: Math.max(0, root.blendModes.indexOf(root.activeBlendMode))
                Accessible.name: "Active layer blend mode"
                onActivated: index => editor.setLayerBlendMode(editor.activeLayerId, root.blendModes[index])
            }
            Label {
                text: "Opacity:"
                color: root.app.tokens.inkSecondary
                font.pixelSize: 11
            }
            SpinBox {
                objectName: "layerOpacityField"
                Layout.preferredWidth: 86
                enabled: root.hasActive
                from: 0
                to: 100
                editable: true
                value: Math.round(root.activeOpacity * 100)
                textFromValue: value => value + "%"
                valueFromText: text => parseInt(text)
                Accessible.name: "Active layer opacity"
                onValueModified: editor.setLayerOpacity(editor.activeLayerId, value / 100)
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 4
            Label {
                text: "Lock:"
                color: root.app.tokens.inkSecondary
                font.pixelSize: 11
            }
            Repeater {
                // [name, tooltip, which lock]
                model: [["▧", "Lock transparent pixels", "transparent", ""], ["✎\uFE0E", "Lock image pixels", "pixels", "brush"],
                        ["✥\uFE0E", "Lock position", "position", "transform"], ["All", "Lock all", "all", "lock"]]
                delegate: CommandButton {
                    required property var modelData
                    objectName: "layerLock-" + modelData[2]
                    text: modelData[0]
                    iconName: modelData[3]
                    iconOnly: modelData[3].length > 0
                    highlighted: checked
                    enabled: root.hasActive
                    checkable: true
                    checked: modelData[2] === "transparent" ? root.activeLockTransparent
                           : modelData[2] === "pixels" ? root.activeLockPixels
                           : modelData[2] === "position" ? root.activeLockPosition
                           : root.activeLockTransparent && root.activeLockPixels && root.activeLockPosition
                    ToolTip.text: modelData[1]
                    Accessible.name: modelData[1]
                    onClicked: {
                        const all = !(root.activeLockTransparent && root.activeLockPixels && root.activeLockPosition);
                        editor.setLayerLocks(editor.activeLayerId,
                                             modelData[2] === "all" ? all : modelData[2] === "transparent" ? !root.activeLockTransparent : root.activeLockTransparent,
                                             modelData[2] === "all" ? all : modelData[2] === "pixels" ? !root.activeLockPixels : root.activeLockPixels,
                                             modelData[2] === "all" ? all : modelData[2] === "position" ? !root.activeLockPosition : root.activeLockPosition);
                    }
                }
            }
            Item {
                Layout.fillWidth: true
            }
        }
        ListView {
            id: layerList
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 1
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
                required property bool isClipped
                required property bool lockTransparent
                required property bool lockPixels
                required property bool lockPosition
                required property var adjustmentFilter
                required property int linkGroup
                required property bool isSmartObject
                required property var smartFilters
                required property var blendIf
                // U9: an artboard group shows its own mark.
                required property var artboard
                readonly property bool isArtboard: artboard !== undefined && artboard !== null
                                                   && artboard.width !== undefined
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
                required property real semanticBoxWidth
                required property string semanticAlign
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
                // Rows the Kind filter hides take no room.
                readonly property bool shown: root.kindShown(nodeKind, isSmartObject)
                visible: shown
                height: shown ? 46 : 0
                radius: 4
                Binding { target: root; property: "activeOpacity"; value: layerOpacity; when: activeLayer }
                Binding { target: root; property: "activeBlendMode"; value: blendMode; when: activeLayer }
                Binding { target: root; property: "activeLockTransparent"; value: lockTransparent; when: activeLayer }
                Binding { target: root; property: "activeLockPixels"; value: lockPixels; when: activeLayer }
                Binding { target: root; property: "activeLockPosition"; value: lockPosition; when: activeLayer }
                // H8: selected-but-not-active rows get the focus ring too, on the quieter fill.
                readonly property bool selectedLayer: editor.selectedLayerIds.indexOf(layerId) >= 0
                color: activeLayer ? root.app.tokens.borderSubtle : root.app.tokens.surfaceSunken
                border.color: activeLayer || selectedLayer ? root.app.tokens.focusRing : root.app.tokens.borderSubtle
                border.width: selectedLayer && !activeLayer ? 2 : 1
                Menu {
                    id: layerActions
                    MenuItem {
                        objectName: "renameLayerAction-" + layerId
                        text: "Rename layer"
                        onTriggered: nameField.startRename()
                    }
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
                        onTriggered: root.textDialog.openEdit(
                            layerId, semanticTextSource, semanticOriginX, semanticOriginY,
                            semanticFontSize, semanticColor.toString(), semanticFontId,
                            semanticFontFamily, semanticBoxWidth, semanticAlign)
                    }
                    MenuItem {
                        objectName: "editSemanticVectorAction-" + layerId
                        text: semanticRectangleRecognized ? "Edit vector rectangle" : "Edit unavailable (not a rectangle)"
                        enabled: canEditVector && semanticRectangleRecognized
                        Accessible.name: semanticRectangleRecognized
                            ? "Edit recognized rectangle " + layerName
                            : "Arbitrary vectors cannot be edited as rectangles"
                        onTriggered: root.vectorDialog.openEdit(
                            layerId, semanticRectangleX, semanticRectangleY,
                            semanticRectangleWidth, semanticRectangleHeight,
                            semanticRectangleFill.toString(),
                            semanticRectangleStroke.toString(),
                            semanticRectangleStrokeWidth)
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
                    MenuItem {
                        objectName: "blendIfAction-" + layerId
                        text: "Blending options (blend if)…"
                        enabled: nodeKind === "raster"
                        onTriggered: root.blendIfWindow.openFor(layerId, blendIf)
                    }
                    MenuItem {
                        objectName: "smartFiltersAction-" + layerId
                        text: "Smart filters…"
                        visible: isSmartObject
                        height: visible ? implicitHeight : 0
                        onTriggered: root.smartFiltersWindow.openFor(layerId, smartFilters)
                    }
                    MenuItem {
                        objectName: "editAdjustmentAction-" + layerId
                        text: "Edit adjustment…"
                        visible: nodeKind === "adjustment"
                        height: visible ? implicitHeight : 0
                        onTriggered: root.filterWindow.openForAdjustment(layerId, adjustmentFilter)
                    }
                    MenuSeparator {}
                    // M2: Photoshop's four lock buttons, as menu toggles.
                    MenuItem {
                        objectName: "lockTransparentAction-" + layerId
                        text: "Lock transparent pixels"
                        checkable: true
                        checked: lockTransparent
                        onTriggered: editor.setLayerLocks(layerId, !lockTransparent, lockPixels, lockPosition)
                    }
                    MenuItem {
                        objectName: "lockPixelsAction-" + layerId
                        text: "Lock image pixels"
                        checkable: true
                        checked: lockPixels
                        onTriggered: editor.setLayerLocks(layerId, lockTransparent, !lockPixels, lockPosition)
                    }
                    MenuItem {
                        objectName: "lockPositionAction-" + layerId
                        text: "Lock position"
                        checkable: true
                        checked: lockPosition
                        onTriggered: editor.setLayerLocks(layerId, lockTransparent, lockPixels, !lockPosition)
                    }
                    MenuItem {
                        objectName: "lockAllAction-" + layerId
                        readonly property bool all: lockTransparent && lockPixels && lockPosition
                        text: all ? "Unlock all" : "Lock all"
                        onTriggered: editor.setLayerLocks(layerId, !all, !all, !all)
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
                        onTriggered: root.app.maskNodeFromSelection(layerId)
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
                    // H8: Ctrl-click adds or removes a layer, Shift-click selects a range.
                    onTapped: (eventPoint, button) => {
                        const mods = point.modifiers;
                        editor.selectLayer(layerId, (mods & Qt.ControlModifier) ? 1
                                                    : (mods & Qt.ShiftModifier) ? 2 : 0);
                    }
                }
                TapHandler {
                    acceptedButtons: Qt.RightButton
                    onTapped: layerActions.open()
                }
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 4
                    anchors.rightMargin: 8
                    spacing: 6
                    // Visibility: the eye, as in Photoshop and Illustrator.
                    CommandButton {
                        objectName: "layerVisibility-" + layerId
                        text: layerVisible ? "Hide" : "Show"
                        iconName: "eye"
                        iconOnly: true
                        opacity: layerVisible ? 1 : 0.35
                        Accessible.name: "Toggle visibility for " + layerName
                        onClicked: editor.setLayerVisibility(layerId, !layerVisible)
                    }
                    // Thumbnail: the layer's own content over a transparency checkerboard, as in
                    // Photoshop. An adjustment layer holds no pixels, so it keeps its glyph.
                    Rectangle {
                        objectName: "layerThumb-" + layerId
                        Layout.leftMargin: nodeDepth * 14 + (isClipped ? 12 : 0)
                        Layout.preferredWidth: 30
                        Layout.preferredHeight: 30
                        radius: 3
                        clip: true
                        color: root.app.tokens.surfaceBase
                        border.color: activeLayer ? root.app.tokens.focusRing : root.app.tokens.borderSubtle
                        Grid {
                            visible: nodeKind !== "adjustment"
                            anchors.fill: parent
                            anchors.margins: 1
                            columns: 6
                            Repeater {
                                model: 36
                                Rectangle {
                                    required property int index
                                    width: 28 / 6
                                    height: 28 / 6
                                    color: (Math.floor(index / 6) + index % 6) % 2
                                           ? root.app.tokens.borderSubtle : root.app.tokens.surfaceBase
                                }
                            }
                        }
                        Image {
                            objectName: "layerThumbImage-" + layerId
                            visible: nodeKind !== "adjustment"
                            anchors.fill: parent
                            anchors.margins: 1
                            fillMode: Image.PreserveAspectFit
                            smooth: true
                            cache: false
                            sourceSize: Qt.size(56, 56)
                            source: nodeKind === "adjustment" ? ""
                                    : "image://layerthumb/" + layerId + "?g=" + editor.generation
                        }
                        Label {
                            visible: nodeKind === "adjustment"
                            anchors.centerIn: parent
                            text: "◐"
                            color: root.app.tokens.inkSecondary
                            font.pixelSize: 14
                        }
                    }
                    // Mask thumbnail beside the layer's, as in Photoshop: white shows, black hides.
                    // A disabled mask is crossed out in red; Shift+click toggles it, the menu
                    // carries the rest.
                    Rectangle {
                        objectName: "layerMaskThumb-" + layerId
                        visible: hasMask
                        Layout.preferredWidth: 30
                        Layout.preferredHeight: 30
                        radius: 3
                        clip: true
                        color: root.app.tokens.surfaceBase
                        border.color: root.app.tokens.borderSubtle
                        Accessible.name: maskEnabled ? "Raster mask of " + layerName
                                                     : "Disabled raster mask of " + layerName
                        Image {
                            objectName: "layerMaskThumbImage-" + layerId
                            anchors.fill: parent
                            anchors.margins: 1
                            fillMode: Image.PreserveAspectFit
                            smooth: true
                            cache: false
                            sourceSize: Qt.size(56, 56)
                            source: hasMask ? "image://layerthumb/mask/" + layerId + "?g=" + editor.generation : ""
                        }
                        Canvas {
                            objectName: "layerMaskDisabledMark-" + layerId
                            visible: !maskEnabled
                            anchors.fill: parent
                            property color ink: root.app.tokens.statusDanger
                            onInkChanged: requestPaint()
                            onPaint: {
                                const ctx = getContext("2d");
                                ctx.reset();
                                ctx.strokeStyle = ink;
                                ctx.lineWidth = 2;
                                ctx.beginPath();
                                ctx.moveTo(3, 3); ctx.lineTo(width - 3, height - 3);
                                ctx.moveTo(width - 3, 3); ctx.lineTo(3, height - 3);
                                ctx.stroke();
                            }
                        }
                        TapHandler {
                            acceptedButtons: Qt.LeftButton
                            onTapped: (eventPoint, button) => {
                                if (point.modifiers & Qt.ShiftModifier)
                                    editor.setRasterMaskEnabled(layerId, !maskEnabled);
                                else
                                    editor.selectLayer(layerId, 0);
                            }
                        }
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                    RowLayout {
                        Layout.fillWidth: true
                        // M1: a clipped layer is indented with a down-arrow, as in Photoshop.
                        Label {
                            objectName: "artboardMark-" + layerId
                            visible: isArtboard
                            text: "⬚"
                            color: root.app.tokens.inkSecondary
                            Accessible.name: "Artboard"
                            ToolTip.visible: artboardMarkHover.hovered
                            ToolTip.text: isArtboard ? "Artboard " + artboard.width + " x " + artboard.height : ""
                            HoverHandler { id: artboardMarkHover }
                        }
                        Label {
                            objectName: "smartMark-" + layerId
                            visible: isSmartObject
                            text: "▣"
                            color: root.app.tokens.inkSecondary
                            Accessible.name: "Smart object"
                        }
                        Label {
                            objectName: "linkMark-" + layerId
                            visible: linkGroup > 0
                            text: "🔗"
                            color: root.app.tokens.inkSecondary
                            Accessible.name: "Linked"
                        }
                        Label {
                            objectName: "lockMark-" + layerId
                            visible: lockTransparent || lockPixels || lockPosition
                            text: "🔒"
                            color: root.app.tokens.inkSecondary
                            Accessible.name: "Locked"
                        }
                        Label {
                            objectName: "clippedMark-" + layerId
                            visible: isClipped
                            text: "↓"
                            color: root.app.tokens.inkSecondary
                            Accessible.name: "Clipped to the layer below"
                        }
                        Label {
                            visible: nodeKind === "group"
                            text: hasChildren ? "▾" : "▹"
                            color: root.app.tokens.inkSecondary
                            Accessible.name: hasChildren ? "Expanded group" : "Empty group"
                        }
                        TextField {
                            id: nameField
                            objectName: "layerNameField-" + layerId
                            // Photoshop: a click selects the layer, a double-click renames it. A
                            // field that took focus on every click swallowed Ctrl+A/C/V meant for
                            // the canvas, so it only takes focus while renaming.
                            property bool renaming: false
                            Layout.fillWidth: true
                            text: layerName
                            readOnly: !renaming
                            // Plain text until renaming, as a layer name reads in Photoshop.
                            background.opacity: renaming ? 1 : 0
                            activeFocusOnPress: renaming
                            focusPolicy: renaming ? Qt.StrongFocus : Qt.NoFocus
                            selectByMouse: renaming
                            Accessible.name: nodeKind + " node name"
                            function startRename() {
                                renaming = true;
                                forceActiveFocus();
                                selectAll();
                            }
                            function endRename(keep) {
                                if (!renaming)
                                    return;
                                renaming = false;
                                if (keep && text !== layerName)
                                    editor.renameLayer(layerId, text);
                                text = Qt.binding(() => layerName);
                                focus = false;
                            }
                            TapHandler {
                                acceptedButtons: Qt.LeftButton
                                enabled: !nameField.renaming
                                // This handler takes the tap from the row's own TapHandler, so it
                                // selects the layer itself (same modifiers as the row).
                                onTapped: (eventPoint, button) => {
                                    const mods = point.modifiers;
                                    editor.selectLayer(layerId, (mods & Qt.ControlModifier) ? 1
                                                                : (mods & Qt.ShiftModifier) ? 2 : 0);
                                }
                                onDoubleTapped: nameField.startRename()
                            }
                            onEditingFinished: endRename(true)
                            // Right-click on the name opens the layer menu, as on the rest of the
                            // row (Photoshop). Qt's own cut/copy/paste menu is for renaming only.
                            ContextMenu.menu: renaming ? undefined : null
                            TapHandler {
                                acceptedButtons: Qt.RightButton
                                enabled: !nameField.renaming
                                onTapped: layerActions.open()
                            }
                            onActiveFocusChanged: if (!activeFocus) endRename(true)
                            Keys.onEscapePressed: endRename(false)
                        }
                    }
                    Label {
                        objectName: "layerKind-" + layerId
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                        text: (nodeKind === "group" ? "Group"
                              : nodeKind === "text" ? (semanticPreviewTruncated ? "Type · 256+ chars" : "Type · " + semanticPreview.length + " chars")
                              : nodeKind === "vector" ? "Shape · " + semanticPathCount + " paths"
                              : nodeKind === "adjustment" ? "Adjustment"
                              : isSmartObject ? "Smart object" : "Pixel")
                              + (blendMode !== "normal" ? " · " + blendMode : "")
                              + (layerOpacity < 0.995 ? " · " + Math.round(layerOpacity * 100) + "%" : "")
                        color: root.app.tokens.inkMuted
                        font.pixelSize: 10
                    }
                    }
                }
            }
        }
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: root.app.tokens.borderSubtle
        }
        RowLayout {
            objectName: "layerFooter"
            Layout.fillWidth: true
            spacing: 2
            Item {
                Layout.fillWidth: true
            }
            CommandButton {
                objectName: "layerFooterLink"
                text: "Link"
                iconName: "link"
                iconOnly: true
                enabled: editor.selectedLayerIds.length > 1
                ToolTip.text: "Link the selected layers"
                onClicked: editor.linkSelectedLayers()
            }
            CommandButton {
                objectName: "layerFooterStyle"
                text: "fx"
                enabled: root.hasActive
                ToolTip.text: "Layer style (in Properties)"
                onClicked: root.app.showLayerStyle()
            }
            CommandButton {
                objectName: "layerFooterMask"
                text: "Mask"
                iconName: "mask"
                iconOnly: true
                enabled: root.hasActive && !editor.activeNodeHasMask
                ToolTip.text: "Add a layer mask"
                onClicked: editor.addRasterMask(editor.activeLayerId)
            }
            CommandButton {
                objectName: "layerFooterAdjustment"
                text: "Adjustment"
                iconName: "adjust"
                iconOnly: true
                ToolTip.text: "New adjustment layer"
                onClicked: root.filterWindow.open()
            }
            CommandButton {
                objectName: "layerFooterGroup"
                text: "Group"
                iconName: "folder"
                iconOnly: true
                ToolTip.text: "Group the selected layers (Ctrl+G)"
                onClicked: editor.groupSelectedLayers()
            }
            CommandButton {
                objectName: "layerFooterNew"
                text: "Add node"
                iconName: "plus"
                iconOnly: true
                ToolTip.text: "New layer, group, text or vector node"
                onClicked: addNodeMenu.open()
            }
            CommandButton {
                objectName: "layerFooterDelete"
                text: "Delete layer"
                iconName: "trash"
                iconOnly: true
                enabled: layerList.count > 1
                ToolTip.text: "Delete selected layers"
                onClicked: editor.deleteSelectedLayers()
            }
        }
    }
}
