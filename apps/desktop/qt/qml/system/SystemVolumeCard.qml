import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP FEATUI -- A room's or zone's volume (FEAT2's dspi_system_volume_*):
// choose a room or zone and a level or a step, see per unit what it is and
// would be (louder ones marked), then Set. A unit the hub or a limits profile
// refuses says why; nothing is written until Set, and Set asks first when a
// unit gets louder.
ColumnLayout {
    id: card
    objectName: "SystemVolume"
    spacing: Theme.spacingSmall
    readonly property var plan: SystemController.volumePlan
    readonly property var rows: plan.units || []
    readonly property int louder: rows.filter(u => u.louder && u.state === "ready").length
    readonly property int ready: rows.filter(u => u.state === "ready").length
    property string target: ""
    onVisibleChanged: if (visible) SystemController.loadVolumeTargets()
    Component.onCompleted: if (visible) SystemController.loadVolumeTargets()

    function db(v) { return v === undefined || v === null ? "–" : qsTr("%1 dB").arg(Number(v).toFixed(1)) }

    SectionHeading {
        Layout.fillWidth: true
        text: qsTr("Room and zone volume")
    }
    Flow {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        AppComboBox {
            id: where
            objectName: "SystemVolume.target"
            implicitWidth: 200
            label: qsTr("Room or zone")
            model: SystemController.volumeTargets.map(t => t.name)
            onActivated: i => { card.target = SystemController.volumeTargets[i].id; SystemController.dropVolume() }
            onModelChanged: {
                const i = SystemController.volumeTargets.findIndex(t => t.id === card.target)
                currentIndex = i >= 0 ? i : (SystemController.volumeTargets.length > 0 ? 0 : -1)
                card.target = currentIndex >= 0 ? SystemController.volumeTargets[currentIndex].id : ""
            }
        }
        AppComboBox {
            id: how
            objectName: "SystemVolume.how"
            implicitWidth: 120
            label: qsTr("Level or step")
            model: [qsTr("Set to"), qsTr("Step by")]
        }
        AppSpinBox {
            id: level
            objectName: "SystemVolume.level"
            label: qsTr("Volume")
            realFrom: how.currentIndex === 1 ? -20 : -80
            realTo: how.currentIndex === 1 ? 20 : 0
            realStep: 1
            realValue: how.currentIndex === 1 ? -3 : -20
            unit: "dB"
        }
        AppCheckBox {
            id: master
            objectName: "SystemVolume.master"
            text: qsTr("Master volume")
            Accessible.name: qsTr("Set the master volume instead of the user volume")
        }
        AppButton {
            objectName: "SystemVolume.plan"
            text: qsTr("Show Changes")
            enabled: card.target.length > 0 && !SystemController.busy
            toolTip: qsTr("Read each unit's volume and show what would change; nothing is written yet")
            onClicked: SystemController.planVolume(card.target, how.currentIndex === 1, level.realValue, master.checked)
        }
    }
    Repeater {
        model: card.rows
        delegate: RowLayout {
            required property var modelData
            required property int index
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            Text {
                objectName: "SystemVolume.row." + index
                Layout.fillWidth: true
                textFormat: Text.PlainText
                //: %1 a unit, %2 its volume now, %3 what it will be or is
                text: qsTr("%1: %2 → %3").arg(modelData.name).arg(card.db(modelData.beforeDb))
                          .arg(card.db(modelData.afterDb !== undefined && modelData.afterDb !== null ? modelData.afterDb : modelData.targetDb))
                      + (modelData.louder && modelData.state === "ready" ? "  " + qsTr("(louder)") : "")
                      + (modelData.state === "unchanged" ? "  " + qsTr("(already there)") : "")
                      + (modelData.state === "done" ? "  " + qsTr("(set)") : "")
                      + (modelData.state === "refused" ? "  " + qsTr("(not set: %1)").arg(modelData.message) : "")
                font: Theme.fontBody
                color: modelData.state === "refused" ? Theme.dangerText : modelData.louder && modelData.state === "ready" ? Theme.warning : Theme.text
                wrapMode: Text.Wrap
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }
        }
    }
    RowLayout {
        visible: card.rows.length > 0 && !SystemController.volumeSet
        spacing: Theme.spacingSmall
        AppButton {
            objectName: "SystemVolume.set"
            text: qsTr("Set")
            primary: true
            enabled: card.ready > 0 && !SystemController.busy
            onClicked: card.louder > 0 ? louderQuestion.open() : SystemController.applyVolume()
        }
        AppButton {
            objectName: "SystemVolume.cancel"
            text: qsTr("Cancel")
            onClicked: SystemController.dropVolume()
        }
    }
    Text {
        objectName: "SystemVolume.message"
        Layout.fillWidth: true
        visible: text.length > 0
        textFormat: Text.PlainText
        text: SystemController.volumeMessage
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
        Accessible.role: Accessible.StaticText
        Accessible.name: text
    }
    QuestionDialog {
        id: louderQuestion
        objectName: "SystemVolume.louderQuestion"
        title: qsTr("Make it louder?")
        //: %n units that get louder; %1 the room or zone
        text: qsTr("%n unit(s) of %1 get louder.", "", card.louder).arg(card.plan.name || "")
        acceptText: qsTr("Set")
        onAnswered: answer => { if (answer === "accept") SystemController.applyVolume() }
    }
}
