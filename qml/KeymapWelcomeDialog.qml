// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// First start: which program's keys does this person already know? The answer picks the keyboard
// layout (Keymap.qml). Closing the dialog keeps Photoshop's and counts as an answer, so it is asked
// once. Edit > Keyboard layout changes it later.
Dialog {
    id: root
    objectName: "keymapWelcomeDialog"
    required property var tokens
    required property var keymap
    title: "Welcome to Redrob Canvas"
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(560, parent.width - 40)
    closePolicy: Popup.CloseOnEscape

    function pick(name) {
        root.keymap.choose(name);
        root.close();
    }
    onClosed: if (!root.keymap.profileChosen) root.keymap.choose("photoshop")

    ColumnLayout {
        anchors.fill: parent
        spacing: 16
        Label {
            text: "Which shortcuts do your hands already know?"
            font.pixelSize: 16
            font.bold: true
            color: root.tokens.inkPrimary
            Layout.fillWidth: true
            wrapMode: Text.Wrap
        }
        Label {
            text: "Tools, menus and keys follow the program you pick."
            color: root.tokens.inkMuted
            Layout.fillWidth: true
            wrapMode: Text.Wrap
        }
        RowLayout {
            spacing: 12
            Layout.fillWidth: true
            Repeater {
                model: [
                    ["photoshop", "Photoshop", "V move · M marquee · B brush · Ctrl+J duplicate"],
                    ["illustrator", "Illustrator", "V selection · P pen · M rectangle · Ctrl+7 clip"]
                ]
                delegate: Button {
                    required property var modelData
                    objectName: "keymapWelcome-" + modelData[0]
                    Layout.fillWidth: true
                    Layout.preferredHeight: 96
                    Accessible.name: "Use " + modelData[1] + " shortcuts"
                    onClicked: root.pick(modelData[0])
                    contentItem: ColumnLayout {
                        spacing: 6
                        Label {
                            text: modelData[1]
                            font.pixelSize: 15
                            font.bold: true
                            color: root.tokens.inkPrimary
                            Layout.alignment: Qt.AlignHCenter
                        }
                        Label {
                            text: modelData[2]
                            color: root.tokens.inkMuted
                            wrapMode: Text.Wrap
                            horizontalAlignment: Text.AlignHCenter
                            Layout.fillWidth: true
                        }
                    }
                }
            }
        }
        Label {
            text: "Change it any time: Edit > Keyboard layout. F1 lists every key."
            color: root.tokens.inkMuted
            Layout.fillWidth: true
            wrapMode: Text.Wrap
        }
    }
}
