// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Add or edit a recognised vector rectangle. Moved out of Main.qml in P12.
Dialog {
    id: root
    objectName: "vectorRectangleEditor"
    property string nodeId: ""
    title: nodeId.length > 0 ? "Edit recognized vector rectangle" : "Add vector rectangle"
    modal: true
    anchors.centerIn: parent
    standardButtons: Dialog.Ok | Dialog.Cancel
    // The shape tool draws with this dialog's fill and stroke (commitShape in Main.qml), so they
    // are readable from outside. P12: Main.qml used to read the fields by id.
    readonly property string fillColor: vectorFill.text
    readonly property string strokeColor: vectorStrokeColor.text
    readonly property real strokeWidth: Number(vectorStroke.text)
    function openNew(fill) {
        nodeId = ""
        vectorName.text = "New vector"
        vectorX.text = "48"
        vectorY.text = "48"
        vectorW.text = "180"
        vectorH.text = "120"
        vectorStroke.text = "2"
        vectorFill.text = fill
        vectorStrokeColor.text = "#ff20242a"
        open()
    }
    function openEdit(editId, x, y, rectWidth, rectHeight, fill, stroke, strokeWidthValue) {
        nodeId = editId
        vectorX.text = String(x)
        vectorY.text = String(y)
        vectorW.text = String(rectWidth)
        vectorH.text = String(rectHeight)
        vectorFill.text = fill
        vectorStrokeColor.text = stroke
        vectorStroke.text = String(strokeWidthValue)
        open()
    }
    onAccepted: {
        if (nodeId.length > 0)
            editor.setVectorRectangle(nodeId, Number(vectorX.text), Number(vectorY.text),
                                      Number(vectorW.text), Number(vectorH.text),
                                      vectorFill.text, vectorStrokeColor.text,
                                      Number(vectorStroke.text))
        else
            editor.addVectorRectangle(vectorName.text, Number(vectorX.text), Number(vectorY.text),
                                      Number(vectorW.text), Number(vectorH.text),
                                      vectorFill.text, vectorStrokeColor.text,
                                      Number(vectorStroke.text))
    }
    ColumnLayout {
        width: 380
        Label { text: "Core-rendered solid rectangle (no SVG/platform painter)"; wrapMode: Text.Wrap }
        TextField {
            id: vectorName
            Layout.fillWidth: true
            placeholderText: "Node name"
            visible: root.nodeId.length === 0
        }
        GridLayout {
            columns: 4
            Label { text: "X" }
            TextField { id: vectorX; text: "48"; validator: DoubleValidator {} }
            Label { text: "Y" }
            TextField { id: vectorY; text: "48"; validator: DoubleValidator {} }
            Label { text: "Width" }
            TextField { id: vectorW; text: "180"; validator: DoubleValidator { bottom: 0.00390625 } }
            Label { text: "Height" }
            TextField { id: vectorH; text: "120"; validator: DoubleValidator { bottom: 0.00390625 } }
            Label { text: "Stroke" }
            TextField { id: vectorStroke; text: "2"; validator: DoubleValidator { bottom: 0.00390625; top: 4096 } }
        }
        RowLayout {
            Label { text: "Fill" }
            TextField { id: vectorFill; text: "#ff5378dc" }
            Label { text: "Stroke color" }
            TextField { id: vectorStrokeColor; text: "#ff20242a" }
        }
    }
}
