import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP11 -- Settings > Outputs (the Mac's HardwareSettingsTab outputs
// page): each output slot's bus and GPIO, the PDM sub's GPIO, the ADAT bulk
// output, and Reset Pins. Changes apply to the device at once; the save bar
// keeps them (behaviour.md §8).
SettingsPage {
    id: page
    objectName: "OutputsPage"
    readonly property var hw: HardwareController
    readonly property var state: hw.hw
    readonly property bool editable: hw.writable

    Component.onCompleted: { hw.clearStatus(); hw.watch(true) }
    Component.onDestruction: hw.watch(false)

    SettingsGroup {
        objectName: "Outputs.slots"
        title: qsTr("Slots")
        iconName: "layout-grid"
        enabled: page.editable

        Repeater {
            model: page.state.outputs || []
            delegate: RowLayout {
                id: row
                required property var modelData
                required property int index
                objectName: "Outputs.row." + modelData.id
                Layout.fillWidth: true
                spacing: Theme.spacingSmall

                // The channel's colour as a small tag (owner rule)
                Rectangle {
                    implicitWidth: 8
                    implicitHeight: 8
                    radius: 4
                    color: row.modelData.slot ? ChannelPalette.output(row.modelData.channel, false) : ChannelPalette.pdm
                }
                Text {
                    Layout.preferredWidth: 64
                    text: row.modelData.title
                    font: Theme.fontBody
                    color: page.editable ? Theme.text : Theme.textDisabled
                }
                Rectangle {
                    visible: row.modelData.isDefault
                    objectName: "Outputs.default." + row.modelData.id
                    implicitWidth: defaultText.implicitWidth + 14
                    implicitHeight: defaultText.implicitHeight + 4
                    radius: height / 2
                    color: Theme.controlHover
                    Text {
                        id: defaultText
                        anchors.centerIn: parent
                        text: qsTr("Default")
                        font: Theme.fontCaption
                        color: Theme.textMuted
                    }
                }
                Item {
                    Layout.fillWidth: true
                    implicitHeight: 1
                }
                // The bus: S/PDIF or I2S for a slot (I2S where the chip allows
                // it), PDM only for the sub
                ChoicePicker {
                    objectName: "Outputs.type." + row.modelData.id
                    label: qsTr("%1 type").arg(row.modelData.name)
                    implicitWidth: 92
                    enabled: row.modelData.slot
                    choices: row.modelData.slot
                             ? (row.modelData.i2sAllowed ? [{ value: 0, text: qsTr("S/PDIF") }, { value: 1, text: qsTr("I2S") }]
                                                          : [{ value: 0, text: qsTr("S/PDIF") }])
                             : [{ value: 0, text: qsTr("PDM") }]
                    value: row.modelData.type
                    onChosen: v => page.hw.setOutputType(row.modelData.id, v)
                }
                // The STM32H723 drives fixed pins: nothing to assign
                PinPicker {
                    objectName: "Outputs.pin." + row.modelData.id
                    visible: !page.hw.stm32
                    label: qsTr("%1 pin").arg(row.modelData.name)
                    pins: { page.hw.revision; return page.hw.pinChoices(HardwareController.OutputPin, row.modelData.id, row.modelData.pin) }
                    pin: row.modelData.pin
                    onPicked: p => page.hw.setOutputPin(row.modelData.id, p)
                }
            }
        }
    }

    // ADAT bulk output (RP2350): all eight outputs as one lightpipe
    SettingsGroup {
        objectName: "Outputs.adat"
        visible: page.hw.has.adatOut === true
        title: qsTr("Bulk Output")
        iconName: "cable"
        enabled: page.editable
        SettingsToggle {
            objectName: "Outputs.adatEnabled"
            title: qsTr("Enable ADAT")
            caption: qsTr("Stream all 8 output channels as one optical ADAT lightpipe (44.1/48 kHz, 24-bit). Runs alongside the existing outputs; drive a TOSLINK transmitter from the data pin.")
            checked: page.state.adatOutEnabled === true
            onToggled: on => page.hw.setAdatOutEnabled(on)
        }
        HwRow {
            title: qsTr("Serial Data")
            caption: qsTr("GPIO driving the ADAT optical output. Default GPIO %1.").arg(12)
            PinPicker {
                objectName: "Outputs.adatPin"
                label: qsTr("ADAT data pin")
                pins: { page.hw.revision; return page.hw.pinChoices(HardwareController.AdatOut, 0, page.state.adatOutPin) }
                pin: page.state.adatOutPin === undefined ? -1 : page.state.adatOutPin
                onPicked: p => page.hw.setAdatOutPin(p)
            }
        }
    }

    // The outcome of the last change, and Reset Pins
    RowLayout {
        Layout.fillWidth: true
        visible: !page.hw.stm32 || page.hw.statusText.length > 0
        spacing: Theme.spacingSmall
        StatusLine {
            objectName: "Outputs.status"
            Layout.fillWidth: true
        }
        Item {
            Layout.fillWidth: true
            visible: page.hw.statusText.length === 0
        }
        AppButton {
            objectName: "Outputs.resetPins"
            visible: !page.hw.stm32
            text: qsTr("Reset Pins")
            enabled: page.editable
            onClicked: page.hw.resetPins()
        }
    }
}
