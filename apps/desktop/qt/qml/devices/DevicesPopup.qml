import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// Device > Devices...: every DSPi this app can open, in one list (the Mac's
// device picker and its iOS Devices sheet, with the Networking page's "Other
// Hubs"). USB and the demo first, then DSPi Link: the session's state in the
// core's words, the hub's devices, the hubs found on this network and a field
// for a hub's address.
PopoverPanel {
    id: popup
    objectName: "DevicesPopup"
    title: qsTr("Devices")

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

    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 600
    width: Math.min(460, hostWidth - 2 * Theme.spacingLarge)
    height: Math.min(implicitHeight, hostHeight - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: (parent && parent.topInset !== undefined ? parent.topInset : 0) + Theme.spacingSmall

    onOpened: done.forceActiveFocus()
    onClosed: DevicesController.listShown = false

    contentItem: ColumnLayout {
        spacing: Theme.spacingSmall

        RowLayout {
            Layout.fillWidth: true
            Text {
                Layout.fillWidth: true
                text: popup.title
                font: Theme.fontTitle
                color: Theme.text
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            AppButton {
                id: done
                objectName: "DevicesPopup.done"
                text: qsTr("Done")
                primary: true
                onClicked: popup.close()
            }
        }

        Flickable {
            id: flick
            Layout.fillWidth: true
            Layout.fillHeight: true
            implicitHeight: body.implicitHeight
            contentHeight: body.implicitHeight
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}

            ColumnLayout {
                id: body
                width: flick.width
                spacing: 4

                // ---- USB and the demo
                SectionHeading {
                    Layout.fillWidth: true
                    text: qsTr("USB")
                }
                Repeater {
                    model: DevicesController.usbDevices
                    delegate: DeviceRow {
                        required property var modelData
                        Layout.fillWidth: true
                        objectName: "DevicesPopup.usb." + modelData.serial
                        iconName: "usb"
                        title: qsTr("DSPi (%1)").arg(modelData.serial)
                        subtitle: qsTr("USB")
                        current: DeviceModel.connected && !DeviceModel.remote && !DeviceModel.demo
                                 && DeviceModel.serial === modelData.serial
                        onClicked: DevicesController.chooseUsb(modelData.serial)
                    }
                }
                Text {
                    Layout.fillWidth: true
                    Layout.leftMargin: 32
                    visible: DevicesController.usbDevices.length === 0
                    text: qsTr("No DSPi on USB")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
                DeviceRow {
                    Layout.fillWidth: true
                    objectName: "DevicesPopup.demo"
                    iconName: "play"
                    title: qsTr("Demo DSPi")
                    subtitle: qsTr("Simulated DSPi")
                    current: DeviceModel.demo && DeviceModel.connected
                    onClicked: DevicesController.chooseDemo()
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
                        Text {
                            objectName: "DevicesPopup.phaseText"
                            Layout.fillWidth: true
                            text: popup.inSession && LinkController.hubName.length
                                  ? LinkController.hubName + ": " + LinkController.phaseText
                                  : LinkController.phaseText
                            font: Theme.fontBody
                            color: popup.warn ? Theme.warning : Theme.text
                            wrapMode: Text.Wrap
                        }
                        Text {
                            Layout.fillWidth: true
                            visible: popup.phase === "reconnecting" && LinkController.reconnectAttempt > 0
                            text: qsTr("Attempt %1, next try in %2 s").arg(LinkController.reconnectAttempt)
                                  .arg(Math.ceil(LinkController.reconnectInMs / 1000))
                            font: Theme.fontSmall
                            color: Theme.textMuted
                        }
                        Text {
                            Layout.fillWidth: true
                            visible: (popup.phase === "closed" || popup.phase === "refused")
                                     && LinkController.closeReason.length > 0
                            text: LinkController.closeReason
                            font: Theme.fontSmall
                            color: Theme.text
                            wrapMode: Text.Wrap
                        }
                    }
                    AppButton {
                        objectName: "DevicesPopup.retry"
                        visible: popup.phase === "reconnecting"
                        text: qsTr("Retry Now")
                        onClicked: LinkController.retryNow()
                    }
                    // The core's own button for this phase (reconnecting's
                    // Disconnect), else ours while a session is open
                    AppButton {
                        objectName: "DevicesPopup.phaseAction"
                        visible: LinkController.actionText.length > 0 || popup.inSession
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
                        subtitle: [LinkController.hubName, popup.boardName(modelData.platform), modelData.fw]
                                  .filter(part => part && String(part).length).join(" · ")
                        current: popup.phase === "bound" && LinkController.boundSerial === modelData.serial
                        enabled: popup.phase !== "reconnecting" && !LinkController.busy
                        onClicked: DevicesController.chooseHubDevice(modelData.handle, modelData.serial)
                    }
                }
                Text {
                    objectName: "DevicesPopup.error"
                    Layout.fillWidth: true
                    visible: LinkController.errorText.length > 0
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
                        subtitle: modelData.deviceCount === 1
                                  ? qsTr("%1:%2, 1 device").arg(modelData.host).arg(modelData.port)
                                  : qsTr("%1:%2, %3 devices").arg(modelData.host).arg(modelData.port).arg(modelData.deviceCount || 0)
                        current: popup.inSession && LinkController.hubId.toLowerCase() === String(modelData.id).toLowerCase()
                        enabled: !LinkController.busy
                        onClicked: DevicesController.connectFound(modelData)
                    }
                }
                Text {
                    objectName: "DevicesPopup.browseStatus"
                    Layout.fillWidth: true
                    Layout.leftMargin: 32
                    visible: DevicesController.hubs.length === 0
                    text: DevicesController.lookForHubs ? qsTr("Looking for hubs...") : qsTr("No hubs added.")
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
        }
    }
}
