import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5b -- The DSPi Link part of the device list (split from
// DevicesContent): the session's state in the core's words, the hub's devices,
// the hubs found on this network and a field for a hub's address. The list
// shows it (DevicesContent), and Modern's Devices > Hubs.
ColumnLayout {
    id: content
    spacing: 4

    readonly property string phase: LinkController.phase
    readonly property bool inSession: phase !== "idle" && phase !== "closed" && phase !== "refused"
    readonly property bool warn: phase === "reconnecting" || phase === "refused"

    // A hub's device record names its board by number or by name
    function boardName(platform) {
        if (platform === 0 || platform === "0") return "RP2040"
        if (platform === 1 || platform === "1") return "RP2350"
        if (platform === 2 || platform === "2") return "STM32H723"
        return platform === undefined || platform === null ? "" : String(platform)
    }

    // ---- DSPi Link: this session
    SectionHeading {
        Layout.fillWidth: true
        Layout.topMargin: Theme.spacingSmall
        text: qsTr("DSPi Link")
    }
    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            // -- WP SECQT -- red, with " (unsecured)" and its info
            // button, while the session is not encrypted
            RowLayout {
                Layout.fillWidth: true
                spacing: 2
                Text {
                    objectName: "DevicesPopup.phaseText"
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: content.inSession && LinkController.hubName.length
                          ? LinkController.hubName + ": " + LinkController.phaseText
                          : LinkController.phaseText
                    font: Theme.fontBody
                    color: LinkController.unsecured ? Theme.dangerText : content.warn ? Theme.warning : Theme.text
                    wrapMode: Text.Wrap
                }
                Text {
                    objectName: "DevicesPopup.unsecured"
                    Layout.alignment: Qt.AlignTop
                    visible: LinkController.unsecured
                    textFormat: Text.PlainText
                    text: "(" + LinkController.unsecuredLabel + ")"
                    font: Theme.fontBody
                    color: Theme.dangerText
                }
                UnsecuredInfoButton {
                    objectName: "DevicesPopup.unsecuredInfo"
                    Layout.alignment: Qt.AlignTop
                    visible: LinkController.unsecured
                }
            }
            // -- WP SECQT -- the verified key of an encrypted session
            Text {
                objectName: "DevicesPopup.hubKey"
                Layout.fillWidth: true
                visible: content.inSession && LinkController.encrypted && LinkController.hubKey.length > 0
                //: %1 is the hub key's fingerprint, SHA256:K7Q2-M9XD-…
                textFormat: Text.PlainText
                text: qsTr("Encrypted · hub key %1").arg(LinkController.hubKey)
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            Text {
                Layout.fillWidth: true
                visible: content.phase === "reconnecting" && LinkController.reconnectAttempt > 0
                textFormat: Text.PlainText
                text: qsTr("Attempt %1, next try in %2 s").arg(LinkController.reconnectAttempt)
                      .arg(Math.ceil(LinkController.reconnectInMs / 1000))
                font: Theme.fontSmall
                color: Theme.textMuted
            }
            Text {
                Layout.fillWidth: true
                visible: (content.phase === "closed" || content.phase === "refused")
                         && LinkController.closeReason.length > 0
                textFormat: Text.PlainText
                text: LinkController.closeReason
                font: Theme.fontSmall
                color: Theme.text
                wrapMode: Text.Wrap
            }
        }
        AppButton {
            objectName: "DevicesPopup.retry"
            visible: content.phase === "reconnecting"
            text: qsTr("Retry Now")
            onClicked: LinkController.retryNow()
        }
        // The core's own button for this phase (reconnecting's
        // Disconnect), else ours while a session is open
        AppButton {
            objectName: "DevicesPopup.phaseAction"
            visible: LinkController.actionText.length > 0 || content.inSession
            text: LinkController.actionText.length > 0 ? LinkController.actionText : qsTr("Disconnect")
            onClicked: LinkController.closeSession()
        }
    }
    Repeater {
        model: LinkController.devices
        delegate: DeviceRow {
            required property var modelData
            Layout.fillWidth: true
            objectName: "DevicesPopup.hubDevice." + modelData.serial
            iconName: "network"
            title: modelData.name && modelData.name.length ? modelData.name : modelData.serial
            subtitle: [LinkController.hubName, content.boardName(modelData.platform), modelData.fw]
                      .filter(part => part && String(part).length).join(" · ")
            current: content.phase === "bound" && LinkController.boundSerial === modelData.serial
            enabled: content.phase !== "reconnecting" && !LinkController.busy
            onClicked: DevicesController.chooseHubDevice(modelData.handle, modelData.serial)
        }
    }
    Text {
        objectName: "DevicesPopup.error"
        Layout.fillWidth: true
        visible: LinkController.errorText.length > 0
        textFormat: Text.PlainText
        text: LinkController.errorText
        font: Theme.fontSmall
        color: Theme.warning
        wrapMode: Text.Wrap
    }

    // ---- Other hubs: found on this network, or typed in
    AppSwitch {
        objectName: "DevicesPopup.lookForHubs"
        Layout.fillWidth: true
        Layout.topMargin: Theme.spacingSmall
        text: qsTr("Find hubs automatically")
        checked: DevicesController.lookForHubs
        onToggled: DevicesController.lookForHubs = checked
    }
    Text {
        Layout.fillWidth: true
        visible: DevicesController.lookForHubs && DevicesController.browseError.length > 0
        textFormat: Text.PlainText
        text: DevicesController.browseError
        font: Theme.fontSmall
        color: Theme.warning
        wrapMode: Text.Wrap
    }
    Repeater {
        model: DevicesController.hubs
        delegate: DeviceRow {
            required property var modelData
            Layout.fillWidth: true
            objectName: "DevicesPopup.foundHub." + modelData.id
            iconName: modelData.kind === "console" ? "monitor" : "wifi"
            title: modelData.name
            // -- WP SECQT -- the session's hub says whether its key is
            // verified or the connection is unencrypted
            readonly property bool here: content.inSession
                                         && LinkController.hubId.toLowerCase() === String(modelData.id).toLowerCase()
            readonly property string security: !here ? ""
                : LinkController.encrypted ? qsTr("verified")
                : LinkController.unsecured ? qsTr("unencrypted") : ""
            subtitle: (modelData.deviceCount === 1
                       ? qsTr("%1:%2, 1 device").arg(modelData.host).arg(modelData.port)
                       : qsTr("%1:%2, %3 devices").arg(modelData.host).arg(modelData.port).arg(modelData.deviceCount || 0))
                      + (security.length ? " · " + security : "")
            current: here
            enabled: !LinkController.busy
            onClicked: DevicesController.connectFound(modelData)
        }
    }
    Text {
        objectName: "DevicesPopup.browseStatus"
        Layout.fillWidth: true
        Layout.leftMargin: 32
        visible: DevicesController.hubs.length === 0
        textFormat: Text.PlainText
        text: DevicesController.lookForHubs ? qsTr("Looking for hubs…") : qsTr("No hubs added.")
        font: Theme.fontSmall
        color: Theme.textMuted
    }
    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        BorderedField {
            id: address
            objectName: "DevicesPopup.address"
            Layout.fillWidth: true
            label: qsTr("Hub address")
            placeholderText: qsTr("192.168.1.20 or studio.local")
            onAccepted: connect.clicked()
        }
        AppButton {
            id: connect
            objectName: "DevicesPopup.connect"
            text: qsTr("Connect")
            enabled: address.text.trim().length > 0
            onClicked: DevicesController.connectAddress(address.text)
        }
    }
    Text {
        Layout.fillWidth: true
        text: qsTr("A DSPi shared by another console (Settings > Networking there) or by a DSPi Link bridge shows here.")
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
}
