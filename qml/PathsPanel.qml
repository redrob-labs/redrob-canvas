// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The Paths tab, laid out as Photoshop's: one row per stored path with its filled-shape thumbnail.
// Ctrl+click a thumbnail to load the path as a selection (Shift adds, Alt subtracts, both
// intersect). The footer strokes the path with the current brush, loads it as a selection, makes a
// path from the selection, and deletes.
Item {
    id: root
    objectName: "pathsPanel"
    // The window, for its design tokens.
    required property var app
    property string selectedPathId: ""

    function selectionMode(mods) {
        const shift = (mods & Qt.ShiftModifier) !== 0;
        const alt = (mods & Qt.AltModifier) !== 0;
        return shift && alt ? "intersect" : shift ? "add" : alt ? "subtract" : "replace";
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 10
        spacing: 6

        ListView {
            id: pathList
            objectName: "pathList"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: editor.paths
            delegate: Rectangle {
                id: pathRow
                required property var modelData
                readonly property string pathId: modelData.id
                readonly property bool selected: root.selectedPathId === pathId
                objectName: "path-" + pathId
                width: ListView.view.width
                height: 46
                radius: 4
                color: selected ? root.app.tokens.borderSubtle : "transparent"
                TapHandler {
                    acceptedButtons: Qt.LeftButton
                    onTapped: root.selectedPathId = pathRow.pathId
                }
                TapHandler {
                    acceptedButtons: Qt.RightButton
                    onTapped: {
                        root.selectedPathId = pathRow.pathId;
                        pathMenu.open();
                    }
                }
                Menu {
                    id: pathMenu
                    MenuItem {
                        text: "Make selection"
                        onTriggered: editor.loadPathSelection(pathRow.pathId, "replace")
                    }
                    MenuItem {
                        text: "Stroke path with brush"
                        onTriggered: editor.strokePath(pathRow.pathId)
                    }
                    MenuItem {
                        text: modelData.visible ? "Hide path outline" : "Show path outline"
                        onTriggered: editor.setPathVisible(pathRow.pathId, !modelData.visible)
                    }
                    MenuItem {
                        text: "Rename path"
                        onTriggered: pathName.startRename()
                    }
                    MenuItem {
                        text: "Delete path"
                        onTriggered: editor.removePath(pathRow.pathId)
                    }
                }
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    spacing: 8
                    Rectangle {
                        Layout.preferredWidth: 30
                        Layout.preferredHeight: 30
                        radius: 3
                        clip: true
                        color: root.app.tokens.surfaceBase
                        border.color: pathRow.selected ? root.app.tokens.focusRing : root.app.tokens.borderSubtle
                        Image {
                            objectName: "pathThumb-" + pathRow.pathId
                            anchors.fill: parent
                            anchors.margins: 1
                            fillMode: Image.PreserveAspectFit
                            smooth: true
                            cache: false
                            sourceSize: Qt.size(56, 56)
                            source: "image://layerthumb/path/" + pathRow.pathId + "?g=" + editor.generation
                        }
                        TapHandler {
                            acceptedButtons: Qt.LeftButton
                            onTapped: (eventPoint, button) => {
                                root.selectedPathId = pathRow.pathId;
                                if (point.modifiers & Qt.ControlModifier)
                                    editor.loadPathSelection(pathRow.pathId, root.selectionMode(point.modifiers));
                            }
                        }
                    }
                    TextField {
                        id: pathName
                        objectName: "pathName-" + pathRow.pathId
                        property bool renaming: false
                        Layout.fillWidth: true
                        text: modelData.name
                        readOnly: !renaming
                        background.opacity: renaming ? 1 : 0
                        activeFocusOnPress: renaming
                        focusPolicy: renaming ? Qt.StrongFocus : Qt.NoFocus
                        selectByMouse: renaming
                        Accessible.name: "Path name"
                        function startRename() {
                            renaming = true;
                            forceActiveFocus();
                            selectAll();
                        }
                        function endRename(keep) {
                            if (!renaming)
                                return;
                            renaming = false;
                            if (keep && text !== modelData.name)
                                editor.renamePath(pathRow.pathId, text);
                            text = Qt.binding(() => modelData.name);
                            focus = false;
                        }
                        TapHandler {
                            acceptedButtons: Qt.LeftButton
                            enabled: !pathName.renaming
                            onTapped: root.selectedPathId = pathRow.pathId
                            onDoubleTapped: pathName.startRename()
                        }
                        onEditingFinished: endRename(true)
                        onActiveFocusChanged: if (!activeFocus) endRename(true)
                        Keys.onEscapePressed: endRename(false)
                    }
                }
            }
            Label {
                anchors.centerIn: parent
                width: parent.width - 20
                visible: pathList.count === 0
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                text: "No paths. Make a selection and turn it into a path."
                color: root.app.tokens.inkMuted
                font.pixelSize: 11
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: root.app.tokens.borderSubtle
        }
        RowLayout {
            objectName: "pathsFooter"
            Layout.fillWidth: true
            spacing: 2
            Item {
                Layout.fillWidth: true
            }
            CommandButton {
                objectName: "pathsFooterStroke"
                text: "Stroke path with brush"
                iconName: "brush"
                iconOnly: true
                enabled: root.selectedPathId.length > 0
                ToolTip.text: "Stroke path with brush"
                onClicked: editor.strokePath(root.selectedPathId)
            }
            CommandButton {
                objectName: "pathsFooterLoad"
                text: "Load path as a selection"
                iconName: "loadselection"
                iconOnly: true
                enabled: root.selectedPathId.length > 0
                ToolTip.text: "Load path as a selection"
                onClicked: editor.loadPathSelection(root.selectedPathId, "replace")
            }
            CommandButton {
                objectName: "pathsFooterFromSelection"
                text: "Make work path from selection"
                iconName: "pen"
                iconOnly: true
                enabled: editor.selectionActive
                ToolTip.text: "Make work path from selection"
                onClicked: editor.pathFromSelection()
            }
            CommandButton {
                objectName: "pathsFooterDelete"
                text: "Delete path"
                iconName: "trash"
                iconOnly: true
                enabled: root.selectedPathId.length > 0
                ToolTip.text: "Delete path"
                onClicked: {
                    editor.removePath(root.selectedPathId);
                    root.selectedPathId = "";
                }
            }
        }
    }
}
