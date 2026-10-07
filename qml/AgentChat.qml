// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The Agent tab as a chat. One thread (editor.codeRunner.messages) whoever answers: redrob-code
// when it is connected and installed (each message continues the same session), otherwise the
// Redrob agent, otherwise the local no-network subset. Edits still never apply by themselves:
// they wait under PENDING PROPOSALS for Apply.
ColumnLayout {
    id: chat
    objectName: "agentChat"
    required property var app
    readonly property var tokens: app.tokens
    readonly property bool viaCode: editor.mcpEnabled && editor.codeRunner.available
    readonly property bool working: editor.codeRunner.running || editor.agentBusy
    property bool settingsOpen: false
    spacing: 8

    function send() {
        const text = input.text.trim()
        if (text.length === 0 || chat.working)
            return
        if (editor.sendChatMessage(text))
            input.clear()
    }

    Item { Layout.preferredHeight: 2 }

    // Who answers, and the thread controls.
    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        Rectangle {
            Layout.preferredWidth: 8
            Layout.preferredHeight: 8
            radius: 4
            color: chat.working ? chat.tokens.statusWarning
                 : (chat.viaCode || editor.liveAgentConfigured) ? chat.tokens.statusSuccess : chat.tokens.borderSubtle
        }
        Label {
            objectName: "chatRoute"
            Layout.fillWidth: true
            text: chat.viaCode ? "redrob-code · canvas tools"
                 : editor.liveAgentConfigured ? "Redrob agent" : "Local, no network"
            color: chat.tokens.inkPrimary
            font.pixelSize: 12
            font.weight: Font.DemiBold
            elide: Text.ElideRight
        }
        BusyIndicator {
            visible: chat.working
            running: chat.working
            Layout.preferredWidth: 20
            Layout.preferredHeight: 20
            Accessible.name: "The agent is answering"
        }
        Button {
            objectName: "chatNew"
            text: "New chat"
            flat: true
            enabled: !chat.working
            onClicked: editor.codeRunner.newChat()
            Accessible.name: "Start a new chat"
        }
        Button {
            objectName: "chatSettings"
            text: chat.settingsOpen ? "Done" : "Connect"
            flat: true
            onClicked: chat.settingsOpen = !chat.settingsOpen
            Accessible.name: "Agent connection settings"
        }
    }

    // Connection settings, folded away once set up.
    ColumnLayout {
        Layout.fillWidth: true
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        visible: chat.settingsOpen
        spacing: 8
        Label {
            Layout.fillWidth: true
            text: editor.agentStatus
            color: chat.tokens.inkSecondary
            wrapMode: Text.Wrap
            font.pixelSize: 11
        }
        // A3. Sign in to the Redrob console with a code instead of pasting a key. The console page
        // opens in the browser; approving there gives the agent a workspace key, kept in a file
        // only this user can read.
        RowLayout {
            Layout.fillWidth: true
            Button {
                objectName: "consoleConnect"
                Layout.fillWidth: true
                visible: !editor.consoleConnection.connected
                enabled: editor.consoleConnection.state === "idle"
                text: editor.consoleConnection.state === "idle"
                    ? "Connect to Redrob console" : "Waiting for approval…"
                onClicked: editor.consoleConnection.connectToConsole()
            }
            Button {
                objectName: "consoleCancel"
                visible: editor.consoleConnection.state === "waiting"
                    || editor.consoleConnection.state === "starting"
                text: "Cancel"
                onClicked: editor.consoleConnection.cancel()
            }
            Button {
                objectName: "consoleDisconnect"
                Layout.fillWidth: true
                visible: editor.consoleConnection.connected
                    && !editor.consoleConnection.fromEnvironment
                text: "Disconnect from Redrob console"
                onClicked: editor.consoleConnection.disconnectFromConsole()
            }
        }
        Label {
            objectName: "consoleUserCode"
            Layout.fillWidth: true
            visible: editor.consoleConnection.state === "waiting"
            text: editor.consoleConnection.userCode
            horizontalAlignment: Text.AlignHCenter
            color: chat.tokens.inkPrimary
            font.family: "monospace"
            font.pixelSize: 20
            font.weight: Font.DemiBold
            Accessible.name: "Code to approve in the Redrob console"
        }
        Button {
            Layout.fillWidth: true
            visible: editor.consoleConnection.state === "waiting"
            text: "Open the console page again"
            onClicked: editor.consoleConnection.openVerificationPage()
        }
        Label {
            Layout.fillWidth: true
            visible: text.length > 0
            text: editor.consoleConnection.message
            color: chat.tokens.inkSecondary
            wrapMode: Text.Wrap
            font.pixelSize: 11
        }
        // P13. Let redrob-code drive the graphics tools over a loopback MCP endpoint. Off at every
        // launch; its edits land under PENDING PROPOSALS, exactly like the hosted agent's.
        Switch {
            objectName: "mcpEnableSwitch"
            Layout.fillWidth: true
            text: "Answer with redrob-code (this computer only)"
            checked: editor.mcpEnabled
            onToggled: editor.mcpEnabled = checked
            Accessible.name: "Allow redrob-code to propose edits through a local MCP connection"
        }
        Label {
            Layout.fillWidth: true
            text: editor.codeRunner.available ? editor.mcpStatus
                 : "Install redrob-code (the redrob command) to chat with it"
            color: chat.tokens.inkSecondary
            wrapMode: Text.Wrap
            font.pixelSize: 11
        }
        TextArea {
            id: mcpSnippet
            objectName: "mcpConfigSnippet"
            Layout.fillWidth: true
            visible: editor.mcpEnabled
            readOnly: true
            selectByMouse: true
            wrapMode: TextEdit.WrapAnywhere
            font.family: "monospace"
            font.pixelSize: 11
            text: editor.mcpConfigSnippet
            Accessible.name: "redrob-code configuration for this session, including its access token"
        }
        Button {
            objectName: "mcpCopySnippet"
            Layout.fillWidth: true
            visible: editor.mcpEnabled
            text: "Copy redrob-code config"
            onClicked: {
                mcpSnippet.selectAll()
                mcpSnippet.copy()
                mcpSnippet.deselect()
            }
        }
    }

    // The thread.
    ListView {
        id: chatList
        objectName: "chatList"
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.minimumHeight: 120
        clip: true
        spacing: 8
        model: editor.codeRunner.messages
        ScrollBar.vertical: ScrollBar {}
        onCountChanged: Qt.callLater(chatList.positionViewAtEnd)
        header: Item { width: 1; height: 4 }
        footer: Item { width: 1; height: 4 }
        delegate: Item {
            id: row
            required property var modelData
            readonly property string role: modelData.role
            readonly property bool mine: role === "user"
            readonly property bool quiet: role === "tool"
            width: chatList.width
            height: bubble.height
            Rectangle {
                id: bubble
                x: row.mine ? row.width - width - 12 : 12
                width: Math.min(row.width - 48, body.implicitWidth + 20)
                height: body.implicitHeight + (row.quiet ? 6 : 14)
                radius: 10
                color: row.mine ? chat.tokens.surfaceBrandSubtle
                     : row.quiet ? "transparent" : chat.tokens.surfaceSunken
                border.color: row.role === "error" ? chat.tokens.statusDanger
                            : row.quiet ? "transparent" : chat.tokens.borderSubtle
                Label {
                    id: body
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.margins: row.quiet ? 3 : 7
                    anchors.leftMargin: 10
                    width: Math.min(implicitWidth, row.width - 68)
                    text: row.modelData.text
                    wrapMode: Text.Wrap
                    textFormat: Text.PlainText
                    color: row.role === "error" ? chat.tokens.statusDanger
                         : row.quiet ? chat.tokens.inkMuted : chat.tokens.inkPrimary
                    font.pixelSize: row.quiet ? 11 : 12
                    font.family: row.quiet ? "monospace" : Qt.application.font.family
                }
            }
        }
        Label {
            anchors.centerIn: parent
            width: parent.width - 40
            visible: chatList.count === 0
            text: "Ask for an edit, e.g. \"Add a title layer and a soft vignette\".\nEdits wait for Apply."
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            color: chat.tokens.inkMuted
        }
    }

    // Proposals the agent made, waiting for Apply.
    Label {
        Layout.leftMargin: 12
        visible: proposalList.count > 0
        text: "PENDING PROPOSALS · " + proposalList.count
        color: chat.tokens.inkSecondary
        font.pixelSize: 11
        font.weight: Font.DemiBold
    }
    ListView {
        id: proposalList
        objectName: "proposalList"
        Layout.fillWidth: true
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        Layout.preferredHeight: count > 0 ? Math.min(contentHeight, 220) : 0
        visible: count > 0
        clip: true
        spacing: 6
        model: editor.proposals
        delegate: Rectangle {
            required property string proposalId
            required property string proposalTitle
            required property string proposalSummary
            required property string proposalAction
            width: proposalList.width
            height: cardContent.implicitHeight + 16
            radius: 9
            color: chat.tokens.surfaceSunken
            border.color: chat.tokens.borderAi
            ColumnLayout {
                id: cardContent
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: 8
                Label {
                    Layout.fillWidth: true
                    text: proposalTitle
                    font.weight: Font.DemiBold
                    wrapMode: Text.Wrap
                }
                Label {
                    Layout.fillWidth: true
                    text: proposalSummary
                    color: chat.tokens.inkSecondary
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
    }

    Label {
        Layout.fillWidth: true
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        visible: text.length > 0 && chat.working
        text: chat.viaCode ? editor.codeRunner.status : editor.agentStatus
        color: chat.tokens.inkSecondary
        wrapMode: Text.Wrap
        font.pixelSize: 11
    }

    // The composer: Enter sends, Shift+Enter starts a new line.
    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        Layout.bottomMargin: 10
        ScrollView {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(110, Math.max(40, input.implicitHeight))
            TextArea {
                id: input
                objectName: "chatInput"
                placeholderText: chat.viaCode ? "Message redrob-code…" : "Message the agent…"
                wrapMode: TextEdit.Wrap
                font.pixelSize: 12
                Accessible.name: "Message to the agent"
                Keys.onReturnPressed: event => {
                    if (event.modifiers & Qt.ShiftModifier) {
                        event.accepted = false
                        return
                    }
                    chat.send()
                }
                Keys.onEnterPressed: chat.send()
            }
        }
        Button {
            objectName: "chatSend"
            text: "Send"
            highlighted: true
            visible: !chat.working
            enabled: input.text.trim().length > 0
            onClicked: chat.send()
            Accessible.name: "Send the message"
        }
        Button {
            objectName: "chatStop"
            text: "Stop"
            visible: editor.codeRunner.running
            onClicked: editor.codeRunner.stop()
            Accessible.name: "Stop the answer"
        }
    }
}
