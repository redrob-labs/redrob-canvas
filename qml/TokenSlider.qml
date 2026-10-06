// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Was an inline component of Main.qml; a file since P12 so split panels can use it.
// `window.` resolves through the instantiating Main.qml context, as it did inline.
// A slider drawn from the tokens. The default style tints palette.highlight, so the fill measured
// srgb(62,98,254) on screen against the token's #2b52ff -- close enough to pass a glance and not the
// brand's colour. Track, fill and handle are painted here so the pixels are the token's own.
Slider {
    id: tokenSlider
    // The control's own height is what takes the press. The background and handle below set
    // only width/height, which contribute nothing to implicit size, so without this the slider
    // was a few pixels tall and a press on the visible track or handle could miss it.
    implicitHeight: 28
    implicitWidth: 120
    background: Rectangle {
        x: tokenSlider.leftPadding
        y: tokenSlider.topPadding + tokenSlider.availableHeight / 2 - height / 2
        implicitWidth: 120
        implicitHeight: 4
        width: tokenSlider.availableWidth
        height: 4
        radius: 2
        color: window.tokens.borderSubtle
        Rectangle {
            width: tokenSlider.visualPosition * parent.width
            height: parent.height
            radius: 2
            color: tokenSlider.enabled ? window.tokens.actionPrimary : window.tokens.inkMuted
        }
    }
    handle: Rectangle {
        x: tokenSlider.leftPadding + tokenSlider.visualPosition * (tokenSlider.availableWidth - width)
        y: tokenSlider.topPadding + tokenSlider.availableHeight / 2 - height / 2
        implicitWidth: 14
        implicitHeight: 14
        width: 14
        height: 14
        radius: 7
        color: window.tokens.surfaceBase
        border.width: 2
        border.color: tokenSlider.visualFocus ? window.tokens.focusRing
                                              : (tokenSlider.enabled ? window.tokens.actionPrimary : window.tokens.inkMuted)
    }
}
