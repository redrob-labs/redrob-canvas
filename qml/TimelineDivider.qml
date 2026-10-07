// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Was an inline component of Main.qml; a file since P12 so split panels can use it.
// `window.` resolves through the instantiating Main.qml context, as it did inline.
// Thin separator between groups of timeline controls.
Rectangle {
    Layout.preferredWidth: 1
    Layout.preferredHeight: 20
    Layout.leftMargin: 6
    Layout.rightMargin: 6
    color: window.tokens.borderSubtle
}
