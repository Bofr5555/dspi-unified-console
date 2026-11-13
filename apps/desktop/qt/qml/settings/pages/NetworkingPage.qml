import QtQuick
import QtQuick.Layouts
import UConsole

// Settings > Networking (the Mac's NetworkingSettingsTab): share this
// computer's devices on the network, name the hub, choose its access, pair
// clients and manage them, the other hubs, and running in the background. On
// Windows, while sharing, the firewall and Public-network rows (behaviour.md
// §10, §11). While the page shows, the hub browser runs and the firewall is
// read again every few seconds.
SettingsPage {
    id: page
    objectName: "NetworkingPage"
    readonly property bool pinAccess: HubController.access === "pin"
    readonly property bool watching: visible && Window.window !== null && Window.window.visible
    readonly property string fwRules: FirewallController.rules
    readonly property bool firewallShown: FirewallController.supported && HubController.sharing
                                          && (fwRules === "blocked" || fwRules === "none")

    onWatchingChanged: {
        DevicesController.setPageWatching(watching)
        if (watching) HubController.refresh()
    }
    Binding {
        target: FirewallController
        property: "watching"
        value: page.watching && HubController.sharing && FirewallController.supported
    }
    Component.onDestruction: {
        DevicesController.setPageWatching(false)
        FirewallController.watching = false
    }

    // "1:59"
    function countdown(seconds) {
        const s = Math.max(0, seconds)
        return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0")
    }
    // How long ago, in the largest whole unit
    function ago(epochSeconds) {
        const s = Math.max(0, Math.floor(Date.now() / 1000 - epochSeconds))
        if (s < 60) return s === 1 ? qsTr("1 second") : qsTr("%1 seconds").arg(s)
        const m = Math.floor(s / 60)
        if (s < 3600) return m === 1 ? qsTr("1 minute") : qsTr("%1 minutes").arg(m)
        const h = Math.floor(s / 3600)
        if (s < 86400) return h === 1 ? qsTr("1 hour") : qsTr("%1 hours").arg(h)
        const days = Math.floor(s / 86400)
        return days === 1 ? qsTr("1 day") : qsTr("%1 days").arg(days)
    }
    // An access change that would cost paired clients asks first
    function askAccess(mode) {
        const cost = HubController.revokeCost(mode)
        if (cost === 0) {
            HubController.setAccess(mode)
            return
        }
        accessQuestion.mode = mode
        accessQuestion.text = cost === 1
                ? qsTr("1 paired client will have to pair again. This cannot be undone.")
                : qsTr("%1 paired clients will have to pair again. This cannot be undone.").arg(cost)
        accessQuestion.open()
    }

    // ---- Sharing -----------------------------------------------------------------
    SettingsGroup {
        objectName: "Networking.sharing"
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing
            Rectangle {
                implicitWidth: 40
                implicitHeight: 40
                radius: 9
                color: Theme.settingsTint("networking")
                Icon {
                    anchors.centerIn: parent
                    name: "network"
                    size: 20
                    color: Theme.textOnAccent
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 3
                Text {
                    Layout.fillWidth: true
                    text: qsTr("Share Devices on This Network")
                    font: Theme.fontTitle
                    color: Theme.text
                    wrapMode: Text.Wrap
                }
                RowLayout {
                    visible: HubController.running
                    spacing: 5
                    Rectangle {
                        implicitWidth: 7
                        implicitHeight: 7
                        radius: 3.5
                        color: Theme.connected
                    }
                    Text {
                        objectName: "Networking.status"
                        Layout.fillWidth: true
                        text: HubController.statusLine
                        font: Theme.fontBody
                        color: Theme.textMuted
                        elide: Text.ElideRight
                    }
                }
                Text {
                    visible: !HubController.running
                    Layout.fillWidth: true
                    text: HubController.busy
                          ? (HubController.sharing ? qsTr("Starting…") : qsTr("Stopping…"))
                          : qsTr("Lets other consoles and browsers on this network control the DSPi devices plugged in here.")
                    font: Theme.fontBody
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
            AppSwitch {
                id: shareSwitch
                objectName: "Networking.share"
                checked: HubController.sharing
                onToggled: {
                    HubController.sharing = checked
                    checked = Qt.binding(() => HubController.sharing)
                }
                Accessible.name: qsTr("Share devices on this network")
            }
        }
        // A start that failed: the OS's reason, the switch left on
        RowLayout {
            objectName: "Networking.error"
            visible: HubController.lastError.length > 0
            Layout.fillWidth: true
            spacing: 6
            Icon {
                name: "triangle-alert"
                size: 14
                color: Theme.warning
            }
            Text {
                objectName: "Networking.errorText"
                Layout.fillWidth: true
                text: HubController.lastError
                font: Theme.fontBody
                color: Theme.warning
                wrapMode: Text.Wrap
            }
        }
        // The hub serves a demo of its own while the app shows the demo
        RowLayout {
            objectName: "Networking.demoNote"
            visible: HubController.sharesOwnDemo
            Layout.fillWidth: true
            spacing: 6
            Icon {
                name: "info"
                size: 14
                color: Theme.textMuted
            }
            Text {
                Layout.fillWidth: true
                text: qsTr("The hub shares a demo device of its own, separate from the one shown here.")
                font: Theme.fontBody
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
        }
        // Windows Firewall (behaviour.md §11.1): only when it blocks the app or has no rule
        ColumnLayout {
            objectName: "Networking.firewall"
            visible: page.firewallShown
            Layout.fillWidth: true
            spacing: 4
            RowLayout {
                Layout.fillWidth: true
                spacing: 6
                Icon {
                    name: "triangle-alert"
                    size: 14
                    color: Theme.warning
                }
                Text {
                    objectName: "Networking.firewallText"
                    Layout.fillWidth: true
                    text: page.fwRules === "blocked"
                          ? qsTr("Windows Firewall blocks %1.").arg(AppIdentity.displayName)
                          : qsTr("Windows Firewall has no rule for %1.").arg(AppIdentity.displayName)
                    font: Theme.fontBody
                    color: Theme.text
                    wrapMode: Text.Wrap
                }
            }
            Text {
                Layout.fillWidth: true
                text: FirewallController.pathNameable
                      ? qsTr("Allowing it lets it in on Private networks only. Windows will ask permission for “Network Command Shell”, the part of Windows that changes the rule.")
                      : qsTr("Windows Firewall cannot name a program whose folder has a % sign in its name. Move %1 to another folder.").arg(AppIdentity.displayName)
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            Text {
                visible: FirewallController.fixError.length > 0
                Layout.fillWidth: true
                text: FirewallController.fixError
                font: Theme.fontSmall
                color: Theme.warning
                wrapMode: Text.Wrap
            }
            AppButton {
                objectName: "Networking.firewallAllow"
                visible: FirewallController.pathNameable
                enabled: !FirewallController.fixing
                text: qsTr("Allow in Windows Firewall")
                onClicked: FirewallController.allow()
            }
        }
        // A Public network on an adapter with a gateway (behaviour.md §11.2)
        ColumnLayout {
            objectName: "Networking.publicNetwork"
            visible: FirewallController.supported && HubController.sharing && FirewallController.onPublicNetwork
            Layout.fillWidth: true
            spacing: 4
            RowLayout {
                Layout.fillWidth: true
                spacing: 6
                Icon {
                    name: "triangle-alert"
                    size: 14
                    color: Theme.warning
                }
                Text {
                    Layout.fillWidth: true
                    text: qsTr("This network is set to Public, so other devices on it may not reach this hub.")
                    font: Theme.fontBody
                    color: Theme.text
                    wrapMode: Text.Wrap
                }
            }
            AppButton {
                objectName: "Networking.openNetworkSettings"
                text: qsTr("Open Network Settings")
                onClicked: FirewallController.openNetworkSettings()
            }
        }
    }

    // ---- This hub ------------------------------------------------------------------
    SettingsGroup {
        title: qsTr("This Hub")
        iconName: "radio"
        footer: [qsTr("Other consoles see this console as a hub with this name. Sharing works on your local network only. To reach it from elsewhere, use a VPN such as Tailscale. Don't forward the port on your router."),
                 qsTr("The pairing code crosses the network as plain text, without encryption."),
                 HubController.running ? qsTr("Turn sharing off to change the port.") : ""]
                .filter(line => line.length > 0).join("\n")
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing
            Text {
                Layout.fillWidth: true
                text: qsTr("Name")
                font: Theme.fontBody
                color: Theme.text
            }
            BorderedField {
                id: nameField
                objectName: "Networking.name"
                label: qsTr("Hub name")
                placeholderText: qsTr("Hub name")
                Layout.preferredWidth: 260
                horizontalAlignment: TextInput.AlignRight
                text: HubController.hubName
                onCommitted: typed => {
                    if (typed.trim().length > 0) HubController.setHubName(typed)
                    text = Qt.binding(() => HubController.hubName)
                }
            }
        }
        SettingsPickerRow {
            id: accessPicker
            objectName: "Networking.access"
            inline: true
            label: qsTr("Access")
            values: ["none", "pin"]
            texts: [qsTr("Open"), qsTr("Require Pairing")]
            value: HubController.access
            onChosen: mode => page.askAccess(mode)
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing
            Text {
                Layout.fillWidth: true
                text: qsTr("Port")
                font: Theme.fontBody
                color: Theme.text
            }
            BorderedField {
                id: portField
                objectName: "Networking.port"
                label: qsTr("Port")
                Layout.preferredWidth: 80
                horizontalAlignment: TextInput.AlignRight
                enabled: !HubController.running && !HubController.busy
                validator: IntValidator { bottom: 1; top: 65535 }
                text: String(HubController.port)
                onCommitted: typed => {
                    HubController.setPort(parseInt(typed))
                    text = Qt.binding(() => String(HubController.port))
                }
            }
        }
    }

    // ---- Paired clients ----------------------------------------------------------------
    SettingsGroup {
        objectName: "Networking.clients"
        visible: page.pinAccess
        title: qsTr("Paired Clients")
        iconName: "key-round"
        footer: [qsTr("Viewer can only watch. Control can adjust devices. Admin can also change these hub settings."),
                 HubController.running ? "" : qsTr("Turn sharing on to pair a new client.")]
                .filter(line => line.length > 0).join("\n")

        // The open pairing window: the six digits, large, with the time left
        Item {
            objectName: "Networking.pairingCard"
            visible: HubController.pairingCode.length > 0
            Layout.fillWidth: true
            implicitHeight: pairingColumn.implicitHeight + 2 * Theme.spacingSmall
        ColumnLayout {
            id: pairingColumn
            anchors.horizontalCenter: parent.horizontalCenter
            y: Theme.spacingSmall
            spacing: 6
            Text {
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Enter this code on the new client")
                font: Theme.fontBody
                color: Theme.textMuted
            }
            TextEdit {
                objectName: "Networking.pairingCode"
                Layout.alignment: Qt.AlignHCenter
                text: HubController.pairingCode
                readOnly: true
                selectByMouse: true
                font: Qt.font({ family: Shell.monospaceFamily, pixelSize: 34, weight: Font.DemiBold, letterSpacing: 8 })
                color: Theme.text
                selectionColor: Theme.selection
                selectedTextColor: Theme.text
                Accessible.role: Accessible.StaticText
                Accessible.name: qsTr("Pairing code")
                Accessible.description: text
            }
            Text {
                objectName: "Networking.pairingExpiry"
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Expires in %1").arg(page.countdown(HubController.pairingSecondsLeft))
                font: Theme.fontSmall
                color: Theme.textMuted
            }
            AppButton {
                objectName: "Networking.pairingCancel"
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Cancel")
                onClicked: HubController.cancelPairing()
            }
        }
        }

        Text {
            visible: HubController.clients.length === 0
            Layout.fillWidth: true
            text: qsTr("No paired clients yet.")
            font: Theme.fontBody
            color: Theme.textMuted
        }
        Repeater {
            model: HubController.clients
            delegate: RowLayout {
                id: clientRow
                required property var modelData
                required property int index
                objectName: "Networking.client." + index
                Layout.fillWidth: true
                spacing: 10
                Icon {
                    name: "circle-user"
                    size: 22
                    color: Theme.textMuted
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    Text {
                        Layout.fillWidth: true
                        text: clientRow.modelData.name
                        font: Theme.fontBody
                        color: Theme.text
                        elide: Text.ElideRight
                    }
                    Text {
                        Layout.fillWidth: true
                        text: clientRow.modelData.online ? qsTr("Connected now")
                              : clientRow.modelData.lastSeen > 0
                                ? qsTr("Last seen %1 ago").arg(page.ago(clientRow.modelData.lastSeen))
                                : qsTr("Never connected")
                        font: Theme.fontSmall
                        color: Theme.textMuted
                    }
                }
                AppComboBox {
                    objectName: "Networking.client." + clientRow.index + ".role"
                    label: qsTr("Role")
                    Layout.preferredWidth: 110
                    enabled: HubController.running
                    readonly property var roles: ["viewer", "control", "admin"]
                    model: [qsTr("Viewer"), qsTr("Control"), qsTr("Admin")]
                    currentIndex: roles.indexOf(clientRow.modelData.role)
                    onActivated: choice => HubController.setRole(clientRow.modelData.cid, roles[choice])
                }
                IconButton {
                    objectName: "Networking.client." + clientRow.index + ".remove"
                    iconName: "circle-minus"
                    iconColor: Theme.dangerText
                    toolTip: qsTr("Remove this client")
                    enabled: HubController.running
                    onClicked: HubController.revoke(clientRow.modelData.cid)
                }
            }
        }
        RowLayout {
            visible: HubController.pairingCode.length === 0
            Layout.fillWidth: true
            spacing: Theme.spacing
            Text {
                Layout.fillWidth: true
                text: qsTr("Pair a New Client")
                font: Theme.fontBody
                color: HubController.running ? Theme.text : Theme.textDisabled
            }
            // A code issued while the hub is stopped could never be used
            AppButton {
                objectName: "Networking.pair"
                text: qsTr("Pair…")
                enabled: HubController.running
                onClicked: HubController.allowNewClient()
            }
        }
    }

    // ---- Other hubs ------------------------------------------------------------------
    SettingsGroup {
        objectName: "Networking.otherHubs"
        title: qsTr("Other Hubs")
        iconName: "wifi"
        footer: qsTr("Devices from these hubs appear in the device list, labelled with their hub.")
        SettingsToggle {
            objectName: "Networking.lookForHubs"
            title: qsTr("Find hubs automatically")
            checked: DevicesController.lookForHubs
            onToggled: on => DevicesController.lookForHubs = on
        }
        RowLayout {
            visible: DevicesController.lookForHubs && DevicesController.browseError.length > 0
            Layout.fillWidth: true
            spacing: 6
            Icon {
                name: "triangle-alert"
                size: 14
                color: Theme.warning
            }
            Text {
                Layout.fillWidth: true
                text: DevicesController.browseError
                font: Theme.fontBody
                color: Theme.warning
                wrapMode: Text.Wrap
            }
        }
        Repeater {
            model: DevicesController.hubs
            delegate: RowLayout {
                id: hubRow
                required property var modelData
                required property int index
                objectName: "Networking.hub." + index
                readonly property bool open: modelData.auth === "none"
                Layout.fillWidth: true
                spacing: 10
                Icon {
                    name: hubRow.modelData.kind === "console" ? "monitor" : "network"
                    size: 18
                    color: Theme.textMuted
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    Text {
                        Layout.fillWidth: true
                        text: hubRow.modelData.name
                        font: Theme.fontBody
                        color: Theme.text
                        elide: Text.ElideRight
                    }
                    Text {
                        Layout.fillWidth: true
                        readonly property int devices: hubRow.modelData.deviceCount || 0
                        readonly property string where: hubRow.modelData.host + ":" + hubRow.modelData.port
                        text: devices === 1 ? qsTr("%1, 1 device").arg(where)
                                            : qsTr("%1, %2 devices").arg(where).arg(devices)
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        elide: Text.ElideRight
                    }
                }
                Rectangle {
                    implicitWidth: badge.implicitWidth + 14
                    implicitHeight: badge.implicitHeight + 4
                    radius: height / 2
                    color: "transparent"
                    border.width: 1
                    border.color: hubRow.open ? Theme.success : Theme.accentText
                    RowLayout {
                        id: badge
                        anchors.centerIn: parent
                        spacing: 3
                        Icon {
                            name: hubRow.open ? "lock-open" : "lock"
                            size: 11
                            color: hubRow.open ? Theme.success : Theme.accentText
                        }
                        Text {
                            text: hubRow.open ? qsTr("Open") : qsTr("Private")
                            font: Theme.fontCaption
                            color: hubRow.open ? Theme.success : Theme.accentText
                        }
                    }
                }
            }
        }
        Text {
            visible: DevicesController.hubs.length === 0
            Layout.fillWidth: true
            text: DevicesController.lookForHubs ? qsTr("Looking for hubs…") : qsTr("No hubs added.")
            font: Theme.fontBody
            color: Theme.textMuted
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing
            Text {
                text: qsTr("Connect by Address")
                font: Theme.fontBody
                color: Theme.text
            }
            BorderedField {
                id: addressField
                objectName: "Networking.address"
                label: qsTr("Hub address")
                placeholderText: qsTr("192.168.1.20 or studio.local")
                Layout.fillWidth: true
                onAccepted: connectButton.clicked()
                onTextChanged: addressError.visible = false
            }
            AppButton {
                id: connectButton
                objectName: "Networking.connect"
                text: qsTr("Connect")
                enabled: addressField.text.trim().length > 0
                onClicked: {
                    const ok = DevicesController.connectAddress(addressField.text.trim())
                    addressError.visible = !ok
                    if (ok) addressField.text = ""
                }
            }
        }
        Text {
            id: addressError
            objectName: "Networking.addressError"
            visible: false
            Layout.fillWidth: true
            text: qsTr("Hostname or IP address. Add :port if the hub doesn't use %1.").arg(11915)
            font: Theme.fontSmall
            color: Theme.warning
            wrapMode: Text.Wrap
        }
    }

    // ---- Running in the background ----------------------------------------------------
    SettingsGroup {
        objectName: "Networking.background"
        title: qsTr("Running in the Background")
        iconName: "moon"
        footer: TrayController.trayAvailable ? ""
                : qsTr("No tray is available on this desktop, so closing the window quits %1.").arg(AppIdentity.displayName)
        SettingsToggle {
            objectName: "Networking.showInTray"
            title: qsTr("Show in the tray")
            caption: qsTr("Closing the window keeps %1 running in the tray.").arg(AppIdentity.displayName)
            checked: TrayController.showInTray
            onToggled: on => TrayController.showInTray = on
        }
        SettingsToggle {
            objectName: "Networking.startInTray"
            title: qsTr("Start in the tray only")
            enabled: TrayController.showInTray
            checked: TrayController.startMinimised
            onToggled: on => TrayController.startMinimised = on
        }
        SettingsToggle {
            objectName: "Networking.openAtLogin"
            title: qsTr("Open at login")
            enabled: SystemIntegration.loginSupported
            caption: SystemIntegration.loginOtherProgram.length > 0
                     ? qsTr("Opens the copy at %1.").arg(SystemIntegration.loginOtherProgram) : ""
            checked: SystemIntegration.openAtLogin
            onToggled: on => SystemIntegration.openAtLogin = on
        }
        AppButton {
            objectName: "Networking.loginHere"
            visible: SystemIntegration.loginOtherProgram.length > 0
            text: qsTr("Open This Copy at Login")
            onClicked: SystemIntegration.pointLoginHere()
        }
        RowLayout {
            visible: SystemIntegration.loginError.length > 0
            Layout.fillWidth: true
            spacing: 6
            Icon {
                name: "triangle-alert"
                size: 14
                color: Theme.warning
            }
            Text {
                Layout.fillWidth: true
                text: SystemIntegration.loginError
                font: Theme.fontBody
                color: Theme.warning
                wrapMode: Text.Wrap
            }
        }
        SettingsToggle {
            objectName: "Networking.keepAwake"
            title: qsTr("Keep awake while sharing")
            caption: SystemIntegration.awakeError
            enabled: SystemIntegration.keepAwakeSupported
            checked: SystemIntegration.keepAwake
            onToggled: on => SystemIntegration.keepAwake = on
        }
    }

    QuestionDialog {
        id: accessQuestion
        objectName: "Networking.accessQuestion"
        property string mode
        title: qsTr("Change the access?")
        acceptText: qsTr("Change")
        destructive: true
        onAnswered: answer => {
            if (answer === "accept") {
                HubController.setAccess(mode)
                return
            }
            // The popup shows what was picked: back to the access in force
            accessPicker.value = ""
            accessPicker.value = Qt.binding(() => HubController.access)
        }
    }
}
