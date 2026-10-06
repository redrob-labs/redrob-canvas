// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Was an inline component of Main.qml; a file since P12 so split panels can use it.
// `window.` resolves through the instantiating Main.qml context, as it did inline.
ToolButton {
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
    // Photoshop-style tool group: the tools sharing a group name share one rail cell, which
    // shows the one picked last. Right-click or press and hold the cell to choose another.
    property string group: ""
    visible: group.length === 0 || window.groupCurrent[group] === toolId
    Component.onCompleted: if (group.length > 0) window.registerGroupTool(group, toolId, toolName, iconName)
    checkable: true
    checked: window.activeTool === toolId
    // 36 px with no row gap: two columns of 15 rows and the colour swatch fit an 800 px window.
    implicitWidth: 36
    implicitHeight: 36
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
        // The corner mark says "more tools here", as on Photoshop's toolbar.
        Canvas {
            visible: toolButton.group.length > 0
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: 3
            width: 5
            height: 5
            property color ink: toolButton.icon.color
            onInkChanged: requestPaint()
            onPaint: {
                const context = getContext("2d");
                context.reset();
                context.fillStyle = ink;
                context.beginPath();
                context.moveTo(width, 0);
                context.lineTo(width, height);
                context.lineTo(0, height);
                context.closePath();
                context.fill();
            }
        }
    }
    TapHandler {
        enabled: toolButton.group.length > 0
        acceptedButtons: Qt.RightButton
        onTapped: window.openToolGroup(toolButton.group, toolButton)
    }
    onPressAndHold: if (group.length > 0) window.openToolGroup(group, toolButton)
}
