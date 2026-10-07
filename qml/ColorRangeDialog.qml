// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Select > Color Range (batch 4 M6). Picks every pixel of the active layer near the foreground
// colour -- set it with the eyedropper first (I, or Alt while painting) -- or a tone range, with a
// soft edge set by Fuzziness. The result combines with the current selection by the toolbar's
// selection mode, as the other selection tools do.
Dialog {
    id: root
    objectName: "colorRangeDialog"
    required property var tokens
    required property string selectionMode
    title: "Color range"
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: parent
    standardButtons: Dialog.Ok | Dialog.Cancel

    onAccepted: editor.selectColorRange(editor.brushColor, Math.round(fuzziness.value),
                                        ["sampled", "shadows", "midtones", "highlights"][rangeBox.currentIndex],
                                        root.selectionMode)

    GridLayout {
        columns: 3
        columnSpacing: 12
        rowSpacing: 8
        Label { text: "Select"; color: root.tokens.inkPrimary }
        ComboBox {
            id: rangeBox
            objectName: "colorRangeKind"
            Layout.columnSpan: 2
            Layout.fillWidth: true
            model: ["Sampled colour (foreground)", "Shadows", "Midtones", "Highlights"]
        }
        Label { text: "Fuzziness"; color: root.tokens.inkPrimary; visible: rangeBox.currentIndex === 0 }
        TokenSlider {
            id: fuzziness
            objectName: "colorRangeFuzziness"
            visible: rangeBox.currentIndex === 0
            from: 1
            to: 200
            value: 40
            Layout.fillWidth: true
            Accessible.name: "Fuzziness"
        }
        Label {
            visible: rangeBox.currentIndex === 0
            text: Math.round(fuzziness.value)
            color: root.tokens.inkSecondary
            Layout.preferredWidth: 32
        }
        Rectangle {
            visible: rangeBox.currentIndex === 0
            Layout.columnSpan: 3
            implicitWidth: 28
            implicitHeight: 18
            radius: 4
            color: editor.brushColor
            border.color: root.tokens.borderSubtle
            Accessible.name: "Colour to match"
        }
    }
}
