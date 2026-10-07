// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Edit > Stroke (batch 4 M5): a band along the selection edge on the active layer, in the
// foreground colour, as Photoshop's Stroke dialog. Needs a selection.
Dialog {
    id: root
    objectName: "strokeSelectionDialog"
    required property var tokens
    title: "Stroke selection"
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: parent
    standardButtons: Dialog.Ok | Dialog.Cancel

    onAccepted: editor.strokeSelection(widthField.value, editor.brushColor,
                                       ["inside", "center", "outside"][locationBox.currentIndex])

    GridLayout {
        columns: 2
        columnSpacing: 12
        rowSpacing: 8
        Label { text: "Width (px)"; color: root.tokens.inkPrimary }
        SpinBox { id: widthField; objectName: "strokeWidth"; from: 1; to: 250; value: 3; editable: true }
        Label { text: "Location"; color: root.tokens.inkPrimary }
        ComboBox {
            id: locationBox
            objectName: "strokeLocation"
            model: ["Inside", "Center", "Outside"]
            currentIndex: 1
        }
        Label {
            Layout.columnSpan: 2
            text: "Painted in the foreground colour on the active layer."
            color: root.tokens.inkMuted
        }
    }
}
