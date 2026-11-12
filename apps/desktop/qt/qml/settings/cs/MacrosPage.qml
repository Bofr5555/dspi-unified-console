import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- Settings > Macros (the Mac's macros section): short sequences
// of changes one press runs. A macro's steps wait for its Apply; Run plays the
// one stored on the device.
SettingsPage {
    id: page
    objectName: "MacrosPage"
    property var opened: ({})
    function isOpen(key) { return page.opened[key] === true }
    function setOpen(key, on) {
        const o = Object.assign({}, page.opened)
        o[key] = on
        page.opened = o
    }

    CsKeys { id: cards; page: "macros" }
    Connections {
        target: CsController
        function onAdded(key) { page.setOpen(key, true) }
    }

    CsPageState {}

    CsEmptyState {
        objectName: "Cs.macros.empty"
        visible: CsController.loaded && cards.list.length === 0
        iconName: "list-ordered"
        title: qsTr("No Macros Configured")
        text: qsTr("Run a short sequence of changes from a single press: select an input and load a preset, switch monitors, mute after a delay.")
        AppButton {
            objectName: "Cs.macros.addEmpty"
            text: qsTr("Add Macro")
            iconName: "plus"
            primary: true
            enabled: CsController.revision >= 0 && CsController.canAdd("macro")
            onClicked: CsController.addMacro()
        }
    }

    Repeater {
        model: cards.list
        delegate: CsCard {
            id: card
            required property string modelData
            objectName: "Cs." + modelData.replace("/", ".")
            rec: CsController.revision < 0 ? ({}) : CsController.record(modelData)
            iconName: rec.running === true ? "play" : "list-ordered"
            iconColor: rec.running === true ? Theme.success : Theme.accentText
            expanded: page.isOpen(modelData)
            onExpandedChanged: if (expanded !== page.isOpen(modelData)) page.setOpen(modelData, expanded)
            headerExtras: [
                IconButton {
                    objectName: card.objectName + ".run"
                    iconName: card.rec.running === true ? "square" : "play"
                    iconSize: 12
                    toolTip: card.rec.running === true ? qsTr("Stop this macro") : qsTr("Run this macro now")
                    // Running tests the sequence as stored, so it waits for Apply
                    enabled: CsController.writable && (card.rec.running === true || (card.rec.liveSteps || 0) > 0)
                    onClicked: CsController.fire(card.modelData)
                }
            ]

            Repeater {
                model: card.rec.stepCount || 0
                delegate: ColumnLayout {
                    id: stepItem
                    required property int index
                    readonly property var step: (card.rec.steps || [])[index] || ({})
                    Layout.fillWidth: true
                    spacing: 6
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8
                        Text {
                            text: stepItem.index + 1
                            font: Theme.fontMono
                            color: Theme.textMuted
                            horizontalAlignment: Qt.AlignRight
                            Layout.preferredWidth: 16
                        }
                        Text {
                            objectName: card.objectName + ".step." + stepItem.index + ".summary"
                            Layout.fillWidth: true
                            text: stepItem.step.summary || ""
                            font: Theme.fontSmall
                            color: Theme.textMuted
                            elide: Text.ElideRight
                        }
                        IconButton {
                            iconName: "arrow-up"
                            iconSize: 11
                            toolTip: qsTr("Move up")
                            enabled: stepItem.index > 0
                            onClicked: CsController.moveStep(card.modelData, stepItem.index, -1)
                        }
                        IconButton {
                            iconName: "arrow-down"
                            iconSize: 11
                            toolTip: qsTr("Move down")
                            enabled: stepItem.index < (card.rec.stepCount || 0) - 1
                            onClicked: CsController.moveStep(card.modelData, stepItem.index, 1)
                        }
                        IconButton {
                            objectName: card.objectName + ".step." + stepItem.index + ".remove"
                            iconName: "circle-minus"
                            iconSize: 11
                            toolTip: qsTr("Remove this step")
                            onClicked: CsController.removeStep(card.modelData, stepItem.index)
                        }
                    }
                    CsRows {
                        objectName: card.objectName + ".step." + stepItem.index
                        rows: stepItem.step.rows || []
                        enabled: CsController.writable && card.rec.applying !== true
                        onEdited: (id, value) => CsController.editStep(card.modelData, stepItem.index, id, value)
                    }
                }
            }
            RowLayout {
                Layout.fillWidth: true
                AppButton {
                    objectName: card.objectName + ".addStep"
                    text: qsTr("Add Step")
                    iconName: "plus"
                    enabled: CsController.writable && (card.rec.stepCount || 0) < (card.rec.maxSteps || 0)
                    onClicked: CsController.addStep(card.modelData)
                }
                Item { implicitHeight: 1; Layout.fillWidth: true }
                Text {
                    visible: (card.rec.stepCount || 0) >= (card.rec.maxSteps || 0)
                    text: qsTr("A macro holds up to %1 steps.").arg(card.rec.maxSteps || 0)
                    font: Theme.fontCaption
                    color: Theme.textMuted
                }
            }
        }
    }

    RowLayout {
        visible: CsController.loaded && cards.list.length > 0
        Layout.fillWidth: true
        AppButton {
            objectName: "Cs.macros.add"
            text: qsTr("Add Macro")
            iconName: "plus"
            enabled: CsController.revision >= 0 && CsController.canAdd("macro")
            onClicked: CsController.addMacro()
        }
        Item { implicitHeight: 1; Layout.fillWidth: true }
        Text {
            visible: CsController.revision >= 0 && !CsController.canAdd("macro") && CsController.writable
            text: qsTr("All %1 macro slots are in use.").arg(CsController.macroCount)
            font: Theme.fontSmall
            color: Theme.textMuted
        }
    }

    SettingsGroup {
        Text {
            Layout.fillWidth: true
            text: qsTr("A macro runs a short sequence of changes from a single press: select an input and load a preset, switch monitors, mute after a delay. Bind a button, switch, or remote key to \"Macro\" to fire one.\n\nEach step can wait before it runs, and can address a channel group. One macro runs at a time - firing another cancels the first at its current step.\n\nMacros are stored on the device alongside the controls and share their Save and Revert.")
            font: Theme.fontCaption
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }
}
