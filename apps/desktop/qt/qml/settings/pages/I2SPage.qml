import QtQuick 2.15
import QtQuick.Controls 2.15
import "../"
import "../../components"

// I2S clocking: bit clock pins, master clock and the master-mode sample rate.
SettingsPage {
    id: page
    title: qsTr("I2S Configuration")
    subtitle: qsTr("Clock pins and the master clock for I2S outputs and the I2S input. Changes apply at once.")

    readonly property var hw: bridge.hardware
    readonly property bool rp2350: hw.rp2350 === true
    readonly property bool anyI2sOutput: {
        var t = hw.outputTypes || []
        for (var i = 0; i < t.length; i++) if (t[i] === 1) return true
        return false
    }
    readonly property bool slaveMode: hw.i2sClockMode === 1
    readonly property var mckPins: rp2350 ? [13, 15, 21] : [21]

    property string status: ""
    property bool statusError: false
    function report(code, pin, okText) {
        statusError = code !== 0
        status = code === 0 ? (okText || "") : ctx.statusText(code, pin)
    }
    // A BCK candidate needs its LRCLK neighbour free, or held by the same
    // clock pair that is moving
    function pairFree(p, own) {
        if (bridge.validPins.indexOf(p + 1) < 0) return false
        var who = ctx.ownerOf(p + 1)
        return who === "" || own.indexOf(who) >= 0
    }
    readonly property var masterPair: ["I2S BCK", "I2S LRCLK"]
    readonly property var slavePair: ["I2S Slave BCK", "I2S Slave LRCLK"]

    SettingsSection {
        title: qsTr("Bit Clock")
        enabled: bridge.connected
        footnote: page.anyI2sOutput ? qsTr("Set every output to S/PDIF (Outputs) before moving the bit clock pins.") : ""

        SettingsPinRow {
            title: qsTr("BCK Pin")
            detail: qsTr("LRCLK: GPIO %1 (BCK + 1)").arg(page.hw.bckPin + 1)
            enabled: !page.anyI2sOutput
            pin: page.hw.bckPin
            accept: function(p) { return page.pairFree(p, page.masterPair) }
            onChosen: page.report(bridge.setI2sBckPin(0, pin), pin, qsTr("BCK now on GPIO %1, LRCLK on GPIO %2.").arg(pin).arg(pin + 1))
        }
        SettingsSegmentedRow {
            title: qsTr("Clock Pins")
            detail: page.hw.clockPinMode === 1 ? qsTr("Split: separate pins for Master and Slave modes.")
                                               : qsTr("Unified: Master and Slave modes share pins.")
            options: [qsTr("Unified"), qsTr("Split")]
            value: page.hw.clockPinMode === 1 ? 1 : 0
            controlWidth: 168
            onChosen: page.report(bridge.setI2sClockPinMode(index), page.hw.bckPinSlave,
                                  index === 1 ? qsTr("Slave mode uses its own clock pins.") : qsTr("Master and Slave share the clock pins."))
        }
        SettingsPinRow {
            title: qsTr("Slave BCK Pin")
            detail: page.hw.clockPinMode === 1 ? qsTr("LRCLK: GPIO %1 (BCK + 1)").arg(page.hw.bckPinSlave + 1)
                                               : qsTr("Stored but inactive while clock pins are shared.")
            enabled: page.hw.clockPinMode === 1
            pin: page.hw.bckPinSlave
            accept: function(p) { return page.pairFree(p, page.slavePair) }
            onChosen: page.report(bridge.setI2sBckPin(1, pin), pin, qsTr("Slave BCK now on GPIO %1.").arg(pin))
        }
    }

    SettingsSection {
        title: qsTr("Master Clock")
        enabled: bridge.connected

        SettingsSwitchRow {
            title: qsTr("Master Clock (MCK)")
            detail: page.slaveMode ? qsTr("Forced off in I2S slave mode: the external master supplies the clocks.")
                                   : qsTr("Clock reference for external DACs.")
            enabled: !page.slaveMode
            checked: page.hw.mckEnabled === true && !page.slaveMode
            onToggled: page.report(bridge.setMckEnabled(checked), page.hw.mckPin, checked ? qsTr("MCK on GPIO %1.").arg(page.hw.mckPin) : qsTr("MCK off."))
        }
        SettingsPinRow {
            title: qsTr("MCK Pin")
            detail: page.hw.mckEnabled ? qsTr("Turn MCK off to change its pin.") : qsTr("Pin on which MCK is generated.")
            enabled: !page.hw.mckEnabled && !page.slaveMode
            pin: page.hw.mckPin
            accept: function(p) { return page.mckPins.indexOf(p) >= 0 }
            onChosen: page.report(bridge.setMckPin(pin), pin, qsTr("MCK pin is now GPIO %1.").arg(pin))
        }
        SettingsChoiceRow {
            title: qsTr("MCK Multiplier")
            detail: page.hw.inputRate === 2 ? qsTr("Locked to 128x at 96 kHz.") : qsTr("Use 256x for DACs that need a higher MCK rate.")
            enabled: !page.slaveMode && page.hw.inputRate !== 2
            options: [{ text: qsTr("128x"), value: 0 }, { text: qsTr("256x"), value: 1 }]
            value: page.hw.mckMultiplier
            onChosen: page.report(bridge.setMckMultiplier(value), undefined, "")
        }
    }

    SettingsSection {
        title: qsTr("Input Sample Rate")
        enabled: bridge.connected
        footnote: qsTr("Used when DSPi is the clock master for the I2S or ADAT input.")
        SettingsChoiceRow {
            title: qsTr("Sample Rate")
            detail: page.slaveMode ? qsTr("Rate used in master mode; in slave mode it follows the external clock.") : ""
            enabled: !page.slaveMode
            options: [{ text: qsTr("44.1 kHz"), value: 0 }, { text: qsTr("48 kHz"), value: 1 }, { text: qsTr("96 kHz"), value: 2 }]
            value: page.hw.inputRate
            onChosen: bridge.setInputRate(value)
        }
    }

    SettingsStatusLine { text: page.status; error: page.statusError }
}
