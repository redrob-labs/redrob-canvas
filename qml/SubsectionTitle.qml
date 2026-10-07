// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Was an inline component of Main.qml; a file since P12 so split panels can use it.
// `window.` resolves through the instantiating Main.qml context, as it did inline.
// A named group inside an option section. Follows Krita's brush editor, which keeps the tip
// (shape, size, hardness, ratio) apart from how paint lands (opacity) and how the stroke is
// drawn (smoothing, mirror). Sentence case, so it reads below the section's capital heading.
Label {
    Layout.fillWidth: true
    topPadding: 6
    color: window.tokens.inkMuted
    font.pixelSize: 11
    font.weight: Font.DemiBold
}
