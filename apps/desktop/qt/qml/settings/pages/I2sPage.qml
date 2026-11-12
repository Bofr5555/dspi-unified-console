import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP11 -- Settings > I2S Configuration (the Mac's HardwareSettingsTab
// I2S clock section): the bit clock pair, the clock-pin layout, the master
// clock and the I2S input's sample rate. The STM32H723 drives fixed clock
// pins, so the page is not listed there. Changes apply at once.
SettingsPage {
    id: page
    objectName: "I2sPage"
    readonly property var hw: HardwareController
    readonly property var state: hw.hw
    readonly property bool editable: hw.writable
    readonly property bool slaveActive: state.i2sSlaveActive === true
    readonly property bool mckLocked: hw.liveRateHz >= 96000

    Component.onCompleted: { hw.clearStatus(); hw.watch(true) }
    Component.onDestruction: hw.watch(false)

    SettingsGroup {
        objectName: "I2s.clock"
        title: qsTr("I2S Clock")
        iconName: "clock"
        enabled: page.editable

        HwRow {
            title: qsTr("BCK Pin")
            caption: qsTr("LRCK: GPIO %1 (BCK + 1)").arg((page.state.i2sBckPin || 0) + 1)
            // Not while an I2S output runs on it
            enabled: page.state.anySlotIsI2s !== true
            PinPicker {
                objectName: "I2s.bckPin"
                label: qsTr("BCK Pin")
                pins: { page.hw.revision; return page.hw.pinChoices(HardwareController.I2sBck, 0, -1, true) }
                pin: page.state.i2sBckPin === undefined ? -1 : page.state.i2sBckPin
                onPicked: p => page.hw.setBckPin(p)
            }
        }

        // Unified: one pair for both clock roles; Split: the slave role has
        // its own pair (below)
        HwRow {
            visible: page.hw.has.i2sClockPinMode === true
            title: qsTr("Clock Pins")
            caption: qsTr("Unified: Master and Slave modes share pins.") + "\n"
                     + qsTr("Split: Separate pins for Master and Slave modes.")
            AppTabBar {
                objectName: "I2s.clockPins"
                label: qsTr("Clock Pins")
                currentIndex: page.state.clockPinMode === 1 ? 1 : 0
                AppTabButton {
                    text: qsTr("Unified")
                    onClicked: if (page.state.clockPinMode !== 0) page.hw.setClockPinMode(0)
                }
                AppTabButton {
                    text: qsTr("Split")
                    onClicked: if (page.state.clockPinMode !== 1) page.hw.setClockPinMode(1)
                }
            }
        }
        HwRow {
            visible: page.hw.has.i2sClockPinMode === true
            title: qsTr("Slave BCK Pin")
            caption: page.state.clockPinMode === 1
                     ? qsTr("LRCK: GPIO %1 (BCK + 1)").arg((page.state.i2sBckPinSlave || 0) + 1)
                     : qsTr("Stored but inactive while clock pins are shared.")
            PinPicker {
                objectName: "I2s.slaveBckPin"
                label: qsTr("Slave BCK Pin")
                enabled: page.state.clockPinMode === 1
                pins: { page.hw.revision; return page.hw.pinChoices(HardwareController.I2sBckSlave, 0, page.state.i2sBckPinSlave, true) }
                pin: page.state.i2sBckPinSlave === undefined ? -1 : page.state.i2sBckPinSlave
                onPicked: p => page.hw.setSlaveBckPin(p)
            }
        }

        SettingsToggle {
            objectName: "I2s.mck"
            title: qsTr("Master Clock (MCK)")
            caption: page.slaveActive
                     ? qsTr("Forced off in I2S slave mode; a local MCK would be asynchronous to the external clocks. Kept and restored when leaving slave mode.")
                     : qsTr("Clock reference for external DACs")
            enabled: !page.slaveActive
            checked: page.state.mckEnabled === true
            onToggled: on => page.hw.setMckEnabled(on)
        }
        HwRow {
            title: qsTr("MCK Pin")
            // The pin moves only while MCK is off
            enabled: page.state.mckEnabled !== true && !page.slaveActive
            PinPicker {
                objectName: "I2s.mckPin"
                label: qsTr("MCK Pin")
                pins: { page.hw.revision; return page.hw.mckPinChoices() }
                pin: page.state.mckPin === undefined ? -1 : page.state.mckPin
                onPicked: p => page.hw.setMckPin(p)
            }
        }
        HwRow {
            title: qsTr("MCK Multiplier")
            caption: page.mckLocked
                     ? qsTr("Locked to 128x at %1").arg(page.hw.liveRateHz > 0
                                                         ? qsTr("%1 kHz").arg((page.hw.liveRateHz / 1000).toFixed(1))
                                                         : qsTr("unknown sample rate"))
                     : ""
            enabled: !page.mckLocked && !page.slaveActive
            ChoicePicker {
                objectName: "I2s.mckMultiplier"
                label: qsTr("MCK Multiplier")
                implicitWidth: 92
                choices: [{ value: 128, text: qsTr("128x") }, { value: 256, text: qsTr("256x") }]
                value: page.state.mckMultiplier
                onChosen: v => page.hw.setMckMultiplier(v)
            }
        }

        // The I2S input's rate: DSPi drives the clocks, so the source follows
        HwRow {
            visible: page.hw.has.i2sInput === true
            title: qsTr("Input Sample Rate")
            caption: page.slaveActive
                     ? qsTr("Rate used in master mode. In slave mode the external master sets the rate (detected: %1).")
                           .arg(page.hw.i2sLock.detected || "-")
                     : qsTr("Rate for I2S input; DSPi drives the clocks, so the source must follow.")
            enabled: !page.slaveActive
            ChoicePicker {
                objectName: "I2s.inputRate"
                label: qsTr("Input Sample Rate")
                implicitWidth: 100
                choices: [44100, 48000, 96000].map(hz => ({ value: hz,
                                                           text: hz % 1000 === 0 ? qsTr("%1 kHz").arg(hz / 1000)
                                                                                 : qsTr("%1 kHz").arg((hz / 1000).toFixed(1)) }))
                value: page.state.inputRateHz
                onChosen: v => page.hw.setInputRate(v)
            }
        }

        StatusLine {
            objectName: "I2s.status"
        }
    }
}
