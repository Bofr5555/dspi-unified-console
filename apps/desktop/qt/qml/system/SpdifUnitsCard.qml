import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP ATQT -- The S/PDIF transport in Window > System's "Clock and audio
// links" (PLAN-AT 1.2; the web's SpdifUnits, WP ATWEB): between DSPi units
// S/PDIF carries the time base in its user bits. A source stamps its frame
// counter on the output slots it names (never a speaker); a follower plays
// each sample its D after it arrived. Foreign S/PDIF carries none: a follower
// that hears no DSPi time base says so (notice `spdif_no_time_base`). The
// edits are the draft's, saved with it; the hub validates the model.
ColumnLayout {
    id: card
    objectName: "SystemWindow.spdif"
    // Admin, and nothing on its way (SystemWindow's `editable`)
    property bool editable: false
    readonly property var noTimeBase: NoticeCenter.row("spdif_no_time_base")
    spacing: Theme.spacingSmall

    Text {
        text: qsTr("S/PDIF units")
        font: Theme.fontBody
        color: Theme.text
    }
    Text {
        Layout.fillWidth: true
        visible: SystemController.spdifUnits.length === 0
        text: qsTr("No unit is on S/PDIF.")
        font: Theme.fontSmall
        color: Theme.textMuted
    }
    Repeater {
        model: SystemController.spdifUnits
        delegate: ColumnLayout {
            id: row
            required property var modelData
            readonly property bool source: modelData.role === "source"
            objectName: "SystemWindow.spdifUnit." + modelData.id
            property string shownName: modelData.name + ", " + (source ? qsTr("Source") : qsTr("Follows %1").arg(modelData.sourceName))
                                       + ", " + modelData.stateWord
            Layout.fillWidth: true
            spacing: 2
            Accessible.role: Accessible.Row
            Accessible.name: shownName
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacing
                Text {
                    Layout.preferredWidth: 180
                    text: row.modelData.name
                    font: Theme.fontBody
                    color: Theme.text
                    elide: Text.ElideRight
                }
                // A follower's source; a source's role
                Text {
                    visible: row.source || !card.editable
                    Layout.preferredWidth: 170
                    text: row.source ? qsTr("Source") : qsTr("Follows %1").arg(row.modelData.sourceName || row.modelData.source)
                    font: Theme.fontBody
                    color: Theme.text
                    HoverHandler { id: roleHover }
                    ToolTip.visible: roleHover.hovered
                    ToolTip.text: qsTr("A source stamps its frame counter in the S/PDIF user bits; a follower plays each sample its D after it arrived")
                }
                AppComboBox {
                    objectName: "SystemWindow.spdifSource." + row.modelData.id
                    visible: !row.source && card.editable
                    implicitWidth: 170
                    label: qsTr("S/PDIF source of %1").arg(row.modelData.name)
                    readonly property var ids: SystemController.spdifSources.map(s => s.id)
                    model: SystemController.spdifSources.map(s => qsTr("Follows %1").arg(s.name))
                    currentIndex: (SystemController.spdifDrafted, Math.max(0, ids.indexOf(SystemController.spdifSourceOf(row.modelData.id))))
                    onActivated: i => SystemController.setSpdifSource(row.modelData.id, ids[i])
                }
                // A source's time-base slots and D
                Repeater {
                    model: row.source && card.editable ? 4 : 0
                    delegate: AppCheckBox {
                        required property int index
                        objectName: "SystemWindow.spdifSlot." + row.modelData.id + "." + index
                        text: qsTr("%1-%2").arg(2 * index + 1).arg(2 * index + 2)
                        checked: (SystemController.spdifDrafted, SystemController.spdifUnits, (SystemController.spdifMaskOf(row.modelData.id) >> index) & 1) === 1
                        Accessible.name: qsTr("%1 sends the time base on outputs %2-%3").arg(row.modelData.name).arg(2 * index + 1).arg(2 * index + 2)
                        onToggled: if (!SystemController.setSpdifSlot(row.modelData.id, index, checked)) checked = !checked
                    }
                }
                Text {
                    visible: row.source && !card.editable
                    text: row.modelData.slotsWord
                    font: Theme.fontBody
                    color: Theme.text
                }
                BorderedField {
                    id: dField
                    objectName: "SystemWindow.spdifD." + row.modelData.id
                    visible: row.source && card.editable
                    Layout.preferredWidth: 80
                    label: qsTr("S/PDIF D of %1 in samples").arg(row.modelData.name)
                    placeholderText: qsTr("The hub's")
                    validator: IntValidator { bottom: 0; top: 65535 }
                    Accessible.name: label
                    function sync() { if (!activeFocus) text = SystemController.spdifDText(row.modelData.id) }
                    Component.onCompleted: sync()
                    Connections {
                        target: SystemController
                        function onDraftChanged() { dField.sync() }
                        function onSpdifChanged() { dField.sync() }
                    }
                    onCommitted: t => { if (!SystemController.setSpdifD(row.modelData.id, t)) sync() }
                    HoverHandler { id: dHover }
                    ToolTip.visible: dHover.hovered
                    ToolTip.text: qsTr("The domain's D, samples (129 to 399 at 48 kHz); empty: the hub's")
                }
                Text {
                    visible: !(row.source && card.editable)
                    text: row.source ? qsTr("D %1").arg(row.modelData.d !== null && row.modelData.d !== undefined ? row.modelData.d : qsTr("the hub's"))
                                     : qsTr("D %1").arg(row.modelData.unitD !== null && row.modelData.unitD !== undefined ? row.modelData.unitD : "–")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
                Text {
                    objectName: "SystemWindow.spdifState." + row.modelData.id
                    Layout.fillWidth: true
                    text: row.modelData.stateWord
                    font: Theme.fontBody
                    color: row.modelData.lockError ? Theme.dangerText : !row.modelData.connected ? Theme.offline : Theme.text
                }
                AppButton {
                    objectName: "SystemWindow.spdifOff." + row.modelData.id
                    visible: card.editable
                    text: qsTr("Off S/PDIF")
                    toolTip: qsTr("Back on the audio link")
                    onClicked: SystemController.removeSpdif(row.modelData.id)
                }
            }
            // The follower hears no DSPi time base
            Text {
                objectName: "SystemWindow.spdifNoTimeBase." + row.modelData.id
                Layout.fillWidth: true
                visible: row.modelData.noTimeBase
                text: qsTr("%1: %2 %3 %4").arg(card.noTimeBase.title || qsTr("This S/PDIF source does not carry a DSPi time base"))
                          .arg(row.modelData.name).arg(card.noTimeBase.reason || "").arg(card.noTimeBase.action || "")
                font: Theme.fontSmall
                color: Theme.warning
                wrapMode: Text.Wrap
                Accessible.role: Accessible.AlertMessage
            }
        }
    }
    // Put a unit on S/PDIF (Admin)
    RowLayout {
        objectName: "SystemWindow.spdifAdd"
        visible: card.editable && SystemController.spdifCandidates.length > 0
        spacing: Theme.spacingSmall
        AppComboBox {
            id: addUnit
            objectName: "SystemWindow.spdifAddUnit"
            implicitWidth: 200
            label: qsTr("Unit to put on S/PDIF")
            readonly property var ids: SystemController.spdifCandidates.map(u => u.id)
            model: SystemController.spdifCandidates.map(u => u.name)
        }
        AppComboBox {
            id: addAs
            objectName: "SystemWindow.spdifAddAs"
            implicitWidth: 240
            label: qsTr("As")
            readonly property var ids: ["source"].concat(SystemController.spdifSources.map(s => s.id))
            model: [qsTr("S/PDIF time-base source")].concat(SystemController.spdifSources.map(s => qsTr("Fed by S/PDIF from %1").arg(s.name)))
        }
        AppButton {
            objectName: "SystemWindow.spdifAddButton"
            text: qsTr("Add")
            enabled: addUnit.currentIndex >= 0 && addAs.currentIndex >= 0
            onClicked: SystemController.addSpdif(addUnit.ids[addUnit.currentIndex], addAs.ids[addAs.currentIndex])
        }
    }
    Repeater {
        model: SystemController.spdifDrafted
        delegate: Text {
            required property string modelData
            Layout.fillWidth: true
            text: qsTr("To save: %1").arg(modelData)
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }
}
