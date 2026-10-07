// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Was an inline component of Main.qml; a file since P12 so split panels can use it.
// `window.` resolves through the instantiating Main.qml context, as it did inline.
// redrob-ui :focus-visible: a 2px focusRing outline 2px outside the control, on keyboard focus only
// (visualFocus is false after a mouse click). Placed inside a background so it follows its shape.
Rectangle {
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
