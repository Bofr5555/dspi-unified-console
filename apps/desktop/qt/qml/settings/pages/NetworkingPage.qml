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
    title: qsTr("Networking")
    writesToDevice: false
    objectName: "networkingPage"

    Component.onCompleted: hub.watch()       // -- WP QD37 -- a watch of its own
    Component.onDestruction: hub.unwatch()

    readonly property color secondary: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
    readonly property color primary: isMacOS ? MacColors.label : "white"

    function ago(seconds) {
        var s = Math.max(0, Math.floor(Date.now() / 1000) - seconds)
        if (s < 60) return qsTr("just now")
        var n
        if (s < 3600) { n = Math.floor(s / 60); return n === 1 ? qsTr("%1 minute ago").arg(n) : qsTr("%1 minutes ago").arg(n) }
        if (s < 86400) { n = Math.floor(s / 3600); return n === 1 ? qsTr("%1 hour ago").arg(n) : qsTr("%1 hours ago").arg(n) }
        n = Math.floor(s / 86400)
        return n === 1 ? qsTr("%1 day ago").arg(n) : qsTr("%1 days ago").arg(n)
    }
    readonly property string statusLine: {
        var n = hub.connections
        var c = n === 0 ? qsTr("No clients connected") : n === 1 ? qsTr("%1 client connected").arg(n) : qsTr("%1 clients connected").arg(n)
        return hub.address !== "" ? hub.address + " · " + c : c
    }

    // A one-line text field in the settings style, committing on Enter or focus loss
    component Field: Rectangle {
        id: box
        property alias text: input.text
        property alias input: input
        property string placeholder: ""
        // -- WP QD22 -- What a screen reader calls the field: the row's title
        property string accessibleName: placeholder
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
            activeFocusOnTab: true
            Accessible.name: box.accessibleName
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
            title: qsTr("Share Devices on This Network")
            detail: hub.running ? page.statusLine
                    : hub.busy && hub.sharing ? qsTr("Starting…")
                    : qsTr("Lets phones, browsers and other Consoles control the DSPi devices plugged in here.")
            checked: hub.sharing
            onToggled: hub.setSharing(checked)
        }
        // -- WP QD36 -- Design choice (review c4h F4): the app's demo is not a USB device,
        // so the hub shares a demo of its own, opened when sharing starts on the demo
        SettingsRow {
            objectName: "networkingDemoNote"
            visible: demo.active
            title: qsTr("The hub shares its own demo device")
            detail: qsTr("Clients see a separate demo device, not the one shown here: changes made here don't show on theirs. It stays on the hub until sharing restarts.")
        }
        SettingsRow {
            visible: hub.running && hub.inaccessible > 0
            title: hub.inaccessible === 1 ? qsTr("1 device could not be opened") : qsTr("%1 devices could not be opened").arg(hub.inaccessible)
            detail: qsTr("Another program has it, or this user may not open it.")
        }
        // -- WP UPW2 -- What Windows lets through, while sharing is on or starting
        SettingsButtonRow {
            objectName: "networkingFirewall"
            visible: firewall.supported && hub.sharing && (firewall.rules === "blocked" || firewall.rules === "none")
            title: firewall.rules === "blocked" ? qsTr("Windows Firewall blocks DSPi uConsole")
                                                : qsTr("Windows Firewall has no rule for DSPi uConsole")
            titleColor: "#ff9f0a"
            detail: (firewall.rules === "blocked"
                     ? qsTr("Other Consoles can't connect. Pressing Cancel when Windows first asked about DSPi uConsole leaves this block.")
                     : qsTr("Other Consoles may not be able to connect."))
                    + qsTr(" Allowing it needs an administrator's approval, and lets it in on Private networks only.")
            buttonText: firewall.fixing ? qsTr("Waiting for Windows…") : qsTr("Allow in Windows Firewall")
            enabled: !firewall.fixing
            onClicked: firewall.allow()
        }
        SettingsButtonRow {
            objectName: "networkingPublicNetwork"
            visible: firewall.supported && hub.sharing && firewall.publicNetwork
            title: qsTr("This network is set to Public")
            titleColor: "#ff9f0a"
            detail: qsTr("Windows keeps other Consoles out on a Public network. If you trust this network, make it Private in Network Settings.")
            buttonText: qsTr("Open Network Settings")
            onClicked: firewall.openNetworkSettings()
        }
    }
    SettingsStatusLine {
        objectName: "networkingError"
        text: hub.lastError !== "" ? hub.lastError : hub.lastNote
        error: hub.lastError !== ""
    }
    // -- WP UPW2 --
    SettingsStatusLine {
        objectName: "networkingFirewallError"
        text: firewall.supported && hub.sharing ? firewall.fixError : ""
        error: true
    }
    // Read again while sharing is on and this page is open: the user may change the
    // rules or the network in Windows' own settings meanwhile
    Timer {
        interval: 5000
        repeat: true
        triggeredOnStart: true
        running: firewall.supported && hub.sharing && page.visible
        onTriggered: firewall.check()
    }

    // ── This hub ──
    SettingsSection {
        title: qsTr("This Hub")
        footnote: qsTr("Other Consoles see this hub by this name. Sharing works on your local network only. To reach it from elsewhere, use a VPN such as Tailscale. Don't forward the port on your router.")
                  + (hub.access === "pin" ? qsTr(" The pairing code crosses the network as plain text, without encryption: pair only on a network you trust.") : "")
                  + (hub.running ? qsTr(" Turn sharing off to change the port.") : "")

        SettingsRow {
            title: qsTr("Name")
            Field {
                id: nameField
                objectName: "networkingName"
                text: hub.hubName
                placeholder: qsTr("Hub name")
                onCommitted: function (text) { if (!hub.setHubName(text)) input.text = hub.hubName }
                Connections {
                    target: hub
                    function onStateChanged() { if (!nameField.input.activeFocus) nameField.text = hub.hubName }
                }
            }
        }
        SettingsChoiceRow {
            objectName: "networkingAccess"
            title: qsTr("Access")
            options: [ { text: qsTr("Require Pairing"), value: "pin" }, { text: qsTr("Open"), value: "open" } ]
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
            title: qsTr("Port")
            Field {
                accessibleName: qsTr("Port")
                objectName: "networkingPort"
                width: 80
                enabled: !hub.running
                text: hub.port > 0 ? String(hub.port) : ""
                validator: IntValidator { bottom: 1; top: 65535 }
                onCommitted: function (text) { if (!hub.setPort(parseInt(text))) input.text = hub.port > 0 ? String(hub.port) : "" }
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
        title: mode === "open" ? qsTr("Let anyone on this network in?") : qsTr("Require pairing?")
        message: (mode === "open" ? qsTr("Every connection on this network will be an admin, with no code asked. ") : "")
                 + (cost > 0 ? (cost === 1 ? qsTr("The paired client must pair again: changing the access mode forgets every client, and cannot be undone.")
                                           : qsTr("All %1 paired clients must pair again: changing the access mode forgets every client, and cannot be undone.").arg(cost)) : "")
        buttons: [ { key: "cancel", text: qsTr("Cancel") },
                   { key: "change", text: mode === "open" ? qsTr("Make Open") : qsTr("Require Pairing"), role: cost > 0 ? "destructive" : "primary" } ]
        onChosen: function (key) { if (key === "change") hub.setAccess(mode) }
    }

    // ── Paired clients ──
    SettingsSection {
        visible: hub.access === "pin"
        title: qsTr("Paired Clients")
        footnote: qsTr("Viewer can only watch. Control can adjust devices. Admin can also change these hub settings.")
                  + (hub.running ? "" : qsTr(" Turn sharing on to pair a new client."))

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
                    text: qsTr("Enter this code on the new client")
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
                    text: qsTr("Expires in %1:%2").arg(Math.floor(hub.pairingSecondsLeft / 60)).arg(("0" + hub.pairingSecondsLeft % 60).slice(-2))
                    font.pixelSize: 11
                    color: page.secondary
                }
                AppButton {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: qsTr("Cancel")
                    onClicked: hub.cancelPairing()
                }
            }
        }

        SettingsRow {
            visible: hub.clients.length === 0
            title: qsTr("No paired clients yet.")
            titleColor: page.secondary
        }
        Repeater {
            model: hub.clients
            SettingsRow {
                objectName: "networkingClient"
                verbatim: true
                title: modelData.name
                detail: (modelData.online ? qsTr("Connected") : modelData.lastSeen ? qsTr("Last seen %1").arg(page.ago(modelData.lastSeen)) : qsTr("Never connected"))
                Row {
                    spacing: 10
                    StyledComboBox {
                        Accessible.name: qsTr("%1 role").arg(modelData.name)
                        anchors.verticalCenter: parent.verticalCenter
                        enabled: hub.running
                        model: [ qsTr("Viewer"), qsTr("Control"), qsTr("Admin") ]
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
                        ClickArea {
                            id: removeMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            focusRadius: 13
                            Accessible.name: parent.ToolTip.text
                            onActivated: hub.revoke(modelData.cid)
                        }
                        ToolTip.visible: removeMouse.containsMouse
                        ToolTip.delay: 600
                        ToolTip.text: qsTr("Remove this client: it must pair again")
                    }
                }
            }
        }
        SettingsButtonRow {
            objectName: "networkingPair"
            visible: hub.pairingCode === ""
            enabled: hub.running
            title: qsTr("Pair a New Client")
            detail: qsTr("Shows a code to type on the new client, for a few minutes.")
            buttonText: qsTr("Pair…")
            onClicked: hub.allowNewClient()
        }
    }

    // ── Other hubs ──
    SettingsSection {
        title: qsTr("Other Hubs")
        footnote: qsTr("Connecting to a hub shows its devices in place of the ones plugged in here.")

        SettingsSwitchRow {
            objectName: "networkingLook"
            title: qsTr("Find hubs automatically")
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
                detail: modelData.host + ":" + modelData.port + ", " + (modelData.deviceCount === 1 ? qsTr("%1 device").arg(modelData.deviceCount) : qsTr("%1 devices").arg(modelData.deviceCount))
                        + " · " + (modelData.auth === "none" ? qsTr("Open") : qsTr("Private"))
                Row {
                    spacing: 8
                    AppButton {
                        text: qsTr("Connect")
                        onClicked: linkSession.connectToHub(modelData.url, modelData.id)
                    }
                }
            }
        }
        SettingsRow {
            visible: hub.hubs.length === 0
            title: hub.lookForHubs ? qsTr("Looking for hubs…") : qsTr("Not looking for hubs.")
            titleColor: page.secondary
        }
        SettingsRow {
            title: qsTr("Connect by Address")
            detail: qsTr("Host name or IP address. Add :port if the hub doesn't use the usual one.")
            Row {
                spacing: 8
                Field {
                    id: addressField
                    objectName: "networkingAddress"
                    accessibleName: qsTr("Hub address")
                    width: 170
                    placeholder: "192.168.1.20"
                    onAccepted: connectButton.clicked()
                }
                AppButton {
                    id: connectButton
                    text: qsTr("Connect")
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
        title: qsTr("Running in the Background")
        footnote: tray.trayAvailable ? "" : qsTr("No tray icon can be shown right now, so closing the window quits.")
        SettingsSwitchRow {
            objectName: "networkingShowInTray"
            title: qsTr("Show in the tray")
            detail: qsTr("Closing the window keeps the Console running in the tray, and sharing.")
            checked: tray.showInTray
            onToggled: tray.showInTray = checked
        }
        SettingsSwitchRow {
            title: qsTr("Start in the tray only")
            enabled: tray.showInTray
            checked: tray.startMinimised
            onToggled: tray.startMinimised = checked
        }
        // -- WP QD13 -- Open at login, and keeping awake while sharing (`system`)
        SettingsSwitchRow {
            objectName: "networkingOpenAtLogin"
            visible: system.loginSupported
            title: qsTr("Open at login")
            // -- WP QD13b -- An entry another copy of the app made stays that copy's
            detail: system.loginOtherProgram !== "" ? qsTr("Opens another copy: %1").arg(system.loginOtherProgram) : ""
            checked: system.openAtLogin
            onToggled: system.openAtLogin = checked
        }
        // -- WP QD13b --
        SettingsButtonRow {
            objectName: "networkingLoginHere"
            visible: system.loginSupported && system.loginOtherProgram !== ""
            title: qsTr("Open this copy at login instead")
            detail: system.thisProgram
            buttonText: qsTr("Use This Copy")
            onClicked: system.pointLoginHere()
        }
        SettingsSwitchRow {
            objectName: "networkingKeepAwake"
            visible: system.keepAwakeSupported
            title: qsTr("Keep awake while sharing")
            detail: qsTr("Idle sleep waits while devices are shared; the screen can still turn off.")
            checked: system.keepAwake
            onToggled: system.keepAwake = checked
        }
    }
    // -- WP QD13 --
    SettingsStatusLine {
        objectName: "networkingSystemError"
        text: system.loginError !== "" ? system.loginError : system.awakeError
        error: true
    }
}
