import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- A control's card (the Mac's slotSection): the editor of one
// binding slot. The IR receiver holds its remote buttons, the display its
// settings and pages, and an aux output its switch and level, which apply as
// they change.
CsCard {
    id: card
    property string key
    objectName: "Cs." + key.replace("/", ".")
    rec: CsController.revision < 0 ? ({}) : CsController.record(key)
    badgeType: rec.type || 0
    typeChoices: rec.types || []
    showPill: true
    readonly property bool aux: rec.type === CsController.AuxOut || rec.type === CsController.AuxPwm

    RowLayout {
        visible: card.rec.inactive === true
        Layout.fillWidth: true
        spacing: 6
        Icon {
            name: "triangle-alert"
            size: 12
            color: Theme.warning
            Layout.alignment: Qt.AlignTop
        }
        Text {
            objectName: card.objectName + ".inactive"
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: card.rec.inactiveReason || ""
            font: Theme.fontSmall
            color: Theme.warning
            wrapMode: Text.Wrap
        }
    }

    // An aux output's live switch and level: at once, never to flash
    SettingsToggle {
        objectName: card.objectName + ".auxOn"
        visible: card.aux
        title: qsTr("Output")
        caption: card.rec.auxLive === true ? qsTr("Switches the pin now. Instant, and never written to flash.")
                                           : qsTr("Apply the output first; the switch works once it is running.")
        checked: card.rec.auxOn === true
        enabled: card.rec.auxLive === true && CsController.writable
        onToggled: on => CsController.setAuxState(card.rec.slot, on)
    }
    HwRow {
        visible: card.aux && card.rec.type === CsController.AuxPwm
        enabled: card.rec.auxLive === true && CsController.writable
        title: qsTr("Level")
        caption: qsTr("How bright or fast the load runs while the output is on.")
        RowLayout {
            spacing: 8
            CustomSlider {
                id: level
                objectName: card.objectName + ".auxLevel"
                implicitWidth: 160
                from: 0
                to: 100
                value: card.rec.auxLevel || 0
                label: qsTr("Level")
                onValueMoved: v => CsController.setAuxLevel(card.rec.slot, v)
                onCommitted: v => CsController.setAuxLevel(card.rec.slot, v)
            }
            Text {
                textFormat: Text.PlainText
                text: qsTr("%1%").arg(Math.round(level.value))
                font: Theme.fontMono
                color: Theme.text
                horizontalAlignment: Qt.AlignRight
                Layout.preferredWidth: 38
            }
        }
    }

    CsRows {
        objectName: card.objectName + ".rows"
        rows: card.rec.rows || []
        enabled: CsController.writable && card.rec.applying !== true
        onEdited: (id, value) => CsController.edit(card.key, id, value)
    }

    CsRemoteButtons {
        visible: card.rec.type === CsController.Ir
        receiver: card.rec
    }

    Loader {
        Layout.fillWidth: true
        active: card.rec.type === CsController.Display && card.expanded && card.rec.liveConfigured === true
        visible: active
        sourceComponent: CsDisplay {}
    }

    applyLeading: [
        AppButton {
            objectName: card.objectName + ".addRemote"
            visible: card.rec.type === CsController.Ir && card.expanded
            text: qsTr("Add Remote Button")
            enabled: CsController.writable && CsController.revision >= 0 && CsController.canAdd("ir")
            onClicked: CsController.addRemoteButton()
        }
    ]
}
