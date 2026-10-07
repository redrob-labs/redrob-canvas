// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Image > Image Size (Ctrl+Alt+I) and Image > Canvas Size (Ctrl+Alt+C), batch 4 H6. One dialog,
// two modes, because the fields are the same: width, height, linked by default.
//   image  -- resamples every layer to the new size (the engine's resize_canvas command).
//   canvas -- keeps the pixels and grows or cuts the canvas around them; the anchor says which
//             part stays put (the engine's crop_canvas with a rectangle that may reach outside).
Dialog {
    id: root
    objectName: "sizeDialog"
    required property var tokens
    // "nearest" or "bilinear", the app's sampling choice.
    required property string samplingMode
    property string mode: "image"
    title: mode === "image" ? "Image size" : "Canvas size"
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: parent
    standardButtons: Dialog.Ok | Dialog.Cancel

    // Anchor 0..8, row by row; 4 is the middle.
    property int anchor: 4
    property bool linking: false

    function openFor(which) {
        mode = which;
        anchor = 4;
        linking = true;
        widthField.value = editor.documentWidth;
        heightField.value = editor.documentHeight;
        linking = false;
        open();
    }

    function relink(changedWidth) {
        if (linking || !keepRatio.checked || editor.documentWidth <= 0 || editor.documentHeight <= 0)
            return;
        linking = true;
        if (changedWidth)
            heightField.value = Math.max(1, Math.round(widthField.value * editor.documentHeight / editor.documentWidth));
        else
            widthField.value = Math.max(1, Math.round(heightField.value * editor.documentWidth / editor.documentHeight));
        linking = false;
    }

    onAccepted: {
        const w = widthField.value;
        const h = heightField.value;
        if (mode === "image") {
            editor.resizeCanvas(w, h, root.samplingMode);
            return;
        }
        const ax = (anchor % 3) / 2;
        const ay = Math.floor(anchor / 3) / 2;
        const x = Math.round((editor.documentWidth - w) * ax);
        const y = Math.round((editor.documentHeight - h) * ay);
        editor.cropCanvas(x, y, w, h);
    }

    GridLayout {
        columns: 2
        columnSpacing: 12
        rowSpacing: 8
        Label { text: "Width (px)"; color: root.tokens.inkPrimary }
        SpinBox {
            id: widthField
            objectName: "sizeWidth"
            from: 1; to: 32768; editable: true
            onValueChanged: root.relink(true)
        }
        Label { text: "Height (px)"; color: root.tokens.inkPrimary }
        SpinBox {
            id: heightField
            objectName: "sizeHeight"
            from: 1; to: 32768; editable: true
            onValueChanged: root.relink(false)
        }
        Label { text: "Percent"; color: root.tokens.inkPrimary }
        RowLayout {
            Repeater {
                model: [50, 100, 200]
                delegate: Button {
                    required property int modelData
                    text: modelData + "%"
                    flat: true
                    onClicked: {
                        root.linking = true;
                        widthField.value = Math.max(1, Math.round(editor.documentWidth * modelData / 100));
                        heightField.value = Math.max(1, Math.round(editor.documentHeight * modelData / 100));
                        root.linking = false;
                    }
                }
            }
        }
        CheckBox {
            id: keepRatio
            objectName: "sizeKeepRatio"
            Layout.columnSpan: 2
            text: "Keep proportions"
            checked: root.mode === "image"
        }
        Label {
            visible: root.mode === "canvas"
            text: "Anchor"
            color: root.tokens.inkPrimary
        }
        GridLayout {
            visible: root.mode === "canvas"
            objectName: "sizeAnchor"
            columns: 3
            rowSpacing: 2
            columnSpacing: 2
            Repeater {
                model: 9
                delegate: Button {
                    required property int index
                    implicitWidth: 28
                    implicitHeight: 28
                    checkable: true
                    checked: root.anchor === index
                    text: root.anchor === index ? "●" : ""
                    Accessible.name: "Anchor " + (index + 1)
                    onClicked: root.anchor = index
                }
            }
        }
        Label {
            Layout.columnSpan: 2
            Layout.maximumWidth: 340
            wrapMode: Text.Wrap
            color: root.tokens.inkMuted
            text: root.mode === "image" ? "Every layer is resampled (" + root.samplingMode + ")."
                                        : "Pixels keep their size. New space is transparent; space outside is cut off."
        }
    }
}
