// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Layer > Blending Options > Blend If (batch 4 L6), Photoshop's two gray ramps. Each has four
// stops: hidden at or below the first, fading in to the second, shown to the third, fading out
// to the fourth, hidden above. 0 0 255 255 hides nothing. (Photoshop splits a slider with Alt-drag;
// here the four numbers are set directly.)
Dialog {
    id: root
    objectName: "blendIfDialog"
    required property var tokens
    property string nodeId: ""
    title: "Blend if (gray)"
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: parent
    standardButtons: Dialog.Ok | Dialog.Reset | Dialog.Cancel

    function openFor(id, current) {
        nodeId = id;
        const t = current && current.this_layer ? current.this_layer : null;
        const u = current && current.underlying ? current.underlying : null;
        const fill = (row, r) => {
            row.itemAt(0).value = r ? r.black_low : 0;
            row.itemAt(1).value = r ? r.black_high : 0;
            row.itemAt(2).value = r ? r.white_low : 255;
            row.itemAt(3).value = r ? r.white_high : 255;
        };
        fill(thisRow, t);
        fill(underRow, u);
        open();
    }
    function range(row) {
        return { black_low: row.itemAt(0).value, black_high: row.itemAt(1).value,
                 white_low: row.itemAt(2).value, white_high: row.itemAt(3).value };
    }
    onAccepted: editor.setLayerBlendIf(nodeId, range(thisRow), range(underRow))
    onReset: {
        editor.clearLayerBlendIf(nodeId);
        close();
    }

    GridLayout {
        columns: 5
        columnSpacing: 8
        rowSpacing: 8
        Label { text: "This layer"; color: root.tokens.inkPrimary }
        Repeater {
            id: thisRow
            model: [0, 0, 255, 255]
            delegate: SpinBox {
                required property int modelData
                from: 0; to: 255; editable: true
                value: modelData
            }
        }
        Label { text: "Underlying"; color: root.tokens.inkPrimary }
        Repeater {
            id: underRow
            model: [0, 0, 255, 255]
            delegate: SpinBox {
                required property int modelData
                from: 0; to: 255; editable: true
                value: modelData
            }
        }
        Label {
            Layout.columnSpan: 5
            text: "Each row must rise: hidden ≤ first, fade in to second, shown to third, fade out to fourth."
            color: root.tokens.inkMuted
            wrapMode: Text.Wrap
            Layout.maximumWidth: 460
        }
    }
}
