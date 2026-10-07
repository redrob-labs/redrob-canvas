// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Was an inline component of Main.qml; a file since P12 so split panels can use it.
// `window.` resolves through the instantiating Main.qml context, as it did inline.
// Thin separator between groups of tools on the rail (paint, transform, fill, select, view).
Rectangle {
    Layout.alignment: Qt.AlignHCenter
    Layout.preferredWidth: 24
    Layout.preferredHeight: 1
    Layout.topMargin: 2
    Layout.bottomMargin: 2
    color: window.tokens.borderSubtle
}
