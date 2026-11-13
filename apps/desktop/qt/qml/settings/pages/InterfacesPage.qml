import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP11 -- Settings > Control Interfaces (the Mac's
// ControlInterfacesSettingsTab): the UART and I2C target an external
// controller talks to. Each section edits a draft of its own and sends it with
// its own Apply; the device stores it. A draft with no edits follows the
// device, and a device switch drops it.
SettingsPage {
    id: page
    objectName: "InterfacesPage"
    readonly property var hw: HardwareController
    readonly property var state: hw.hw
    readonly property bool editable: hw.writable

    // The drafts, seeded from the device
    property var uart: ({})
    property var i2c: ({})
    readonly property bool uartDirty: JSON.stringify(uart) !== JSON.stringify(state.uart || {})
    readonly property bool i2cDirty: JSON.stringify(i2c) !== JSON.stringify(state.i2c || {})
    property string seededFor

    function seed() {
        uart = Object.assign({}, state.uart || {})
        i2c = Object.assign({}, state.i2c || {})
        seededFor = DeviceModel.serial
    }
    function edit(which, key, value) {
        const copy = Object.assign({}, page[which])
        copy[key] = value
        page[which] = copy
    }

    Component.onCompleted: { seed(); hw.watch(true) }
    Component.onDestruction: hw.watch(false)
    Connections {
        target: HardwareController
        // The device's configuration moved: an untouched draft follows it;
        // another device drops every draft
        function onChanged() {
            if (DeviceModel.serial !== page.seededFor) {
                page.seed()
                return
            }
            if (!page.uartApplying && JSON.stringify(page.uart) === JSON.stringify(page.lastUart))
                page.uart = Object.assign({}, page.state.uart || {})
            if (!page.i2cApplying && JSON.stringify(page.i2c) === JSON.stringify(page.lastI2c))
                page.i2c = Object.assign({}, page.state.i2c || {})
            page.lastUart = Object.assign({}, page.state.uart || {})
            page.lastI2c = Object.assign({}, page.state.i2c || {})
        }
        function onInterfacesChanged() {
            // After an Apply the draft takes what the device kept
            if (!page.uartApplying) page.uart = Object.assign({}, page.state.uart || {})
            if (!page.i2cApplying) page.i2c = Object.assign({}, page.state.i2c || {})
        }
    }
    property var lastUart: state.uart || ({})
    property var lastI2c: state.i2c || ({})
    readonly property bool uartApplying: hw.uartOutcome.applying === true
    readonly property bool i2cApplying: hw.i2cOutcome.applying === true

    // ---- UART
    SettingsGroup {
        objectName: "Interfaces.uart"
        title: qsTr("UART")
        iconName: "cable"
        enabled: page.editable

        InterfaceHeader {
            objectName: "Interfaces.uartEnabled"
            title: qsTr("Enable UART")
            detail: qsTr("Asynchronous 3.3V serial link, fixed 8N1 framing.")
            live: page.state.uartLive === true
            configured: (page.state.uart || {}).enabled === true
            checked: page.uart.enabled === true
            onToggled: on => page.edit("uart", "enabled", on)
        }
        HwRow {
            visible: page.uart.enabled === true
            title: qsTr("TX Pin")
            caption: qsTr("GPIO transmitting to the controller's RX.")
            PinPicker {
                objectName: "Interfaces.uartTx"
                label: qsTr("TX Pin")
                pins: { page.hw.revision; return page.hw.interfacePinChoices(HardwareController.UartCtrl, false, page.uart.txPin, page.uart.rxPin) }
                pin: page.uart.txPin === undefined ? -1 : page.uart.txPin
                onPicked: p => page.edit("uart", "txPin", p)
            }
        }
        HwRow {
            visible: page.uart.enabled === true
            title: qsTr("RX Pin")
            caption: qsTr("GPIO receiving from the controller's TX.")
            PinPicker {
                objectName: "Interfaces.uartRx"
                label: qsTr("RX Pin")
                pins: { page.hw.revision; return page.hw.interfacePinChoices(HardwareController.UartCtrl, true, page.uart.rxPin, page.uart.txPin) }
                pin: page.uart.rxPin === undefined ? -1 : page.uart.rxPin
                onPicked: p => page.edit("uart", "rxPin", p)
            }
        }
        HwRow {
            visible: page.uart.enabled === true
            title: qsTr("Baud Rate")
            caption: qsTr("9600 - 1000000. Must match the controller.")
            ChoicePicker {
                objectName: "Interfaces.uartBaud"
                label: qsTr("Baud Rate")
                implicitWidth: 100
                choices: [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1000000]
                         .map(b => ({ value: b, text: page.hw.baudLabel(b) }))
                value: page.uart.baud
                missingText: page.uart.baud === undefined ? "" : page.hw.baudLabel(page.uart.baud)
                onChosen: v => page.edit("uart", "baud", v)
            }
        }
        SettingsToggle {
            objectName: "Interfaces.uartNotify"
            visible: page.uart.enabled === true
            title: qsTr("Push Notifications")
            caption: qsTr("Stream live parameter/preset/format changes to the controller (type-0x40 frames) instead of polling.")
            checked: page.uart.notify === true
            onToggled: on => page.edit("uart", "notify", on)
        }
        ApplyRow {
            objectName: "Interfaces.uartApply"
            dirty: page.uartDirty
            applying: page.uartApplying
            outcome: page.hw.uartOutcome
            onApply: page.hw.applyUart(page.uart)
            onRevert: page.uart = Object.assign({}, page.state.uart || {})
        }
    }

    // ---- I2C
    SettingsGroup {
        objectName: "Interfaces.i2c"
        title: qsTr("I2C")
        iconName: "cable"
        enabled: page.editable

        InterfaceHeader {
            objectName: "Interfaces.i2cEnabled"
            title: qsTr("Enable I2C Target")
            detail: qsTr("Device acts as an I2C slave; the controller is bus master. Poll-only (no async notifications).")
            live: page.state.i2cLive === true
            configured: (page.state.i2c || {}).enabled === true
            checked: page.i2c.enabled === true
            onToggled: on => page.edit("i2c", "enabled", on)
        }
        HwRow {
            visible: page.i2c.enabled === true
            title: qsTr("SDA Pin")
            caption: qsTr("Serial data line (even GPIO).")
            PinPicker {
                objectName: "Interfaces.i2cSda"
                label: qsTr("SDA Pin")
                pins: { page.hw.revision; return page.hw.interfacePinChoices(HardwareController.I2cCtrl, false, page.i2c.sdaPin, page.i2c.sclPin) }
                pin: page.i2c.sdaPin === undefined ? -1 : page.i2c.sdaPin
                onPicked: p => page.edit("i2c", "sdaPin", p)
            }
        }
        HwRow {
            visible: page.i2c.enabled === true
            title: qsTr("SCL Pin")
            caption: qsTr("Serial clock line (next odd GPIO, same instance).")
            PinPicker {
                objectName: "Interfaces.i2cScl"
                label: qsTr("SCL Pin")
                pins: { page.hw.revision; return page.hw.interfacePinChoices(HardwareController.I2cCtrl, true, page.i2c.sclPin, page.i2c.sdaPin) }
                pin: page.i2c.sclPin === undefined ? -1 : page.i2c.sclPin
                onPicked: p => page.edit("i2c", "sclPin", p)
            }
        }
        HwRow {
            visible: page.i2c.enabled === true
            title: qsTr("Target Address")
            caption: qsTr("7-bit address, 0x08 - 0x77.")
            AppSpinBox {
                id: address
                objectName: "Interfaces.i2cAddress"
                label: qsTr("Target Address")
                realFrom: 0x08
                realTo: 0x77
                realValue: page.i2c.address === undefined ? 0x08 : page.i2c.address
                textFromValue: (v, locale) => "0x" + (v / address.scale).toString(16).toUpperCase().padStart(2, "0")
                valueFromText: (t, locale) => Math.round(parseInt(t, 16) * address.scale)
                validator: RegularExpressionValidator { regularExpression: /(0[xX])?[0-9a-fA-F]{0,2}/ }
                onCommitted: v => {
                    page.edit("i2c", "address", Math.round(v))
                    // The step wrote realValue: follow the draft again (Revert)
                    realValue = Qt.binding(() => page.i2c.address === undefined ? 0x08 : page.i2c.address)
                }
            }
        }
        ApplyRow {
            objectName: "Interfaces.i2cApply"
            dirty: page.i2cDirty
            applying: page.i2cApplying
            outcome: page.hw.i2cOutcome
            onApply: page.hw.applyI2c(page.i2c)
            onRevert: page.i2c = Object.assign({}, page.state.i2c || {})
        }
    }

    // What both have in common
    SettingsGroup {
        footer: page.state.protocolVersion ? qsTr("External control protocol version %1.").arg(page.state.protocolVersion) : ""
        Text {
            Layout.fillWidth: true
            text: qsTr("Both control interfaces are configured over USB only - an external controller can read the configuration but can never reconfigure or disable the transport it is talking on, so it cannot lock itself out. Settings persist across reboots and survive a factory reset.")
                  + "\n\n"
                  + qsTr("UART TX/RX and I2C SDA/SCL must land on GPIOs that carry the matching peripheral mux function; the device validates this when you apply. Fit external pull-ups (2.2k - 4.7k) on the I2C bus.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    // An interface's first row: title, description, a status pill (Active,
    // Inactive: stored as on but not running, Disabled) and the switch
    component InterfaceHeader: RowLayout {
        id: header
        property string title
        property string detail
        property bool live
        property bool configured
        property bool checked
        signal toggled(bool on)
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            Text {
                Layout.fillWidth: true
                text: header.title
                font: Theme.fontBody
                color: header.enabled ? Theme.text : Theme.textDisabled
            }
            Text {
                Layout.fillWidth: true
                text: header.detail
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            Text {
                Layout.fillWidth: true
                visible: header.configured && !header.live
                text: qsTr("Enabled in flash but not running - its pins likely collide with the current output wiring. Reassign the conflicting pin or move this interface, then apply.")
                font: Theme.fontCaption
                color: Theme.warning
                wrapMode: Text.Wrap
            }
        }
        Rectangle {
            objectName: header.objectName.length ? header.objectName + ".pill" : ""
            readonly property color tone: header.live ? Theme.success : header.configured ? Theme.warning : Theme.textMuted
            implicitWidth: pillText.implicitWidth + 14
            implicitHeight: pillText.implicitHeight + 4
            radius: height / 2
            color: "transparent"
            border.width: 1
            border.color: tone
            Text {
                id: pillText
                anchors.centerIn: parent
                text: header.live ? qsTr("Active") : header.configured ? qsTr("Inactive") : qsTr("Disabled")
                font: Theme.fontCaption
                color: parent.tone
            }
            Accessible.role: Accessible.StaticText
            Accessible.name: pillText.text
        }
        AppSwitch {
            objectName: header.objectName.length ? header.objectName + ".switch" : ""
            checked: header.checked
            onToggled: header.toggled(checked)
            Accessible.name: header.title
        }
    }

    // Apply and Revert for one interface, with the outcome of the last Apply
    component ApplyRow: RowLayout {
        id: applyRow
        property bool dirty
        property bool applying
        property var outcome: ({})
        signal apply()
        signal revert()
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        Icon {
            visible: !applyRow.dirty && (applyRow.outcome.text || "").length > 0
            name: applyRow.outcome.error ? "triangle-alert" : "circle-check"
            size: 12
            color: applyRow.outcome.error ? Theme.warning : Theme.success
        }
        Text {
            objectName: applyRow.objectName.length ? applyRow.objectName + ".text" : ""
            Layout.fillWidth: true
            text: applyRow.dirty ? qsTr("Unapplied changes") : (applyRow.outcome.text || "")
            font: Theme.fontSmall
            color: !applyRow.dirty && applyRow.outcome.error ? Theme.warning : Theme.textMuted
            wrapMode: Text.Wrap
        }
        Icon {
            visible: applyRow.applying
            name: "loader-circle"
            size: 14
            color: Theme.textMuted
        }
        AppButton {
            objectName: applyRow.objectName.length ? applyRow.objectName + ".revert" : ""
            text: qsTr("Revert")
            enabled: applyRow.dirty && !applyRow.applying
            onClicked: applyRow.revert()
        }
        AppButton {
            objectName: applyRow.objectName.length ? applyRow.objectName + ".apply" : ""
            text: qsTr("Apply")
            enabled: applyRow.dirty && !applyRow.applying && page.editable
            onClicked: applyRow.apply()
        }
    }
}
