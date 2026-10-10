// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The Channels tab, laid out as Photoshop's: the composite and its colour channels (thumbnails of
// the current picture), then the document's alpha channels. Ctrl+click an alpha channel's
// thumbnail to load it as the selection (Shift adds, Alt subtracts, both intersect).
Item {
    id: root
    objectName: "channelsPanel"
    // The window, for its design tokens.
    required property var app
    property string selectedChannelId: ""

    // Photoshop's modifiers for "load as selection" on a channel or path thumbnail.
    function selectionMode(mods) {
        const shift = (mods & Qt.ShiftModifier) !== 0;
        const alt = (mods & Qt.AltModifier) !== 0;
        return shift && alt ? "intersect" : shift ? "add" : alt ? "subtract" : "replace";
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 10
        spacing: 6

        // Composite and colour channels: read-only views of the current picture.
        Repeater {
            model: [["rgb", "RGB"], ["red", "Red"], ["green", "Green"], ["blue", "Blue"]]
            delegate: RowLayout {
                required property var modelData
                objectName: "compositeChannel-" + modelData[0]
                Layout.fillWidth: true
                Layout.preferredHeight: 38
                spacing: 8
                Rectangle {
                    Layout.preferredWidth: 30
                    Layout.preferredHeight: 30
                    radius: 3
                    clip: true
                    color: root.app.tokens.surfaceBase
                    border.color: root.app.tokens.borderSubtle
                    Image {
                        objectName: "compositeChannelThumb-" + modelData[0]
                        anchors.fill: parent
                        anchors.margins: 1
                        fillMode: Image.PreserveAspectFit
                        smooth: true
                        cache: false
                        sourceSize: Qt.size(56, 56)
                        source: "image://layerthumb/composite/" + modelData[0] + "?g=" + editor.generation
                    }
                }
                Label {
                    Layout.fillWidth: true
                    text: modelData[1]
                    color: root.app.tokens.inkPrimary
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: root.app.tokens.borderSubtle
        }

        ListView {
            id: channelList
            objectName: "alphaChannelList"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: editor.channels
            delegate: Rectangle {
                id: channelRow
                required property var modelData
                readonly property string channelId: modelData.id
                readonly property bool selected: root.selectedChannelId === channelId
                objectName: "alphaChannel-" + channelId
                width: ListView.view.width
                height: 46
                radius: 4
                color: selected ? root.app.tokens.borderSubtle : "transparent"
                TapHandler {
                    acceptedButtons: Qt.LeftButton
                    onTapped: root.selectedChannelId = channelRow.channelId
                }
                TapHandler {
                    acceptedButtons: Qt.RightButton
                    onTapped: {
                        root.selectedChannelId = channelRow.channelId;
                        channelMenu.open();
                    }
                }
                Menu {
                    id: channelMenu
                    MenuItem {
                        text: "Load as selection"
                        onTriggered: editor.loadChannelSelection(channelRow.channelId, "replace")
                    }
                    MenuItem {
                        text: "Rename channel"
                        onTriggered: channelName.startRename()
                    }
                    MenuItem {
                        text: "Delete channel"
                        onTriggered: editor.removeChannel(channelRow.channelId)
                    }
                }
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 4
                    anchors.rightMargin: 8
                    spacing: 6
                    CommandButton {
                        objectName: "alphaChannelVisibility-" + channelRow.channelId
                        text: modelData.visible ? "Hide" : "Show"
                        iconName: "eye"
                        iconOnly: true
                        opacity: modelData.visible ? 1 : 0.35
                        Accessible.name: "Toggle the overlay of " + modelData.name
                        onClicked: editor.setChannelVisible(channelRow.channelId, !modelData.visible)
                    }
                    Rectangle {
                        Layout.preferredWidth: 30
                        Layout.preferredHeight: 30
                        radius: 3
                        clip: true
                        color: root.app.tokens.surfaceBase
                        border.color: channelRow.selected ? root.app.tokens.focusRing : root.app.tokens.borderSubtle
                        Image {
                            objectName: "alphaChannelThumb-" + channelRow.channelId
                            anchors.fill: parent
                            anchors.margins: 1
                            fillMode: Image.PreserveAspectFit
                            smooth: true
                            cache: false
                            sourceSize: Qt.size(56, 56)
                            source: "image://layerthumb/channel/" + channelRow.channelId + "?g=" + editor.generation
                        }
                        TapHandler {
                            acceptedButtons: Qt.LeftButton
                            onTapped: (eventPoint, button) => {
                                root.selectedChannelId = channelRow.channelId;
                                if (point.modifiers & Qt.ControlModifier)
                                    editor.loadChannelSelection(channelRow.channelId, root.selectionMode(point.modifiers));
                            }
                        }
                    }
                    TextField {
                        id: channelName
                        objectName: "alphaChannelName-" + channelRow.channelId
                        property bool renaming: false
                        Layout.fillWidth: true
                        text: modelData.name
                        readOnly: !renaming
                        background.opacity: renaming ? 1 : 0
                        activeFocusOnPress: renaming
                        focusPolicy: renaming ? Qt.StrongFocus : Qt.NoFocus
                        selectByMouse: renaming
                        Accessible.name: "Channel name"
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
                                editor.renameChannel(channelRow.channelId, text);
                            text = Qt.binding(() => modelData.name);
                            focus = false;
                        }
                        TapHandler {
                            acceptedButtons: Qt.LeftButton
                            enabled: !channelName.renaming
                            onTapped: root.selectedChannelId = channelRow.channelId
                            onDoubleTapped: channelName.startRename()
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
                visible: channelList.count === 0
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                text: "No alpha channels. Make a selection and save it as a channel."
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
            objectName: "channelsFooter"
            Layout.fillWidth: true
            spacing: 2
            Item {
                Layout.fillWidth: true
            }
            CommandButton {
                objectName: "channelsFooterLoad"
                text: "Load channel as selection"
                iconName: "loadselection"
                iconOnly: true
                enabled: root.selectedChannelId.length > 0
                ToolTip.text: "Load channel as selection"
                onClicked: editor.loadChannelSelection(root.selectedChannelId, "replace")
            }
            CommandButton {
                objectName: "channelsFooterSave"
                text: "Save selection as channel"
                iconName: "mask"
                iconOnly: true
                enabled: editor.selectionActive
                ToolTip.text: "Save selection as channel"
                onClicked: editor.addChannel(true)
            }
            CommandButton {
                objectName: "channelsFooterNew"
                text: "New channel"
                iconName: "plus"
                iconOnly: true
                ToolTip.text: "Create a new channel"
                onClicked: editor.addChannel(false)
            }
            CommandButton {
                objectName: "channelsFooterDelete"
                text: "Delete channel"
                iconName: "trash"
                iconOnly: true
                enabled: root.selectedChannelId.length > 0
                ToolTip.text: "Delete channel"
                onClicked: {
                    editor.removeChannel(root.selectedChannelId);
                    root.selectedChannelId = "";
                }
            }
        }
    }
}
