// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Redrob.Graphics 1.0

ApplicationWindow {
    id: window
    width: 1440
    height: 900
    minimumWidth: 760
    minimumHeight: 540
    visible: true
    title: editor.currentFile.length > 0 ? "Redrob Graphics — " + editor.currentFile : "Redrob Graphics"
    color: "#17191d"
    palette.window: "#17191d"
    palette.windowText: "#eceff4"
    palette.base: "#202328"
    palette.alternateBase: "#292d33"
    palette.text: "#eceff4"
    palette.button: "#292d33"
    palette.buttonText: "#eceff4"
    palette.highlight: "#5d7ef7"
    palette.highlightedText: "#ffffff"

    property string activeTool: "brush"
    property real canvasZoom: 1.0
    property string selectionMode: "replace"
    property string gradientKind: "linear"
    property color gradientStartColor: "#f4f6ff"
    property color gradientEndColor: "#4267c9"
    property string samplingMode: "bilinear"
    property string exportFormat: "png"
    property bool exportAllowLoss: false
    property int exportJpegQuality: 90
    property color exportMatte: "#ffffff"

    function maskNodeFromSelection(nodeId) {
        editor.rasterMaskFromSelection(nodeId)
    }

    onActiveToolChanged: canvasPointer.cancelGesture()

    component CommandButton: ToolButton {
        id: commandButton
        implicitHeight: 38
        leftPadding: 10
        rightPadding: 10
        font.pixelSize: 13
        ToolTip.visible: hovered
        ToolTip.delay: 500
        Accessible.name: ToolTip.text.length > 0 ? ToolTip.text : text
        background: Rectangle {
            radius: 7
            color: commandButton.down ? "#3d4658" : commandButton.hovered ? "#343941" : "transparent"
        }
    }

    component ToolRailButton: ToolButton {
        id: toolButton
        required property string toolId
        checkable: true
        checked: window.activeTool === toolId
        implicitWidth: 44
        implicitHeight: 40
        font.pixelSize: 13
        ToolTip.visible: hovered
        ToolTip.delay: 450
        Accessible.name: ToolTip.text
        onClicked: window.activeTool = toolId
        background: Rectangle {
            radius: 8
            color: toolButton.checked ? "#43557f" : toolButton.hovered ? "#30343b" : "transparent"
            border.color: toolButton.checked ? "#6f8ff7" : "transparent"
        }
    }

    component SectionTitle: Label {
        Layout.fillWidth: true
        topPadding: 8
        text: "SECTION"
        color: "#929aa5"
        font.pixelSize: 10
        font.weight: Font.DemiBold
    }

    component NumericField: TextField {
        Layout.fillWidth: true
        selectByMouse: true
        horizontalAlignment: TextInput.AlignRight
        Accessible.name: placeholderText
    }

    FileDialog {
        id: openProjectDialog
        title: "Open Redrob Project"
        fileMode: FileDialog.OpenFile
        nameFilters: ["Redrob projects (*.rrg)"]
        onAccepted: editor.openProject(selectedFile)
    }
    FileDialog {
        id: importDialog
        title: "Import Interchange File"
        fileMode: FileDialog.OpenFile
        nameFilters: ["Supported interchange (*.png *.jpg *.jpeg *.webp *.ora *.svg)",
                      "PNG images (*.png)", "JPEG images (*.jpg *.jpeg)",
                      "Lossless WebP images (*.webp)", "OpenRaster documents (*.ora)",
                      "Limited SVG documents (*.svg)"]
        onAccepted: editor.importFile(selectedFile)
    }
    FileDialog {
        id: saveProjectDialog
        title: "Save Redrob Project As"
        fileMode: FileDialog.SaveFile
        defaultSuffix: "rrg"
        nameFilters: ["Redrob projects (*.rrg)"]
        onAccepted: editor.saveProject(selectedFile)
    }
    FileDialog {
        id: exportFileDialog
        title: "Export Current Frame"
        fileMode: FileDialog.SaveFile
        defaultSuffix: window.exportFormat === "jpeg" ? "jpg" : window.exportFormat
        nameFilters: window.exportFormat === "png" ? ["PNG images (*.png)"]
            : window.exportFormat === "jpeg" ? ["JPEG images (*.jpg *.jpeg)"]
            : window.exportFormat === "webp" ? ["Lossless WebP images (*.webp)"]
            : window.exportFormat === "ora" ? ["OpenRaster documents (*.ora)"]
            : ["Limited SVG documents (*.svg)"]
        onAccepted: editor.exportFile(selectedFile, window.exportFormat,
                                      window.exportAllowLoss, editor.currentFrame,
                                      window.exportJpegQuality, window.exportMatte)
    }
    Dialog {
        id: exportOptionsDialog
        objectName: "exportOptionsDialog"
        anchors.centerIn: parent
        modal: true
        title: "Export Current Frame"
        standardButtons: Dialog.Ok | Dialog.Cancel
        onAccepted: exportFileDialog.open()
        ColumnLayout {
            width: 390
            spacing: 10
            Label {
                Layout.fillWidth: true
                text: "Choose an interchange format. Export never changes the project path or current frame."
                wrapMode: Text.Wrap
                color: "#b9c0ca"
            }
            RowLayout {
                Layout.fillWidth: true
                Label { text: "Format"; Layout.preferredWidth: 90 }
                ComboBox {
                    id: exportFormatControl
                    objectName: "exportFormatControl"
                    Layout.fillWidth: true
                    model: ["png", "jpeg", "webp", "ora", "svg"]
                    currentIndex: model.indexOf(window.exportFormat)
                    onActivated: window.exportFormat = currentText
                }
            }
            CheckBox {
                id: allowLossControl
                objectName: "allowLossControl"
                Layout.fillWidth: true
                text: "Allow documented representational loss"
                checked: window.exportAllowLoss
                onToggled: window.exportAllowLoss = checked
            }
            Label {
                Layout.fillWidth: true
                text: window.exportAllowLoss
                    ? "Allowed loss is returned as machine-readable warnings in the status bar."
                    : "Strict mode rejects omitted frames, selection, metadata, flattening, and rasterization."
                color: window.exportAllowLoss ? "#f0c674" : "#9ca4af"
                wrapMode: Text.Wrap
                font.pixelSize: 11
            }
            RowLayout {
                Layout.fillWidth: true
                visible: window.exportFormat === "jpeg"
                Label { text: "JPEG quality"; Layout.preferredWidth: 90 }
                SpinBox {
                    objectName: "jpegQualityControl"
                    from: 1
                    to: 100
                    value: window.exportJpegQuality
                    editable: true
                    onValueModified: window.exportJpegQuality = value
                }
            }
            RowLayout {
                Layout.fillWidth: true
                visible: window.exportFormat === "jpeg"
                Label { text: "Opaque matte"; Layout.preferredWidth: 90 }
                Button {
                    text: "Choose…"
                    onClicked: jpegMatteDialog.open()
                }
                Rectangle {
                    width: 28
                    height: 22
                    radius: 4
                    color: window.exportMatte
                    border.color: "#7b838e"
                }
            }
            Label {
                Layout.fillWidth: true
                visible: window.exportFormat === "jpeg"
                text: "JPEG has no alpha. Transparent pixels are explicitly composited over this opaque matte; alpha is never silently dropped."
                color: "#f0c674"
                wrapMode: Text.Wrap
                font.pixelSize: 11
            }
        }
    }
    ColorDialog {
        id: jpegMatteDialog
        title: "Choose opaque JPEG matte"
        selectedColor: window.exportMatte
        onAccepted: window.exportMatte = Qt.rgba(selectedColor.r, selectedColor.g,
                                                selectedColor.b, 1)
    }
    ColorDialog {
        id: brushColorDialog
        title: "Choose brush color"
        selectedColor: editor.brushColor
        onAccepted: editor.brushColor = selectedColor
    }
    ColorDialog {
        id: gradientStartDialog
        title: "Choose gradient start color"
        selectedColor: window.gradientStartColor
        onAccepted: window.gradientStartColor = selectedColor
    }
    ColorDialog {
        id: gradientEndDialog
        title: "Choose gradient end color"
        selectedColor: window.gradientEndColor
        onAccepted: window.gradientEndColor = selectedColor
    }

    Shortcut {
        sequences: [StandardKey.Undo]
        enabled: editor.canUndo
        onActivated: editor.undo()
    }
    Shortcut {
        sequences: [StandardKey.Redo]
        enabled: editor.canRedo
        onActivated: editor.redo()
    }
    Shortcut {
        sequences: [StandardKey.Open]
        onActivated: openProjectDialog.open()
    }
    Shortcut {
        sequences: [StandardKey.Save]
        onActivated: editor.currentFile.length > 0 ? editor.saveProject() : saveProjectDialog.open()
    }
    Shortcut {
        sequence: "Escape"
        onActivated: canvasPointer.cancelGesture()
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 52
            color: "#202328"
            border.color: "#30343a"
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 12
                anchors.rightMargin: 12
                spacing: 3
                Image {
                    source: "qrc:/icons/redrob.svg"
                    sourceSize: Qt.size(28, 28)
                    Layout.rightMargin: 7
                    Accessible.name: "Redrob Graphics"
                }
                Label {
                    text: "REDROB"
                    font.pixelSize: 14
                    font.weight: Font.DemiBold
                    color: "#f4f6fa"
                    Layout.rightMargin: 14
                }
                CommandButton {
                    objectName: "openProjectAction"
                    text: "Open Project"
                    ToolTip.text: "Open an editable RRG project (Ctrl+O)"
                    onClicked: openProjectDialog.open()
                }
                CommandButton {
                    objectName: "importFileAction"
                    text: "Import"
                    ToolTip.text: "Import PNG, JPEG, lossless WebP, ORA, or limited SVG"
                    onClicked: importDialog.open()
                }
                CommandButton {
                    objectName: "saveProjectAction"
                    text: "Save"
                    ToolTip.text: "Save the current RRG project (Ctrl+S)"
                    onClicked: editor.currentFile.length > 0 ? editor.saveProject() : saveProjectDialog.open()
                }
                CommandButton {
                    objectName: "saveProjectAsAction"
                    text: "Save As"
                    ToolTip.text: "Choose a new RRG project path"
                    onClicked: saveProjectDialog.open()
                }
                Rectangle {
                    Layout.preferredWidth: 1
                    Layout.preferredHeight: 24
                    color: "#3a3e45"
                }
                CommandButton {
                    text: "Undo"
                    enabled: editor.canUndo
                    ToolTip.text: "Undo"
                    onClicked: editor.undo()
                }
                CommandButton {
                    text: "Redo"
                    enabled: editor.canRedo
                    ToolTip.text: "Redo"
                    onClicked: editor.redo()
                }
                Item {
                    Layout.fillWidth: true
                }
                Label {
                    text: editor.documentWidth + " × " + editor.documentHeight
                    color: "#9da4ae"
                    font.pixelSize: 12
                }
                CommandButton {
                    objectName: "exportCurrentFrameAction"
                    text: "Export"
                    ToolTip.text: "Export the current frame with explicit loss and JPEG options"
                    onClicked: exportOptionsDialog.open()
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            Rectangle {
                Layout.preferredWidth: 58
                Layout.fillHeight: true
                color: "#1d2024"
                border.color: "#2c3036"
                ScrollView {
                    anchors.fill: parent
                    anchors.topMargin: 6
                    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
                    ColumnLayout {
                        width: 56
                        spacing: 3
                        ToolRailButton {
                            objectName: "brushToolAction"
                            text: "B"
                            toolId: "brush"
                            enabled: editor.activeNodeCanEditRaster
                            ToolTip.text: enabled ? "Brush" : "Brush requires a raster node"
                        }
                        ToolRailButton {
                            objectName: "fillToolAction"
                            text: "F"
                            toolId: "fill"
                            enabled: editor.activeNodeCanEditRaster
                            ToolTip.text: enabled ? "Fill active layer" : "Fill requires a raster node"
                        }
                        ToolRailButton {
                            text: "R"
                            toolId: "rectangle"
                            ToolTip.text: "Rectangle selection"
                        }
                        ToolRailButton {
                            text: "E"
                            toolId: "ellipse"
                            ToolTip.text: "Ellipse selection"
                        }
                        ToolRailButton {
                            objectName: "gradientToolAction"
                            text: "G"
                            toolId: "gradient"
                            enabled: editor.activeNodeCanEditRaster
                            ToolTip.text: enabled ? "Gradient" : "Gradient requires a raster node"
                        }
                        ToolRailButton {
                            text: "C"
                            toolId: "crop"
                            ToolTip.text: "Crop canvas"
                        }
                        ToolRailButton {
                            objectName: "transformToolAction"
                            text: "T"
                            toolId: "transform"
                            enabled: editor.activeNodeCanEditRaster
                            ToolTip.text: enabled ? "Translate active layer" : "Transform requires a raster node"
                        }
                        ToolRailButton {
                            text: "I"
                            toolId: "inspect"
                            ToolTip.text: "Inspect canvas"
                        }
                        Rectangle {
                            Layout.alignment: Qt.AlignHCenter
                            Layout.preferredWidth: 32
                            Layout.preferredHeight: 1
                            color: "#393d44"
                        }
                        ToolButton {
                            implicitWidth: 38
                            implicitHeight: 38
                            ToolTip.visible: hovered
                            ToolTip.text: "Brush color"
                            Accessible.name: "Brush color"
                            onClicked: brushColorDialog.open()
                            background: Rectangle {
                                anchors.centerIn: parent
                                width: 24
                                height: 24
                                radius: 6
                                color: editor.brushColor
                                border.color: "#777d86"
                            }
                        }
                        Label {
                            text: Math.round(editor.brushSize)
                            color: "#aeb4bd"
                            Layout.alignment: Qt.AlignHCenter
                            font.pixelSize: 10
                        }
                    }
                }
            }

            Rectangle {
                id: workspace
                Layout.fillWidth: true
                Layout.fillHeight: true
                color: "#111317"
                clip: true

                CanvasItem {
                    id: canvas
                    anchors.top: parent.top
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: timelinePanel.top
                    image: editor.renderImage
                    selectionMask: editor.selectionMask
                    selectionActive: editor.selectionActive
                    zoom: window.canvasZoom
                    Accessible.name: "Document canvas with selection overlay"

                    PointHandler {
                        id: canvasPointer
                        objectName: "canvasPointer"
                        target: null
                        acceptedButtons: Qt.LeftButton
                        property bool gestureActive: false
                        property point startCanvas: Qt.point(0, 0)
                        property point endCanvas: Qt.point(0, 0)

                        function boundedCanvasPoint(position) {
                            const raw = canvas.canvasPoint(position);
                            return Qt.point(Math.max(0, Math.min(editor.documentWidth, raw.x)), Math.max(0, Math.min(editor.documentHeight, raw.y)));
                        }
                        function normalizedPressure(measuredPressure, deviceType) {
                            const measured = Number(measuredPressure);
                            if (deviceType === PointerDevice.Mouse || deviceType === PointerDevice.TouchPad)
                                return 1;
                            if (isFinite(measured))
                                return Math.max(0, Math.min(1, measured));
                            return 1;
                        }
                        function pointPressure(handlerPoint) {
                            return normalizedPressure(handlerPoint.pressure, handlerPoint.device.deviceType);
                        }
                        function activeToolNeedsRaster() {
                            return window.activeTool === "brush" || window.activeTool === "fill"
                                || window.activeTool === "gradient" || window.activeTool === "transform";
                        }
                        function cancelGesture() {
                            if (gestureActive && window.activeTool === "brush")
                                editor.cancelStroke();
                            gestureActive = false;
                            canvas.clearPreview();
                        }
                        function commitGesture() {
                            if (!gestureActive)
                                return;
                            gestureActive = false;
                            const dx = endCanvas.x - startCanvas.x;
                            const dy = endCanvas.y - startCanvas.y;
                            if (window.activeTool === "brush") {
                                editor.endStroke();
                            } else if (window.activeTool === "fill") {
                                editor.fill(editor.brushColor);
                            } else if (window.activeTool === "rectangle") {
                                editor.selectRectangle(startCanvas.x, startCanvas.y, dx, dy, window.selectionMode);
                            } else if (window.activeTool === "ellipse") {
                                editor.selectEllipse(startCanvas.x, startCanvas.y, dx, dy, window.selectionMode);
                            } else if (window.activeTool === "gradient") {
                                if (window.gradientKind === "linear")
                                    editor.linearGradient(startCanvas.x, startCanvas.y, endCanvas.x, endCanvas.y, window.gradientStartColor, window.gradientEndColor);
                                else
                                    editor.radialGradient(startCanvas.x, startCanvas.y, Math.hypot(dx, dy), window.gradientStartColor, window.gradientEndColor);
                            } else if (window.activeTool === "crop") {
                                editor.cropCanvas(startCanvas.x, startCanvas.y, dx, dy);
                            } else if (window.activeTool === "transform") {
                                editor.transformActive(1, 0, 0, 1, dx, dy, window.samplingMode);
                            }
                            canvas.clearPreview();
                        }
                        onActiveChanged: {
                            if (active) {
                                const position = point.position;
                                if (!canvas.containsCanvasPoint(position) || window.activeTool === "inspect"
                                        || (activeToolNeedsRaster() && !editor.activeNodeCanEditRaster))
                                    return;
                                startCanvas = boundedCanvasPoint(position);
                                endCanvas = startCanvas;
                                gestureActive = true;
                                if (window.activeTool === "brush") {
                                    editor.beginStroke(startCanvas.x, startCanvas.y, pointPressure(point));
                                } else if (window.activeTool !== "fill") {
                                    canvas.previewStart = startCanvas;
                                    canvas.previewEnd = endCanvas;
                                    canvas.previewKind = window.activeTool === "rectangle" ? "rectangle" : window.activeTool === "ellipse" ? "ellipse" : window.activeTool === "gradient" ? window.gradientKind : window.activeTool;
                                    canvas.previewVisible = true;
                                }
                            } else {
                                commitGesture();
                            }
                        }
                        onPointChanged: {
                            if (!active || !gestureActive)
                                return;
                            const position = point.position;
                            endCanvas = boundedCanvasPoint(position);
                            if (window.activeTool === "brush") {
                                if (canvas.containsCanvasPoint(position))
                                    editor.addStrokePoint(endCanvas.x, endCanvas.y, pointPressure(point));
                            } else {
                                canvas.previewEnd = endCanvas;
                            }
                        }
                        onCanceled: point => cancelGesture()
                    }

                    HoverHandler {
                        cursorShape: window.activeTool === "inspect" ? Qt.ArrowCursor : Qt.CrossCursor
                    }

                    WheelHandler {
                        target: null
                        acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
                        onWheel: event => {
                            const factor = event.angleDelta.y > 0 ? 1.12 : 1 / 1.12;
                            window.canvasZoom = Math.max(0.05, Math.min(32, window.canvasZoom * factor));
                            event.accepted = true;
                        }
                    }
                }

                Rectangle {
                    anchors.right: parent.right
                    anchors.bottom: timelinePanel.top
                    anchors.margins: 14
                    width: zoomRow.implicitWidth + 16
                    height: 40
                    radius: 9
                    color: "#d922252a"
                    border.color: "#3a3f47"
                    RowLayout {
                        id: zoomRow
                        anchors.centerIn: parent
                        spacing: 2
                        CommandButton {
                            text: "−"
                            ToolTip.text: "Zoom out"
                            onClicked: window.canvasZoom = Math.max(0.05, window.canvasZoom / 1.2)
                        }
                        Label {
                            text: Math.round(window.canvasZoom * 100) + "%"
                            color: "#d8dce2"
                            Layout.preferredWidth: 50
                            horizontalAlignment: Text.AlignHCenter
                        }
                        CommandButton {
                            text: "+"
                            ToolTip.text: "Zoom in"
                            onClicked: window.canvasZoom = Math.min(32, window.canvasZoom * 1.2)
                        }
                        CommandButton {
                            text: "1:1"
                            ToolTip.text: "Actual pixels"
                            onClicked: window.canvasZoom = 1
                        }
                    }
                }

                Rectangle {
                    id: timelinePanel
                    objectName: "timelinePanel"
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 140
                    color: "#1a1d22"
                    border.color: "#30353d"

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 8
                        spacing: 6

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 5
                            CommandButton {
                                objectName: "previousFrameAction"
                                text: "◀"
                                enabled: editor.currentFrameIndex > 0
                                ToolTip.text: "Previous frame"
                                onClicked: editor.setCurrentFrame(editor.frames.frameIdAt(editor.currentFrameIndex - 1))
                            }
                            CommandButton {
                                objectName: "playFrameAction"
                                text: editor.playing ? "■" : "▶"
                                ToolTip.text: editor.playing ? "Stop playback" : "Play range"
                                onClicked: editor.setPlaying(!editor.playing)
                            }
                            CommandButton {
                                objectName: "nextFrameAction"
                                text: "▶|"
                                enabled: editor.currentFrameIndex + 1 < editor.frameCount
                                ToolTip.text: "Next frame"
                                onClicked: editor.setCurrentFrame(editor.frames.frameIdAt(editor.currentFrameIndex + 1))
                            }
                            Rectangle { width: 1; height: 24; color: "#3a3f47" }
                            CommandButton {
                                objectName: "addFrameAction"
                                text: "+ Frame"
                                ToolTip.text: "Add blank sparse frame"
                                onClicked: editor.addFrame(editor.currentFrameIndex + 1)
                            }
                            CommandButton {
                                objectName: "duplicateFrameAction"
                                text: "Duplicate"
                                ToolTip.text: "Duplicate current frame cels"
                                onClicked: editor.duplicateFrame(editor.currentFrame, editor.currentFrameIndex + 1)
                            }
                            CommandButton {
                                objectName: "deleteFrameAction"
                                text: "Delete"
                                enabled: editor.frameCount > 1
                                ToolTip.text: "Delete current frame"
                                onClicked: editor.removeFrame(editor.currentFrame)
                            }
                            CommandButton {
                                objectName: "moveFrameLeftAction"
                                text: "←"
                                enabled: editor.currentFrameIndex > 0
                                ToolTip.text: "Move current frame left"
                                onClicked: editor.moveFrame(editor.currentFrame, editor.currentFrameIndex - 1)
                            }
                            CommandButton {
                                objectName: "moveFrameRightAction"
                                text: "→"
                                enabled: editor.currentFrameIndex + 1 < editor.frameCount
                                ToolTip.text: "Move current frame right"
                                onClicked: editor.moveFrame(editor.currentFrame, editor.currentFrameIndex + 1)
                            }
                            Item { Layout.fillWidth: true }
                            Label { text: "FPS"; color: "#9ca4af" }
                            SpinBox {
                                objectName: "timelineFpsControl"
                                from: 1
                                to: 240
                                value: Math.round(editor.fps)
                                editable: true
                                onValueModified: editor.setTimelineFps(value)
                                Accessible.name: "Timeline frames per second"
                            }
                            CheckBox {
                                objectName: "timelineLoopControl"
                                text: "Loop"
                                checked: editor.looping
                                onToggled: if (checked !== editor.looping) editor.setLooping(checked)
                            }
                            Label { text: "Range"; color: "#9ca4af" }
                            ComboBox {
                                id: rangeStartControl
                                objectName: "rangeStartControl"
                                model: editor.frames
                                textRole: "index"
                                valueRole: "frameId"
                                currentIndex: editor.frames.indexOf(editor.rangeStart)
                                onActivated: if (currentIndex <= rangeEndControl.currentIndex)
                                    editor.setPlaybackRange(currentValue, rangeEndControl.currentValue)
                            }
                            Label { text: "–"; color: "#9ca4af" }
                            ComboBox {
                                id: rangeEndControl
                                objectName: "rangeEndControl"
                                model: editor.frames
                                textRole: "index"
                                valueRole: "frameId"
                                currentIndex: editor.frames.indexOf(editor.rangeEnd)
                                onActivated: if (currentIndex >= rangeStartControl.currentIndex)
                                    editor.setPlaybackRange(rangeStartControl.currentValue, currentValue)
                            }
                        }

                        ListView {
                            id: timelineList
                            objectName: "timelineFrameList"
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            orientation: ListView.Horizontal
                            spacing: 6
                            clip: true
                            model: editor.frames
                            delegate: Rectangle {
                                required property var frameId
                                required property int index
                                required property int duration
                                required property bool current
                                required property bool inRange
                                required property bool isFirst
                                required property bool isLast
                                width: 82
                                height: timelineList.height
                                radius: 7
                                color: current ? "#40547d" : inRange ? "#2b3039" : "#22252b"
                                border.width: current ? 2 : 1
                                border.color: current ? "#86a7ff" : inRange ? "#495363" : "#343941"
                                TapHandler { onTapped: editor.setCurrentFrame(frameId) }
                                Column {
                                    anchors.centerIn: parent
                                    spacing: 2
                                    Label {
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        text: "Frame " + (index + 1)
                                        color: "#e3e7ed"
                                        font.weight: current ? Font.DemiBold : Font.Normal
                                    }
                                    Label {
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        text: duration + " ms"
                                        color: "#9099a6"
                                        font.pixelSize: 10
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Rectangle {
                id: inspector
                Layout.preferredWidth: Math.min(370, Math.max(270, window.width * 0.25))
                Layout.fillHeight: true
                color: "#1d2024"
                border.color: "#30343a"
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0
                    TabBar {
                        id: tabs
                        Layout.fillWidth: true
                        TabButton {
                            text: "Layers"
                            Accessible.name: "Layers inspector"
                        }
                        TabButton {
                            text: "Options"
                            Accessible.name: "Tool and operation options"
                        }
                        TabButton {
                            text: "Agent"
                            Accessible.name: "Agent proposals"
                        }
                    }
                    StackLayout {
                        currentIndex: tabs.currentIndex
                        Layout.fillWidth: true
                        Layout.fillHeight: true

                        Item {
                            Dialog {
                                id: textSemanticDialog
                                objectName: "textSemanticEditor"
                                property string nodeId: ""
                                property string sourceFontId: "font8x8-basic-0.3.1"
                                property string sourceFontFamily: "font8x8 Basic Latin"
                                title: nodeId.length > 0 ? "Edit deterministic text" : "Add deterministic text"
                                modal: true
                                anchors.centerIn: parent
                                standardButtons: Dialog.Ok | Dialog.Cancel
                                onAccepted: {
                                    if (nodeId.length > 0)
                                        editor.setTextContent(nodeId, semanticText.text, Number(textX.text),
                                                              Number(textY.text), Number(textSize.text), textColor.text,
                                                              sourceFontFamily, sourceFontId)
                                    else
                                        editor.addTextNode(textName.text, semanticText.text, Number(textX.text),
                                                           Number(textY.text), Number(textSize.text), textColor.text)
                                }
                                ColumnLayout {
                                    width: 360
                                    Label { text: "Printable ASCII + newline only · embedded font8x8"; wrapMode: Text.Wrap }
                                    TextField {
                                        id: textName
                                        Layout.fillWidth: true
                                        placeholderText: "Node name"
                                        visible: textSemanticDialog.nodeId.length === 0
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
                                    Label {
                                        text: "Font: " + textSemanticDialog.sourceFontFamily + " (" + textSemanticDialog.sourceFontId + ")"
                                        wrapMode: Text.Wrap
                                    }
                                }
                            }
                            Dialog {
                                id: vectorSemanticDialog
                                objectName: "vectorRectangleEditor"
                                property string nodeId: ""
                                title: nodeId.length > 0 ? "Edit recognized vector rectangle" : "Add vector rectangle"
                                modal: true
                                anchors.centerIn: parent
                                standardButtons: Dialog.Ok | Dialog.Cancel
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
                                        visible: vectorSemanticDialog.nodeId.length === 0
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
                            Dialog {
                                id: rasterizeSemanticWarning
                                objectName: "rasterizeSemanticWarning"
                                property string nodeId: ""
                                title: "Rasterize semantic node?"
                                modal: true
                                anchors.centerIn: parent
                                standardButtons: Dialog.Ok | Dialog.Cancel
                                onAccepted: editor.rasterizeSemanticNode(nodeId)
                                Label {
                                    width: 360
                                    wrapMode: Text.Wrap
                                    text: "This replaces editable text/vector content with a raster cel on the current frame only. Other frames are blank. Node ID, name, hierarchy, visibility, opacity, and blend are preserved."
                                }
                            }
                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 10
                                spacing: 8
                                Menu {
                                    id: addNodeMenu
                                    MenuItem {
                                        text: "Add raster layer"
                                        Accessible.name: "Add raster layer at document root"
                                        onTriggered: editor.addLayer()
                                    }
                                    MenuItem {
                                        text: "Add group"
                                        Accessible.name: "Add layer group at document root"
                                        onTriggered: editor.addGroup()
                                    }
                                    MenuItem {
                                        objectName: "addTextNodeAction"
                                        text: "Add text"
                                        Accessible.name: "Add deterministic text node"
                                        onTriggered: {
                                            textSemanticDialog.nodeId = ""
                                            textName.text = "New text"
                                            semanticText.text = "Text"
                                            textX.text = "24"
                                            textY.text = "24"
                                            textSize.text = "32"
                                            textColor.text = editor.brushColor.toString()
                                            textSemanticDialog.sourceFontId = "font8x8-basic-0.3.1"
                                            textSemanticDialog.sourceFontFamily = "font8x8 Basic Latin"
                                            textSemanticDialog.open()
                                        }
                                    }
                                    MenuItem {
                                        objectName: "addVectorNodeAction"
                                        text: "Add vector rectangle"
                                        Accessible.name: "Add deterministic vector rectangle"
                                        onTriggered: {
                                            vectorSemanticDialog.nodeId = ""
                                            vectorName.text = "New vector"
                                            vectorX.text = "48"
                                            vectorY.text = "48"
                                            vectorW.text = "180"
                                            vectorH.text = "120"
                                            vectorStroke.text = "2"
                                            vectorFill.text = editor.brushColor.toString()
                                            vectorStrokeColor.text = "#ff20242a"
                                            vectorSemanticDialog.open()
                                        }
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "LAYER STACK"
                                        color: "#9199a4"
                                        font.pixelSize: 11
                                        font.weight: Font.DemiBold
                                    }
                                    Item {
                                        Layout.fillWidth: true
                                    }
                                    CommandButton {
                                        text: "+"
                                        ToolTip.text: "Add raster, group, text, or vector node"
                                        onClicked: addNodeMenu.open()
                                    }
                                    CommandButton {
                                        text: "−"
                                        enabled: layerList.count > 1
                                        ToolTip.text: "Delete active layer"
                                        onClicked: editor.deleteLayer(editor.activeLayerId)
                                    }
                                }
                                ListView {
                                    id: layerList
                                    Layout.fillWidth: true
                                    Layout.fillHeight: true
                                    spacing: 5
                                    clip: true
                                    model: editor.layers
                                    delegate: Rectangle {
                                        required property string layerId
                                        required property string layerName
                                        required property bool layerVisible
                                        required property real layerOpacity
                                        required property string blendMode
                                        required property bool activeLayer
                                        required property string nodeKind
                                        required property string parentId
                                        required property int nodeDepth
                                        required property int siblingIndex
                                        required property int siblingCount
                                        required property bool isTopSibling
                                        required property bool isBottomSibling
                                        required property bool hasChildren
                                        required property bool hasMask
                                        required property bool maskEnabled
                                        required property bool canEditRaster
                                        required property bool canEditText
                                        required property bool canEditVector
                                        required property bool canRasterize
                                        required property bool isSemantic
                                        required property string semanticPreview
                                        required property string semanticTextSource
                                        required property bool semanticPreviewTruncated
                                        required property int semanticPathCount
                                        required property int semanticCommandCount
                                        required property string semanticFontId
                                        required property string semanticFontFamily
                                        required property real semanticFontSize
                                        required property real semanticOriginX
                                        required property real semanticOriginY
                                        required property color semanticColor
                                        required property bool semanticRectangleRecognized
                                        required property real semanticRectangleX
                                        required property real semanticRectangleY
                                        required property real semanticRectangleWidth
                                        required property real semanticRectangleHeight
                                        required property color semanticRectangleFill
                                        required property color semanticRectangleStroke
                                        required property real semanticRectangleStrokeWidth
                                        width: layerList.width
                                        height: 74
                                        radius: 8
                                        color: activeLayer ? "#313b50" : "#25282d"
                                        border.color: activeLayer ? "#607fdc" : "#33373e"
                                        Menu {
                                            id: layerActions
                                            MenuItem {
                                                text: "Move to document root"
                                                enabled: parentId.length > 0
                                                Accessible.name: "Move " + layerName + " to document root"
                                                onTriggered: editor.moveNode(layerId, "", 0)
                                            }
                                            MenuItem {
                                                text: "Move active node into this group"
                                                enabled: nodeKind === "group" && !activeLayer
                                                Accessible.name: "Move active node into group " + layerName
                                                onTriggered: editor.moveNode(editor.activeLayerId, layerId, 0)
                                            }
                                            MenuItem {
                                                objectName: "moveTowardTopAction-" + layerId
                                                text: "Move toward top"
                                                enabled: !isTopSibling
                                                Accessible.name: "Move " + layerName + " toward top among siblings"
                                                onTriggered: editor.moveNode(layerId, parentId, siblingIndex + 1)
                                            }
                                            MenuItem {
                                                objectName: "moveTowardBottomAction-" + layerId
                                                text: "Move toward bottom"
                                                enabled: !isBottomSibling
                                                Accessible.name: "Move " + layerName + " toward bottom among siblings"
                                                onTriggered: editor.moveNode(layerId, parentId, siblingIndex - 1)
                                            }
                                            MenuSeparator {}
                                            MenuItem {
                                                objectName: "editSemanticTextAction-" + layerId
                                                text: "Edit text content"
                                                enabled: canEditText
                                                Accessible.name: "Edit bounded text content for " + layerName
                                                onTriggered: {
                                                    textSemanticDialog.nodeId = layerId
                                                    semanticText.text = semanticTextSource
                                                    textX.text = String(semanticOriginX)
                                                    textY.text = String(semanticOriginY)
                                                    textSize.text = String(semanticFontSize)
                                                    textColor.text = semanticColor.toString()
                                                    textSemanticDialog.sourceFontId = semanticFontId
                                                    textSemanticDialog.sourceFontFamily = semanticFontFamily
                                                    textSemanticDialog.open()
                                                }
                                            }
                                            MenuItem {
                                                objectName: "editSemanticVectorAction-" + layerId
                                                text: semanticRectangleRecognized ? "Edit vector rectangle" : "Edit unavailable (not a rectangle)"
                                                enabled: canEditVector && semanticRectangleRecognized
                                                Accessible.name: semanticRectangleRecognized
                                                    ? "Edit recognized rectangle " + layerName
                                                    : "Arbitrary vectors cannot be edited as rectangles"
                                                onTriggered: {
                                                    vectorSemanticDialog.nodeId = layerId
                                                    vectorX.text = String(semanticRectangleX)
                                                    vectorY.text = String(semanticRectangleY)
                                                    vectorW.text = String(semanticRectangleWidth)
                                                    vectorH.text = String(semanticRectangleHeight)
                                                    vectorFill.text = semanticRectangleFill.toString()
                                                    vectorStrokeColor.text = semanticRectangleStroke.toString()
                                                    vectorStroke.text = String(semanticRectangleStrokeWidth)
                                                    vectorSemanticDialog.open()
                                                }
                                            }
                                            MenuItem {
                                                objectName: "rasterizeSemanticAction-" + layerId
                                                text: "Rasterize on current frame…"
                                                enabled: canRasterize
                                                Accessible.name: "Rasterize semantic node " + layerName + " on current frame only"
                                                onTriggered: {
                                                    rasterizeSemanticWarning.nodeId = layerId
                                                    rasterizeSemanticWarning.open()
                                                }
                                            }
                                            MenuSeparator {}
                                            MenuItem {
                                                text: "Add raster mask"
                                                enabled: !hasMask && (nodeKind === "raster" || nodeKind === "group")
                                                Accessible.name: "Add raster mask to " + layerName
                                                onTriggered: editor.addRasterMask(layerId)
                                            }
                                            MenuItem {
                                                text: "Mask from selection"
                                                enabled: editor.selectionActive && (nodeKind === "raster" || nodeKind === "group")
                                                Accessible.name: "Copy selection into raster mask for " + layerName
                                                onTriggered: window.maskNodeFromSelection(layerId)
                                            }
                                            MenuItem {
                                                text: maskEnabled ? "Disable raster mask" : "Enable raster mask"
                                                enabled: hasMask
                                                Accessible.name: text + " for " + layerName
                                                onTriggered: editor.setRasterMaskEnabled(layerId, !maskEnabled)
                                            }
                                            MenuItem {
                                                text: "Remove raster mask"
                                                enabled: hasMask
                                                Accessible.name: "Remove raster mask from " + layerName
                                                onTriggered: editor.removeRasterMask(layerId)
                                            }
                                        }
                                        TapHandler {
                                            acceptedButtons: Qt.LeftButton
                                            onTapped: editor.setActiveLayer(layerId)
                                        }
                                        TapHandler {
                                            acceptedButtons: Qt.RightButton
                                            onTapped: layerActions.open()
                                        }
                                        ColumnLayout {
                                            anchors.fill: parent
                                            anchors.margins: 8
                                            RowLayout {
                                                Layout.fillWidth: true
                                                Layout.leftMargin: nodeDepth * 14
                                                Label {
                                                    visible: nodeKind === "group"
                                                    text: hasChildren ? "▾" : "▹"
                                                    color: "#aeb6c2"
                                                    Accessible.name: hasChildren ? "Expanded group" : "Empty group"
                                                }
                                                CheckBox {
                                                    checked: layerVisible
                                                    Accessible.name: "Toggle visibility for " + layerName
                                                    onToggled: editor.setLayerVisibility(layerId, checked)
                                                }
                                                TextField {
                                                    Layout.fillWidth: true
                                                    text: layerName
                                                    selectByMouse: true
                                                    Accessible.name: nodeKind + " node name"
                                                    onEditingFinished: if (text !== layerName)
                                                        editor.renameLayer(layerId, text)
                                                }
                                                Label {
                                                    visible: hasMask
                                                    text: maskEnabled ? "MASK" : "MASK OFF"
                                                    color: maskEnabled ? "#8fb5ff" : "#777f8b"
                                                    font.pixelSize: 9
                                                    Accessible.name: maskEnabled ? "Raster mask enabled" : "Raster mask disabled"
                                                }
                                                Label {
                                                    text: nodeKind === "group" ? "group"
                                                          : nodeKind === "text" ? (semanticPreviewTruncated ? "text · 256+ chars" : "text · " + semanticPreview.length + " chars")
                                                          : nodeKind === "vector" ? "vector · " + semanticPathCount + " paths / " + semanticCommandCount + " commands"
                                                          : blendMode
                                                    color: "#8f97a2"
                                                    font.pixelSize: 10
                                                }
                                            }
                                            Slider {
                                                Layout.fillWidth: true
                                                Layout.leftMargin: nodeDepth * 14
                                                from: 0
                                                to: 1
                                                value: layerOpacity
                                                Accessible.name: "Opacity for " + layerName
                                                onPressedChanged: if (!pressed && Math.abs(value - layerOpacity) > 0.001)
                                                    editor.setLayerOpacity(layerId, value)
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        ScrollView {
                            id: optionsScroll
                            clip: true
                            ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
                            ColumnLayout {
                                width: Math.max(240, optionsScroll.availableWidth - 14)
                                x: 7
                                spacing: 6

                                SectionTitle {
                                    text: "BRUSH"
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Size"
                                        Layout.preferredWidth: 72
                                    }
                                    Slider {
                                        Layout.fillWidth: true
                                        from: 1
                                        to: 1000
                                        value: editor.brushSize
                                        Accessible.name: "Brush size 1 to 1000"
                                        onMoved: editor.brushSize = value
                                    }
                                    Label {
                                        text: Math.round(editor.brushSize)
                                        Layout.preferredWidth: 36
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Opacity"
                                        Layout.preferredWidth: 72
                                    }
                                    Slider {
                                        Layout.fillWidth: true
                                        from: 0
                                        to: 1
                                        value: editor.brushOpacity
                                        Accessible.name: "Brush opacity"
                                        onMoved: editor.brushOpacity = value
                                    }
                                    Label {
                                        text: Math.round(editor.brushOpacity * 100) + "%"
                                        Layout.preferredWidth: 40
                                    }
                                }
                                ComboBox {
                                    Layout.fillWidth: true
                                    model: ["none", "moving_average"]
                                    currentIndex: editor.brushSmoothingKind === "moving_average" ? 1 : 0
                                    Accessible.name: "Brush smoothing kind"
                                    onActivated: editor.brushSmoothingKind = currentText
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Window"
                                    }
                                    SpinBox {
                                        Layout.fillWidth: true
                                        from: 2
                                        to: 64
                                        value: editor.brushSmoothingWindow
                                        enabled: editor.brushSmoothingKind === "moving_average"
                                        Accessible.name: "Moving average smoothing window"
                                        onValueModified: editor.brushSmoothingWindow = value
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    CheckBox {
                                        text: "Mirror X"
                                        checked: editor.mirrorXEnabled
                                        onToggled: editor.mirrorXEnabled = checked
                                        Accessible.name: "Mirror brush across X axis"
                                    }
                                    SpinBox {
                                        Layout.fillWidth: true
                                        from: 0
                                        to: Math.max(1, editor.documentWidth)
                                        value: Math.round(editor.mirrorXAxis)
                                        onValueModified: editor.mirrorXAxis = value
                                        Accessible.name: "Mirror X axis position"
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    CheckBox {
                                        text: "Mirror Y"
                                        checked: editor.mirrorYEnabled
                                        onToggled: editor.mirrorYEnabled = checked
                                        Accessible.name: "Mirror brush across Y axis"
                                    }
                                    SpinBox {
                                        Layout.fillWidth: true
                                        from: 0
                                        to: Math.max(1, editor.documentHeight)
                                        value: Math.round(editor.mirrorYAxis)
                                        onValueModified: editor.mirrorYAxis = value
                                        Accessible.name: "Mirror Y axis position"
                                    }
                                }

                                SectionTitle {
                                    text: "SELECTION"
                                }
                                ComboBox {
                                    Layout.fillWidth: true
                                    model: ["replace", "add", "subtract", "intersect"]
                                    currentIndex: model.indexOf(window.selectionMode)
                                    Accessible.name: "Selection combination mode"
                                    onActivated: window.selectionMode = currentText
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Button {
                                        text: "All"
                                        Layout.fillWidth: true
                                        onClicked: editor.selectAll()
                                        Accessible.name: "Select all"
                                    }
                                    Button {
                                        text: "Invert"
                                        Layout.fillWidth: true
                                        onClicked: editor.invertSelection()
                                        Accessible.name: "Invert selection"
                                    }
                                    Button {
                                        text: "Clear"
                                        Layout.fillWidth: true
                                        onClicked: editor.clearSelection()
                                        Accessible.name: "Clear selection"
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    SpinBox {
                                        id: morphologyRadius
                                        from: 0
                                        to: 4096
                                        value: 4
                                        Layout.fillWidth: true
                                        Accessible.name: "Selection morphology radius"
                                    }
                                    Button {
                                        text: "Feather"
                                        onClicked: editor.featherSelection(morphologyRadius.value)
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Button {
                                        text: "Grow"
                                        Layout.fillWidth: true
                                        onClicked: editor.growSelection(morphologyRadius.value)
                                    }
                                    Button {
                                        text: "Shrink"
                                        Layout.fillWidth: true
                                        onClicked: editor.shrinkSelection(morphologyRadius.value)
                                    }
                                }

                                SectionTitle {
                                    text: "GRADIENT"
                                }
                                ComboBox {
                                    Layout.fillWidth: true
                                    model: ["linear", "radial"]
                                    currentIndex: window.gradientKind === "radial" ? 1 : 0
                                    Accessible.name: "Gradient kind"
                                    onActivated: window.gradientKind = currentText
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Button {
                                        text: "Start"
                                        Layout.fillWidth: true
                                        onClicked: gradientStartDialog.open()
                                        Accessible.name: "Choose gradient start color"
                                        background: Rectangle {
                                            radius: 5
                                            color: window.gradientStartColor
                                            border.color: "#89909a"
                                        }
                                    }
                                    Button {
                                        text: "End"
                                        Layout.fillWidth: true
                                        onClicked: gradientEndDialog.open()
                                        Accessible.name: "Choose gradient end color"
                                        background: Rectangle {
                                            radius: 5
                                            color: window.gradientEndColor
                                            border.color: "#89909a"
                                        }
                                    }
                                }

                                SectionTitle {
                                    text: "CANVAS / TRANSFORM"
                                }
                                ComboBox {
                                    id: samplingCombo
                                    Layout.fillWidth: true
                                    model: ["nearest", "bilinear"]
                                    currentIndex: window.samplingMode === "bilinear" ? 1 : 0
                                    Accessible.name: "Transform sampling mode"
                                    onActivated: window.samplingMode = currentText
                                }
                                GridLayout {
                                    columns: 4
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Crop"
                                    }
                                    NumericField {
                                        id: cropX
                                        text: "0"
                                        placeholderText: "Crop X"
                                    }
                                    NumericField {
                                        id: cropY
                                        text: "0"
                                        placeholderText: "Crop Y"
                                    }
                                    Button {
                                        text: "Apply"
                                        onClicked: editor.cropCanvas(Number(cropX.text), Number(cropY.text), Number(cropW.text), Number(cropH.text))
                                        Accessible.name: "Apply numeric crop"
                                    }
                                    Label {
                                        text: "W × H"
                                    }
                                    NumericField {
                                        id: cropW
                                        text: String(editor.documentWidth)
                                        placeholderText: "Crop width"
                                    }
                                    NumericField {
                                        id: cropH
                                        text: String(editor.documentHeight)
                                        placeholderText: "Crop height"
                                    }
                                    Item {
                                        Layout.preferredWidth: 1
                                        Layout.preferredHeight: 1
                                    }
                                    Label {
                                        text: "Pad L/T/R/B"
                                    }
                                    NumericField {
                                        id: padLeft
                                        text: "0"
                                        placeholderText: "Pad left"
                                    }
                                    NumericField {
                                        id: padTop
                                        text: "0"
                                        placeholderText: "Pad top"
                                    }
                                    Button {
                                        text: "Apply"
                                        onClicked: editor.padCanvas(Number(padLeft.text), Number(padTop.text), Number(padRight.text), Number(padBottom.text))
                                        Accessible.name: "Pad canvas"
                                    }
                                    Item {
                                        Layout.preferredWidth: 1
                                        Layout.preferredHeight: 1
                                    }
                                    NumericField {
                                        id: padRight
                                        text: "0"
                                        placeholderText: "Pad right"
                                    }
                                    NumericField {
                                        id: padBottom
                                        text: "0"
                                        placeholderText: "Pad bottom"
                                    }
                                    Item {
                                        Layout.preferredWidth: 1
                                        Layout.preferredHeight: 1
                                    }
                                    Label {
                                        text: "Resize"
                                    }
                                    NumericField {
                                        id: resizeW
                                        text: String(editor.documentWidth)
                                        placeholderText: "Resize width"
                                    }
                                    NumericField {
                                        id: resizeH
                                        text: String(editor.documentHeight)
                                        placeholderText: "Resize height"
                                    }
                                    Button {
                                        text: "Apply"
                                        onClicked: editor.resizeCanvas(Number(resizeW.text), Number(resizeH.text), window.samplingMode)
                                        Accessible.name: "Resize canvas"
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Button {
                                        objectName: "flipHorizontalAction"
                                        text: "Flip H"
                                        enabled: editor.activeNodeCanEditRaster
                                        Layout.fillWidth: true
                                        onClicked: editor.flipActive(true, false)
                                    }
                                    Button {
                                        objectName: "flipVerticalAction"
                                        text: "Flip V"
                                        enabled: editor.activeNodeCanEditRaster
                                        Layout.fillWidth: true
                                        onClicked: editor.flipActive(false, true)
                                    }
                                    Button {
                                        objectName: "rotateCounterclockwiseAction"
                                        text: "↶ 90"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.rotateActive90(false)
                                        Accessible.name: "Rotate counterclockwise 90 degrees"
                                    }
                                    Button {
                                        objectName: "rotateClockwiseAction"
                                        text: "↷ 90"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.rotateActive90(true)
                                        Accessible.name: "Rotate clockwise 90 degrees"
                                    }
                                }
                                GridLayout {
                                    columns: 4
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Affine"
                                    }
                                    NumericField {
                                        id: m11
                                        text: "1"
                                        placeholderText: "m11"
                                    }
                                    NumericField {
                                        id: m12
                                        text: "0"
                                        placeholderText: "m12"
                                    }
                                    NumericField {
                                        id: tx
                                        text: "0"
                                        placeholderText: "translate X"
                                    }
                                    Item {
                                        Layout.preferredWidth: 1
                                        Layout.preferredHeight: 1
                                    }
                                    NumericField {
                                        id: m21
                                        text: "0"
                                        placeholderText: "m21"
                                    }
                                    NumericField {
                                        id: m22
                                        text: "1"
                                        placeholderText: "m22"
                                    }
                                    NumericField {
                                        id: ty
                                        text: "0"
                                        placeholderText: "translate Y"
                                    }
                                }
                                Button {
                                    objectName: "affineTransformAction"
                                    Layout.fillWidth: true
                                    enabled: editor.activeNodeCanEditRaster
                                    text: "Apply affine transform"
                                    Accessible.name: "Apply affine transform to active layer"
                                    onClicked: editor.transformActive(Number(m11.text), Number(m12.text), Number(m21.text), Number(m22.text), Number(tx.text), Number(ty.text), window.samplingMode)
                                }

                                SectionTitle {
                                    text: "FILTERS"
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Button {
                                        objectName: "invertFilterAction"
                                        text: "Invert"
                                        enabled: editor.activeNodeCanEditRaster
                                        Layout.fillWidth: true
                                        onClicked: editor.applyFilter("invert")
                                    }
                                    Button {
                                        objectName: "grayscaleFilterAction"
                                        text: "Grayscale"
                                        enabled: editor.activeNodeCanEditRaster
                                        Layout.fillWidth: true
                                        onClicked: editor.applyFilter("grayscale")
                                    }
                                    Button {
                                        objectName: "clearLayerAction"
                                        text: "Clear layer"
                                        enabled: editor.activeNodeCanEditRaster
                                        Layout.fillWidth: true
                                        onClicked: editor.clearActiveLayer()
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Brightness"
                                    }
                                    SpinBox {
                                        id: brightness
                                        from: -255
                                        to: 255
                                        value: 0
                                        Layout.fillWidth: true
                                        Accessible.name: "Brightness minus 255 to 255"
                                    }
                                    Label {
                                        text: "Contrast"
                                    }
                                    SpinBox {
                                        id: contrast
                                        from: -100
                                        to: 100
                                        value: 0
                                        Layout.fillWidth: true
                                        Accessible.name: "Contrast minus 100 to 100"
                                    }
                                }
                                Button {
                                    objectName: "brightnessContrastAction"
                                    Layout.fillWidth: true
                                    text: "Apply brightness / contrast"
                                    enabled: editor.activeNodeCanEditRaster
                                    onClicked: editor.applyBrightnessContrast(brightness.value, contrast.value)
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Gaussian σ"
                                    }
                                    NumericField {
                                        id: sigma
                                        text: "4"
                                        placeholderText: "Gaussian sigma 0–1024"
                                    }
                                    Button {
                                        objectName: "gaussianBlurAction"
                                        text: "Apply"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyGaussianBlur(Number(sigma.text))
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Threshold"
                                    }
                                    SpinBox {
                                        id: threshold
                                        from: 0
                                        to: 255
                                        value: 128
                                        Layout.fillWidth: true
                                        Accessible.name: "Threshold 0 to 255"
                                    }
                                    Button {
                                        objectName: "thresholdAction"
                                        text: "Apply"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyThreshold(threshold.value)
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Posterize"
                                    }
                                    SpinBox {
                                        id: posterize
                                        from: 2
                                        to: 256
                                        value: 8
                                        Layout.fillWidth: true
                                        Accessible.name: "Posterize levels 2 to 256"
                                    }
                                    Button {
                                        objectName: "posterizeAction"
                                        text: "Apply"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyPosterize(posterize.value)
                                    }
                                }
                                GridLayout {
                                    columns: 4
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Levels in"
                                    }
                                    SpinBox {
                                        id: inputBlack
                                        from: 0
                                        to: 255
                                        value: 0
                                    }
                                    SpinBox {
                                        id: inputWhite
                                        from: 0
                                        to: 255
                                        value: 255
                                    }
                                    NumericField {
                                        id: gamma
                                        text: "1"
                                        placeholderText: "Gamma 0.01–100"
                                    }
                                    Label {
                                        text: "Levels out"
                                    }
                                    SpinBox {
                                        id: outputBlack
                                        from: 0
                                        to: 255
                                        value: 0
                                    }
                                    SpinBox {
                                        id: outputWhite
                                        from: 0
                                        to: 255
                                        value: 255
                                    }
                                    Button {
                                        objectName: "levelsAction"
                                        text: "Apply"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyLevels(inputBlack.value, inputWhite.value, Number(gamma.text), outputBlack.value, outputWhite.value)
                                    }
                                }
                                GridLayout {
                                    columns: 4
                                    Layout.fillWidth: true
                                    Label {
                                        text: "H/S/L"
                                    }
                                    SpinBox {
                                        id: hue
                                        from: -180
                                        to: 180
                                        value: 0
                                        Accessible.name: "Hue degrees"
                                    }
                                    SpinBox {
                                        id: saturation
                                        from: -100
                                        to: 100
                                        value: 0
                                        Accessible.name: "Saturation"
                                    }
                                    SpinBox {
                                        id: lightness
                                        from: -100
                                        to: 100
                                        value: 0
                                        Accessible.name: "Lightness"
                                    }
                                }
                                Button {
                                    objectName: "hueSaturationAction"
                                    Layout.fillWidth: true
                                    text: "Apply hue / saturation / lightness"
                                    enabled: editor.activeNodeCanEditRaster
                                    onClicked: editor.applyHueSaturation(hue.value, saturation.value, lightness.value)
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Box blur"
                                    }
                                    SpinBox {
                                        id: boxRadius
                                        from: 1
                                        to: 4096
                                        value: 3
                                        Layout.fillWidth: true
                                        Accessible.name: "Box blur radius 1 to 4096"
                                    }
                                    Button {
                                        objectName: "boxBlurAction"
                                        text: "Apply"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applyBoxBlur(boxRadius.value)
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: "Sharpen"
                                    }
                                    NumericField {
                                        id: sharpenAmount
                                        text: "1"
                                        placeholderText: "Sharpen amount 0–10"
                                    }
                                    Button {
                                        objectName: "sharpenAction"
                                        text: "Apply"
                                        enabled: editor.activeNodeCanEditRaster
                                        onClicked: editor.applySharpen(Number(sharpenAmount.text))
                                    }
                                }

                                SectionTitle {
                                    text: "ACTIVE LAYER"
                                }
                                ComboBox {
                                    id: blendMode
                                    Layout.fillWidth: true
                                    model: ["normal", "multiply", "screen", "overlay", "add"]
                                    Accessible.name: "Active layer blend mode"
                                }
                                Button {
                                    Layout.fillWidth: true
                                    text: "Set blend mode"
                                    onClicked: editor.setLayerBlendMode(editor.activeLayerId, blendMode.currentText)
                                }
                                Item {
                                    Layout.preferredHeight: 12
                                }
                            }
                        }

                        Item {
                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 12
                                spacing: 10
                                Rectangle {
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: agentStatusRow.implicitHeight + 18
                                    radius: 8
                                    color: "#25292f"
                                    border.color: "#353a43"
                                    RowLayout {
                                        id: agentStatusRow
                                        anchors.fill: parent
                                        anchors.margins: 9
                                        Rectangle {
                                            Layout.preferredWidth: 8
                                            Layout.preferredHeight: 8
                                            radius: 4
                                            color: editor.agentBusy ? "#d9a441" : editor.liveAgentConfigured ? "#61c48a" : "#7f8da6"
                                        }
                                        Label {
                                            Layout.fillWidth: true
                                            text: editor.agentStatus
                                            color: "#c9ced6"
                                            wrapMode: Text.Wrap
                                            font.pixelSize: 11
                                        }
                                        BusyIndicator {
                                            visible: editor.agentBusy
                                            running: editor.agentBusy
                                            Layout.preferredWidth: 24
                                            Layout.preferredHeight: 24
                                            Accessible.name: "Redrob request in progress"
                                        }
                                    }
                                }
                                Label {
                                    text: editor.liveAgentConfigured ? "Ask Redrob for a safe edit proposal" : "Create an explicitly no-network local proposal"
                                    font.weight: Font.DemiBold
                                    wrapMode: Text.Wrap
                                    Layout.fillWidth: true
                                }
                                TextArea {
                                    id: prompt
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 90
                                    placeholderText: editor.liveAgentConfigured ? "Describe an edit for review." : "Local subset: selection, gradient, threshold, blur, flip, rotate, clear, fill, layers, undo/redo"
                                    wrapMode: TextEdit.Wrap
                                    Accessible.name: "Agent prompt"
                                }
                                Button {
                                    Layout.fillWidth: true
                                    text: editor.agentBusy ? "Contacting Redrob…" : editor.liveAgentConfigured ? "Create live proposal" : "Create local no-network proposal"
                                    enabled: prompt.text.trim().length > 0 && !editor.agentBusy
                                    Accessible.name: "Create a proposal without applying it"
                                    onClicked: if (editor.proposePrompt(prompt.text))
                                        prompt.clear()
                                }
                                Label {
                                    Layout.fillWidth: true
                                    visible: editor.assistantText.length > 0
                                    text: editor.assistantText
                                    color: "#c9ced6"
                                    wrapMode: Text.Wrap
                                    font.pixelSize: 12
                                }
                                Label {
                                    text: "PENDING PROPOSALS"
                                    color: "#929aa5"
                                    font.pixelSize: 11
                                    font.weight: Font.DemiBold
                                }
                                ListView {
                                    id: proposalList
                                    Layout.fillWidth: true
                                    Layout.fillHeight: true
                                    spacing: 8
                                    clip: true
                                    model: editor.proposals
                                    delegate: Rectangle {
                                        required property string proposalId
                                        required property string proposalTitle
                                        required property string proposalSummary
                                        required property string proposalAction
                                        width: proposalList.width
                                        height: cardContent.implicitHeight + 20
                                        radius: 9
                                        color: "#25292f"
                                        border.color: "#3b414a"
                                        ColumnLayout {
                                            id: cardContent
                                            anchors.left: parent.left
                                            anchors.right: parent.right
                                            anchors.top: parent.top
                                            anchors.margins: 10
                                            Label {
                                                Layout.fillWidth: true
                                                text: proposalTitle
                                                font.weight: Font.DemiBold
                                                wrapMode: Text.Wrap
                                            }
                                            Label {
                                                Layout.fillWidth: true
                                                text: proposalSummary
                                                color: "#aeb5bf"
                                                wrapMode: Text.Wrap
                                                font.pixelSize: 12
                                            }
                                            RowLayout {
                                                Layout.fillWidth: true
                                                Button {
                                                    Layout.fillWidth: true
                                                    text: "Reject"
                                                    Accessible.name: "Reject " + proposalTitle
                                                    onClicked: editor.rejectProposal(proposalId)
                                                }
                                                Button {
                                                    Layout.fillWidth: true
                                                    text: "Apply"
                                                    highlighted: true
                                                    Accessible.name: "Apply " + proposalTitle
                                                    onClicked: editor.applyProposal(proposalId)
                                                }
                                            }
                                        }
                                    }
                                    Label {
                                        anchors.centerIn: parent
                                        visible: proposalList.count === 0
                                        text: "No pending proposals\nEdits always wait for Apply."
                                        horizontalAlignment: Text.AlignHCenter
                                        color: "#737b86"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            color: "#202328"
            border.color: "#30343a"
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 10
                anchors.rightMargin: 10
                Label {
                    text: editor.statusMessage
                    color: "#aeb4bd"
                    font.pixelSize: 11
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Label {
                    text: editor.selectionActive ? "Selection active" : "No selection"
                    color: editor.selectionActive ? "#79a9ff" : "#858d98"
                    font.pixelSize: 11
                }
                Label {
                    text: "Gen " + editor.generation
                    color: "#858d98"
                    font.pixelSize: 11
                }
                Label {
                    text: window.activeTool + " · " + Math.round(editor.brushSize) + " px"
                    color: "#858d98"
                    font.pixelSize: 11
                }
            }
        }
    }
}
