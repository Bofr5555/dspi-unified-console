import QtQuick 2.15
import QtQuick.Controls 2.15
import "../"
import "../../components"

// WP QD14: Networking (NetworkingSettingsView.swift). Share this machine's devices
// on the network, the hub's name, access and port, the paired clients and their
// roles, a pairing code, other hubs, and running in the background. `hub` is
// HubController; the background switches are the tray's (QD12).
SettingsPage {
    id: page
    title: "Networking"
    writesToDevice: false
    objectName: "networkingPage"

    Component.onCompleted: hub.watching = true
    Component.onDestruction: hub.watching = false

    readonly property color secondary: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
    readonly property color primary: isMacOS ? MacColors.label : "white"

    function plural(n, one) { return n + " " + one + (n === 1 ? "" : "s") }
    function ago(seconds) {
        var s = Math.max(0, Math.floor(Date.now() / 1000) - seconds)
        if (s < 60) return "just now"
        if (s < 3600) return plural(Math.floor(s / 60), "minute") + " ago"
        if (s < 86400) return plural(Math.floor(s / 3600), "hour") + " ago"
        return plural(Math.floor(s / 86400), "day") + " ago"
    }
    readonly property string statusLine: {
        var n = hub.connections
        var c = n === 0 ? "No clients connected" : plural(n, "client") + " connected"
        return hub.address !== "" ? hub.address + " · " + c : c
    }

    // A one-line text field in the settings style, committing on Enter or focus loss
    component Field: Rectangle {
        id: box
        property alias text: input.text
        property alias input: input
        property string placeholder: ""
        property var validator: null
        signal committed(string text)
        signal accepted()
        width: 190
        height: 26
        radius: 6
        opacity: enabled ? 1 : 0.5
        color: Qt.rgba(1, 1, 1, input.activeFocus ? 0.10 : 0.05)
        border.width: input.activeFocus ? 1 : 0
        border.color: isMacOS ? MacColors.accent : "#0a7cff"
        TextInput {
            id: input
            x: 8
            width: parent.width - 16
            height: parent.height
            verticalAlignment: Text.AlignVCenter
            selectByMouse: true
            clip: true
            font.pixelSize: 13
            color: page.primary
            selectionColor: isMacOS ? MacColors.selectedControl : "#0a7cff"
            selectedTextColor: "white"
            validator: box.validator
            onAccepted: { focus = false; box.accepted() }
            onActiveFocusChanged: if (!activeFocus) box.committed(text)
            Text {
                visible: input.text === ""
                anchors.verticalCenter: parent.verticalCenter
                text: box.placeholder
                font: input.font
                color: isMacOS ? MacColors.tertiaryLabel : Qt.rgba(1, 1, 1, 0.35)
            }
        }
    }

    // ── Sharing ──
    SettingsSection {
        SettingsSwitchRow {
            objectName: "networkingShare"
            title: "Share Devices on This Network"
            detail: hub.running ? page.statusLine
                    : hub.busy && hub.sharing ? "Starting…"
                    : "Lets phones, browsers and other Consoles control the DSPi devices plugged in here."
            checked: hub.sharing
            onToggled: hub.setSharing(checked)
        }
        SettingsRow {
            visible: hub.running && hub.inaccessible > 0
            title: hub.inaccessible === 1 ? "1 device could not be opened" : hub.inaccessible + " devices could not be opened"
            detail: "Another program has it, or this user may not open it."
        }
    }
    SettingsStatusLine {
        objectName: "networkingError"
        text: hub.lastError !== "" ? hub.lastError : hub.lastNote
        error: hub.lastError !== ""
    }

    // ── This hub ──
    SettingsSection {
        title: "This Hub"
        footnote: "Other Consoles see this hub by this name. Sharing works on your local network only. To reach it from elsewhere, use a VPN such as Tailscale. Don't forward the port on your router."
                  + (hub.access === "pin" ? " The pairing code crosses the network as plain text, without encryption: pair only on a network you trust." : "")
                  + (hub.running ? " Turn sharing off to change the port." : "")

        SettingsRow {
            title: "Name"
            Field {
                id: nameField
                objectName: "networkingName"
                text: hub.hubName
                placeholder: "Hub name"
                onCommitted: if (!hub.setHubName(text)) input.text = hub.hubName
                Connections {
                    target: hub
                    function onStateChanged() { if (!nameField.input.activeFocus) nameField.text = hub.hubName }
                }
            }
        }
        SettingsChoiceRow {
            objectName: "networkingAccess"
            title: "Access"
            options: [ { text: "Require Pairing", value: "pin" }, { text: "Open", value: "open" } ]
            value: hub.access
            onChosen: function (mode) {
                if (mode === hub.access) return
                var cost = hub.revokeCost(mode)
                if (cost > 0 || mode === "open") {
                    accessDialog.mode = mode
                    accessDialog.cost = cost
                    accessDialog.open()
                } else {
                    hub.setAccess(mode)
                }
            }
        }
        SettingsRow {
            title: "Port"
            Field {
                objectName: "networkingPort"
                width: 80
                enabled: !hub.running
                text: hub.port > 0 ? String(hub.port) : ""
                validator: IntValidator { bottom: 1; top: 65535 }
                onCommitted: if (!hub.setPort(parseInt(text))) input.text = hub.port > 0 ? String(hub.port) : ""
            }
        }
    }

    AppDialog {
        id: accessDialog
        objectName: "networkingAccessDialog"
        property string mode: ""
        property int cost: 0
        icon: "warning"
        iconTint: "#ff9f0a"
        title: mode === "open" ? "Let anyone on this network in?" : "Require pairing?"
        message: (mode === "open" ? "Every connection on this network will be an admin, with no code asked. " : "")
                 + (cost > 0 ? (cost === 1 ? "The paired client must" : "All " + cost + " paired clients must")
                               + " pair again: changing the access mode forgets every client, and cannot be undone." : "")
        buttons: [ { key: "cancel", text: "Cancel" },
                   { key: "change", text: mode === "open" ? "Make Open" : "Require Pairing", role: cost > 0 ? "destructive" : "primary" } ]
        onChosen: function (key) { if (key === "change") hub.setAccess(mode) }
    }

    // ── Paired clients ──
    SettingsSection {
        visible: hub.access === "pin"
        title: "Paired Clients"
        footnote: "Viewer can only watch. Control can adjust devices. Admin can also change these hub settings."
                  + (hub.running ? "" : " Turn sharing on to pair a new client.")

        // The code to type on the new client
        Item {
            objectName: "networkingPairingCode"
            visible: hub.pairingCode !== ""
            width: parent.width
            height: visible ? codeColumn.height + 20 : 0
            Column {
                id: codeColumn
                anchors.centerIn: parent
                spacing: 6
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "Enter this code on the new client"
                    font.pixelSize: 12
                    color: page.secondary
                }
                TextEdit {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: hub.pairingCode
                    readOnly: true
                    selectByMouse: true
                    font.pixelSize: 32
                    font.family: "monospace"
                    font.weight: Font.DemiBold
                    font.letterSpacing: 8
                    color: page.primary
                }
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "Expires in " + Math.floor(hub.pairingSecondsLeft / 60) + ":" + ("0" + hub.pairingSecondsLeft % 60).slice(-2)
                    font.pixelSize: 11
                    color: page.secondary
                }
                AppButton {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "Cancel"
                    onClicked: hub.cancelPairing()
                }
            }
        }

        SettingsRow {
            visible: hub.clients.length === 0
            title: "No paired clients yet."
            titleColor: page.secondary
        }
        Repeater {
            model: hub.clients
            SettingsRow {
                objectName: "networkingClient"
                verbatim: true
                title: modelData.name
                detail: (modelData.online ? "Connected" : modelData.lastSeen ? "Last seen " + page.ago(modelData.lastSeen) : "Never connected")
                Row {
                    spacing: 10
                    StyledComboBox {
                        anchors.verticalCenter: parent.verticalCenter
                        enabled: hub.running
                        model: [ "Viewer", "Control", "Admin" ]
                        currentIndex: [ "viewer", "control", "admin" ].indexOf(modelData.role)
                        onActivated: function (index) { hub.setRole(modelData.cid, [ "viewer", "control", "admin" ][index]) }
                    }
                    Item {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 22; height: 22
                        enabled: hub.running
                        opacity: enabled ? 1 : 0.4
                        Icon {
                            anchors.centerIn: parent
                            name: "minus-circle"
                            size: 16
                            color: removeMouse.containsMouse ? "#ff6961" : page.secondary
                        }
                        MouseArea {
                            id: removeMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: hub.revoke(modelData.cid)
                        }
                        ToolTip.visible: removeMouse.containsMouse
                        ToolTip.delay: 600
                        ToolTip.text: "Remove this client: it must pair again"
                    }
                }
            }
        }
        SettingsButtonRow {
            objectName: "networkingPair"
            visible: hub.pairingCode === ""
            enabled: hub.running
            title: "Pair a New Client"
            detail: "Shows a code to type on the new client, for a few minutes."
            buttonText: "Pair…"
            onClicked: hub.allowNewClient()
        }
    }

    // ── Other hubs ──
    SettingsSection {
        title: "Other Hubs"
        footnote: "Connecting to a hub shows its devices in place of the ones plugged in here."

        SettingsSwitchRow {
            objectName: "networkingLook"
            title: "Find hubs automatically"
            checked: hub.lookForHubs
            onToggled: hub.lookForHubs = checked
        }
        SettingsRow {
            visible: hub.lookForHubs && hub.browseError !== ""
            title: hub.browseError
            titleColor: "#ff9f0a"
            verbatim: true
        }
        Repeater {
            model: hub.hubs
            SettingsRow {
                verbatim: true
                title: modelData.name
                detail: modelData.host + ":" + modelData.port + ", " + page.plural(modelData.deviceCount, "device")
                        + (modelData.auth === "none" ? " · Open" : " · Private")
                Row {
                    spacing: 8
                    AppButton {
                        text: "Connect"
                        onClicked: linkSession.connectToHub(modelData.url, modelData.id)
                    }
                }
            }
        }
        SettingsRow {
            visible: hub.hubs.length === 0
            title: hub.lookForHubs ? "Looking for hubs…" : "Not looking for hubs."
            titleColor: page.secondary
        }
        SettingsRow {
            title: "Connect by Address"
            detail: "Host name or IP address. Add :port if the hub doesn't use the usual one."
            Row {
                spacing: 8
                Field {
                    id: addressField
                    objectName: "networkingAddress"
                    width: 170
                    placeholder: "192.168.1.20"
                    onAccepted: connectButton.clicked()
                }
                AppButton {
                    id: connectButton
                    text: "Connect"
                    enabled: addressField.text.trim() !== ""
                    onClicked: if (linkSession.connectToAddress(addressField.text.trim())) addressField.text = ""
                }
            }
        }
    }
    SettingsStatusLine {
        text: linkSession.errorText
        error: true
    }

    // ── Running in the background (QD12's tray) ──
    SettingsSection {
        title: "Running in the Background"
        footnote: tray.trayAvailable ? "" : "This desktop shows no tray icons right now, so closing the window quits."
        SettingsSwitchRow {
            objectName: "networkingShowInTray"
            title: "Show in the tray"
            detail: "Closing the window keeps the Console running in the tray, and sharing."
            checked: tray.showInTray
            onToggled: tray.showInTray = checked
        }
        SettingsSwitchRow {
            title: "Start in the tray only"
            enabled: tray.showInTray
            checked: tray.startMinimised
            onToggled: tray.startMinimised = checked
        }
        // Open at login and keeping awake while sharing are QD13's rows
    }
}
