// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Add or edit a text node (P10 paragraph fields included). Moved out of Main.qml in P12.
Dialog {
    id: root
    objectName: "textSemanticEditor"
    property string nodeId: ""
    property string sourceFontId: "font8x8-basic-0.3.1"
    property string sourceFontFamily: "font8x8 Basic Latin"
    title: nodeId.length > 0 ? "Edit deterministic text" : "Add deterministic text"
    modal: true
    // Centre on the window, not on the side-panel tab this item lives in:
    // centred on the panel the dialog ran off the window's right edge.
    parent: Overlay.overlay
    anchors.centerIn: parent
    standardButtons: Dialog.Ok | Dialog.Cancel
    // One entry point for a NEW text node, shared by the layer menu's
    // "Add text" and the text tool, which passes the clicked canvas point.
    function openNew(originX, originY) {
        nodeId = ""
        textName.text = "New text"
        semanticText.text = "Text"
        textX.text = String(Math.round(originX))
        textY.text = String(Math.round(originY))
        textSize.text = "32"
        textColor.text = editor.brushColor.toString()
        sourceFontId = "font8x8-basic-0.3.1"
        sourceFontFamily = "font8x8 Basic Latin"
        textBoxWidth.text = ""
        textAlign.currentIndex = 0
        open()
        semanticText.forceActiveFocus()
        semanticText.selectAll()
    }
    // Loads an existing node for editing (layer menu). P12: the layer delegate used to write
    // each field by id, which only worked while both lived in one file.
    function openEdit(editId, text, originX, originY, fontSize, color, fontId, fontFamily, boxWidth, align) {
        nodeId = editId
        semanticText.text = text
        textX.text = String(originX)
        textY.text = String(originY)
        textSize.text = String(fontSize)
        textColor.text = color
        sourceFontId = fontId
        sourceFontFamily = fontFamily
        textBoxWidth.text = boxWidth > 0 ? String(boxWidth) : ""
        textAlign.currentIndex = Math.max(0, ["left", "center", "right"].indexOf(align))
        open()
    }
    onAccepted: {
        const boxWidth = textBoxWidth.text.length > 0 ? Number(textBoxWidth.text) : -1
        if (nodeId.length > 0)
            editor.setTextContent(nodeId, semanticText.text, Number(textX.text),
                                  Number(textY.text), Number(textSize.text), textColor.text,
                                  sourceFontFamily, sourceFontId, boxWidth, textAlign.currentText)
        else
            editor.addTextNode(textName.text, semanticText.text, Number(textX.text),
                               Number(textY.text), Number(textSize.text), textColor.text,
                               "", -1, boxWidth, textAlign.currentText)
    }
    ColumnLayout {
        width: 360
        Label { text: "Printable ASCII + newline only · embedded font8x8"; wrapMode: Text.Wrap }
        TextField {
            id: textName
            Layout.fillWidth: true
            placeholderText: "Node name"
            visible: root.nodeId.length === 0
        }
        TextArea {
            id: semanticText
            objectName: "semanticTextInput"
            Layout.fillWidth: true
            Layout.preferredHeight: 110
            wrapMode: TextEdit.NoWrap
            onTextChanged: if (length > 262144) text = text.slice(0, 262144)
            Accessible.name: "Bounded semantic text content"
        }
        RowLayout {
            Label { text: "X" }
            TextField { id: textX; text: "24"; validator: DoubleValidator {} }
            Label { text: "Y" }
            TextField { id: textY; text: "24"; validator: DoubleValidator {} }
            Label { text: "Size" }
            TextField { id: textSize; text: "32"; validator: DoubleValidator { bottom: 0.00390625; top: 4096 } }
        }
        RowLayout {
            Label { text: "Color" }
            TextField {
                id: textColor
                Layout.fillWidth: true
                text: "#ff000000"
            }
        }
        // Paragraph text (P10): a box width wraps lines at words; empty is
        // point text, which breaks only at newlines.
        RowLayout {
            Label { text: "Box width" }
            TextField {
                id: textBoxWidth
                objectName: "textBoxWidthInput"
                Layout.fillWidth: true
                placeholderText: "none (point text)"
                validator: DoubleValidator { bottom: 0.00390625 }
                Accessible.name: "Paragraph box width in pixels, empty for point text"
            }
            Label { text: "Align" }
            ComboBox {
                id: textAlign
                objectName: "textAlignCombo"
                model: ["left", "center", "right"]
                Accessible.name: "Text line alignment"
            }
        }
        Label {
            text: "Font: " + root.sourceFontFamily + " (" + root.sourceFontId + ")"
            wrapMode: Text.Wrap
        }
    }
}
