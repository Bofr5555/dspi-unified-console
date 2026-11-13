import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP11 -- Settings > Inputs (the Mac's HardwareSettingsTab inputs page):
// the S/PDIF receivers and LG Sound Sync, the I2S input (clock mode, lock,
// channels, data pins), the ADAT input, and the sources' names. Pins and
// modes apply at once; the names are held for the save bar.
SettingsPage {
    id: page
    objectName: "InputsPage"
    readonly property var hw: HardwareController
    readonly property var state: hw.hw
    readonly property var has: hw.has
    readonly property bool editable: hw.writable

    Component.onCompleted: { hw.clearStatus(); hw.watch(true) }
    Component.onDestruction: hw.watch(false)

    // A lock state's dot: grey, waiting, relocking, locked
    function toneColor(tone) {
        return tone === 3 ? Theme.success : tone === 2 ? Theme.dangerText : tone === 1 ? Theme.warning : Theme.textMuted
    }

    // ---- S/PDIF Input
    SettingsGroup {
        objectName: "Inputs.spdif"
        title: qsTr("S/PDIF Input")
        iconName: "arrow-down-to-line"
        enabled: page.editable

        // How many selectable inputs share the one receiver
        HwRow {
            visible: page.has.multiSpdif === true
            title: qsTr("Instances")
            caption: page.state.spdifEnabledCount === 1
                     ? qsTr("1 selectable input sharing one receiver")
                     : qsTr("%1 selectable inputs sharing one receiver").arg(page.state.spdifEnabledCount)
            AppTabBar {
                objectName: "Inputs.spdifCount"
                label: qsTr("Instances")
                currentIndex: (page.state.spdifEnabledCount || 1) - 1
                Repeater {
                    model: Math.max(1, page.state.spdifCount || 1)
                    delegate: AppTabButton {
                        required property int index
                        text: String(index + 1)
                        onClicked: page.hw.setSpdifCount(index + 1)
                    }
                }
            }
        }
        Repeater {
            model: page.has.multiSpdif === true ? (page.state.spdifEnabledCount || 1) : 1
            delegate: HwRow {
                id: spdifRow
                required property int index
                readonly property bool multi: page.has.multiSpdif === true
                readonly property int pin: (page.state.spdifPins || [])[index] === undefined ? -1 : page.state.spdifPins[index]
                title: multi ? qsTr("S/PDIF %1").arg(index + 1) : qsTr("SPDIF RX")
                caption: multi ? qsTr("GPIO pin for S/PDIF input %1 (TOSLINK RX module or comparator).").arg(index + 1)
                               : qsTr("GPIO pin for incoming S/PDIF signal from a TOSLINK RX module or comparator.")
                PinPicker {
                    objectName: "Inputs.spdifPin." + index
                    label: qsTr("S/PDIF %1 pin").arg(index + 1)
                    pins: { page.hw.revision; return page.hw.pinChoices(HardwareController.SpdifRx, spdifRow.index, spdifRow.pin) }
                    pin: spdifRow.pin
                    onPicked: p => page.hw.setSpdifPin(spdifRow.index, p)
                }
            }
        }
        // LG Sound Sync reads the LG TV's signalling on the optical input
        SettingsToggle {
            objectName: "Inputs.lg"
            title: qsTr("LG Sound Sync")
            caption: qsTr("Decode the LG TV's TOSLINK volume + mute signaling and apply it as the volume - TV remote becomes the volume control. Per-preset; saved with the active preset.")
            enabled: page.has.lgSoundSync === true
            checked: page.state.lgEnabled === true
            onToggled: on => page.hw.setLgSoundSync(on)
        }
        Text {
            Layout.fillWidth: true
            visible: page.has.lgSoundSync !== true
            text: qsTr("Connected device firmware doesn't support LG Sound Sync. Update to firmware V8 or later.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        StatusLine {
            objectName: "Inputs.status"
        }
    }

    // ---- I2S Input
    SettingsGroup {
        objectName: "Inputs.i2s"
        visible: page.has.i2sInput === true
        title: qsTr("I2S Input")
        iconName: "waves-arrow-down"
        enabled: page.editable
        footer: (page.state.i2sMaxPairs || 1) > 1
                ? qsTr("Wire one ADC serial-data line per stereo pair to the GPIOs above; the shared bit clock and sample rate live in I2S Configuration. DSPi is the clock master and the pairs are sample-aligned. Save a preset to keep this wiring.")
                : qsTr("Wire the ADC's serial-data line to the GPIO above. The bit clock and sample rate live in I2S Configuration; DSPi is the clock master, so the source must follow.")

        HwRow {
            visible: page.has.i2sClockMode === true
            title: qsTr("Clock Mode")
            caption: qsTr("Master: DSPi drives BCK/LRCLK. Slave: an external master drives the clocks and the rate is auto-detected.")
            AppTabBar {
                id: clockMode
                objectName: "Inputs.i2sClockMode"
                label: qsTr("I2S clock mode")
                currentIndex: page.state.i2sClockMode === 1 ? 1 : 0
                function pick(mode) {
                    if (mode === page.state.i2sClockMode) return
                    // Switching the clock role restarts the I2S clocks: ask
                    // first while an I2S DAC is on an output
                    if (page.state.anySlotIsI2s === true) {
                        clockQuestion.mode = mode
                        clockQuestion.open()
                    } else {
                        page.hw.setI2sClockMode(mode)
                    }
                    currentIndex = Qt.binding(() => page.state.i2sClockMode === 1 ? 1 : 0)
                }
                AppTabButton {
                    text: qsTr("Master")
                    onClicked: clockMode.pick(0)
                }
                AppTabButton {
                    text: qsTr("Slave")
                    onClicked: clockMode.pick(1)
                }
            }
        }
        // While the device runs as the slave
        HwRow {
            objectName: "Inputs.i2sLock"
            visible: page.has.i2sClockMode === true && page.state.i2sSlaveActive === true
            title: qsTr("Lock Status")
            caption: page.hw.i2sLock.locked === true
                     ? qsTr("Locked to external clock at %1.").arg(page.hw.i2sLock.detected)
                     : qsTr("Waiting for external clock (measured %1).").arg(page.hw.i2sLock.measured || "-")
            RowLayout {
                spacing: 4
                Rectangle {
                    implicitWidth: 6
                    implicitHeight: 6
                    radius: 3
                    color: page.toneColor(page.hw.i2sLock.tone)
                }
                Text {
                    text: page.hw.i2sLock.text || qsTr("Inactive")
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
            }
        }
        HwRow {
            title: qsTr("Channels")
            caption: page.state.i2sActivePairs === 1
                     ? qsTr("1 stereo pair of 24-bit audio, sample-aligned")
                     : qsTr("%1 stereo pairs of 24-bit audio, sample-aligned").arg(page.state.i2sActivePairs)
            Text {
                visible: (page.state.i2sMaxPairs || 1) <= 1
                text: "2"
                font: Theme.fontMono
                color: Theme.textMuted
            }
            AppTabBar {
                objectName: "Inputs.i2sChannels"
                visible: (page.state.i2sMaxPairs || 1) > 1
                label: qsTr("Channels")
                currentIndex: (page.state.i2sActivePairs || 1) - 1
                Repeater {
                    model: page.state.i2sMaxPairs || 1
                    delegate: AppTabButton {
                        required property int index
                        text: String((index + 1) * 2)
                        onClicked: if ((index + 1) * 2 !== page.state.i2sChannels) page.hw.setI2sChannels((index + 1) * 2)
                    }
                }
            }
        }
        // One data pin per active pair; the parked pins of the others can be
        // set too, so a refused raise can be fixed before trying again
        Repeater {
            model: page.state.i2sActivePairs || 1
            delegate: I2sPairRow { pair: index; required property int index }
        }
        ColumnLayout {
            Layout.fillWidth: true
            visible: (page.state.i2sMaxPairs || 1) > (page.state.i2sActivePairs || 1)
            spacing: Theme.spacingSmall
            AppButton {
                id: unusedToggle
                objectName: "Inputs.unusedPairs"
                property bool open: false
                text: open ? qsTr("Hide unused pairs") : qsTr("Unused pairs")
                iconName: open ? "chevron-down" : "chevron-right"
                onClicked: open = !open
            }
            Repeater {
                model: unusedToggle.open ? Math.max(0, (page.state.i2sMaxPairs || 1) - (page.state.i2sActivePairs || 1)) : 0
                delegate: I2sPairRow { pair: (page.state.i2sActivePairs || 1) + index; required property int index }
            }
        }
    }

    component I2sPairRow: HwRow {
        id: pairRow
        property int pair
        readonly property int pin: (page.state.i2sRxPins || [])[pair] === undefined ? -1 : page.state.i2sRxPins[pair]
        title: qsTr("Serial Data %1").arg(pair + 1)
        caption: qsTr("GPIO data pin for input channels %1-%2").arg(pair * 2 + 1).arg(pair * 2 + 2)
        PinPicker {
            objectName: "Inputs.i2sPin." + pairRow.pair
            label: qsTr("Serial Data %1").arg(pairRow.pair + 1)
            pins: { page.hw.revision; return page.hw.pinChoices(HardwareController.I2sRx, pairRow.pair, pairRow.pin) }
            pin: pairRow.pin
            onPicked: p => page.hw.setI2sRxPin(pairRow.pair, p)
        }
    }

    // ---- ADAT Input (RP2350)
    SettingsGroup {
        objectName: "Inputs.adat"
        visible: page.has.adatIn === true
        title: qsTr("ADAT Input")
        iconName: "cable"
        enabled: page.editable
        footer: qsTr("Pairs with an outboard ADC (for example a Behringer ADA8200) to turn DSPi into an analog-in processor. In master mode, wire DSPi's ADAT output to the box's ADAT input and the box's ADAT output back to the data pin here. The 8 channels arrive as input channels 1-8 with full per-input EQ and metering. Save a preset to keep this wiring.")

        // No default pin: one must be set before it can be switched on
        SettingsToggle {
            objectName: "Inputs.adatEnabled"
            title: qsTr("Enable ADAT Input")
            caption: qsTr("Receive 8 channels of 24-bit audio (44.1/48 kHz) from one TOSLINK optical input into input channels 1-8. Assign a data pin below, then select ADAT as the input source.")
            enabled: page.state.adatInEnabled === true || page.state.adatInPin !== 255
            checked: page.state.adatInEnabled === true
            onToggled: on => page.hw.setAdatInEnabled(on)
        }
        HwRow {
            title: qsTr("Serial Data")
            caption: qsTr("GPIO receiving the ADAT optical input. No default - assign a spare pin (it may match the ADAT output pin for a loopback self-test).")
            PinPicker {
                objectName: "Inputs.adatPin"
                label: qsTr("ADAT input pin")
                pins: { page.hw.revision; return page.hw.pinChoices(HardwareController.AdatIn, 0, page.state.adatInPin) }
                pin: page.state.adatInPin === undefined ? -1 : page.state.adatInPin
                onPicked: p => page.hw.setAdatInPin(p)
            }
        }
        HwRow {
            title: qsTr("Clock Mode")
            caption: qsTr("Master: DSPi owns the sample rate (set in I2S Configuration) and the source syncs to the ADAT output. Slave: an external master owns the clock and the rate is auto-detected.")
            AppTabBar {
                objectName: "Inputs.adatClockMode"
                label: qsTr("ADAT clock mode")
                currentIndex: page.state.adatInClockMode === 1 ? 1 : 0
                AppTabButton {
                    text: qsTr("Master")
                    onClicked: if (page.state.adatInClockMode !== 0) page.hw.setAdatInClockMode(0)
                }
                AppTabButton {
                    text: qsTr("Slave")
                    onClicked: if (page.state.adatInClockMode !== 1) page.hw.setAdatInClockMode(1)
                }
            }
        }
        // Master mode clocks the outboard box from the ADAT output: without it
        // the box free-runs and its stream slips against DSPi's
        Rectangle {
            objectName: "Inputs.adatFreeRunning"
            visible: page.state.adatInEnabled === true && page.state.adatInClockMode === 0
                     && page.state.adatOutEnabled !== true
            Layout.fillWidth: true
            implicitHeight: warning.implicitHeight + 2 * Theme.spacing
            radius: Theme.radius
            color: "transparent"
            border.width: 1
            border.color: Theme.warning
            RowLayout {
                id: warning
                x: Theme.spacing
                y: Theme.spacing
                width: parent.width - 2 * Theme.spacing
                spacing: Theme.spacingSmall
                Icon {
                    Layout.alignment: Qt.AlignTop
                    name: "triangle-alert"
                    size: 14
                    color: Theme.warning
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                    Text {
                        Layout.fillWidth: true
                        text: qsTr("Clock is free-running")
                        font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
                        color: Theme.text
                    }
                    Text {
                        Layout.fillWidth: true
                        text: qsTr("Master mode uses the ADAT output to clock the outboard device but the ADAT output is off. The source runs on its own clock, asynchronous to DSPi, so you will hear periodic disturbances.")
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                    RowLayout {
                        spacing: Theme.spacingSmall
                        AppButton {
                            objectName: "Inputs.enableAdatOut"
                            text: qsTr("Enable ADAT Output")
                            primary: true
                            enabled: page.editable
                            onClicked: page.hw.setAdatOutEnabled(true)
                        }
                        Text {
                            text: qsTr("or switch to Slave mode above")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                        }
                    }
                }
            }
        }
        // While ADAT is the source; the receiver locks in both clock modes
        HwRow {
            objectName: "Inputs.adatLock"
            visible: page.state.adatInActive === true
            title: qsTr("Lock Status")
            caption: page.hw.adatLock.locked === true
                     ? qsTr("Locked and decoding at %1.").arg(page.hw.adatLock.detected)
                     : page.state.adatInSlaveActive === true
                       ? qsTr("Waiting for external clock (measured %1).").arg(page.hw.adatLock.measured || "-")
                       : qsTr("Waiting for a valid ADAT signal on the data pin.")
            RowLayout {
                spacing: 4
                Rectangle {
                    implicitWidth: 6
                    implicitHeight: 6
                    radius: 3
                    color: page.toneColor(page.hw.adatLock.tone)
                }
                Text {
                    text: page.hw.adatLock.text || qsTr("Inactive")
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
            }
        }
    }

    // ---- Source Names (held for the save bar)
    SettingsGroup {
        objectName: "Inputs.names"
        visible: NamesController.sourceNamesSupported
        title: qsTr("Source Names")
        iconName: "tag"
        enabled: page.editable
        footer: qsTr("Shown in the sidebar, on the front panel and in every connected console. An empty name uses the built-in one.")
        Repeater {
            model: NamesController.sources
            delegate: HwRow {
                id: nameRow
                required property var modelData
                title: modelData.title
                caption: modelData.detail
                NameField {
                    objectName: "Inputs.name." + nameRow.modelData.source
                    label: qsTr("Name of %1").arg(nameRow.modelData.title)
                    text: nameRow.modelData.text
                    edited: nameRow.modelData.edited
                    clearHelp: qsTr("Use the built-in name")
                    edit: t => NamesController.setSourceName(nameRow.modelData.source, t)
                }
            }
        }
    }

    QuestionDialog {
        id: clockQuestion
        objectName: "Inputs.clockQuestion"
        property int mode: 0
        title: qsTr("Change I2S clock mode?")
        text: qsTr("One or more I2S outputs are active. Switching between Master and Slave modes may cause sustained loud noises to be emitted by the connected I2S DAC if wiring has not been adjusted.")
        acceptText: qsTr("Change Clock Mode")
        destructive: true
        onAnswered: answer => {
            if (answer === "accept") page.hw.setI2sClockMode(mode)
        }
    }
}
