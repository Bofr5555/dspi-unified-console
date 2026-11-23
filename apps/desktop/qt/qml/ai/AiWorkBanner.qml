import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP AIVIS -- What an AI agent does now (AiController.work), under the top bar
// in both layouts: which agent, what, on which device, with Pause and Stop, and
// Stop Measuring while its measurement plays. It is there while a call or a job
// of an agent's runs, while one waits for the person, for a while after the last
// call, and as long as the agents are paused.
Rectangle {
    id: strip
    objectName: "AiWorkBanner"
    readonly property var work: AiController.work
    readonly property string text: work.text !== undefined ? work.text : ""
    // -- WP ODEC -- a grant lasts ten minutes: the strip says so while it does
    readonly property var grant: AiController.grants.length ? AiController.grants[0] : null
    visible: work.shown === true || grant !== null
    implicitHeight: row.implicitHeight + 8
    color: Theme.bannerFill("info")
    // In the accent's colour, so that it is none of the other strips
    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 2
        color: Theme.accent
    }
    Rectangle {
        width: 4
        height: parent.height
        color: Theme.accent
    }

    RowLayout {
        id: row
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.leftMargin: Theme.spacing
        anchors.rightMargin: Theme.spacing
        spacing: Theme.spacingSmall
        AiWorkMark {
            objectName: "AiWorkBanner.mark"
            working: AiController.working
        }
        Icon {
            name: "bot"
            size: 14
            color: Theme.text
        }
        Text {
            objectName: "AiWorkBanner.agent"
            visible: text.length > 0
            Layout.maximumWidth: strip.width / 4
            textFormat: Text.PlainText
            text: strip.work.agent !== undefined ? strip.work.agent : ""
            font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
            color: Theme.text
            elide: Text.ElideRight
        }
        Text {
            objectName: "AiWorkBanner.text"
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: strip.text
            font: Theme.fontBody
            color: Theme.text
            elide: Text.ElideRight
            maximumLineCount: 1
        }
        Text {
            objectName: "AiWorkBanner.grant"
            visible: strip.grant !== null
            textFormat: Text.PlainText
            //: The strip: an AI agent may play test signals without asking for %1 ("9:41") more
            text: strip.grant ? qsTr("Test signals allowed, %1 left").arg(strip.grant.left) : ""
            font: Theme.fontSmall
            color: Theme.text
        }
        AppButton {
            objectName: "AiWorkBanner.revokeGrant"
            visible: strip.grant !== null
            implicitHeight: 24
            text: qsTr("Revoke")
            toolTip: qsTr("Ask again before the agent's next test signal")
            onClicked: AiController.revokeGrant(strip.grant.id)
        }
        AppButton {
            objectName: "AiWorkBanner.show"
            visible: strip.work.elsewhere === true
            implicitHeight: 24
            text: qsTr("Show")
            //: %1 is a device's name
            toolTip: qsTr("Show %1").arg(strip.work.device !== undefined ? strip.work.device : "")
            onClicked: AiController.showWork()
        }
        AppButton {
            objectName: "AiWorkBanner.stopJob"
            visible: strip.work.job === true
            implicitHeight: 24
            text: qsTr("Stop Measuring")
            onClicked: AiController.stopJob()
        }
        AppButton {
            objectName: "AiWorkBanner.pause"
            implicitHeight: 24
            text: AiController.paused ? qsTr("Resume") : qsTr("Pause")
            iconName: AiController.paused ? "play" : "pause"
            toolTip: AiController.paused ? qsTr("Let AI agents go on") : qsTr("AI agents wait until Resume; nothing is changed meanwhile")
            onClicked: AiController.setPaused(!AiController.paused)
        }
        AppButton {
            objectName: "AiWorkBanner.stop"
            implicitHeight: 24
            text: qsTr("Stop")
            iconName: "square"
            toolTip: qsTr("Turn AI agents off: what they run stops, their timed previews are put back")
            onClicked: AiController.stop()
        }
    }

    Accessible.role: Accessible.StatusBar
    //: %1 is an AI agent's name, %2 what it does
    Accessible.name: qsTr("%1: %2").arg(work.agent !== undefined ? work.agent : "").arg(text)
}
