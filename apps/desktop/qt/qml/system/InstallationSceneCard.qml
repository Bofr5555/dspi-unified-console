import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP FEATUI -- A scene (FEAT1's actions included): its actions in words,
// each with Remove, and a row to add one: mute, unmute, a master volume, or
// switch a zone's source (stored and routed by the hub at the recall).
Card {
    id: card
    required property string sceneId
    property bool editable: false
    readonly property var scene: InstallationController.element(sceneId, InstallationController.generation)
    readonly property var actions: scene.actions || []
    readonly property var ops: [
        { id: "source", name: qsTr("Switch zone source") },
        { id: "mute", name: qsTr("Mute") },
        { id: "unmute", name: qsTr("Unmute") },
        { id: "master_volume", name: qsTr("Master volume") }
    ]
    property string op: "source"
    objectName: "Installation.scene." + sceneId
    Accessible.name: scene.name || ""

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        InstallationName {
            objectName: "Installation.name." + card.sceneId
            label: qsTr("Name of scene %1").arg(card.scene.name || "")
            elementId: card.sceneId
            name: card.scene.name || ""
            editable: card.editable
        }
        Text {
            Layout.fillWidth: true
            text: card.scene.systemMute ? qsTr("Mutes the whole system") : ""
            textFormat: Text.PlainText
            font: Theme.fontSmall
            color: Theme.textMuted
            elide: Text.ElideRight
        }
        AppButton {
            objectName: "Installation.remove." + card.sceneId
            visible: card.editable
            text: qsTr("Remove")
            toolTip: qsTr("Take %1 out of the installation").arg(card.scene.name || "")
            onClicked: InstallationController.remove(card.sceneId)
        }
    }
    Text {
        visible: card.actions.length === 0
        text: qsTr("No actions yet.")
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.textMuted
    }
    Repeater {
        model: card.actions
        delegate: RowLayout {
            required property var modelData
            required property int index
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            Text {
                objectName: "Installation.action." + card.sceneId + "." + index
                Layout.fillWidth: true
                text: "• " + modelData.text
                textFormat: Text.PlainText
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.Wrap
                Accessible.role: Accessible.StaticText
                Accessible.name: modelData.text
            }
            IconButton {
                objectName: "Installation.removeAction." + card.sceneId + "." + index
                visible: card.editable
                iconName: "x"
                Accessible.name: qsTr("Remove the action %1").arg(modelData.text)
                toolTip: Accessible.name
                onClicked: InstallationController.removeSceneAction(card.sceneId, index)
            }
        }
    }
    Flow {
        Layout.fillWidth: true
        visible: card.editable
        spacing: Theme.spacingSmall
        InstallationChoice {
            objectName: "Installation.actionOp." + card.sceneId
            label: qsTr("What the new action of %1 does").arg(card.scene.name || "")
            choices: card.ops
            value: card.op
            onChosen: id => card.op = id
        }
        InstallationChoice {
            id: target
            objectName: "Installation.actionTarget." + card.sceneId
            label: qsTr("Where the new action of %1 acts").arg(card.scene.name || "")
            choices: InstallationController.sceneTargetChoices(InstallationController.generation)
                .filter(c => card.op !== "source" || c.id.startsWith("zone:"))
            onChosen: id => value = id
            onChoicesChanged: if (choices.findIndex(c => c.id === value) < 0) value = choices.length > 0 ? choices[0].id : ""
        }
        InstallationChoice {
            id: source
            objectName: "Installation.actionSource." + card.sceneId
            visible: card.op === "source"
            label: qsTr("The source the zone switches to")
            choices: InstallationController.sourceChoices(InstallationController.generation).filter(c => c.id.length > 0)
            onChosen: id => value = id
            onChoicesChanged: if (choices.findIndex(c => c.id === value) < 0) value = choices.length > 0 ? choices[0].id : ""
        }
        AppSpinBox {
            id: level
            objectName: "Installation.actionLevel." + card.sceneId
            visible: card.op === "master_volume"
            realFrom: -60
            realTo: 0
            realStep: 1
            realValue: -12
            unit: "dB"
            label: qsTr("Master volume of the new action")
        }
        AppButton {
            objectName: "Installation.addAction." + card.sceneId
            text: qsTr("Add Action")
            onClicked: InstallationController.addSceneAction(card.sceneId, card.op, target.value, level.realValue, source.value)
        }
    }
    Text {
        objectName: "Installation.issue." + card.sceneId
        Layout.fillWidth: true
        visible: text.length > 0
        text: InstallationController.issueOf(card.sceneId, "actions", InstallationController.generation)
              || InstallationController.issueOf(card.sceneId, "name", InstallationController.generation)
              || InstallationController.issueOf(card.sceneId, "", InstallationController.generation)
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.dangerText
        wrapMode: Text.Wrap
        Accessible.role: Accessible.AlertMessage
        Accessible.name: text
    }
}
