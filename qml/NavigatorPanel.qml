// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Redrob.Graphics 1.0

// Window > Navigator (batch 4 M9): the whole image small, with a frame showing what the main view
// shows. Click or drag in it to move the view there; the slider zooms, as in Photoshop.
Rectangle {
    id: root
    objectName: "navigatorPanel"
    required property var app
    required property var mainCanvas
    color: app.tokens.surfaceSunken
    border.color: app.tokens.borderSubtle
    implicitHeight: 170

    // Canvas-coordinate corners of what the main view shows (rotation ignored: the frame is the
    // bounding box of the view).
    function viewRect() {
        const a = mainCanvas.canvasPoint(Qt.point(0, 0));
        const b = mainCanvas.canvasPoint(Qt.point(mainCanvas.width, mainCanvas.height));
        return Qt.rect(Math.min(a.x, b.x), Math.min(a.y, b.y), Math.abs(b.x - a.x), Math.abs(b.y - a.y));
    }
    function moveViewTo(itemPoint) {
        const r = thumb.imageRect;
        if (r.width <= 0 || r.height <= 0)
            return;
        const cx = (itemPoint.x - r.x) / r.width * editor.documentWidth;
        const cy = (itemPoint.y - r.y) / r.height * editor.documentHeight;
        mainCanvas.anchorCanvasPoint(Qt.point(cx, cy), Qt.point(mainCanvas.width / 2, mainCanvas.height / 2));
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 6
        spacing: 4
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            CanvasItem {
                id: thumb
                objectName: "navigatorThumbnail"
                anchors.fill: parent
                image: editor.renderImage
                zoom: editor.documentWidth > 0 && editor.documentHeight > 0
                      ? Math.min(width / editor.documentWidth, height / editor.documentHeight) : 1
                Accessible.name: "Navigator: the whole image"
            }
            // The main view's frame, recomputed whenever the view or the document moves.
            Rectangle {
                id: frame
                objectName: "navigatorFrame"
                color: "transparent"
                border.width: 2
                border.color: root.app.tokens.focusRing
                property rect view: {
                    // mainCanvas.imageRect too: canvasPoint() reads the main view's image size,
                    // which changes after documentWidth on a crop or resize. Without it the frame
                    // was computed against the old image and sat offset by half the size change.
                    root.mainCanvas.zoom; root.mainCanvas.pan; root.mainCanvas.width; root.mainCanvas.height;
                    root.mainCanvas.imageRect; editor.documentWidth; thumb.imageRect;
                    return root.viewRect();
                }
                readonly property real sx: editor.documentWidth > 0 ? thumb.imageRect.width / editor.documentWidth : 0
                readonly property real sy: editor.documentHeight > 0 ? thumb.imageRect.height / editor.documentHeight : 0
                x: thumb.imageRect.x + Math.max(0, view.x) * sx
                y: thumb.imageRect.y + Math.max(0, view.y) * sy
                width: Math.min(editor.documentWidth, view.x + view.width) * sx - Math.max(0, view.x) * sx
                height: Math.min(editor.documentHeight, view.y + view.height) * sy - Math.max(0, view.y) * sy
                visible: width > 0 && height > 0
            }
            MouseArea {
                anchors.fill: parent
                onPressed: mouse => root.moveViewTo(Qt.point(mouse.x, mouse.y))
                onPositionChanged: mouse => { if (pressed) root.moveViewTo(Qt.point(mouse.x, mouse.y)) }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: Math.round(root.app.canvasZoom * 100) + "%"
                color: root.app.tokens.inkSecondary
                Layout.preferredWidth: 44
                font.features: { "tnum": 1 }
            }
            // Logarithmic, so 5% .. 3200% is usable on one slider.
            TokenSlider {
                objectName: "navigatorZoom"
                Layout.fillWidth: true
                from: Math.log(0.05)
                to: Math.log(32)
                value: Math.log(root.app.canvasZoom)
                onMoved: root.app.canvasZoom = Math.exp(value)
                Accessible.name: "Zoom"
            }
        }
    }
}
