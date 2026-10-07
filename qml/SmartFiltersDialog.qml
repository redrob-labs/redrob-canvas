// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Layer > Smart filters (U5), Photoshop's Smart Filters list under a smart object. A filter run on
// a smart object lands here instead of in its pixels; each one can be hidden, edited (in the
// filter window), removed or moved, and the smart object re-renders from its source each time.
// The list applies top to bottom.
Dialog {
    id: root
    objectName: "smartFiltersDialog"
    required property var tokens
    required property var filterWindow
    property string nodeId: ""
    // [{filter: {kind, ...}, visible}]
    property var filters: []
    title: "Smart filters"
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: parent
    standardButtons: Dialog.Close

    function openFor(id, current) {
        nodeId = id;
        editor.setActiveLayer(id);
        filters = current ? current : [];
        open();
    }
    // Sends the edited list; keeps it only when the engine took it.
    function commit(list) {
        if (editor.setSmartFilters(list))
            filters = list;
    }
    function copyList() {
        return filters.map(entry => ({ filter: entry.filter, visible: entry.visible }));
    }
    function setVisible(index, visible) {
        const list = copyList();
        list[index].visible = visible;
        commit(list);
    }
    function remove(index) {
        const list = copyList();
        list.splice(index, 1);
        commit(list);
    }
    function move(index, by) {
        const to = index + by;
        if (to < 0 || to >= filters.length)
            return;
        const list = copyList();
        const moved = list.splice(index, 1)[0];
        list.splice(to, 0, moved);
        commit(list);
    }

    ColumnLayout {
        width: 420
        spacing: 6
        Label {
            visible: root.filters.length === 0
            text: "No smart filters yet. Run a filter on this smart object to add one."
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
            color: root.tokens.inkMuted
        }
        Repeater {
            model: root.filters
            delegate: RowLayout {
                required property var modelData
                required property int index
                Layout.fillWidth: true
                spacing: 6
                CheckBox {
                    objectName: "smartFilterVisible-" + index
                    checked: modelData.visible
                    onToggled: root.setVisible(index, checked)
                    Accessible.name: "Show " + modelData.filter.kind
                }
                Label {
                    text: modelData.filter.kind
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                CommandButton {
                    objectName: "smartFilterEdit-" + index
                    text: "Edit"
                    iconName: "transform"
                    ToolTip.text: "Open this filter's settings"
                    onClicked: {
                        root.close();
                        root.filterWindow.openForSmartFilter(root.filters, index);
                    }
                }
                CommandButton {
                    objectName: "smartFilterUp-" + index
                    text: ""
                    iconName: "arrowLeft"
                    enabled: index > 0
                    ToolTip.text: "Apply earlier"
                    Accessible.name: "Move " + modelData.filter.kind + " up"
                    onClicked: root.move(index, -1)
                }
                CommandButton {
                    objectName: "smartFilterDown-" + index
                    text: ""
                    iconName: "arrowRight"
                    enabled: index < root.filters.length - 1
                    ToolTip.text: "Apply later"
                    Accessible.name: "Move " + modelData.filter.kind + " down"
                    onClicked: root.move(index, 1)
                }
                CommandButton {
                    objectName: "smartFilterRemove-" + index
                    text: ""
                    iconName: "trash"
                    ToolTip.text: "Remove this filter"
                    Accessible.name: "Remove " + modelData.filter.kind
                    onClicked: root.remove(index)
                }
            }
        }
    }
}
