// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// File > New (Ctrl+N), batch 4 H4. Size, then what the one layer starts as: white, the background
// colour, or transparent -- Photoshop's three "Background Contents" that matter. Replacing the
// document drops the current one, so the dialog says so; it does not save for the user.
Dialog {
    id: root
    objectName: "newDocumentDialog"
    required property var tokens
    // The window's background colour (X / D).
    required property color backgroundColor
    title: "New document"
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: parent
    standardButtons: Dialog.Ok | Dialog.Cancel

    readonly property var presets: [
        ["Current size", 0, 0],
        ["1920 × 1080 (HD)", 1920, 1080],
        ["3840 × 2160 (4K)", 3840, 2160],
        ["2480 × 3508 (A4, 300 ppi)", 2480, 3508],
        ["1080 × 1080 (square)", 1080, 1080],
        ["800 × 600", 800, 600]
    ]

    function openNew() {
        widthField.value = editor.documentWidth;
        heightField.value = editor.documentHeight;
        presetBox.currentIndex = 0;
        open();
    }

    onAccepted: {
        const fill = contentsBox.currentIndex === 0 ? Qt.rgba(1, 1, 1, 1)
                   : contentsBox.currentIndex === 1 ? root.backgroundColor
                   : Qt.rgba(0, 0, 0, 0);
        editor.newDocument(widthField.value, heightField.value, fill);
    }

    GridLayout {
        columns: 2
        columnSpacing: 12
        rowSpacing: 8
        Label { text: "Preset"; color: root.tokens.inkPrimary }
        ComboBox {
            id: presetBox
            objectName: "newDocumentPreset"
            Layout.fillWidth: true
            model: root.presets.map(p => p[0])
            onActivated: index => {
                if (index > 0) {
                    widthField.value = root.presets[index][1];
                    heightField.value = root.presets[index][2];
                }
            }
        }
        Label { text: "Width (px)"; color: root.tokens.inkPrimary }
        SpinBox { id: widthField; objectName: "newDocumentWidth"; from: 1; to: 30000; editable: true }
        Label { text: "Height (px)"; color: root.tokens.inkPrimary }
        SpinBox { id: heightField; objectName: "newDocumentHeight"; from: 1; to: 30000; editable: true }
        Label { text: "Contents"; color: root.tokens.inkPrimary }
        ComboBox {
            id: contentsBox
            objectName: "newDocumentContents"
            Layout.fillWidth: true
            model: ["White", "Background colour", "Transparent"]
        }
        Label {
            Layout.columnSpan: 2
            text: "The open image is replaced. Save it first if you want to keep it."
            color: root.tokens.inkMuted
            wrapMode: Text.Wrap
            Layout.maximumWidth: 360
        }
    }
}
