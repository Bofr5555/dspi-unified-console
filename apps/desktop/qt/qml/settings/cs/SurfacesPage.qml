import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- Settings > Control Surfaces (the Mac's
// ControlSurfacesSettingsTab, controls section): the wired components, one
// card each, collapsed to an overview; a newly added one opens.
SettingsPage {
    id: page
    objectName: "SurfacesPage"
    property var opened: ({})
    function isOpen(key) { return page.opened[key] === true }
    function setOpen(key, on) {
        const o = Object.assign({}, page.opened)
        o[key] = on
        page.opened = o
    }

    CsKeys { id: cards; page: "controls" }
    Connections {
        target: CsController
        function onAdded(key) { page.setOpen(key, true) }
    }
    // The Mac reads the configuration again when this page appears; a learn
    // waiting for a press ends when it goes
    Component.onCompleted: CsController.fetch()
    Component.onDestruction: CsController.cancelLearn()

    CsPageState {}

    CsEmptyState {
        objectName: "Cs.controls.empty"
        visible: CsController.loaded && cards.list.length === 0
        badgeTypes: CsController.revision < 0 ? [] : CsController.addableTypes("controls").map(t => t.type)
        title: qsTr("No Controls Configured")
        text: qsTr("Wire a button, switch, knob, encoder, or LED to a spare GPIO and bind it to a device function.")
        CsAddButton {
            objectName: "Cs.controls.addEmpty"
            text: qsTr("Add Control")
            primary: true
            types: CsController.revision < 0 ? [] : CsController.addableTypes("controls")
        }
    }

    Repeater {
        model: cards.list
        delegate: CsBindingCard {
            required property string modelData
            key: modelData
            expanded: page.isOpen(modelData)
            onExpandedChanged: if (expanded !== page.isOpen(modelData)) page.setOpen(modelData, expanded)
        }
    }

    RowLayout {
        visible: CsController.loaded && cards.list.length > 0
        Layout.fillWidth: true
        CsAddButton {
            objectName: "Cs.controls.add"
            text: qsTr("Add Control")
            types: CsController.revision < 0 ? [] : CsController.addableTypes("controls")
        }
        Item { Layout.fillWidth: true }
        Text {
            visible: CsController.revision >= 0 && !CsController.canAdd("binding") && CsController.writable
            text: qsTr("All %1 control slots are in use.").arg(CsController.slotCount)
            font: Theme.fontSmall
            color: Theme.textMuted
        }
    }

    SettingsGroup {
        footer: CsController.capsVersion !== 0 ? qsTr("Control-surface capability version %1.").arg(CsController.capsVersion) : ""
        Text {
            Layout.fillWidth: true
            text: qsTr("Wire push buttons, toggle switches, potentiometers, rotary encoders, indicator LEDs, and an IR remote receiver to spare GPIOs and bind each to a device function. Buttons and switches wire between the GPIO and GND (internal pull-up); pots use an ADC pin (GPIO 26, 27, or 28) between 3V3 and GND; encoders use two GPIOs with the common wired to GND. LEDs drive active-high by default. An IR receiver module's OUT pin connects to any GPIO, and its remote buttons are learned by pressing them at the device.\n\nThis wiring is a board-level setting: it is stored on the device, survives preset changes, and survives a factory reset. Changes take effect immediately as a live preview; use Save to keep them across a reboot, or Revert to discard them.")
            font: Theme.fontCaption
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }
}
