// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Was an inline component of Main.qml; a file since P12 so split panels can use it.
// `window.` resolves through the instantiating Main.qml context, as it did inline.
// One group of options. Tool groups show only while their tool is active, so the panel holds
// what the current tool needs instead of every operation at once. Image-wide groups collapse.
ColumnLayout {
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
