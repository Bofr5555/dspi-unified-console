import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- Settings > Auxiliary Outputs (the Mac's aux section): the
// binding slots that hold an on/off or dimmable output, as cards like the
// controls'.
SettingsPage {
    id: page
    objectName: "AuxPage"
    property var opened: ({})
    function isOpen(key) { return page.opened[key] === true }
    function setOpen(key, on) {
        const o = Object.assign({}, page.opened)
        o[key] = on
        page.opened = o
    }

    CsKeys { id: cards; page: "aux" }
    Connections {
        target: CsController
        function onAdded(key) { page.setOpen(key, true) }
    }

    CsPageState {}

    CsEmptyState {
        objectName: "Cs.aux.empty"
        visible: CsController.loaded && cards.list.length === 0
        badgeTypes: CsController.revision < 0 ? [] : CsController.addableTypes("aux").map(t => t.type)
        title: qsTr("No Auxiliary Outputs Set Up")
        text: qsTr("Put a relay, lamp or fan on a spare GPIO, then point a button, knob or remote key at it from the Control Surfaces page.")
        CsAddButton {
            objectName: "Cs.aux.addEmpty"
            text: qsTr("Add Output")
            primary: true
            types: CsController.revision < 0 ? [] : CsController.addableTypes("aux")
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
            objectName: "Cs.aux.add"
            text: qsTr("Add Output")
            types: CsController.revision < 0 ? [] : CsController.addableTypes("aux")
        }
        Item { implicitHeight: 1; Layout.fillWidth: true }
        Text {
            visible: CsController.revision >= 0 && !CsController.canAdd("binding") && CsController.writable
            text: qsTr("All %1 control slots are in use.").arg(CsController.slotCount)
            font: Theme.fontSmall
            color: Theme.textMuted
        }
    }

    SettingsGroup {
        Text {
            Layout.fillWidth: true
            text: qsTr("An auxiliary output is a GPIO the DSPi switches or dims for you and never reads itself: it changes nothing about the sound. It exists so a button, knob, remote key or macro can drive something the device knows nothing about - an amplifier trigger, a speaker relay, a panel lamp, a fan.\n\nAn on/off output follows its switch. A dimmable output follows its switch and its level, so one button and one knob can share a lamp. Turn on \"Active-Low Output\" for the relay and opto-isolator boards that switch when the pin goes low.\n\nA GPIO is a 3.3 V pin good for a few milliamps. Anything real needs a MOSFET, a transistor with a flyback diode, or an opto-isolated relay module in between, and a dimmed load should have its own supply so its switching noise stays out of the DAC.\n\nSwitching an output is instant and never writes to flash. The pin, name and power-on behaviour are stored on the device alongside the controls and share their Save and Revert.")
            font: Theme.fontCaption
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }
}
