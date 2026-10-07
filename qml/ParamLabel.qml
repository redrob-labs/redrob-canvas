// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Was an inline component of Main.qml; a file since P12 so split panels can use it.
// `window.` resolves through the instantiating Main.qml context, as it did inline.
// Label column for one-value-per-row parameters, so every field in a group starts at the same x.
Label {
    Layout.preferredWidth: 84
    elide: Text.ElideRight
}
