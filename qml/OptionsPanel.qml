// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Redrob.Graphics 1.0

// The Options tab: every tool's settings, history, adjustments. Moved out of Main.qml in P12, whole;
// it is still the largest file and splits further by section.
ScrollView {
    id: optionsScroll
    objectName: "optionsPanel"
    // The window's tool state and the three colour/tip pickers it opens (P12).
    required property var app
    required property var tipDialog
    required property var gradientStartPicker
    required property var gradientEndPicker
    clip: true
    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
    ColumnLayout {
        width: Math.max(240, optionsScroll.availableWidth - 14)
        x: 7
        spacing: 6

        Label {
            Layout.fillWidth: true
            topPadding: 8
            visible: optionsScroll.app.activeTool === "transform" || optionsScroll.app.activeTool === "inspect"
            text: optionsScroll.app.activeTool === "transform" ? "Drag on the canvas to move the active layer."
                                                      : "View only: clicks on the canvas do not edit."
            color: optionsScroll.app.tokens.inkSecondary
            wrapMode: Text.Wrap
        }
        OptionSection {
            title: "COLOR"
            shown: optionsScroll.app.panelShown("color")
            collapsible: true
            expanded: true
        ColorWheel {
            id: mainColorWheel
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: 200
            Layout.preferredHeight: 200
            current: editor.brushColor
            gamutStart: optionsScroll.app.gamutStart
            gamutSpan: optionsScroll.app.gamutMaskOn ? optionsScroll.app.gamutSpan : 0
            onColorPicked: (picked) => { editor.brushColor = picked; }
        }
        // Gamut mask (Krita): constrain hue selection to an arc.
        RowLayout {
            Layout.fillWidth: true
            CheckBox {
                text: "Gamut mask"
                checked: optionsScroll.app.gamutMaskOn
                onToggled: { optionsScroll.app.gamutMaskOn = checked; mainColorWheel.repaint(); }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: optionsScroll.app.gamutMaskOn
            Label { text: "Start°"; Layout.preferredWidth: 48 }
            Slider {
                Layout.fillWidth: true
                from: 0; to: 360; stepSize: 1
                value: optionsScroll.app.gamutStart
                onMoved: { optionsScroll.app.gamutStart = value; mainColorWheel.repaint(); }
            }
            Label { text: "Span°"; Layout.preferredWidth: 48 }
            Slider {
                Layout.fillWidth: true
                from: 10; to: 300; stepSize: 1
                value: optionsScroll.app.gamutSpan
                onMoved: { optionsScroll.app.gamutSpan = value; mainColorWheel.repaint(); }
            }
        }
        // HSV read-out / entry.
        GridLayout {
            columns: 2
            Layout.fillWidth: true
            Label { text: "H"; color: optionsScroll.app.tokens.inkSecondary }
            NumericField {
                id: colorH
                Layout.fillWidth: true
                text: Math.round((editor.brushColor.hsvHue < 0 ? 0 : editor.brushColor.hsvHue) * 360).toString()
                onEditingFinished: editor.brushColor = Qt.hsva(Math.max(0, Math.min(1, Number(colorH.text) / 360)), editor.brushColor.hsvSaturation, editor.brushColor.hsvValue, 1)
            }
            Label { text: "S"; color: optionsScroll.app.tokens.inkSecondary }
            NumericField {
                id: colorS
                Layout.fillWidth: true
                text: Math.round(editor.brushColor.hsvSaturation * 100).toString()
                onEditingFinished: editor.brushColor = Qt.hsva((editor.brushColor.hsvHue < 0 ? 0 : editor.brushColor.hsvHue), Math.max(0, Math.min(1, Number(colorS.text) / 100)), editor.brushColor.hsvValue, 1)
            }
            Label { text: "V"; color: optionsScroll.app.tokens.inkSecondary }
            NumericField {
                id: colorV
                Layout.fillWidth: true
                text: Math.round(editor.brushColor.hsvValue * 100).toString()
                onEditingFinished: editor.brushColor = Qt.hsva((editor.brushColor.hsvHue < 0 ? 0 : editor.brushColor.hsvHue), editor.brushColor.hsvSaturation, Math.max(0, Math.min(1, Number(colorV.text) / 100)), 1)
            }
        }
        // Hex entry.
        RowLayout {
            Layout.fillWidth: true
            Label { text: "#"; color: optionsScroll.app.tokens.inkSecondary }
            NumericField {
                id: colorHex
                Layout.fillWidth: true
                text: editor.brushColor.toString().replace("#", "").slice(0, 6)
                onEditingFinished: {
                    var c = "#" + colorHex.text.replace("#", "").slice(0, 6);
                    var parsed = Qt.color(c);
                    if (parsed.valid !== false) editor.brushColor = parsed;
                }
            }
        }
        }
        OptionSection {
            title: "PRESETS"
            shown: optionsScroll.app.panelShown("presets")
            collapsible: true
            expanded: false
        RowLayout {
            Layout.fillWidth: true
            TextField {
                id: presetName
                Layout.fillWidth: true
                placeholderText: "Preset name"
            }
            Button {
                text: "Save"
                onClicked: { editor.saveBrushPreset(presetName.text); presetName.text = ""; }
            }
        }
        GridLayout {
            columns: 3
            Layout.fillWidth: true
            columnSpacing: 6
            rowSpacing: 6
            Repeater {
                model: editor.brushPresets
                delegate: ColumnLayout {
                    required property int index
                    required property var modelData
                    spacing: 2
                    // Thumbnail: a dab of the preset's hardness/aspect.
                    Rectangle {
                        Layout.preferredWidth: 48
                        Layout.preferredHeight: 48
                        radius: 6
                        color: optionsScroll.app.tokens.surfaceSunken
                        border.color: optionsScroll.app.tokens.borderStrong
                        Canvas {
                            anchors.fill: parent
                            onPaint: {
                                var ctx = getContext("2d");
                                ctx.clearRect(0, 0, width, height);
                                var cx = width / 2, cy = height / 2;
                                var r = Math.min(width, height) * 0.4;
                                var aspect = modelData.aspect !== undefined ? modelData.aspect : 1;
                                var hardness = modelData.hardness !== undefined ? modelData.hardness : 1;
                                var grad = ctx.createRadialGradient(cx, cy, r * hardness, cx, cy, r);
                                grad.addColorStop(0, editor.brushColor);
                                grad.addColorStop(1, "transparent");
                                ctx.fillStyle = grad;
                                ctx.save();
                                ctx.translate(cx, cy);
                                ctx.scale(1, aspect > 0 ? 1 / aspect : 1);
                                ctx.beginPath();
                                ctx.arc(0, 0, r, 0, 2 * Math.PI);
                                ctx.fill();
                                ctx.restore();
                            }
                        }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: editor.applyBrushPreset(index)
                            onPressAndHold: editor.removeBrushPreset(index)
                        }
                    }
                    Label {
                        text: modelData.name !== undefined ? modelData.name : "Preset"
                        Layout.preferredWidth: 48
                        elide: Text.ElideRight
                        font.pixelSize: 9
                        color: optionsScroll.app.tokens.inkSecondary
                    }
                }
            }
        }
        Label {
            text: "Click a preset to apply; press-and-hold to remove."
            font.pixelSize: 9
            color: optionsScroll.app.tokens.inkSecondary
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        }
        OptionSection {
            title: "PALETTE"
            shown: optionsScroll.app.panelShown("swatches")
            collapsible: true
            expanded: false
        GridLayout {
            columns: 8
            Layout.fillWidth: true
            columnSpacing: 4
            rowSpacing: 4
            Repeater {
                model: editor.palette
                delegate: Rectangle {
                    required property int index
                    required property var modelData
                    Layout.preferredWidth: 20
                    Layout.preferredHeight: 20
                    radius: 4
                    color: modelData
                    border.color: optionsScroll.app.tokens.borderStrong
                    MouseArea {
                        anchors.fill: parent
                        onClicked: editor.brushColor = modelData
                        onPressAndHold: editor.removePaletteColor(index)
                    }
                }
            }
        }
        Button {
            Layout.fillWidth: true
            text: "Add current colour"
            onClicked: editor.addPaletteColor(editor.brushColor)
        }
        // M7: swatch files, as Photoshop's Swatches menu (Load / Save Swatches).
        RowLayout {
            Layout.fillWidth: true
            Button {
                objectName: "swatchesLoad"
                Layout.fillWidth: true
                text: "Load…"
                onClicked: swatchOpenDialog.open()
                Accessible.name: "Load swatches from a .gpl or .aco file"
            }
            Button {
                objectName: "swatchesSave"
                Layout.fillWidth: true
                text: "Save…"
                onClicked: swatchSaveDialog.open()
                Accessible.name: "Save swatches as a GIMP palette"
            }
        }
        FileDialog {
            id: swatchOpenDialog
            popupType: Popup.Item
            title: "Load swatches"
            nameFilters: ["Swatches (*.gpl *.aco)", "All files (*)"]
            onAccepted: editor.loadSwatches(selectedFile, false)
        }
        FileDialog {
            id: swatchSaveDialog
            popupType: Popup.Item
            options: FileDialog.DontConfirmOverwrite
            title: "Save swatches"
            fileMode: FileDialog.SaveFile
            defaultSuffix: "gpl"
            nameFilters: ["GIMP palette (*.gpl)"]
            onAccepted: optionsScroll.app.saveWithConfirm(swatchSaveDialog, selectedFile, defaultSuffix,
                                                          () => editor.saveSwatches(selectedFile))
        }
        Label {
            text: "Click a swatch to pick; press-and-hold to remove."
            font.pixelSize: 9
            color: optionsScroll.app.tokens.inkSecondary
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        }
        OptionSection {
            title: "GRADIENT"
            shown: optionsScroll.app.panelShown("gradients")
            collapsible: true
            expanded: false
        // Live preview of the gradient the gradient tool will paint.
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 24
            radius: 4
            border.color: optionsScroll.app.tokens.borderStrong
            gradient: Gradient {
                orientation: Gradient.Horizontal
                GradientStop { position: 0; color: optionsScroll.app.gradientStartColor }
                GradientStop { position: 1; color: optionsScroll.app.gradientEndColor }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Start"; Layout.preferredWidth: 48 }
            Rectangle {
                Layout.preferredWidth: 28; Layout.preferredHeight: 20; radius: 4
                color: optionsScroll.app.gradientStartColor
                border.color: optionsScroll.app.tokens.borderStrong
                MouseArea { anchors.fill: parent; onClicked: optionsScroll.app.gradientStartColor = editor.brushColor }
            }
            Button { text: "← brush"; onClicked: optionsScroll.app.gradientStartColor = editor.brushColor }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "End"; Layout.preferredWidth: 48 }
            Rectangle {
                Layout.preferredWidth: 28; Layout.preferredHeight: 20; radius: 4
                color: optionsScroll.app.gradientEndColor
                border.color: optionsScroll.app.tokens.borderStrong
                MouseArea { anchors.fill: parent; onClicked: optionsScroll.app.gradientEndColor = editor.brushColor }
            }
            Button { text: "← brush"; onClicked: optionsScroll.app.gradientEndColor = editor.brushColor }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Kind"; Layout.preferredWidth: 48 }
            ComboBox {
                Layout.fillWidth: true
                model: ["linear", "radial"]
                currentIndex: optionsScroll.app.gradientKind === "radial" ? 1 : 0
                onActivated: optionsScroll.app.gradientKind = model[currentIndex]
            }
        }
        Button {
            Layout.fillWidth: true
            text: "Swap endpoints"
            onClicked: { var t = optionsScroll.app.gradientStartColor; optionsScroll.app.gradientStartColor = optionsScroll.app.gradientEndColor; optionsScroll.app.gradientEndColor = t; }
        }
        }
        OptionSection {
            title: "PATTERN"
            shown: optionsScroll.app.panelShown("patterns")
            collapsible: true
            expanded: false
        Label {
            text: "Fill the active layer with a procedural pattern."
            font.pixelSize: 9
            color: optionsScroll.app.tokens.inkSecondary
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Cell"; Layout.preferredWidth: 40 }
            SpinBox { id: patternCell; from: 2; to: 128; value: 16; Layout.fillWidth: true }
        }
        GridLayout {
            columns: 2
            Layout.fillWidth: true
            Button {
                Layout.fillWidth: true
                text: "Checker"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyCheckerboard(patternCell.value, editor.brushColor, Qt.rgba(1, 1, 1, 1))
            }
            Button {
                Layout.fillWidth: true
                text: "Cells"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyCellNoise(Math.max(1, Math.round(editor.documentWidth / patternCell.value)), 1)
            }
            Button {
                Layout.fillWidth: true
                text: "Plasma"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyPlasma(1.5, 1)
            }
            Button {
                Layout.fillWidth: true
                text: "Solid noise"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applySolidNoise(4, 1)
            }
        }
        }
        OptionSection {
            title: "HISTOGRAM"
            shown: optionsScroll.app.panelShown("histogram")
            collapsible: true
            expanded: false
        Canvas {
            id: histogramCanvas
            Layout.fillWidth: true
            Layout.preferredHeight: 90
            property var data: ({})
            Connections {
                target: editor
                function onRenderImageChanged() {
                    histogramCanvas.data = editor.histogram();
                    histogramCanvas.requestPaint();
                }
            }
            Component.onCompleted: { data = editor.histogram(); requestPaint(); }
            onPaint: {
                var ctx = getContext("2d");
                ctx.clearRect(0, 0, width, height);
                if (!data || !data.luma) return;
                // Draw R, G, B as additive translucent curves.
                var channels = [["r", "#e03131"], ["g", "#2f9e44"], ["b", "#1971c2"]];
                var peak = 1;
                for (var c = 0; c < channels.length; c++) {
                    var bins = data[channels[c][0]];
                    for (var i = 0; i < 256; i++) peak = Math.max(peak, bins[i]);
                }
                for (var k = 0; k < channels.length; k++) {
                    var b = data[channels[k][0]];
                    ctx.fillStyle = channels[k][1];
                    ctx.globalAlpha = 0.5;
                    for (var x = 0; x < 256; x++) {
                        var h = (b[x] / peak) * height;
                        var px = x / 256 * width;
                        ctx.fillRect(px, height - h, width / 256 + 0.5, h);
                    }
                }
                ctx.globalAlpha = 1;
            }
        }
        Label {
            text: "Red / green / blue distribution of the composite."
            font.pixelSize: 9
            color: optionsScroll.app.tokens.inkSecondary
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        }
        OptionSection {
            title: "CHANNEL MIXER"
            shown: optionsScroll.app.panelShown("channelMixer")
            collapsible: true
            expanded: false
        GridLayout {
            columns: 4
            Layout.fillWidth: true
            Label { text: "" }
            Label { text: "R"; horizontalAlignment: Text.AlignHCenter; Layout.fillWidth: true }
            Label { text: "G"; horizontalAlignment: Text.AlignHCenter; Layout.fillWidth: true }
            Label { text: "B"; horizontalAlignment: Text.AlignHCenter; Layout.fillWidth: true }
            Label { text: "→R" }
            NumericField { id: mxRR; text: "1" }
            NumericField { id: mxRG; text: "0" }
            NumericField { id: mxRB; text: "0" }
            Label { text: "→G" }
            NumericField { id: mxGR; text: "0" }
            NumericField { id: mxGG; text: "1" }
            NumericField { id: mxGB; text: "0" }
            Label { text: "→B" }
            NumericField { id: mxBR; text: "0" }
            NumericField { id: mxBG; text: "0" }
            NumericField { id: mxBB; text: "1" }
        }
        Button {
            objectName: "channelMixerAction"
            Layout.fillWidth: true
            text: "Apply channel mixer"
            enabled: editor.activeNodeCanEditRaster
            onClicked: editor.applyChannelMixer(
                [Number(mxRR.text), Number(mxRG.text), Number(mxRB.text),
                 Number(mxGR.text), Number(mxGG.text), Number(mxGB.text),
                 Number(mxBR.text), Number(mxBG.text), Number(mxBB.text)],
                [0, 0, 0])
        }
        }
        OptionSection {
            title: "HISTORY"
            shown: optionsScroll.app.panelShown("history")
            collapsible: true
            expanded: false
        // Current position = undoDepth steps done; redoDepth steps ahead.
        Repeater {
            model: editor.undoDepth + editor.redoDepth + 1
            delegate: Rectangle {
                required property int index
                Layout.fillWidth: true
                Layout.preferredHeight: 22
                radius: 4
                // index 0 = base state, then each applied step.
                property bool isCurrent: index === editor.undoDepth
                property bool isFuture: index > editor.undoDepth
                color: isCurrent ? optionsScroll.app.tokens.surfaceBrandSubtle : "transparent"
                opacity: isFuture ? 0.5 : 1.0
                objectName: "historyRow" + index
                Accessible.role: Accessible.Button
                Accessible.name: "Go to history step " + index
                MouseArea {
                    // Click a row to return the document to that point: undo
                    // back to an earlier row, redo forward to a dimmed one.
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: editor.jumpToHistory(index)
                }
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 6
                    Label {
                        // The engine's label is a snake_case command tag
                        // ("apply_filter"); shown as words, first one capitalised.
                        property string raw: index === 0 ? "" : (editor.historyLabels[index - 1] || "")
                        text: index === 0 ? "Opened"
                              : raw.length === 0 ? ("Step " + index)
                              : raw.charAt(0).toUpperCase() + raw.slice(1).replace(/_/g, " ")
                        color: optionsScroll.app.tokens.inkPrimary
                        font.pixelSize: 10
                        Layout.fillWidth: true
                    }
                    Label {
                        visible: parent.parent.isCurrent
                        text: "● now"
                        color: optionsScroll.app.tokens.inkBrand
                        font.pixelSize: 9
                    }
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Button {
                Layout.fillWidth: true
                text: "Undo"
                enabled: editor.canUndo
                onClicked: editor.undo()
            }
            Button {
                Layout.fillWidth: true
                text: "Redo"
                enabled: editor.canRedo
                onClicked: editor.redo()
            }
        }
        Label {
            text: editor.undoDepth + " done · " + editor.redoDepth + " ahead"
            font.pixelSize: 9
            color: optionsScroll.app.tokens.inkSecondary
            Layout.fillWidth: true
        }
        }
        OptionSection {
            title: "DIGITAL MIXER"
            shown: optionsScroll.app.panelShown("digitalMixer")
            collapsible: true
            expanded: false
        RowLayout {
            Layout.fillWidth: true
            Rectangle {
                Layout.preferredWidth: 28; Layout.preferredHeight: 20; radius: 4
                color: optionsScroll.app.mixerColorA
                border.color: optionsScroll.app.tokens.borderStrong
                MouseArea { anchors.fill: parent; onClicked: optionsScroll.app.mixerColorA = editor.brushColor }
            }
            Slider {
                id: mixAmount
                Layout.fillWidth: true
                from: 0; to: 1; value: 0.5
            }
            Rectangle {
                Layout.preferredWidth: 28; Layout.preferredHeight: 20; radius: 4
                color: optionsScroll.app.mixerColorB
                border.color: optionsScroll.app.tokens.borderStrong
                MouseArea { anchors.fill: parent; onClicked: optionsScroll.app.mixerColorB = editor.brushColor }
            }
        }
        // Live mixed result; click to adopt as the brush colour.
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 24
            radius: 4
            border.color: optionsScroll.app.tokens.borderStrong
            property color mixed: Qt.rgba(
                optionsScroll.app.mixerColorA.r * (1 - mixAmount.value) + optionsScroll.app.mixerColorB.r * mixAmount.value,
                optionsScroll.app.mixerColorA.g * (1 - mixAmount.value) + optionsScroll.app.mixerColorB.g * mixAmount.value,
                optionsScroll.app.mixerColorA.b * (1 - mixAmount.value) + optionsScroll.app.mixerColorB.b * mixAmount.value,
                1)
            color: mixed
            MouseArea { anchors.fill: parent; onClicked: editor.brushColor = parent.mixed }
        }
        Label {
            text: "Click a swatch to load the brush colour; click the bar to adopt the mix."
            font.pixelSize: 9
            color: optionsScroll.app.tokens.inkSecondary
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        }
        OptionSection {
            id: wideGamutSection
            title: "WIDE GAMUT"
            shown: optionsScroll.app.panelShown("wideGamut")
            collapsible: true
            expanded: false
        // Linear-light R/G/B selection (Krita's wide-gamut feel). We pick in
        // linear space and gamma-encode to the sRGB brush colour, which is how
        // blending-correct colour reads brighter than a plain sRGB slider. Our
        // pipeline is sRGB-bound, so values clamp at the sRGB gamut edge.
        function linToSrgb(c) {
            return c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "R (lin)"; Layout.preferredWidth: 56 }
            Slider { id: wgR; Layout.fillWidth: true; from: 0; to: 1; value: 0.5 }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "G (lin)"; Layout.preferredWidth: 56 }
            Slider { id: wgG; Layout.fillWidth: true; from: 0; to: 1; value: 0.5 }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "B (lin)"; Layout.preferredWidth: 56 }
            Slider { id: wgB; Layout.fillWidth: true; from: 0; to: 1; value: 0.5 }
        }
        Rectangle {
            id: wideGamutSwatch
            Layout.fillWidth: true
            Layout.preferredHeight: 24
            radius: 4
            border.color: optionsScroll.app.tokens.borderStrong
            // Addressed by id, NOT through `parent`: OptionSection re-parents its
            // children into an inner layout (`content: sectionBody.data`), so this
            // Rectangle's parent is that layout and not the section that declares
            // linToSrgb. Through `parent` the call resolved to nothing and the
            // swatch stayed unbound — visible only as a QML warning at runtime,
            // which is why no build or test caught it.
            property color encoded: Qt.rgba(wideGamutSection.linToSrgb(wgR.value), wideGamutSection.linToSrgb(wgG.value), wideGamutSection.linToSrgb(wgB.value), 1)
            color: encoded
            MouseArea { anchors.fill: parent; onClicked: editor.brushColor = wideGamutSwatch.encoded }
        }
        Label {
            text: "Pick in linear light; click the bar to set the brush colour (gamma-encoded)."
            font.pixelSize: 9
            color: optionsScroll.app.tokens.inkSecondary
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        }
        OptionSection {
            title: "STORYBOARD"
            shown: optionsScroll.app.panelShown("storyboard")
            collapsible: true
            expanded: false
        // Frame strip (timeline extension): one cell per frame, current marked.
        Flow {
            Layout.fillWidth: true
            spacing: 4
            Repeater {
                model: editor.frames
                delegate: Rectangle {
                    required property int index
                    required property int duration
                    required property bool current
                    required property int frameId
                    width: 44; height: 36; radius: 4
                    color: current ? optionsScroll.app.tokens.surfaceBrandSubtle : optionsScroll.app.tokens.surfaceSunken
                    border.color: current ? optionsScroll.app.tokens.inkBrand : optionsScroll.app.tokens.borderStrong
                    ColumnLayout {
                        anchors.centerIn: parent
                        spacing: 0
                        Label { text: "#" + (index + 1); font.pixelSize: 10; color: optionsScroll.app.tokens.inkPrimary; Layout.alignment: Qt.AlignHCenter }
                        Label { text: duration + "ms"; font.pixelSize: 8; color: optionsScroll.app.tokens.inkSecondary; Layout.alignment: Qt.AlignHCenter }
                    }
                    MouseArea { anchors.fill: parent; onClicked: editor.setCurrentFrame(frameId) }
                }
            }
        }
        // Onion skin: bound straight to the bridge, which renders the ghosted
        // composite through the core's onion-skin path. Changing any of these
        // repaints the canvas.
        RowLayout {
            Layout.fillWidth: true
            CheckBox {
                text: "Onion skin"
                checked: editor.onionSkinEnabled
                onToggled: editor.onionSkinEnabled = checked
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: editor.onionSkinEnabled
            Label { text: "Before"; Layout.preferredWidth: 48 }
            SpinBox { from: 0; to: 8; value: editor.onionSkinBefore; onValueModified: editor.onionSkinBefore = value; Layout.fillWidth: true }
            Label { text: "After"; Layout.preferredWidth: 48 }
            SpinBox { from: 0; to: 8; value: editor.onionSkinAfter; onValueModified: editor.onionSkinAfter = value; Layout.fillWidth: true }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: editor.onionSkinEnabled
            Label { text: "Ghost"; Layout.preferredWidth: 48 }
            Slider {
                Layout.fillWidth: true
                from: 0.05
                to: 1.0
                value: editor.onionSkinOpacity
                onMoved: editor.onionSkinOpacity = value
            }
            Label {
                text: Math.round(editor.onionSkinOpacity * 100) + "%"
                font.pixelSize: 9
                color: optionsScroll.app.tokens.inkSecondary
            }
        }
        Label {
            text: editor.frameCount + " frames. Click a cell to go to it."
            font.pixelSize: 9
            color: optionsScroll.app.tokens.inkSecondary
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        }
        OptionSection {
            title: "OP GRAPH"
            shown: optionsScroll.app.panelShown("opGraph")
            collapsible: true
            expanded: false
        Label {
            text: "Chain two operations and apply them in order (GEGL-style)."
            font.pixelSize: 9
            color: optionsScroll.app.tokens.inkSecondary
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        // Each op is {filter kind, amount}. The combos pick from a few ops that
        // need no parameters so the chain is one click to apply.
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Op 1"; Layout.preferredWidth: 40 }
            ComboBox {
                id: graphOp1
                Layout.fillWidth: true
                model: ["grayscale", "invert", "laplace", "edge_detect", "emboss"]
            }
            Slider { id: graphAmt1; Layout.preferredWidth: 70; from: 0; to: 1; value: 1 }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Op 2"; Layout.preferredWidth: 40 }
            ComboBox {
                id: graphOp2
                Layout.fillWidth: true
                model: ["none", "grayscale", "invert", "laplace", "edge_detect", "emboss"]
            }
            Slider { id: graphAmt2; Layout.preferredWidth: 70; from: 0; to: 1; value: 1 }
        }
        Button {
            objectName: "opGraphAction"
            Layout.fillWidth: true
            text: "Apply op graph"
            enabled: editor.activeNodeCanEditRaster
            onClicked: {
                function node(kind, amt) {
                    // edge_detect takes an amount param; the rest are parameterless here.
                    var f = { "kind": kind };
                    if (kind === "edge_detect") f.amount = 1.0;
                    return { "filter": f, "amount": amt, "enabled": true };
                }
                var nodes = [node(graphOp1.currentText, graphAmt1.value)];
                if (graphOp2.currentText !== "none")
                    nodes.push(node(graphOp2.currentText, graphAmt2.value));
                editor.applyOpGraph(JSON.stringify(nodes));
            }
        }
        }
        OptionSection {
            title: "LAYER STYLE"
            shown: optionsScroll.app.panelShown("layerStyle")
            collapsible: true
            expanded: false
        function brushRgba() {
            var c = editor.brushColor;
            return { "r": Math.round(c.r * 255), "g": Math.round(c.g * 255), "b": Math.round(c.b * 255), "a": 255 };
        }
        CheckBox { id: lsShadow; text: "Drop shadow" }
        RowLayout {
            Layout.fillWidth: true
            visible: lsShadow.checked
            Label { text: "dx/dy"; Layout.preferredWidth: 44 }
            SpinBox { id: lsShX; from: -64; to: 64; value: 6 }
            SpinBox { id: lsShY; from: -64; to: 64; value: 6 }
            Label { text: "blur" }
            SpinBox { id: lsShBlur; from: 0; to: 64; value: 4 }
        }
        CheckBox { id: lsGlow; text: "Outer glow" }
        RowLayout {
            Layout.fillWidth: true
            visible: lsGlow.checked
            Label { text: "blur"; Layout.preferredWidth: 44 }
            SpinBox { id: lsGlowBlur; from: 1; to: 64; value: 6 }
        }
        CheckBox { id: lsBevel; text: "Bevel" }
        RowLayout {
            Layout.fillWidth: true
            visible: lsBevel.checked
            Label { text: "depth"; Layout.preferredWidth: 44 }
            NumericField { id: lsBevelDepth; text: "6" }
            Label { text: "blur" }
            SpinBox { id: lsBevelBlur; from: 1; to: 32; value: 3 }
        }
        Button {
            objectName: "layerStyleAction"
            Layout.fillWidth: true
            text: "Bake layer style"
            enabled: editor.activeNodeCanEditRaster
            onClicked: {
                var style = {};
                if (lsShadow.checked)
                    style.drop_shadow = { "color": { "r": 0, "g": 0, "b": 0, "a": 255 }, "offset_x": lsShX.value, "offset_y": lsShY.value, "blur": lsShBlur.value, "opacity": 0.6 };
                if (lsGlow.checked)
                    style.outer_glow = { "color": brushRgba(), "blur": lsGlowBlur.value, "opacity": 0.8 };
                if (lsBevel.checked)
                    style.bevel = { "azimuth_degrees": 135, "depth": Number(lsBevelDepth.text), "blur": lsBevelBlur.value };
                editor.applyLayerStyle(JSON.stringify(style));
            }
        }
        Label {
            text: "Bakes the effects into the layer (destructive)."
            font.pixelSize: 9
            color: optionsScroll.app.tokens.inkSecondary
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        }
        OptionSection {
            title: "FILL"
            shown: optionsScroll.app.activeTool === "fill"
        RowLayout {
            Layout.fillWidth: true
            visible: optionsScroll.app.activeTool === "fill"
            Label {
                text: "Tolerance"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                Layout.fillWidth: true
                from: 0
                to: 255
                stepSize: 1
                value: optionsScroll.app.fillTolerance
                Accessible.name: "Fill tolerance 0 to 255"
                onMoved: optionsScroll.app.fillTolerance = Math.round(value)
            }
            Label {
                text: optionsScroll.app.fillTolerance
                Layout.preferredWidth: 36
            }
        }
        }
        OptionSection {
            title: "BRUSH"
            shown: optionsScroll.app.brushLike
        SubsectionTitle { text: "Tip" }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Shape"
                Layout.preferredWidth: 72
            }
            ComboBox {
                objectName: "brushTipControl"
                Layout.fillWidth: true
                model: ["Round"].concat(editor.brushTipNames)
                currentIndex: editor.brushTipIndex + 1
                Accessible.name: "Brush tip"
                onActivated: editor.brushTipIndex = currentIndex - 1
            }
            CommandButton {
                objectName: "loadBrushTipsAction"
                text: "Load"
                iconName: "folderOpen"
                ToolTip.text: "Load brush tips from a GIMP .gbr or Photoshop .abr file"
                onClicked: optionsScroll.tipDialog.open()
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Edge"
                Layout.preferredWidth: 72
            }
            // Pencil = GIMP's hard, aliased edge (vs the paintbrush's soft one).
            CheckBox {
                objectName: "brushPencilControl"
                text: "Pencil (hard edge)"
                leftPadding: 0
                Layout.fillWidth: true
                checked: editor.brushPencil
                onToggled: editor.brushPencil = checked
                Accessible.name: "Pencil hard edge"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Mode"
                Layout.preferredWidth: 72
            }
            // Airbrush = GIMP's airbrush: paint builds up while held.
            CheckBox {
                objectName: "brushAirbrushControl"
                text: "Airbrush (build up)"
                leftPadding: 0
                Layout.fillWidth: true
                checked: editor.brushAirbrush
                onToggled: editor.brushAirbrush = checked
                Accessible.name: "Airbrush build up"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: ""
                Layout.preferredWidth: 72
            }
            // Smudge = GIMP's smudge: drag the colour already on the layer.
            CheckBox {
                objectName: "brushSmudgeControl"
                text: "Smudge (drag colour)"
                leftPadding: 0
                Layout.fillWidth: true
                checked: editor.brushSmudge
                onToggled: editor.brushSmudge = checked
                Accessible.name: "Smudge drag colour"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: ""
                Layout.preferredWidth: 72
            }
            // Clone = GIMP's clone tool. Alt-click sets the source, then paint.
            CheckBox {
                objectName: "brushCloneControl"
                text: "Clone (Alt-click src)"
                leftPadding: 0
                Layout.fillWidth: true
                checked: editor.brushClone
                onToggled: editor.brushClone = checked
                Accessible.name: "Clone from source"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: ""
                Layout.preferredWidth: 72
            }
            // Heal = GIMP's heal: clone, but match the patch to local colour.
            CheckBox {
                objectName: "brushHealControl"
                text: "Heal (match colour)"
                leftPadding: 0
                Layout.fillWidth: true
                enabled: editor.brushClone
                checked: editor.brushHeal
                onToggled: editor.brushHeal = checked
                Accessible.name: "Heal match colour"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Convolve"
                Layout.preferredWidth: 72
            }
            // GIMP's blur/sharpen brush: process pixels under the dab in place.
            ComboBox {
                objectName: "brushConvolveControl"
                Layout.fillWidth: true
                model: ["off", "blur", "sharpen"]
                currentIndex: Math.max(0, model.indexOf(editor.brushConvolveMode))
                Accessible.name: "Convolve mode"
                onActivated: editor.brushConvolveMode = model[currentIndex]
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Dodge/Burn"
                Layout.preferredWidth: 72
            }
            // GIMP's dodge/burn brush: lighten or darken a tonal range.
            ComboBox {
                objectName: "brushDodgeBurnControl"
                Layout.fillWidth: true
                model: ["off", "dodge", "burn"]
                currentIndex: Math.max(0, model.indexOf(editor.brushDodgeBurnMode))
                Accessible.name: "Dodge or burn"
                onActivated: editor.brushDodgeBurnMode = model[currentIndex]
            }
            ComboBox {
                objectName: "brushDodgeRangeControl"
                Layout.preferredWidth: 110
                enabled: editor.brushDodgeBurnMode !== "off"
                model: ["shadows", "midtones", "highlights"]
                currentIndex: Math.max(0, model.indexOf(editor.brushDodgeRange))
                Accessible.name: "Dodge burn tonal range"
                onActivated: editor.brushDodgeRange = model[currentIndex]
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: ""
                Layout.preferredWidth: 72
            }
            // Ink = GIMP's ink nib: the line thins as the pen moves faster.
            CheckBox {
                objectName: "brushInkControl"
                text: "Ink (speed thins line)"
                leftPadding: 0
                Layout.fillWidth: true
                checked: editor.brushInk
                onToggled: editor.brushInk = checked
                Accessible.name: "Ink speed thins line"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: ""
                Layout.preferredWidth: 72
            }
            // MyPaint = scattered, grainy dabs for a textured line.
            CheckBox {
                objectName: "brushMyPaintControl"
                text: "MyPaint (grainy)"
                leftPadding: 0
                Layout.fillWidth: true
                checked: editor.brushMyPaint
                onToggled: editor.brushMyPaint = checked
                Accessible.name: "MyPaint grainy scatter"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Size from"
                Layout.preferredWidth: 72
            }
            // Krita sensor/preset engine: bind an input sensor to brush size.
            ComboBox {
                objectName: "brushSizeDynamicControl"
                Layout.fillWidth: true
                model: ["off", "pressure", "speed", "random", "tilt"]
                currentIndex: Math.max(0, model.indexOf(editor.brushSizeDynamic))
                Accessible.name: "Size dynamics sensor"
                onActivated: editor.brushSizeDynamic = model[currentIndex]
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Opacity from"
                Layout.preferredWidth: 72
            }
            // Its own channel, not a second size binding: pressure already
            // drives the diameter, so without this a harder press cannot be
            // asked to darken without also widening.
            ComboBox {
                objectName: "brushOpacityDynamicControl"
                Layout.fillWidth: true
                model: ["off", "pressure", "speed", "random", "tilt"]
                currentIndex: Math.max(0, model.indexOf(editor.brushOpacityDynamic))
                Accessible.name: "Opacity dynamics sensor"
                onActivated: editor.brushOpacityDynamic = model[currentIndex]
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Flow from"
                Layout.preferredWidth: 72
            }
            // Flow is how much paint a dab lays down; opacity is how dark the
            // stroke can get. Low flow at full opacity builds up over passes.
            ComboBox {
                objectName: "brushFlowDynamicControl"
                Layout.fillWidth: true
                model: ["off", "pressure", "speed", "random", "tilt"]
                currentIndex: Math.max(0, model.indexOf(editor.brushFlowDynamic))
                Accessible.name: "Flow dynamics sensor"
                onActivated: editor.brushFlowDynamic = model[currentIndex]
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: ""
                Layout.preferredWidth: 72
            }
            // GIH image pipe: cycle every loaded tip, one per dab.
            CheckBox {
                objectName: "brushPipeControl"
                text: "Pipe loaded tips"
                leftPadding: 0
                Layout.fillWidth: true
                enabled: editor.brushTipNames.length >= 2
                checked: editor.brushPipe
                onToggled: editor.brushPipe = checked
                Accessible.name: "Cycle loaded brush tips"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Size"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                Layout.fillWidth: true
                from: 0
                to: 1
                value: optionsScroll.app.brushSizeToSlider(editor.brushSize)
                Accessible.name: "Brush size 1 to 1000"
                onMoved: editor.brushSize = optionsScroll.app.sliderToBrushSize(value)
            }
            Label {
                text: Math.round(editor.brushSize)
                Layout.preferredWidth: 44
                horizontalAlignment: Text.AlignRight
                font.features: { "tnum": 1 }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            // An image tip replaces the generated dab, so its shape controls do nothing.
            enabled: editor.brushTipIndex < 0
            Label {
                text: "Hardness"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                objectName: "brushHardnessControl"
                Layout.fillWidth: true
                from: 0
                to: 1
                value: editor.brushHardness
                Accessible.name: "Brush hardness"
                onMoved: editor.brushHardness = value
            }
            Label {
                text: Math.round(editor.brushHardness * 100) + "%"
                Layout.preferredWidth: 44
                horizontalAlignment: Text.AlignRight
                font.features: { "tnum": 1 }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            enabled: editor.brushTipIndex < 0
            Label {
                text: "Roundness"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                objectName: "brushAspectControl"
                Layout.fillWidth: true
                from: 0.05
                to: 1
                value: editor.brushAspect
                Accessible.name: "Brush roundness, height as a fraction of width"
                onMoved: editor.brushAspect = value
            }
            Label {
                text: Math.round(editor.brushAspect * 100) + "%"
                Layout.preferredWidth: 44
                horizontalAlignment: Text.AlignRight
                font.features: { "tnum": 1 }
            }
        }
        SubsectionTitle { text: "Paint" }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Mode"
                Layout.preferredWidth: 72
            }
            // Krita's eraser is a mode of the current brush, toggled with E,
            // not a separate tool: same tip, size and smoothing, removing paint.
            CheckBox {
                objectName: "brushEraseControl"
                text: "Erase"
                ToolTip.visible: hovered
                ToolTip.delay: 450
                ToolTip.text: "Eraser mode   E"
                leftPadding: 0
                Layout.fillWidth: true
                checked: editor.brushErase
                onToggled: editor.brushErase = checked
                Accessible.name: "Eraser mode"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Opacity"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                Layout.fillWidth: true
                from: 0
                to: 1
                value: editor.brushOpacity
                Accessible.name: "Brush opacity"
                onMoved: editor.brushOpacity = value
            }
            Label {
                text: Math.round(editor.brushOpacity * 100) + "%"
                Layout.preferredWidth: 44
                horizontalAlignment: Text.AlignRight
                font.features: { "tnum": 1 }
            }
        }
        // M3: Photoshop's Flow -- how much each dab lays down; low flow builds up over a stroke.
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Flow"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                objectName: "brushFlowControl"
                Layout.fillWidth: true
                from: 0.01
                to: 1
                value: editor.brushFlow
                Accessible.name: "Brush flow"
                onMoved: editor.brushFlow = value
            }
            Label {
                text: Math.round(editor.brushFlow * 100) + "%"
                Layout.preferredWidth: 44
                horizontalAlignment: Text.AlignRight
                font.features: { "tnum": 1 }
            }
        }
        // L9: Photoshop's Mixer Brush settings, shown while the mixer tool is active.
        // U2: the preset list sets Wet, Load and Mix together, as Photoshop's does.
        RowLayout {
            Layout.fillWidth: true
            visible: editor.brushMixer
            Label {
                text: "Preset"
                Layout.preferredWidth: 72
            }
            ComboBox {
                id: mixerPreset
                objectName: "brushMixerPresetControl"
                Layout.fillWidth: true
                // [name, wet, load, mix]
                readonly property var presets: [
                    ["Dry", 0, 0.5, 0],
                    ["Dry, heavy load", 0, 1, 0],
                    ["Moist", 0.1, 0.5, 0.5],
                    ["Wet", 0.5, 0.5, 0.5],
                    ["Wet, heavy load", 0.5, 1, 0.5],
                    ["Very wet", 1, 0.5, 1]
                ]
                model: presets.map(p => p[0]).concat(["Custom"])
                currentIndex: {
                    for (let i = 0; i < presets.length; ++i) {
                        const p = presets[i];
                        if (Math.abs(editor.brushMixerWet - p[1]) < 0.005
                                && Math.abs(editor.brushMixerLoad - p[2]) < 0.005
                                && Math.abs(editor.brushMixerMix - p[3]) < 0.005)
                            return i;
                    }
                    return presets.length;
                }
                onActivated: index => {
                    if (index >= presets.length)
                        return;
                    editor.brushMixerWet = presets[index][1];
                    editor.brushMixerLoad = presets[index][2];
                    editor.brushMixerMix = presets[index][3];
                }
                Accessible.name: "Mixer brush preset"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: editor.brushMixer
            Label {
                text: "Wet"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                objectName: "brushMixerWetControl"
                Layout.fillWidth: true
                from: 0
                to: 1
                value: editor.brushMixerWet
                Accessible.name: "Mixer wet: canvas colour picked up"
                onMoved: editor.brushMixerWet = value
            }
            Label {
                text: Math.round(editor.brushMixerWet * 100) + "%"
                Layout.preferredWidth: 44
                horizontalAlignment: Text.AlignRight
                font.features: { "tnum": 1 }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: editor.brushMixer
            Label {
                text: "Load"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                objectName: "brushMixerLoadControl"
                Layout.fillWidth: true
                from: 0
                to: 1
                value: editor.brushMixerLoad
                Accessible.name: "Mixer load: paint held"
                onMoved: editor.brushMixerLoad = value
            }
            Label {
                text: Math.round(editor.brushMixerLoad * 100) + "%"
                Layout.preferredWidth: 44
                horizontalAlignment: Text.AlignRight
                font.features: { "tnum": 1 }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: editor.brushMixer
            Label {
                text: "Mix"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                objectName: "brushMixerMixControl"
                Layout.fillWidth: true
                from: 0
                to: 1
                value: editor.brushMixerMix
                Accessible.name: "Mixer mix: canvas share"
                onMoved: editor.brushMixerMix = value
            }
            Label {
                text: Math.round(editor.brushMixerMix * 100) + "%"
                Layout.preferredWidth: 44
                horizontalAlignment: Text.AlignRight
                font.features: { "tnum": 1 }
            }
        }
        // U2: what the brush holds now, and Photoshop's Load / Clean buttons and options.
        RowLayout {
            Layout.fillWidth: true
            visible: editor.brushMixer
            Rectangle {
                objectName: "mixerWellSwatch"
                Layout.preferredWidth: 28
                Layout.preferredHeight: 28
                radius: 4
                // The paint itself (document colour), dimmed by how much is left.
                color: Qt.rgba(editor.mixerWellColor.r, editor.mixerWellColor.g,
                               editor.mixerWellColor.b,
                               editor.mixerWellColor.a * Math.max(0.15, editor.mixerWellLevel))
                border.color: optionsScroll.app.tokens.borderStrong
                Accessible.name: "Paint on the brush, " + Math.round(editor.mixerWellLevel * 100) + "% full"
            }
            CommandButton {
                objectName: "mixerLoadBrushAction"
                text: "Load"
                iconName: "fill"
                ToolTip.text: "Fill the brush with the brush colour"
                onClicked: editor.mixerLoadBrush()
            }
            CommandButton {
                objectName: "mixerCleanBrushAction"
                text: "Clean"
                iconName: "eraser"
                ToolTip.text: "Wipe the paint off the brush"
                onClicked: editor.mixerCleanBrush()
            }
        }
        CheckBox {
            objectName: "brushMixerAutoLoadControl"
            visible: editor.brushMixer
            text: "Load the brush after each stroke"
            checked: editor.brushMixerAutoLoad
            onToggled: editor.brushMixerAutoLoad = checked
        }
        CheckBox {
            objectName: "brushMixerAutoCleanControl"
            visible: editor.brushMixer
            text: "Clean the brush after each stroke"
            checked: editor.brushMixerAutoClean
            onToggled: editor.brushMixerAutoClean = checked
        }
        CheckBox {
            objectName: "brushMixerSampleAllControl"
            visible: editor.brushMixer
            text: "Sample all layers"
            checked: editor.brushMixerSampleAll
            onToggled: editor.brushMixerSampleAll = checked
        }
        // L3: dab angle (shows on an elliptical tip) and Photoshop's Angle Jitter: Pen Tilt.
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Angle"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                objectName: "brushAngleControl"
                Layout.fillWidth: true
                from: -180
                to: 180
                value: editor.brushAngle
                Accessible.name: "Brush angle"
                onMoved: editor.brushAngle = value
            }
            Label {
                text: Math.round(editor.brushAngle) + "°"
                Layout.preferredWidth: 44
                horizontalAlignment: Text.AlignRight
                font.features: { "tnum": 1 }
            }
        }
        CheckBox {
            objectName: "brushAngleFromTiltControl"
            text: "Angle follows pen tilt"
            leftPadding: 0
            checked: editor.brushAngleFromTilt
            onToggled: editor.brushAngleFromTilt = checked
            Accessible.name: "Angle follows pen tilt"
        }
        SubsectionTitle { text: "Stroke" }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Smoothing"
                Layout.preferredWidth: 72
            }
            ComboBox {
                Layout.fillWidth: true
                textRole: "text"
                valueRole: "value"
                model: [
                    { text: "Off", value: "none" },
                    { text: "Moving average", value: "moving_average" }
                ]
                currentIndex: editor.brushSmoothingKind === "moving_average" ? 1 : 0
                Accessible.name: "Brush smoothing kind"
                onActivated: editor.brushSmoothingKind = currentValue
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: editor.brushSmoothingKind === "moving_average"
            Label {
                text: "Window"
                Layout.preferredWidth: 72
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
                leftPadding: 0
                Layout.fillWidth: true
                checked: editor.mirrorXEnabled
                onToggled: editor.mirrorXEnabled = checked
                Accessible.name: "Mirror brush across X axis"
            }
            SpinBox {
                Layout.preferredWidth: 120
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
                leftPadding: 0
                Layout.fillWidth: true
                checked: editor.mirrorYEnabled
                onToggled: editor.mirrorYEnabled = checked
                Accessible.name: "Mirror brush across Y axis"
            }
            SpinBox {
                Layout.preferredWidth: 120
                from: 0
                to: Math.max(1, editor.documentHeight)
                value: Math.round(editor.mirrorYAxis)
                onValueModified: editor.mirrorYAxis = value
                Accessible.name: "Mirror Y axis position"
            }
        }
        RowLayout {
            // Multihand radial symmetry (Krita multibrush): N rotated copies
            // about the canvas centre. 0 = off.
            Layout.fillWidth: true
            Label { text: "Symmetry"; Layout.preferredWidth: 72 }
            SpinBox {
                from: 0
                to: 32
                value: editor.brushSymmetryOrder
                onValueModified: editor.brushSymmetryOrder = value
                Accessible.name: "Radial symmetry order"
            }
            Label { text: editor.brushSymmetryOrder >= 2 ? "× copies" : "off" }
        }
        RowLayout {
            // Drawing assistant (Krita assistants): snap the stroke to a guide.
            Layout.fillWidth: true
            Label { text: "Assistant"; Layout.preferredWidth: 72 }
            ComboBox {
                Layout.fillWidth: true
                model: ["none", "vanishing", "parallel", "ellipse"]
                currentIndex: Math.max(0, model.indexOf(editor.brushAssistantKind))
                onActivated: editor.brushAssistantKind = model[currentIndex]
                Accessible.name: "Drawing assistant"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: editor.brushAssistantKind !== "none"
            Label { text: "Guide"; Layout.preferredWidth: 72 }
            NumericField { id: asstP0; Layout.fillWidth: true; text: "0"; placeholderText: "p0" }
            NumericField { id: asstP1; Layout.fillWidth: true; text: "0"; placeholderText: "p1" }
            NumericField { id: asstP2; Layout.fillWidth: true; text: "0"; placeholderText: "p2" }
            NumericField { id: asstP3; Layout.fillWidth: true; text: "0"; placeholderText: "p3" }
            Button {
                text: "Set"
                onClicked: editor.setBrushAssistantParams(Number(asstP0.text), Number(asstP1.text), Number(asstP2.text), Number(asstP3.text))
            }
        }
        RowLayout {
            // Dyna brush (GIMP): mass-spring smoothing of the stroke.
            Layout.fillWidth: true
            CheckBox {
                text: "Dyna"
                checked: editor.brushDynaEnabled
                onToggled: editor.brushDynaEnabled = checked
                Accessible.name: "Dynamic (mass-spring) brush"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: editor.brushDynaEnabled
            Label { text: "Mass"; Layout.preferredWidth: 72 }
            Slider {
                Layout.fillWidth: true
                from: 0.0; to: 1.0; stepSize: 0.05
                value: editor.brushDynaMass
                onMoved: editor.brushDynaMass = value
            }
            Label { text: editor.brushDynaMass.toFixed(2) }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: editor.brushDynaEnabled
            Label { text: "Drag"; Layout.preferredWidth: 72 }
            Slider {
                Layout.fillWidth: true
                from: 0.0; to: 1.0; stepSize: 0.05
                value: editor.brushDynaDrag
                onMoved: editor.brushDynaDrag = value
            }
            Label { text: editor.brushDynaDrag.toFixed(2) }
        }
        }
        OptionSection {
            title: "WARP"
            shown: optionsScroll.app.activeTool === "warp"
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Mode"; Layout.preferredWidth: 72 }
            ComboBox {
                Layout.fillWidth: true
                model: ["move", "grow", "shrink", "swirl_cw", "swirl_ccw"]
                currentIndex: Math.max(0, model.indexOf(optionsScroll.app.warpMode))
                onActivated: optionsScroll.app.warpMode = model[currentIndex]
                Accessible.name: "Warp mode"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Radius"; Layout.preferredWidth: 72 }
            Slider {
                Layout.fillWidth: true
                from: 4; to: 200; stepSize: 1
                value: optionsScroll.app.warpRadius
                onMoved: optionsScroll.app.warpRadius = value
            }
            Label { text: Math.round(optionsScroll.app.warpRadius) + " px" }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Strength"; Layout.preferredWidth: 72 }
            Slider {
                Layout.fillWidth: true
                from: 0.05; to: 1.0; stepSize: 0.05
                value: optionsScroll.app.warpStrength
                onMoved: optionsScroll.app.warpStrength = value
            }
            Label { text: optionsScroll.app.warpStrength.toFixed(2) }
        }
        }
        OptionSection {
            title: "ALIGN"
            shown: optionsScroll.app.activeTool === "align"
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Relative to"
                Layout.preferredWidth: 72
            }
            ComboBox {
                id: alignTarget
                Layout.fillWidth: true
                model: ["canvas", "itself"]
                Accessible.name: "Align relative to"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Horizontal"
                Layout.preferredWidth: 72
            }
            Button {
                Layout.fillWidth: true
                text: "Left"
                onClicked: editor.alignActiveLayer(1, 0, alignTarget.currentIndex === 0)
            }
            Button {
                Layout.fillWidth: true
                text: "Centre"
                onClicked: editor.alignActiveLayer(2, 0, alignTarget.currentIndex === 0)
            }
            Button {
                Layout.fillWidth: true
                text: "Right"
                onClicked: editor.alignActiveLayer(3, 0, alignTarget.currentIndex === 0)
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Vertical"
                Layout.preferredWidth: 72
            }
            Button {
                Layout.fillWidth: true
                text: "Top"
                onClicked: editor.alignActiveLayer(0, 1, alignTarget.currentIndex === 0)
            }
            Button {
                Layout.fillWidth: true
                text: "Middle"
                onClicked: editor.alignActiveLayer(0, 2, alignTarget.currentIndex === 0)
            }
            Button {
                Layout.fillWidth: true
                text: "Bottom"
                onClicked: editor.alignActiveLayer(0, 3, alignTarget.currentIndex === 0)
            }
        }
        }
        OptionSection {
            title: "WAND"
            shown: optionsScroll.app.activeTool === "wand"
        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Tolerance"
                Layout.preferredWidth: 72
            }
            TokenSlider {
                Layout.fillWidth: true
                from: 0
                to: 255
                stepSize: 1
                value: optionsScroll.app.fillTolerance
                Accessible.name: "Wand tolerance 0 to 255"
                onMoved: optionsScroll.app.fillTolerance = Math.round(value)
            }
            Label {
                text: optionsScroll.app.fillTolerance
                Layout.preferredWidth: 36
                horizontalAlignment: Text.AlignRight
                font.features: { "tnum": 1 }
            }
        }
        CheckBox {
            objectName: "wandContiguousControl"
            text: "Contiguous"
            leftPadding: 0
            Layout.fillWidth: true
            checked: optionsScroll.app.wandContiguous
            onToggled: optionsScroll.app.wandContiguous = checked
            Accessible.name: "Wand contiguous region only"
        }
        }
        OptionSection {
            title: "SELECTION"
            shown: optionsScroll.app.activeTool === "rectangle" || optionsScroll.app.activeTool === "ellipse"
        ComboBox {
            Layout.fillWidth: true
            model: ["replace", "add", "subtract", "intersect"]
            currentIndex: model.indexOf(optionsScroll.app.selectionMode)
            Accessible.name: "Selection combination mode"
            onActivated: optionsScroll.app.selectionMode = currentText
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

        }
        OptionSection {
            title: "SHAPE"
            shown: optionsScroll.app.activeTool === "shape"
        ComboBox {
            objectName: "shapeKindControl"
            Layout.fillWidth: true
            model: ["rectangle", "rounded_rectangle", "ellipse", "line", "regular_polygon", "star"]
            currentIndex: model.indexOf(optionsScroll.app.shapeKind)
            Accessible.name: "Shape kind"
            onActivated: optionsScroll.app.shapeKind = currentText
        }
        RowLayout {
            Layout.fillWidth: true
            visible: optionsScroll.app.shapeKind === "regular_polygon" || optionsScroll.app.shapeKind === "star"
            Label {
                text: "Sides"
                color: optionsScroll.app.tokens.inkSecondary
            }
            SpinBox {
                objectName: "shapeSidesControl"
                Layout.fillWidth: true
                from: 3
                to: 512
                value: optionsScroll.app.shapeSides
                onValueModified: optionsScroll.app.shapeSides = value
                Accessible.name: "Shape side count"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: optionsScroll.app.shapeKind === "star"
            Label {
                text: "Inner"
                color: optionsScroll.app.tokens.inkSecondary
            }
            TokenSlider {
                Layout.fillWidth: true
                from: 0.05
                to: 0.95
                value: optionsScroll.app.shapeInnerRatio
                onMoved: optionsScroll.app.shapeInnerRatio = value
                Accessible.name: "Star inner radius ratio"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: optionsScroll.app.shapeKind === "rounded_rectangle"
            Label {
                text: "Corner"
                color: optionsScroll.app.tokens.inkSecondary
            }
            SpinBox {
                Layout.fillWidth: true
                from: 0
                to: 512
                value: Math.round(optionsScroll.app.shapeCornerRadius)
                onValueModified: optionsScroll.app.shapeCornerRadius = value
                Accessible.name: "Rounded rectangle corner radius"
            }
        }
        Label {
            Layout.fillWidth: true
            text: "Drag on the canvas. Shapes use the vector fill and stroke colours."
            wrapMode: Text.Wrap
            color: optionsScroll.app.tokens.inkSecondary
            font.pixelSize: 11
        }

        }
        OptionSection {
            title: "GRADIENT"
            shown: optionsScroll.app.activeTool === "gradient"
        ComboBox {
            Layout.fillWidth: true
            model: ["linear", "radial"]
            currentIndex: optionsScroll.app.gradientKind === "radial" ? 1 : 0
            Accessible.name: "Gradient kind"
            onActivated: optionsScroll.app.gradientKind = currentText
        }
        RowLayout {
            Layout.fillWidth: true
            Button {
                text: "Start"
                Layout.fillWidth: true
                onClicked: optionsScroll.gradientStartPicker.open()
                Accessible.name: "Choose gradient start color"
                background: Rectangle {
                    radius: 5
                    color: optionsScroll.app.gradientStartColor
                    border.color: optionsScroll.app.tokens.borderStrong
                }
            }
            Button {
                text: "End"
                Layout.fillWidth: true
                onClicked: optionsScroll.gradientEndPicker.open()
                Accessible.name: "Choose gradient end color"
                background: Rectangle {
                    radius: 5
                    color: optionsScroll.app.gradientEndColor
                    border.color: optionsScroll.app.tokens.borderStrong
                }
            }
        }

        }
        OptionSection {
            title: "IMAGE"
            shown: optionsScroll.app.panelShown("image")
            collapsible: true
            expanded: false
            autoExpand: optionsScroll.app.activeTool === "crop" || optionsScroll.app.activeTool === "transform"
        ComboBox {
            id: samplingCombo
            Layout.fillWidth: true
            model: ["nearest", "bilinear"]
            currentIndex: optionsScroll.app.samplingMode === "bilinear" ? 1 : 0
            Accessible.name: "Transform sampling mode"
            onActivated: optionsScroll.app.samplingMode = currentText
        }
        GridLayout {
            columns: 4
            Layout.fillWidth: true
            Label {
                text: "Crop"
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: cropX
                text: "0"
                placeholderText: "Crop X"
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
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
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: cropW
                text: String(editor.documentWidth)
                placeholderText: "Crop width"
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
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
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: padLeft
                text: "0"
                placeholderText: "Pad left"
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
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
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: padRight
                text: "0"
                placeholderText: "Pad right"
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
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
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: resizeW
                text: String(editor.documentWidth)
                placeholderText: "Resize width"
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: resizeH
                text: String(editor.documentHeight)
                placeholderText: "Resize height"
            }
            Button {
                text: "Apply"
                onClicked: editor.resizeCanvas(Number(resizeW.text), Number(resizeH.text), optionsScroll.app.samplingMode)
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
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: m11
                text: "1"
                placeholderText: "m11"
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: m12
                text: "0"
                placeholderText: "m12"
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: tx
                text: "0"
                placeholderText: "translate X"
            }
            Item {
                Layout.preferredWidth: 1
                Layout.preferredHeight: 1
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: m21
                text: "0"
                placeholderText: "m21"
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
                id: m22
                text: "1"
                placeholderText: "m22"
            }
            NumericField {
                Layout.fillWidth: true
                Layout.minimumWidth: 44
                Layout.preferredWidth: 64
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
            onClicked: editor.transformActive(Number(m11.text), Number(m12.text), Number(m21.text), Number(m22.text), Number(tx.text), Number(ty.text), optionsScroll.app.samplingMode)
        }
        // Convenience transforms about the layer centre (C.9).
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Rotate"; Layout.preferredWidth: 60 }
            NumericField {
                id: rotDeg
                Layout.fillWidth: true
                text: "15"
                placeholderText: "degrees"
            }
            Button {
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.rotateActive(Number(rotDeg.text), optionsScroll.app.samplingMode)
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Scale"; Layout.preferredWidth: 60 }
            NumericField { id: scaleX; Layout.fillWidth: true; text: "1"; placeholderText: "x" }
            NumericField { id: scaleY; Layout.fillWidth: true; text: "1"; placeholderText: "y" }
            Button {
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.scaleActive(Number(scaleX.text), Number(scaleY.text), optionsScroll.app.samplingMode)
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Shear"; Layout.preferredWidth: 60 }
            NumericField { id: shearX; Layout.fillWidth: true; text: "0"; placeholderText: "x" }
            NumericField { id: shearY; Layout.fillWidth: true; text: "0"; placeholderText: "y" }
            Button {
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.shearActive(Number(shearX.text), Number(shearY.text), optionsScroll.app.samplingMode)
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Perspective"; Layout.fillWidth: true }
            // Full corner-handle dragging is a follow-up; this applies a
            // keystone (pull the top edge in by 25%) as a working perspective.
            Button {
                text: "Keystone"
                enabled: editor.activeNodeCanEditRaster
                onClicked: {
                    const w = editor.documentWidth;
                    const h = editor.documentHeight;
                    editor.perspectiveActive([w * 0.25, 0, w * 0.75, 0, w, h, 0, h], optionsScroll.app.samplingMode)
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "3D rotate"; Layout.preferredWidth: 60 }
            NumericField { id: rot3dX; Layout.fillWidth: true; text: "0"; placeholderText: "X°" }
            NumericField { id: rot3dY; Layout.fillWidth: true; text: "25"; placeholderText: "Y°" }
            NumericField { id: rot3dZ; Layout.fillWidth: true; text: "0"; placeholderText: "Z°" }
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Distance"; Layout.preferredWidth: 60 }
            NumericField { id: dist3d; Layout.fillWidth: true; text: "2"; placeholderText: "widths" }
            Button {
                text: "Apply 3D"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.transform3d(Number(rot3dX.text), Number(rot3dY.text), Number(rot3dZ.text), Number(dist3d.text), optionsScroll.app.samplingMode)
            }
        }

        }
        OptionSection {
            title: "ADJUSTMENTS"
            shown: optionsScroll.app.panelShown("adjustments")
            collapsible: true
            expanded: false
        // One adjustment at a time: pick it, set its values, Apply. Listing all nine
        // with their own Apply rows made the panel a wall of fields.
        // Every Apply keeps its objectName: main.cpp's smoke test finds each
        // one and checks it follows activeNodeCanEditRaster.
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
        }
        Button {
            // Smart patch (Krita): content-aware fill of the current selection.
            objectName: "smartPatchAction"
            text: "Smart patch (fill selection)"
            enabled: editor.activeNodeCanEditRaster
            Layout.fillWidth: true
            onClicked: editor.smartPatch(32)
        }
        ComboBox {
            id: adjustmentPicker
            Layout.fillWidth: true
            Layout.topMargin: 4
            Accessible.name: "Adjustment"
            model: ["Brightness / contrast", "Levels", "Curves", "Hue / saturation",
                "Gaussian blur", "Box blur", "Sharpen", "Threshold", "Posterize",
                "Motion blur", "Lens blur", "Edge detect", "Emboss", "Laplace",
                "Pixelize", "Waves", "Ripple", "Whirl-pinch", "Lens distortion",
                "RGB noise", "HSV noise", "Hurl", "Pick", "Spread",
                "Checkerboard", "Gradient map", "Plasma", "Solid noise", "Cell noise",
                "Color balance", "Color temperature", "Exposure", "Hue-chroma", "Saturation", "Dither",
                "Oilify", "Cartoon", "Soft glow", "Photocopy", "Apply canvas", "Cubism",
                "Bump map", "Displace", "Fractal trace", "Warp map",
                "Halftone", "Phong bump", "Palettize", "Normal map",
                "Lab adjust"]
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 0
            RowLayout {
                ParamLabel { text: "Brightness" }
                SpinBox {
                    id: brightness
                    from: -255
                    to: 255
                    value: 0
                    Layout.fillWidth: true
                    Accessible.name: "Brightness minus 255 to 255"
                }
            }
            RowLayout {
                ParamLabel { text: "Contrast" }
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
                text: "Apply"
                Accessible.name: "Apply brightness / contrast"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyBrightnessContrast(brightness.value, contrast.value)
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 1
            RowLayout {
                ParamLabel { text: "Input black" }
                SpinBox {
                    id: inputBlack
                    Layout.fillWidth: true
                    from: 0
                    to: 255
                    value: 0
                    Accessible.name: "Levels input black"
                }
            }
            RowLayout {
                ParamLabel { text: "Input white" }
                SpinBox {
                    id: inputWhite
                    Layout.fillWidth: true
                    from: 0
                    to: 255
                    value: 255
                    Accessible.name: "Levels input white"
                }
            }
            RowLayout {
                ParamLabel { text: "Gamma" }
                NumericField {
                    id: gamma
                    text: "1"
                    placeholderText: "Gamma 0.01–100"
                }
            }
            RowLayout {
                ParamLabel { text: "Output black" }
                SpinBox {
                    id: outputBlack
                    Layout.fillWidth: true
                    from: 0
                    to: 255
                    value: 0
                    Accessible.name: "Levels output black"
                }
            }
            RowLayout {
                ParamLabel { text: "Output white" }
                SpinBox {
                    id: outputWhite
                    Layout.fillWidth: true
                    from: 0
                    to: 255
                    value: 255
                    Accessible.name: "Levels output white"
                }
            }
            Button {
                objectName: "levelsAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply levels"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyLevels(inputBlack.value, inputWhite.value, Number(gamma.text), outputBlack.value, outputWhite.value)
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 2
            RowLayout {
                ParamLabel { text: "Shadows" }
                SpinBox {
                    id: curveQuarter
                    Layout.fillWidth: true
                    from: 0
                    to: 255
                    value: 64
                    Accessible.name: "Curves output at 25% input"
                    ToolTip.visible: hovered
                    ToolTip.text: "Output at 25% input"
                }
            }
            RowLayout {
                ParamLabel { text: "Midtones" }
                SpinBox {
                    id: curveMiddle
                    Layout.fillWidth: true
                    from: 0
                    to: 255
                    value: 128
                    Accessible.name: "Curves output at 50% input"
                    ToolTip.visible: hovered
                    ToolTip.text: "Output at 50% input"
                }
            }
            RowLayout {
                ParamLabel { text: "Highlights" }
                SpinBox {
                    id: curveThreeQuarter
                    Layout.fillWidth: true
                    from: 0
                    to: 255
                    value: 191
                    Accessible.name: "Curves output at 75% input"
                    ToolTip.visible: hovered
                    ToolTip.text: "Output at 75% input"
                }
            }
            Button {
                objectName: "curvesAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply curves"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyCurves(curveQuarter.value, curveMiddle.value, curveThreeQuarter.value)
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 3
            RowLayout {
                ParamLabel { text: "Hue" }
                SpinBox {
                    id: hue
                    Layout.fillWidth: true
                    from: -180
                    to: 180
                    value: 0
                    Accessible.name: "Hue degrees"
                }
            }
            RowLayout {
                ParamLabel { text: "Saturation" }
                SpinBox {
                    id: saturation
                    Layout.fillWidth: true
                    from: -100
                    to: 100
                    value: 0
                    Accessible.name: "Saturation"
                }
            }
            RowLayout {
                ParamLabel { text: "Lightness" }
                SpinBox {
                    id: lightness
                    Layout.fillWidth: true
                    from: -100
                    to: 100
                    value: 0
                    Accessible.name: "Lightness"
                }
            }
            Button {
                objectName: "hueSaturationAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply hue / saturation / lightness"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyHueSaturation(hue.value, saturation.value, lightness.value)
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 4
            RowLayout {
                ParamLabel { text: "Radius (σ)" }
                NumericField {
                    id: sigma
                    text: "4"
                    placeholderText: "Gaussian sigma 0–1024"
                }
            }
            Button {
                objectName: "gaussianBlurAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply Gaussian blur"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyGaussianBlur(Number(sigma.text))
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 5
            RowLayout {
                ParamLabel { text: "Radius" }
                SpinBox {
                    id: boxRadius
                    from: 1
                    to: 4096
                    value: 3
                    Layout.fillWidth: true
                    Accessible.name: "Box blur radius 1 to 4096"
                }
            }
            Button {
                objectName: "boxBlurAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply box blur"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyBoxBlur(boxRadius.value)
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 6
            RowLayout {
                ParamLabel { text: "Amount" }
                NumericField {
                    id: sharpenAmount
                    text: "1"
                    placeholderText: "Sharpen amount 0–10"
                }
            }
            Button {
                objectName: "sharpenAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply sharpen"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applySharpen(Number(sharpenAmount.text))
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 7
            RowLayout {
                ParamLabel { text: "Level" }
                SpinBox {
                    id: threshold
                    from: 0
                    to: 255
                    value: 128
                    Layout.fillWidth: true
                    Accessible.name: "Threshold 0 to 255"
                }
            }
            Button {
                objectName: "thresholdAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply threshold"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyThreshold(threshold.value)
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 8
            RowLayout {
                ParamLabel { text: "Levels" }
                SpinBox {
                    id: posterize
                    from: 2
                    to: 256
                    value: 8
                    Layout.fillWidth: true
                    Accessible.name: "Posterize levels 2 to 256"
                }
            }
            Button {
                objectName: "posterizeAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply posterize"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyPosterize(posterize.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 9
            RowLayout {
                ParamLabel { text: "Angle°" }
                NumericField { id: motionAngle; text: "0"; placeholderText: "angle" }
                ParamLabel { text: "Distance" }
                SpinBox { id: motionDist; from: 1; to: 512; value: 16; Layout.fillWidth: true }
            }
            Button {
                objectName: "motionBlurAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply motion blur"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyMotionBlur(Number(motionAngle.text), motionDist.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 10
            RowLayout {
                ParamLabel { text: "Radius" }
                SpinBox { id: lensRadius; from: 1; to: 256; value: 8; Layout.fillWidth: true }
            }
            Button {
                objectName: "lensBlurAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply lens blur"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyLensBlur(lensRadius.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 11
            RowLayout {
                ParamLabel { text: "Amount" }
                NumericField { id: edgeAmount; text: "1"; placeholderText: "0–10" }
            }
            Button {
                objectName: "edgeDetectAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply edge detect"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyEdgeDetect(Number(edgeAmount.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 12
            RowLayout {
                ParamLabel { text: "Light angle°" }
                NumericField { id: embossAngle; text: "135"; placeholderText: "angle" }
            }
            Button {
                objectName: "embossAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply emboss"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyEmboss(Number(embossAngle.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 13
            Button {
                objectName: "laplaceAction"
                Layout.fillWidth: true
                text: "Apply Laplace"
                Accessible.name: "Apply Laplace edge"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyLaplace()
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 14
            RowLayout {
                ParamLabel { text: "Block" }
                SpinBox { id: pixelBlock; from: 1; to: 256; value: 8; Layout.fillWidth: true }
            }
            Button {
                objectName: "pixelizeAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply pixelize"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyPixelize(pixelBlock.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 15
            RowLayout {
                ParamLabel { text: "Amplitude" }
                NumericField { id: wavesAmp; text: "6"; placeholderText: "px" }
                ParamLabel { text: "Wavelength" }
                NumericField { id: wavesWl; text: "20"; placeholderText: "px" }
            }
            Button {
                objectName: "wavesAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply waves"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyWaves(Number(wavesAmp.text), Number(wavesWl.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 16
            RowLayout {
                ParamLabel { text: "Amplitude" }
                NumericField { id: rippleAmp; text: "6"; placeholderText: "px" }
                ParamLabel { text: "Wavelength" }
                NumericField { id: rippleWl; text: "20"; placeholderText: "px" }
            }
            CheckBox {
                id: rippleHoriz
                text: "Horizontal"
                checked: true
            }
            Button {
                objectName: "rippleAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply ripple"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyRipple(Number(rippleAmp.text), Number(rippleWl.text), rippleHoriz.checked)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 17
            RowLayout {
                ParamLabel { text: "Whirl°" }
                NumericField { id: whirlDeg; text: "90"; placeholderText: "degrees" }
                ParamLabel { text: "Pinch" }
                NumericField { id: pinchAmt; text: "0.3"; placeholderText: "-1..1" }
            }
            Button {
                objectName: "whirlPinchAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply whirl-pinch"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyWhirlPinch(Number(whirlDeg.text), Number(pinchAmt.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 18
            RowLayout {
                ParamLabel { text: "Amount" }
                NumericField { id: lensDistAmt; text: "30"; placeholderText: "-100..100" }
            }
            Button {
                objectName: "lensDistortionAction"
                Layout.fillWidth: true
                text: "Apply"
                Accessible.name: "Apply lens distortion"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyLensDistortion(Number(lensDistAmt.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 19
            RowLayout {
                ParamLabel { text: "Amount" }
                NumericField { id: rgbNoiseAmt; text: "0.2"; placeholderText: "0–1" }
                ParamLabel { text: "Seed" }
                SpinBox { id: rgbNoiseSeed; from: 0; to: 99999; value: 1; Layout.fillWidth: true }
            }
            Button {
                objectName: "rgbNoiseAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyRgbNoise(Number(rgbNoiseAmt.text), rgbNoiseSeed.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 20
            RowLayout {
                ParamLabel { text: "Hue" }
                NumericField { id: hsvNoiseH; text: "0.1"; placeholderText: "0–1" }
                ParamLabel { text: "Sat" }
                NumericField { id: hsvNoiseS; text: "0.1"; placeholderText: "0–1" }
                ParamLabel { text: "Val" }
                NumericField { id: hsvNoiseV; text: "0.1"; placeholderText: "0–1" }
            }
            RowLayout {
                ParamLabel { text: "Seed" }
                SpinBox { id: hsvNoiseSeed; from: 0; to: 99999; value: 1; Layout.fillWidth: true }
            }
            Button {
                objectName: "hsvNoiseAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyHsvNoise(Number(hsvNoiseH.text), Number(hsvNoiseS.text), Number(hsvNoiseV.text), hsvNoiseSeed.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 21
            RowLayout {
                ParamLabel { text: "Amount" }
                NumericField { id: hurlAmt; text: "0.1"; placeholderText: "0–1" }
                ParamLabel { text: "Seed" }
                SpinBox { id: hurlSeed; from: 0; to: 99999; value: 1; Layout.fillWidth: true }
            }
            Button {
                objectName: "hurlAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyHurl(Number(hurlAmt.text), hurlSeed.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 22
            RowLayout {
                ParamLabel { text: "Amount" }
                NumericField { id: pickAmt; text: "0.3"; placeholderText: "0–1" }
                ParamLabel { text: "Seed" }
                SpinBox { id: pickSeed; from: 0; to: 99999; value: 1; Layout.fillWidth: true }
            }
            Button {
                objectName: "pickAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyPick(Number(pickAmt.text), pickSeed.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 23
            RowLayout {
                ParamLabel { text: "Amount" }
                SpinBox { id: spreadAmt; from: 0; to: 256; value: 5; Layout.fillWidth: true }
                ParamLabel { text: "Seed" }
                SpinBox { id: spreadSeed; from: 0; to: 99999; value: 1; Layout.fillWidth: true }
            }
            Button {
                objectName: "spreadAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applySpread(spreadAmt.value, spreadSeed.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 24
            RowLayout {
                ParamLabel { text: "Size" }
                SpinBox { id: checkerSize; from: 1; to: 256; value: 16; Layout.fillWidth: true }
            }
            Button {
                objectName: "checkerboardAction"
                Layout.fillWidth: true
                text: "Fill checkerboard (brush + white)"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyCheckerboard(checkerSize.value, editor.brushColor, Qt.rgba(1, 1, 1, 1))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 25
            Button {
                objectName: "gradientMapAction"
                Layout.fillWidth: true
                text: "Map luma → black…brush"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyGradientMap(Qt.rgba(0, 0, 0, 1), editor.brushColor)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 26
            RowLayout {
                ParamLabel { text: "Turbulence" }
                NumericField { id: plasmaTurb; text: "1.5"; placeholderText: "0.1–10" }
                ParamLabel { text: "Seed" }
                SpinBox { id: plasmaSeed; from: 0; to: 99999; value: 1; Layout.fillWidth: true }
            }
            Button {
                objectName: "plasmaAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyPlasma(Number(plasmaTurb.text), plasmaSeed.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 27
            RowLayout {
                ParamLabel { text: "Detail" }
                SpinBox { id: solidDetail; from: 1; to: 8; value: 4; Layout.fillWidth: true }
                ParamLabel { text: "Seed" }
                SpinBox { id: solidSeed; from: 0; to: 99999; value: 1; Layout.fillWidth: true }
            }
            Button {
                objectName: "solidNoiseAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applySolidNoise(solidDetail.value, solidSeed.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 28
            RowLayout {
                ParamLabel { text: "Density" }
                SpinBox { id: cellDensity; from: 1; to: 128; value: 8; Layout.fillWidth: true }
                ParamLabel { text: "Seed" }
                SpinBox { id: cellSeed; from: 0; to: 99999; value: 1; Layout.fillWidth: true }
            }
            Button {
                objectName: "cellNoiseAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyCellNoise(cellDensity.value, cellSeed.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 29
            RowLayout {
                ParamLabel { text: "R" }
                NumericField { id: cbR; text: "0"; placeholderText: "-100..100" }
                ParamLabel { text: "G" }
                NumericField { id: cbG; text: "0"; placeholderText: "-100..100" }
                ParamLabel { text: "B" }
                NumericField { id: cbB; text: "0"; placeholderText: "-100..100" }
            }
            Button {
                objectName: "colorBalanceAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyColorBalance(Number(cbR.text), Number(cbG.text), Number(cbB.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 30
            RowLayout {
                ParamLabel { text: "Warm↔Cool" }
                NumericField { id: tempAmt; text: "0"; placeholderText: "-100..100" }
            }
            Button {
                objectName: "colorTemperatureAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyColorTemperature(Number(tempAmt.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 31
            RowLayout {
                ParamLabel { text: "Stops" }
                NumericField { id: exposureStops; text: "0"; placeholderText: "-10..10" }
            }
            Button {
                objectName: "exposureAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyExposure(Number(exposureStops.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 32
            RowLayout {
                ParamLabel { text: "Hue°" }
                NumericField { id: hueChromaH; text: "0"; placeholderText: "degrees" }
                ParamLabel { text: "Chroma" }
                NumericField { id: hueChromaC; text: "0"; placeholderText: "-100..100" }
            }
            Button {
                objectName: "hueChromaAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyHueChroma(Number(hueChromaH.text), Number(hueChromaC.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 33
            RowLayout {
                ParamLabel { text: "Scale" }
                NumericField { id: satScale; text: "1"; placeholderText: "0–4" }
            }
            Button {
                objectName: "saturationAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applySaturation(Number(satScale.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 34
            RowLayout {
                ParamLabel { text: "Levels" }
                SpinBox { id: ditherLevels; from: 2; to: 64; value: 4; Layout.fillWidth: true }
            }
            Button {
                objectName: "ditherAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyDither(ditherLevels.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 35
            RowLayout {
                ParamLabel { text: "Radius" }
                SpinBox { id: oilifyRadius; from: 1; to: 32; value: 4; Layout.fillWidth: true }
            }
            Button {
                objectName: "oilifyAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyOilify(oilifyRadius.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 36
            RowLayout {
                ParamLabel { text: "Amount" }
                NumericField { id: cartoonAmt; text: "1.5"; placeholderText: "0–10" }
            }
            Button {
                objectName: "cartoonAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyCartoon(Number(cartoonAmt.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 37
            RowLayout {
                ParamLabel { text: "Radius" }
                SpinBox { id: glowRadius; from: 1; to: 64; value: 8; Layout.fillWidth: true }
                ParamLabel { text: "Amount" }
                NumericField { id: glowAmt; text: "0.5"; placeholderText: "0–1" }
            }
            Button {
                objectName: "softGlowAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applySoftGlow(glowRadius.value, Number(glowAmt.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 38
            RowLayout {
                ParamLabel { text: "Amount" }
                NumericField { id: photocopyAmt; text: "1.5"; placeholderText: "0–10" }
            }
            Button {
                objectName: "photocopyAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyPhotocopy(Number(photocopyAmt.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 39
            RowLayout {
                ParamLabel { text: "Depth" }
                NumericField { id: canvasDepth; text: "0.5"; placeholderText: "0–1" }
            }
            Button {
                objectName: "applyCanvasAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyApplyCanvas(Number(canvasDepth.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 40
            RowLayout {
                ParamLabel { text: "Tile" }
                SpinBox { id: cubismTile; from: 2; to: 64; value: 12; Layout.fillWidth: true }
                ParamLabel { text: "Seed" }
                SpinBox { id: cubismSeed; from: 0; to: 99999; value: 1; Layout.fillWidth: true }
            }
            Button {
                objectName: "cubismAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyCubism(cubismTile.value, cubismSeed.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 41
            RowLayout {
                ParamLabel { text: "Azimuth°" }
                NumericField { id: bumpAz; text: "135"; placeholderText: "deg" }
                ParamLabel { text: "Elev°" }
                NumericField { id: bumpEl; text: "45"; placeholderText: "deg" }
            }
            RowLayout {
                ParamLabel { text: "Depth" }
                NumericField { id: bumpDepth; text: "4"; placeholderText: "0–100" }
            }
            Button {
                objectName: "bumpMapAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyBumpMap(Number(bumpAz.text), Number(bumpEl.text), Number(bumpDepth.text), mapLayerPicker.currentValue)
            }
        }
        // Shared map-layer picker for the four map filters (H.18). One row rather
        // than four copies: the choice means the same thing in each, and a bump
        // map is a SEPARATE grey image — shading a picture by its own brightness
        // lights its content instead of its surface.
        RowLayout {
            Layout.fillWidth: true
            visible: adjustmentPicker.currentIndex >= 41 && adjustmentPicker.currentIndex <= 44
            ParamLabel { text: "Map" }
            ComboBox {
                id: mapLayerPicker
                Layout.fillWidth: true
                // Index 0 is the self-map, which is what these filters did before
                // a map layer could be named.
                textRole: "text"
                valueRole: "value"
                model: {
                    var entries = [{ text: "This layer", value: "" }];
                    for (var i = 0; i < editor.layers.rowCount(); ++i) {
                        var index = editor.layers.index(i, 0);
                        entries.push({
                            text: editor.layers.data(index, Qt.UserRole + 1),
                            value: editor.layers.data(index, Qt.UserRole)
                        });
                    }
                    return entries;
                }
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 42
            RowLayout {
                ParamLabel { text: "Amount" }
                NumericField { id: displaceAmt; text: "20"; placeholderText: "px" }
            }
            Button {
                objectName: "displaceAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyDisplace(Number(displaceAmt.text), mapLayerPicker.currentValue)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 43
            RowLayout {
                ParamLabel { text: "Depth" }
                SpinBox { id: fractalDepth; from: 1; to: 32; value: 3; Layout.fillWidth: true }
                ParamLabel { text: "Scale" }
                NumericField { id: fractalScale; text: "1"; placeholderText: "zoom" }
            }
            Button {
                objectName: "fractalTraceAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyFractalTrace(fractalDepth.value, Number(fractalScale.text), mapLayerPicker.currentValue)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 44
            RowLayout {
                ParamLabel { text: "Amount" }
                NumericField { id: warpMapAmt; text: "20"; placeholderText: "px" }
                ParamLabel { text: "Steps" }
                SpinBox { id: warpMapSteps; from: 1; to: 32; value: 4; Layout.fillWidth: true }
            }
            Button {
                objectName: "warpMapAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyWarpMap(Number(warpMapAmt.text), warpMapSteps.value, mapLayerPicker.currentValue)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 45
            RowLayout {
                ParamLabel { text: "Cell" }
                SpinBox { id: halftoneCell; from: 2; to: 64; value: 8; Layout.fillWidth: true }
            }
            Button {
                objectName: "halftoneAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyHalftone(halftoneCell.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 46
            RowLayout {
                ParamLabel { text: "Azimuth°" }
                NumericField { id: phongAz; text: "135"; placeholderText: "deg" }
                ParamLabel { text: "Elev°" }
                NumericField { id: phongEl; text: "45"; placeholderText: "deg" }
            }
            RowLayout {
                ParamLabel { text: "Depth" }
                NumericField { id: phongDepth; text: "4"; placeholderText: "0–100" }
                ParamLabel { text: "Shiny" }
                NumericField { id: phongShiny; text: "16"; placeholderText: "1–128" }
            }
            Button {
                objectName: "phongBumpAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyPhongBump(Number(phongAz.text), Number(phongEl.text), Number(phongDepth.text), Number(phongShiny.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 47
            RowLayout {
                ParamLabel { text: "Levels" }
                SpinBox { id: palettizeLevels; from: 2; to: 64; value: 6; Layout.fillWidth: true }
            }
            Button {
                objectName: "palettizeAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyPalettize(palettizeLevels.value)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 48
            RowLayout {
                ParamLabel { text: "Strength" }
                NumericField { id: normalStrength; text: "4"; placeholderText: "height scale" }
            }
            Button {
                objectName: "normalMapAction"
                Layout.fillWidth: true
                text: "Apply"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyNormalMap(Number(normalStrength.text))
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: adjustmentPicker.currentIndex === 49
            RowLayout {
                ParamLabel { text: "Lightness" }
                NumericField { id: labLightness; text: "0"; placeholderText: "-100..100" }
                ParamLabel { text: "Chroma" }
                NumericField { id: labChroma; text: "1"; placeholderText: "0..4" }
            }
            Button {
                objectName: "labAdjustAction"
                Layout.fillWidth: true
                text: "Apply (CIE Lab)"
                enabled: editor.activeNodeCanEditRaster
                onClicked: editor.applyLabAdjust(Number(labLightness.text), Number(labChroma.text))
            }
        }
        }
        OptionSection {
            title: "ACTIVE LAYER"
            collapsible: true
            expanded: false
        ComboBox {
            id: blendMode
            Layout.fillWidth: true
            model: ["normal", "multiply", "screen", "overlay", "add", "darken_only", "lighten_only", "luma_darken_only", "luma_lighten_only", "dodge", "burn", "linear_burn", "linear_light", "vivid_light", "pin_light", "hard_mix", "hard_light", "soft_light", "grain_extract", "grain_merge", "difference", "exclusion", "subtract", "divide", "hsv_hue", "hsv_saturation", "hsv_value", "hsl_color", "lch_hue", "lch_chroma", "lch_color", "lch_lightness", "luminance", "dissolve", "behind", "erase", "anti_erase", "color_erase", "replace", "overwrite", "pass_through", "merge", "split"]
            Accessible.name: "Active layer blend mode"
        }
        Button {
            Layout.fillWidth: true
            text: "Set blend mode"
            onClicked: editor.setLayerBlendMode(editor.activeLayerId, blendMode.currentText)
        }
        Button {
            // Moved here from Adjustments: it empties the layer, it does not adjust it.
            objectName: "clearLayerAction"
            Layout.fillWidth: true
            text: "Clear layer"
            enabled: editor.activeNodeCanEditRaster
            onClicked: editor.clearActiveLayer()
        }
        }
        Item {
            Layout.preferredHeight: 12
        }
    }
}
