// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Was an inline component of Main.qml; a file since P12 so split panels can use it.
// `window.` resolves through the instantiating Main.qml context, as it did inline.
TextField {
    Layout.fillWidth: true
    selectByMouse: true
    horizontalAlignment: TextInput.AlignRight
    Accessible.name: placeholderText
}
