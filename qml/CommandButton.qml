// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Was an inline component of Main.qml; a file since P12 so split panels can use it.
// `window.` resolves through the instantiating Main.qml context, as it did inline.
ToolButton {
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
               : commandButton.down || commandButton.hovered || commandButton.highlighted
                 ? window.tokens.borderSubtle : "transparent"
        FocusOutline { shown: commandButton.visualFocus; innerRadius: 7 }
    }
}
