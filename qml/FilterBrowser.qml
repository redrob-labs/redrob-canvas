// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// UI-1: every engine filter in one searchable list, with a form built from the filter's own
// defaults (editor.filterCatalog). No per-filter QML: a field per key of the defaults object,
// typed by the default's JSON type -- number, boolean, string; objects and arrays (colours,
// matrices, curves) edit as JSON text. The engine validates on apply.
//
// P12: moved out of Main.qml unchanged, apart from `filterBrowser.` -> `root.` and the window's
// tokens arriving as a property.
Dialog {
    id: root
    objectName: "filterBrowser"
    // The design tokens of the window that owns this dialog. `editor` needs no property: it is a
    // root-context property, visible to every file the engine loads.
    required property var tokens
    title: "Filters"
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(760, parent.width - 40)
    height: Math.min(560, parent.height - 40)
    standardButtons: Dialog.Close
    property string selectedKind: ""
    property var selectedDefaults: null
    property var fieldValues: ({})
    function humanName(kind) {
        return kind.charAt(0).toUpperCase() + kind.slice(1).replace(/_/g, " ")
    }
    function select(entry) {
        selectedKind = entry.kind
        selectedDefaults = entry.defaults
        var values = {}
        if (entry.defaults) {
            for (var key in entry.defaults) {
                if (key === "kind")
                    continue
                var v = entry.defaults[key]
                values[key] = (typeof v === "object" && v !== null) ? JSON.stringify(v) : v
            }
        }
        fieldValues = values
    }
    function fieldKeys() {
        var keys = []
        if (selectedDefaults)
            for (var key in selectedDefaults)
                if (key !== "kind")
                    keys.push(key)
        return keys
    }
    // The typed parameter map for the selected filter, or null (with the reason in
    // filterStatus) when a field does not parse. Shared by Apply and Add adjustment layer.
    function collectParams() {
        var params = {}
        for (var key in fieldValues) {
            var original = selectedDefaults[key]
            var value = fieldValues[key]
            if (typeof original === "object" && original !== null) {
                try {
                    params[key] = JSON.parse(value)
                } catch (e) {
                    filterStatus.text = key + ": not valid JSON"
                    return null
                }
            } else if (typeof original === "number") {
                params[key] = Number(value)
            } else {
                params[key] = value
            }
        }
        filterStatus.text = ""
        return params
    }
    function apply() {
        var params = collectParams()
        if (params !== null)
            editor.applyFilterParams(selectedKind, params)
    }
    // P11: the same filter as a non-destructive layer above the active node.
    function addAdjustment() {
        var params = collectParams()
        if (params !== null)
            editor.addAdjustmentNode(selectedKind, params)
    }
    onOpened: filterSearch.forceActiveFocus()

    RowLayout {
        anchors.fill: parent
        spacing: 12
        ColumnLayout {
            Layout.preferredWidth: 240
            Layout.maximumWidth: 240
            Layout.fillHeight: true
            TextField {
                id: filterSearch
                objectName: "filterSearch"
                Layout.fillWidth: true
                placeholderText: "Search " + editor.filterCatalog.length + " filters"
                Accessible.name: "Search filters"
            }
            ListView {
                id: filterList
                objectName: "filterList"
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: editor.filterCatalog.filter(function (entry) {
                    var q = filterSearch.text.toLowerCase().replace(/ /g, "_")
                    return q.length === 0 || entry.kind.indexOf(q) >= 0
                })
                delegate: ItemDelegate {
                    required property var modelData
                    width: ListView.view.width
                    height: 26
                    highlighted: modelData.kind === root.selectedKind
                    text: root.humanName(modelData.kind)
                          + (modelData.defaults === null ? "  ·  needs parameters" : "")
                    onClicked: root.select(modelData)
                    Accessible.name: "Filter " + modelData.kind
                }
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Label {
                text: root.selectedKind.length === 0 ? "Pick a filter"
                      : root.humanName(root.selectedKind)
                font.pixelSize: 15
                font.weight: Font.DemiBold
                color: root.tokens.inkPrimary
            }
            Label {
                visible: root.selectedKind.length > 0 && root.selectedDefaults === null
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: "This filter has a parameter with no default, so it cannot be applied from here yet."
                color: root.tokens.inkSecondary
            }
            ScrollView {
                id: filterFormScroll
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                GridLayout {
                    width: filterFormScroll.availableWidth
                    columns: 2
                    columnSpacing: 10
                    Repeater {
                        model: root.selectedDefaults ? root.fieldKeys() : []
                        delegate: RowLayout {
                            required property string modelData
                            Layout.columnSpan: 2
                            Layout.fillWidth: true
                            property var original: root.selectedDefaults[modelData]
                            Label {
                                text: modelData.replace(/_/g, " ")
                                color: root.tokens.inkPrimary
                                Layout.preferredWidth: 150
                                elide: Text.ElideRight
                            }
                            CheckBox {
                                visible: typeof parent.original === "boolean"
                                checked: visible && root.fieldValues[modelData] === true
                                onToggled: root.fieldValues[modelData] = checked
                                Accessible.name: modelData
                            }
                            TextField {
                                visible: typeof parent.original !== "boolean"
                                Layout.fillWidth: true
                                text: visible ? String(root.fieldValues[modelData]) : ""
                                validator: typeof parent.original === "number" ? numberValidator : null
                                onTextEdited: root.fieldValues[modelData] = text
                                Accessible.name: modelData
                            }
                        }
                    }
                }
            }
            DoubleValidator { id: numberValidator; notation: DoubleValidator.StandardNotation }
            Label {
                id: filterStatus
                color: root.tokens.statusDanger
                Layout.fillWidth: true
            }
            RowLayout {
                Layout.fillWidth: true
                BusyIndicator {
                    objectName: "filterBusyIndicator"
                    running: editor.filterBusy
                    visible: running
                    Layout.preferredWidth: 24
                    Layout.preferredHeight: 24
                }
                Label {
                    Layout.fillWidth: true
                    text: editor.statusMessage
                    color: root.tokens.inkSecondary
                    elide: Text.ElideRight
                }
                Button {
                    objectName: "filterCancel"
                    text: "Cancel"
                    visible: editor.filterBusy
                    onClicked: editor.cancelFilter()
                    Accessible.name: "Cancel the running filter"
                }
                Button {
                    objectName: "filterAddAdjustment"
                    text: "Add as adjustment layer"
                    // Works on any active node: it adds a layer, it does not edit pixels.
                    enabled: root.selectedDefaults !== null && !editor.filterBusy
                    onClicked: root.addAdjustment()
                    Accessible.name: "Add the filter as a non-destructive adjustment layer"
                }
                Button {
                    objectName: "filterApply"
                    text: editor.filterBusy ? "Applying…" : "Apply"
                    enabled: root.selectedDefaults !== null && editor.activeNodeCanEditRaster
                             && !editor.filterBusy
                    onClicked: root.apply()
                }
            }
        }
    }
}
