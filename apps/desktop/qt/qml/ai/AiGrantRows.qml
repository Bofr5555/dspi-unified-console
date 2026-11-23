import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP ODEC -- What the user allowed an agent for a while (AiController.grants):
// one question let it play test signals on a device for ten minutes. A row each,
// with the agent, the device, the time left and Revoke (its next test signal is
// asked again). In the agents' panel, the AI page and the Modern popover.
ColumnLayout {
    id: rows
    property string prefix: "AiGrants"
    property bool compact: false
    objectName: prefix
    visible: AiController.grants.length > 0
    spacing: 4

    Repeater {
        model: AiController.grants
        delegate: RowLayout {
            id: grant
            required property var modelData
            required property int index
            objectName: rows.prefix + "." + index
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            Icon {
                name: "volume-2"
                size: 14
                color: Theme.text
            }
            Text {
                objectName: grant.objectName + ".text"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: grant.modelData.detect
                      //: An AI agent may play test signals and detect the speakers without asking: %1 its name, %2 a device's name, %3 the time left ("9:41")
                      ? qsTr("Test signals and speaker detection allowed: %1 on %2, %3 left").arg(grant.modelData.agent).arg(grant.modelData.device).arg(grant.modelData.left)
                      //: An AI agent may play test signals without asking: %1 its name, %2 a device's name, %3 the time left ("9:41")
                      : qsTr("Test signals allowed: %1 on %2, %3 left").arg(grant.modelData.agent).arg(grant.modelData.device).arg(grant.modelData.left)
                font: rows.compact ? Theme.fontSmall : Theme.fontBody
                color: Theme.text
                wrapMode: rows.compact ? Text.NoWrap : Text.Wrap
                elide: rows.compact ? Text.ElideRight : Text.ElideNone
            }
            AppButton {
                objectName: grant.objectName + ".revoke"
                implicitHeight: 24
                text: qsTr("Revoke")
                toolTip: grant.modelData.detect ? qsTr("Ask again before this agent's next test signal or speaker detection; a detection under way stops")   // -- WP ADETQ --
                                                : qsTr("Ask again before this agent's next test signal")
                onClicked: AiController.revokeGrant(grant.modelData.id)
            }
        }
    }
}
