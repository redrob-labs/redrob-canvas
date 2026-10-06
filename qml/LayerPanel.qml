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
            Label {
                text: "LAYER STACK"
                color: root.app.tokens.inkSecondary
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
                ToolTip.text: "Delete selected layers"
                onClicked: editor.deleteSelectedLayers()
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
                height: 74
                radius: 8
                // H8: selected-but-not-active rows get the focus ring too, on the quieter fill.
                readonly property bool selectedLayer: editor.selectedLayerIds.indexOf(layerId) >= 0
                color: activeLayer ? root.app.tokens.borderSubtle : root.app.tokens.surfaceSunken
                border.color: activeLayer || selectedLayer ? root.app.tokens.focusRing : root.app.tokens.borderSubtle
                border.width: selectedLayer && !activeLayer ? 2 : 1
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
                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 8
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: nodeDepth * 14
                        Label {
                            visible: nodeKind === "group"
                            text: hasChildren ? "▾" : "▹"
                            color: root.app.tokens.inkSecondary
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
                            color: maskEnabled ? root.app.tokens.statusInfo : root.app.tokens.inkMuted
                            font.pixelSize: 9
                            Accessible.name: maskEnabled ? "Raster mask enabled" : "Raster mask disabled"
                        }
                        Label {
                            text: nodeKind === "group" ? "group"
                                  : nodeKind === "text" ? (semanticPreviewTruncated ? "text · 256+ chars" : "text · " + semanticPreview.length + " chars")
                                  : nodeKind === "vector" ? "vector · " + semanticPathCount + " paths / " + semanticCommandCount + " commands"
                                  : blendMode
                            color: root.app.tokens.inkSecondary
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
