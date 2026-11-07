import QtQuick 2.15
import QtQuick.Controls 2.15
import "components"

// WP QD9: the one list of what to open (reference IOSDevicesSheet in IOSApp.swift):
// the DSPis on USB, the demo, and the devices of the hub this session is on; under
// it the hubs found on the network and one typed in by address, and where the
// session stands in CG10's words, every LinkPhase with its button. Choosing one
// leaves what was open. A hub that wants a PIN asks in PairingDialog, which
// main.qml shows by itself, so this list needs not be open for it.
//
// Nothing here blocks: dials, pairing and binds are linkSession's, on its worker.
// A hub's id goes with its address (`expected_hub_id`), so a hub announcing an id
// it can't prove gets no token (wrong_hub, said in linkSession.errorText).
Popup {
    id: pop
    objectName: "devicesPopup"

    // main.qml's device switch for a USB serial: it asks about unsaved changes first
    property var switchTo: function (serial) { bridge.selectDevice(serial) }
    // Any other switch, with the same question: guard(action) runs action or not
    property var guard: function (action) { action() }

    // FakeDevice::demo's serial (src/DemoDevice.h): under --demo it is in the USB list
    readonly property string demoSerial: "DEMO0000DSPI0001"
    readonly property var usbSerials: bridge.availableSerials.filter(function (s) { return s !== pop.demoSerial })
    readonly property bool sessionUp: ["idle", "closed", "refused"].indexOf(linkSession.phase) < 0
    readonly property bool onHub: sessionUp || linkSession.hubName !== ""
    readonly property color secondary: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.55)
    readonly property color warning: "#ff9f0a"

    // ── What the rows do ──
    // A local device: the hub goes first, which owns the connection while it is up
    function chooseUsb(serial) {
        if (serial === bridge.selectedSerial && !sessionUp) return
        if (sessionUp) guard(function () { linkSession.closeSession(); pop.switchTo(serial) })
        else switchTo(serial)
    }
    function chooseDemo() {
        if (demo.active && !sessionUp) return
        guard(function () {
            if (pop.sessionUp) linkSession.closeSession()
            if (!demo.active) demo.enter()
        })
    }
    function chooseHubDevice(device) {
        if (linkSession.phase === "bound" && bridge.selectedSerial === device.serial) return
        guard(function () { linkSession.bindDevice(device.handle, device.serial) })
    }
    // A row of hub.hubs ({id, name, url, ...}): held to the id it announced
    function connectFound(found) {
        guard(function () { linkSession.connectToHub(found.url, found.id) })
    }
    function connectTyped(text) {
        var typed = text.trim()
        if (typed === "") return
        guard(function () { if (linkSession.connectToAddress(typed)) addressField.text = "" })
    }

    // Browsing runs while this shows, as on Settings > Networking: what that had
    // before comes back on close
    property bool watchedBefore: false
    onAboutToShow: { watchedBefore = hub.watching; hub.watching = true }
    onClosed: hub.watching = watchedBefore

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(460, parent ? parent.width - 48 : 460)
    height: Math.min(body.implicitHeight + topPadding + bottomPadding, parent ? parent.height - 48 : 640)
    padding: 18
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    Overlay.modal: Rectangle { color: Qt.rgba(0, 0, 0, 0.45) }
    background: Rectangle {
        radius: 14
        color: MenuStyle.background
        border.color: MenuStyle.border
    }

    component Heading: Text {
        width: parent.width
        font.pixelSize: MenuStyle.headerFontSize + 1
        font.weight: Font.DemiBold
        font.capitalization: Font.AllUppercase
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.55)
        topPadding: 6
    }
    component Note: Text {
        width: parent.width
        wrapMode: Text.WordWrap
        font.pixelSize: 12
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.55)
    }
    // One device or hub: icon, two lines, and a tick on the one open
    component DeviceRow: Rectangle {
        id: row
        property string icon: "chip"
        property string title: ""
        property string detail: ""
        property bool current: false
        property bool pending: false
        signal clicked()
        width: parent.width
        height: 44
        radius: MenuStyle.rowRadius
        opacity: enabled ? 1 : 0.45
        color: rowMouse.pressed ? Qt.rgba(1, 1, 1, 0.1) : rowMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.06) : "transparent"
        Icon {
            id: rowIcon
            x: 8
            anchors.verticalCenter: parent.verticalCenter
            name: row.icon
            size: 18
            color: MenuStyle.iconColor
        }
        Column {
            anchors.left: rowIcon.right
            anchors.leftMargin: 12
            anchors.right: mark.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2
            Text {
                width: parent.width
                elide: Text.ElideRight
                text: row.title
                font.pixelSize: MenuStyle.fontSize
                color: MenuStyle.text
            }
            Text {
                width: parent.width
                elide: Text.ElideRight
                visible: text !== ""
                text: row.detail
                font.pixelSize: MenuStyle.smallFontSize
                color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.55)
            }
        }
        Item {
            id: mark
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 18; height: 18
            Icon { anchors.centerIn: parent; visible: row.current && !row.pending; name: "check-circle"; size: 16; color: "#32d74b" }
            Spinner { anchors.centerIn: parent; visible: row.pending; size: 16 }
        }
        MouseArea {
            id: rowMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: row.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: row.clicked()
        }
    }

    contentItem: Flickable {
        clip: true
        contentHeight: body.implicitHeight
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

        Column {
            id: body
            width: parent.width
            spacing: 6

            Item {
                width: parent.width
                height: 30
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Devices"
                    font.pixelSize: 15
                    font.weight: Font.DemiBold
                    color: isMacOS ? MacColors.label : "white"
                }
                AppButton {
                    objectName: "devicesDone"
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Done"
                    primary: true
                    onClicked: pop.close()
                }
            }

            // ── This machine ──
            Heading { text: "USB" }
            Repeater {
                model: pop.usbSerials
                DeviceRow {
                    objectName: "devicesUsb_" + modelData
                    icon: "plug"
                    title: "DSPi " + modelData.slice(-8)
                    detail: "USB · " + modelData
                    current: !pop.sessionUp && bridge.connected && bridge.selectedSerial === modelData
                    onClicked: pop.chooseUsb(modelData)
                }
            }
            Note {
                visible: pop.usbSerials.length === 0
                text: bridge.inaccessibleDevices > 0
                      ? "A DSPi is plugged in, but this account can't open it."
                      : "No DSPi on USB."
            }
            DeviceRow {
                objectName: "devicesDemo"
                icon: "play"
                title: "Demo"
                detail: "Simulated DSPi, nothing is sent to hardware"
                current: demo.active
                // The hub's session closes first, so the demo is never refused here
                onClicked: pop.chooseDemo()
            }
            Note {
                visible: demo.failure !== ""
                text: demo.failure
                color: pop.warning
            }

            // ── The hub this session is on ──
            Heading { text: "DSPi Link" }
            Item {
                objectName: "devicesPhase"
                // CG10's sentence for the phase, as linkSession has it
                readonly property string text: linkSession.phaseText
                width: parent.width
                height: Math.max(phaseLine.implicitHeight, phaseButtons.height)
                Text {
                    id: phaseLine
                    objectName: "devicesPhaseText"
                    anchors.left: parent.left
                    anchors.right: phaseButtons.left
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.WordWrap
                    text: (linkSession.hubName !== "" && pop.sessionUp ? linkSession.hubName + ": " : "") + linkSession.phaseText
                    font.pixelSize: 12
                    color: linkSession.phase === "reconnecting" || linkSession.phase === "refused" ? pop.warning : MenuStyle.text
                }
                Row {
                    id: phaseButtons
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 6
                    // The state's own way out (CG10: reconnecting's Disconnect)
                    AppButton {
                        objectName: "devicesPhaseAction"
                        visible: linkSession.actionText !== ""
                        text: linkSession.actionText
                        onClicked: linkSession.closeSession()
                    }
                    AppButton {
                        objectName: "devicesRetry"
                        visible: linkSession.phase === "reconnecting"
                        text: "Retry Now"
                        onClicked: linkSession.retryNow()
                    }
                    AppButton {
                        objectName: "devicesLeaveHub"
                        visible: pop.sessionUp && linkSession.actionText === ""
                        text: "Disconnect"
                        onClicked: linkSession.closeSession()
                    }
                }
            }
            Note {
                visible: linkSession.phase === "reconnecting" && linkSession.reconnectInMs > 0
                text: "Try " + (linkSession.reconnectAttempt + 1) + " in " + Math.ceil(linkSession.reconnectInMs / 1000) + " s."
            }
            Note {
                visible: (linkSession.phase === "closed" || linkSession.phase === "refused") && linkSession.closeReason !== ""
                text: linkSession.closeReason
            }
            Repeater {
                model: pop.sessionUp ? linkSession.devices : []
                DeviceRow {
                    objectName: "devicesHub_" + modelData.serial
                    icon: "link"
                    title: modelData.name || ("DSPi " + String(modelData.serial).slice(-8))
                    detail: [linkSession.hubName, modelData.platform, modelData.fw,
                             modelData.lockedBy ? "in use by " + modelData.lockedBy : ""]
                            .filter(function (s) { return !!s }).join(" · ")
                    current: linkSession.phase === "bound" && bridge.selectedSerial === modelData.serial
                    pending: linkSession.busy && !current
                    enabled: linkSession.phase !== "reconnecting" && linkSession.phase !== "needs_pairing"
                    onClicked: pop.chooseHubDevice(modelData)
                }
            }

            // ── Hubs to connect to ──
            Item {
                width: parent.width
                height: 30
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Find hubs automatically"
                    font.pixelSize: MenuStyle.fontSize
                    color: MenuStyle.text
                }
                ToggleSwitch {
                    objectName: "devicesLook"
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    checked: hub.lookForHubs
                    onToggled: hub.lookForHubs = checked
                }
            }
            Note {
                visible: hub.lookForHubs && hub.browseError !== ""
                text: hub.browseError
                color: pop.warning
            }
            Repeater {
                model: hub.hubs
                DeviceRow {
                    objectName: "devicesFound_" + modelData.id
                    icon: "globe"
                    title: modelData.name
                    detail: modelData.host + ":" + modelData.port + " · "
                            + (modelData.deviceCount === 1 ? "1 device" : modelData.deviceCount + " devices")
                            + (modelData.auth === "none" ? " · Open" : " · Private")
                    current: pop.sessionUp && linkSession.hubId === modelData.id
                    onClicked: pop.connectFound(modelData)
                }
            }
            Note {
                visible: hub.hubs.length === 0
                text: hub.lookForHubs ? "Looking for hubs…" : "Not looking for hubs."
            }
            Row {
                width: parent.width
                spacing: 8
                TextField {
                    id: addressField
                    objectName: "devicesAddress"
                    width: parent.width - addButton.width - parent.spacing
                    height: 28
                    topPadding: 0
                    bottomPadding: 0
                    leftPadding: 10
                    rightPadding: 10
                    verticalAlignment: Text.AlignVCenter
                    font.pixelSize: 13
                    color: isMacOS ? MacColors.label : "white"
                    selectByMouse: true
                    placeholderText: "Hub address (host or host:port)"
                    placeholderTextColor: Qt.rgba(1, 1, 1, 0.35)
                    onAccepted: pop.connectTyped(text)
                    background: Rectangle {
                        radius: 7
                        color: Qt.rgba(1, 1, 1, 0.06)
                        border.width: addressField.activeFocus ? 1.5 : 1
                        border.color: addressField.activeFocus ? "#0a7cff" : Qt.rgba(1, 1, 1, 0.12)
                    }
                }
                AppButton {
                    id: addButton
                    objectName: "devicesConnect"
                    text: "Connect"
                    enabled: addressField.text.trim() !== ""
                    onClicked: pop.connectTyped(addressField.text)
                }
            }
            Note {
                objectName: "devicesError"
                visible: text !== ""
                text: linkSession.errorText
                color: pop.warning
            }
            Note {
                text: "A DSPi on another computer appears here once that Console shares it (Settings > Networking there), as does one on a DSPi Link bridge."
            }
        }
    }
}
