import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP AIVIS -- What an AI agent does now (AiController.work), under the top bar
// in both layouts: which agent, what, on which device, with Pause and Stop, and
// Stop Measuring while its measurement plays. It is there while a call or a job
// of an agent's runs, while one waits for the person, for a while after the last
// call, and as long as the agents are paused.
// -- WP DL5 -- It always says something (SHOW finding 5): what the agent does,
// else the grant that keeps it there, else that the agents are paused or
// connected. In Modern it is dressed as the status strip's banners (the panel
// tone, a 3 px accent edge, no line under it), and a grant's Revoke is in the
// AI popover, which the strip's last button opens; Classic keeps its look and
// its Revoke.
// -- WP UB5 -- (UXP B5) Once an agent's work rests, its last line says what it left
// unsaved on the device ("Changed 12 settings, not saved yet") with Save to Device,
// and stays until the device is saved or its changes are gone.
Rectangle {
    id: strip
    objectName: "AiWorkBanner"
    readonly property var work: AiController.work
    readonly property string workText: work.text !== undefined ? work.text : ""
    // -- WP ODEC -- a grant lasts ten minutes: the strip says so while it does
    readonly property var grant: AiController.grants.length ? AiController.grants[0] : null
    readonly property bool modern: LayoutController.modern
    // -- WP DL5 -- what the grant allows, and for how long
    readonly property string grantText: !grant ? ""
          //: The strip: an AI agent may play test signals and detect the speakers without asking for %1 ("9:41") more
          : grant.detect ? qsTr("Test signals and speaker detection allowed, %1 left").arg(grant.left)   // -- WP ADETQ --
          //: The strip: an AI agent may play test signals without asking for %1 ("9:41") more
          : qsTr("Test signals allowed, %1 left").arg(grant.left)
    // -- WP UAG -- (UXP A7) the console's refusal of an agent's call, for a few seconds
    readonly property var refusal: AiController.refusal
    readonly property string refusalText: refusal.text === undefined ? ""
          //: The strip: the console refused an AI agent's change; %1 why ("kept Right tweeter's high-pass at 1 kHz")
          : refusal.device.length ? qsTr("Refused: %1 · %2").arg(refusal.text).arg(refusal.device)
          : qsTr("Refused: %1").arg(refusal.text)
    // -- WP UB5 -- (UXP B5) what agents left unsaved on the device
    readonly property var unsaved: AiController.unsaved
    readonly property string unsavedText: unsaved.text !== undefined ? unsaved.text : ""
    // -- WP DL5 -- the line it shows: never empty while the strip is
    readonly property string text: refusalText.length ? refusalText
                                 // -- WP UB5 -- the last line, once the agent rests: not "Last: …"
                                 : unsavedText.length && !AiController.working ? unsavedText
                                 : workText.length ? workText
                                 : grant ? grantText
                                 : AiController.paused ? qsTr("AI agents are paused. Nothing is changed until Resume.")
                                 : AiController.agents.length ? qsTr("Connected. Nothing runs now.")
                                 : AiController.statusLine
    readonly property string agent: refusal.agent !== undefined ? refusal.agent   // -- WP UAG --
                                  : work.agent !== undefined && work.agent.length ? work.agent
                                  : grant && grant.agent !== undefined ? grant.agent
                                  : unsaved.agent !== undefined ? unsaved.agent   // -- WP UB5 --
                                  : !AiController.paused && AiController.agents.length ? AiController.agents[0].name : ""
    visible: work.shown === true || grant !== null || refusalText.length > 0   // -- WP UAG --
             || unsavedText.length > 0   // -- WP UB5 --
    implicitHeight: row.implicitHeight + 8
    color: Theme.bannerFill("info")   // -- WP DL5 -- (Modern: the panel tone, INTC)
    // In the accent's colour, so that it is none of the other strips
    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 2
        visible: !strip.modern   // -- WP DL5 --
        color: Theme.accent
    }
    Rectangle {
        objectName: "AiWorkBanner.edge"
        width: strip.modern ? 3 : 4   // -- WP DL5 --
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
            text: strip.agent   // -- WP DL5 --
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
            // -- WP DL5 -- beside what the agent does; alone, it is the line itself
            visible: strip.grant !== null && strip.workText.length > 0
            textFormat: Text.PlainText
            text: strip.grantText
            font: Theme.fontSmall
            color: Theme.text
        }
        AppButton {
            objectName: "AiWorkBanner.revokeGrant"
            visible: strip.grant !== null && !strip.modern   // -- WP DL5 -- Modern: in the AI popover
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
        // -- WP UB5 -- (UXP B5) beside its own line only
        AppButton {
            objectName: "AiWorkBanner.save"
            visible: strip.unsavedText.length > 0 && strip.text === strip.unsavedText
            implicitHeight: 24
            text: strip.unsaved.save !== undefined ? strip.unsaved.save : ""
            toolTip: qsTr("Save the agent's changes on the device, so they are kept when it is switched off")
            onClicked: AiController.saveUnsaved()
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
            text: qsTr("Stop Agents")   // -- WP UAG -- (UXP A7, C4) not the measurement's Stop
            iconName: "square"
            toolTip: qsTr("Turn AI agents off: what they run stops, their timed previews are put back")
            onClicked: AiController.stop()
        }
        // -- WP DL5 -- Modern: the AI popover, with what the agents may do and Revoke
        IconButton {
            objectName: "AiWorkBanner.details"
            visible: strip.modern
            iconName: "chevron-down"
            toolTip: strip.grant !== null ? qsTr("AI agents: what they may do, and Revoke") : qsTr("AI agents")
            onClicked: ContextMenus.showAi()
        }
    }

    Accessible.role: Accessible.StatusBar
    //: %1 is an AI agent's name, %2 what it does
    Accessible.name: qsTr("%1: %2").arg(agent).arg(text)   // -- WP DL5 --
}
