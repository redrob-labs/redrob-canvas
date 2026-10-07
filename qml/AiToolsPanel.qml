// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs

// The AI tab: IOPaint's tools (Sanster/IOPaint 1.6.0, Apache-2.0) on this computer. Erase or
// replace inside the selection with any of its models, expand the canvas, remove the background,
// upscale, restore faces, and select a subject by clicking it. Results arrive as new layers or as
// the selection, so the original pixels stay and Undo takes a result back.
ScrollView {
    id: ai
    objectName: "aiToolsPanel"
    required property var app
    readonly property var tokens: app.tokens
    readonly property var engine: editor.iopaint
    readonly property bool ready: engine.state === "ready"
    readonly property bool busy: engine.state === "busy" || engine.state === "starting" || engine.state === "installing"
    readonly property var info: engine.modelInfo
    readonly property bool diffusion: engine.diffusionModels.indexOf(engine.model) >= 0
    readonly property bool needPrompt: diffusion || (info && info.need_prompt === true)
    clip: true
    contentWidth: availableWidth
    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

    function options() {
        const o = {}
        if (ai.needPrompt) {
            o.prompt = promptField.text
            o.negative_prompt = negativeField.text
            o.sd_steps = stepsBox.value
            o.sd_strength = strengthSlider.value
            o.sd_guidance_scale = guidanceSlider.value
            o.sd_seed = seedBox.value
            o.sd_keep_unmasked_area = keepUnmasked.checked
            o.sd_match_histograms = matchHist.checked
            o.sd_lcm_lora = lcmBox.checked
            if (samplerBox.currentText.length > 0)
                o.sd_sampler = samplerBox.currentText
            if (engine.model.indexOf("PowerPaint") >= 0)
                o.powerpaint_task = powerTask.currentText
            if (controlnetBox.checked && ai.info && ai.info.controlnets && ai.info.controlnets.length > 0) {
                o.enable_controlnet = true
                o.controlnet_method = controlnetMethod.currentText
            }
            if (brushnetBox.checked && ai.info && ai.info.brushnets && ai.info.brushnets.length > 0) {
                o.enable_brushnet = true
                o.brushnet_method = brushnetMethod.currentText
            }
        }
        return o
    }

    FileDialog {
        id: exampleDialog
        popupType: Popup.Item
        title: "Example image"
        nameFilters: ["Images (*.png *.jpg *.jpeg *.webp *.bmp)"]
        onAccepted: ai.engine.paintByExample(selectedFile, ai.options())
    }
    FolderDialog {
        id: batchImages
        popupType: Popup.Item
        title: "Folder of images"
    }
    FolderDialog {
        id: batchMasks
        popupType: Popup.Item
        title: "Folder of masks (same file names)"
    }
    FolderDialog {
        id: batchOutput
        popupType: Popup.Item
        title: "Folder for the results"
    }

    ColumnLayout {
        x: 12
        width: ai.availableWidth - 24
        spacing: 8

        SectionTitle { text: "AI ENGINE" }
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: engineRow.implicitHeight + 16
            radius: 8
            color: ai.tokens.surfaceSunken
            border.color: ai.tokens.borderSubtle
            RowLayout {
                id: engineRow
                anchors.fill: parent
                anchors.margins: 8
                Rectangle {
                    Layout.preferredWidth: 8
                    Layout.preferredHeight: 8
                    radius: 4
                    color: ai.ready ? ai.tokens.statusSuccess
                         : ai.busy ? ai.tokens.statusWarning
                         : ai.engine.state === "error" ? ai.tokens.statusDanger : ai.tokens.borderSubtle
                }
                Label {
                    objectName: "aiEngineMessage"
                    Layout.fillWidth: true
                    text: ai.engine.message
                    color: ai.tokens.inkPrimary
                    wrapMode: Text.Wrap
                    font.pixelSize: 11
                }
                BusyIndicator {
                    visible: ai.busy
                    running: ai.busy
                    Layout.preferredWidth: 20
                    Layout.preferredHeight: 20
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Button {
                objectName: "aiInstall"
                Layout.fillWidth: true
                visible: ai.engine.state === "missing"
                text: "Install AI engine"
                onClicked: ai.engine.install()
            }
            Button {
                objectName: "aiStart"
                Layout.fillWidth: true
                visible: ai.engine.state === "stopped" || ai.engine.state === "error"
                text: "Start AI engine"
                onClicked: ai.engine.start()
            }
            Button {
                objectName: "aiStop"
                Layout.fillWidth: true
                visible: ai.ready || ai.engine.state === "starting"
                text: "Stop"
                onClicked: ai.engine.stopServer()
            }
            Button {
                objectName: "aiCancel"
                visible: ai.engine.state === "busy" || ai.engine.state === "installing"
                text: "Cancel"
                onClicked: ai.engine.cancel()
            }
        }
        Label {
            Layout.fillWidth: true
            visible: ai.engine.state === "missing"
            text: ai.engine.installMethod.length > 0
                ? "Installs IOPaint 1.6.0 and PyTorch into this app's own folder with " + ai.engine.installMethod
                  + ". About 1.5 GB; models download on first use."
                : "Needs uv (docs.astral.sh/uv) or Python 3.11 on this computer first."
            color: ai.tokens.inkSecondary
            wrapMode: Text.Wrap
            font.pixelSize: 11
        }
        RowLayout {
            Layout.fillWidth: true
            Label { text: "Device"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
            ComboBox {
                objectName: "aiDevice"
                Layout.fillWidth: true
                model: ["cpu", "cuda", "mps"]
                currentIndex: Math.max(0, model.indexOf(ai.engine.device))
                onActivated: ai.engine.device = currentText
            }
        }
        CheckBox {
            id: showLog
            text: "Show engine log"
        }
        Label {
            objectName: "aiEngineLog"
            Layout.fillWidth: true
            visible: showLog.checked && ai.engine.log.length > 0
            text: ai.engine.log.slice(-10).join("\n")
            color: ai.tokens.inkSecondary
            wrapMode: Text.WrapAnywhere
            font.family: "monospace"
            font.pixelSize: 10
        }

        SectionTitle { text: "MODEL" }
        ComboBox {
            id: modelBox
            objectName: "aiModel"
            Layout.fillWidth: true
            model: ai.engine.eraseModels.concat(ai.engine.diffusionModels)
            currentIndex: Math.max(0, model.indexOf(ai.engine.model))
            enabled: !ai.busy
            displayText: currentText + (ai.engine.downloadedModels.indexOf(currentText) >= 0 ? "" : "  (downloads)")
            onActivated: ai.engine.setModel(currentText)
            Accessible.name: "AI model"
        }
        Label {
            Layout.fillWidth: true
            text: ai.diffusion
                ? "Diffusion model: replaces the selection from a prompt. Slow on a CPU; several GB to download."
                : "Erase model: removes what is selected and fills it in from the surroundings."
            color: ai.tokens.inkSecondary
            wrapMode: Text.Wrap
            font.pixelSize: 11
        }

        SectionTitle { text: ai.needPrompt ? "REPLACE SELECTION" : "ERASE SELECTION" }
        ColumnLayout {
            Layout.fillWidth: true
            visible: ai.needPrompt
            spacing: 6
            TextArea {
                id: promptField
                objectName: "aiPrompt"
                Layout.fillWidth: true
                Layout.preferredHeight: 60
                placeholderText: "What should be there, e.g. \"a wooden bench\""
                wrapMode: TextEdit.Wrap
                font.pixelSize: 12
            }
            TextField {
                id: negativeField
                Layout.fillWidth: true
                placeholderText: "Leave out (negative prompt)"
                font.pixelSize: 12
            }
            RowLayout {
                Layout.fillWidth: true
                visible: ai.engine.model.indexOf("PowerPaint") >= 0
                Label { text: "Task"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
                ComboBox {
                    id: powerTask
                    Layout.fillWidth: true
                    model: ["text-guided", "object-remove", "context-aware", "shape-guided", "outpainting"]
                }
            }
            GridLayout {
                Layout.fillWidth: true
                columns: 2
                Label { text: "Steps"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
                SpinBox { id: stepsBox; from: 1; to: 100; value: 20; editable: true; Layout.fillWidth: true }
                Label { text: "Strength"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
                Slider { id: strengthSlider; from: 0.1; to: 1.0; value: 1.0; Layout.fillWidth: true }
                Label { text: "Guidance"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
                Slider { id: guidanceSlider; from: 1; to: 20; value: 7.5; Layout.fillWidth: true }
                Label { text: "Seed"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
                SpinBox { id: seedBox; from: -1; to: 2147483647; value: -1; editable: true; Layout.fillWidth: true }
                Label { text: "Sampler"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
                ComboBox { id: samplerBox; model: ai.engine.samplers; Layout.fillWidth: true }
            }
            CheckBox { id: keepUnmasked; checked: true; text: "Keep everything outside the selection" }
            CheckBox { id: matchHist; text: "Match the picture's colours" }
            CheckBox {
                id: lcmBox
                visible: ai.info && ai.info.support_lcm_lora === true
                text: "Fast (LCM LoRA, 2 to 8 steps)"
            }
            RowLayout {
                Layout.fillWidth: true
                visible: ai.info && ai.info.support_controlnet === true
                CheckBox { id: controlnetBox; text: "ControlNet" }
                ComboBox {
                    id: controlnetMethod
                    Layout.fillWidth: true
                    enabled: controlnetBox.checked
                    model: ai.info && ai.info.controlnets ? ai.info.controlnets : []
                }
            }
            RowLayout {
                Layout.fillWidth: true
                visible: ai.info && ai.info.support_brushnet === true
                CheckBox { id: brushnetBox; text: "BrushNet" }
                ComboBox {
                    id: brushnetMethod
                    Layout.fillWidth: true
                    enabled: brushnetBox.checked
                    model: ai.info && ai.info.brushnets ? ai.info.brushnets : []
                }
            }
        }
        Button {
            objectName: "aiInpaint"
            Layout.fillWidth: true
            enabled: ai.ready && editor.selectionActive
            text: editor.selectionActive ? (ai.needPrompt ? "Replace selection" : "Erase selection")
                                         : "Select an area first"
            highlighted: true
            onClicked: ai.engine.inpaint(ai.options())
        }
        Button {
            objectName: "aiPaintByExample"
            Layout.fillWidth: true
            visible: ai.engine.model.indexOf("Paint-by-Example") >= 0
            enabled: ai.ready && editor.selectionActive
            text: "Fill selection from an example image…"
            onClicked: exampleDialog.open()
        }

        SectionTitle { text: "EXPAND CANVAS" }
        GridLayout {
            Layout.fillWidth: true
            columns: 4
            Label { text: "Left"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
            SpinBox { id: padLeft; from: 0; to: 4096; stepSize: 32; editable: true; Layout.fillWidth: true }
            Label { text: "Right"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
            SpinBox { id: padRight; from: 0; to: 4096; stepSize: 32; editable: true; Layout.fillWidth: true }
            Label { text: "Top"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
            SpinBox { id: padTop; from: 0; to: 4096; stepSize: 32; editable: true; Layout.fillWidth: true }
            Label { text: "Bottom"; color: ai.tokens.inkSecondary; font.pixelSize: 11 }
            SpinBox { id: padBottom; from: 0; to: 4096; stepSize: 32; editable: true; Layout.fillWidth: true }
        }
        Button {
            objectName: "aiOutpaint"
            Layout.fillWidth: true
            enabled: ai.ready && padLeft.value + padRight.value + padTop.value + padBottom.value > 0
            text: "Expand and fill the new edges"
            onClicked: ai.engine.outpaint(padLeft.value, padTop.value, padRight.value, padBottom.value, ai.options())
        }

        SectionTitle { text: "SELECT WITH AI" }
        Button {
            objectName: "aiClickSelect"
            Layout.fillWidth: true
            enabled: ai.ready && ai.engine.plugins.indexOf("InteractiveSeg") >= 0
            checkable: true
            checked: ai.app.activeTool === "aiselect"
            text: checked ? "Click the subject · Alt+click to leave out" : "Click to select a subject"
            onClicked: {
                ai.app.aiClicks = []
                ai.app.activeTool = checked ? "aiselect" : "rectangle"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: ai.app.activeTool === "aiselect"
            Label {
                Layout.fillWidth: true
                text: ai.app.aiClicks.length + " click(s)"
                color: ai.tokens.inkSecondary
                font.pixelSize: 11
            }
            Button {
                text: "Start over"
                onClicked: {
                    ai.app.aiClicks = []
                    editor.clearSelection()
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Button {
                objectName: "aiSelectSubject"
                Layout.fillWidth: true
                enabled: ai.ready && ai.engine.plugins.indexOf("RemoveBG") >= 0
                text: "Select subject"
                onClicked: ai.engine.pluginMask("RemoveBG", ai.app.selectionMode)
            }
            Button {
                Layout.fillWidth: true
                enabled: ai.ready && ai.engine.plugins.indexOf("AnimeSeg") >= 0
                text: "Select anime subject"
                onClicked: ai.engine.pluginMask("AnimeSeg", ai.app.selectionMode)
            }
        }

        SectionTitle { text: "ONE-CLICK" }
        Button {
            objectName: "aiRemoveBackground"
            Layout.fillWidth: true
            enabled: ai.ready && ai.engine.plugins.indexOf("RemoveBG") >= 0
            text: "Remove background"
            onClicked: ai.engine.runPlugin("RemoveBG", 1)
        }
        ComboBox {
            Layout.fillWidth: true
            visible: ai.engine.plugins.indexOf("RemoveBG") >= 0
            enabled: ai.ready
            model: ai.engine.pluginModels.RemoveBG ? ai.engine.pluginModels.RemoveBG.choices : []
            currentIndex: ai.engine.pluginModels.RemoveBG ? Math.max(0, model.indexOf(ai.engine.pluginModels.RemoveBG.current)) : 0
            onActivated: ai.engine.switchPluginModel("RemoveBG", currentText)
        }
        Button {
            Layout.fillWidth: true
            enabled: ai.ready && ai.engine.plugins.indexOf("AnimeSeg") >= 0
            text: "Anime cut-out"
            onClicked: ai.engine.runPlugin("AnimeSeg", 1)
        }
        RowLayout {
            Layout.fillWidth: true
            Button {
                objectName: "aiUpscale"
                Layout.fillWidth: true
                enabled: ai.ready && ai.engine.plugins.indexOf("RealESRGAN") >= 0
                text: "Upscale ×" + upscaleBox.currentText
                onClicked: ai.engine.runPlugin("RealESRGAN", Number(upscaleBox.currentText))
            }
            ComboBox { id: upscaleBox; model: ["2", "3", "4"] }
        }
        ComboBox {
            Layout.fillWidth: true
            visible: ai.engine.plugins.indexOf("RealESRGAN") >= 0
            enabled: ai.ready
            model: ai.engine.pluginModels.RealESRGAN ? ai.engine.pluginModels.RealESRGAN.choices : []
            currentIndex: ai.engine.pluginModels.RealESRGAN ? Math.max(0, model.indexOf(ai.engine.pluginModels.RealESRGAN.current)) : 0
            onActivated: ai.engine.switchPluginModel("RealESRGAN", currentText)
        }
        RowLayout {
            Layout.fillWidth: true
            Button {
                Layout.fillWidth: true
                enabled: ai.ready && ai.engine.plugins.indexOf("GFPGAN") >= 0
                text: "Restore faces (GFPGAN)"
                onClicked: ai.engine.runPlugin("GFPGAN", 1)
            }
            Button {
                Layout.fillWidth: true
                enabled: ai.ready && ai.engine.plugins.indexOf("RestoreFormer") >= 0
                text: "RestoreFormer"
                onClicked: ai.engine.runPlugin("RestoreFormer", 1)
            }
        }

        SectionTitle { text: "PLUGINS" }
        Label {
            Layout.fillWidth: true
            text: "Each plugin downloads its model on the next engine start."
            color: ai.tokens.inkSecondary
            wrapMode: Text.Wrap
            font.pixelSize: 11
        }
        Repeater {
            model: [
                ["InteractiveSeg", "Click to select (Segment Anything)"],
                ["RemoveBG", "Remove background / select subject"],
                ["AnimeSeg", "Anime segmentation"],
                ["RealESRGAN", "Upscale (RealESRGAN)"],
                ["GFPGAN", "Face restore (GFPGAN)"],
                ["RestoreFormer", "Face restore (RestoreFormer)"]
            ]
            delegate: CheckBox {
                required property var modelData
                Layout.fillWidth: true
                text: modelData[1]
                checked: ai.engine.enabledPlugins.indexOf(modelData[0]) >= 0
                enabled: !ai.busy
                onToggled: {
                    const list = ai.engine.enabledPlugins.filter(name => name !== modelData[0])
                    if (checked)
                        list.push(modelData[0])
                    ai.engine.enabledPlugins = list
                }
            }
        }

        SectionTitle { text: "BATCH ERASE" }
        Label {
            Layout.fillWidth: true
            text: "Erase a whole folder: each image with a same-named black and white mask."
            color: ai.tokens.inkSecondary
            wrapMode: Text.Wrap
            font.pixelSize: 11
        }
        Button { Layout.fillWidth: true; text: batchImages.selectedFolder.toString().length > 0 ? "Images: " + batchImages.selectedFolder.toString().replace("file://", "") : "Choose image folder…"; onClicked: batchImages.open() }
        Button { Layout.fillWidth: true; text: batchMasks.selectedFolder.toString().length > 0 ? "Masks: " + batchMasks.selectedFolder.toString().replace("file://", "") : "Choose mask folder…"; onClicked: batchMasks.open() }
        Button { Layout.fillWidth: true; text: batchOutput.selectedFolder.toString().length > 0 ? "Output: " + batchOutput.selectedFolder.toString().replace("file://", "") : "Choose output folder…"; onClicked: batchOutput.open() }
        Button {
            objectName: "aiBatch"
            Layout.fillWidth: true
            enabled: ai.engine.state !== "missing" && ai.engine.state !== "installing"
                && batchImages.selectedFolder.toString().length > 0
                && batchMasks.selectedFolder.toString().length > 0
                && batchOutput.selectedFolder.toString().length > 0
            text: "Run batch with " + ai.engine.model
            onClicked: ai.engine.runBatch(batchImages.selectedFolder, batchMasks.selectedFolder, batchOutput.selectedFolder)
        }
        Label {
            Layout.fillWidth: true
            Layout.bottomMargin: 12
            text: "Powered by IOPaint (Apache-2.0). The engine runs on this computer and listens on 127.0.0.1 only."
            color: ai.tokens.inkMuted
            wrapMode: Text.Wrap
            font.pixelSize: 10
        }
    }
}
