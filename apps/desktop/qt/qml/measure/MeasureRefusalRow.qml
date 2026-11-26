import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP UA2 -- (A2) One problem that kept Measure from starting, in the core's
// words, with its fix where the app has one: the volume lowered to the most a
// measurement starts at (−12 dB; the row goes once it is there), the
// output's crossover opened, the auto-tune that waits shown. The rule and its
// numbers stay the core's (preflight.rs): a fix changes nothing the person did
// not press. Modern: a notice row; Classic: the line in the error colour with
// the button after it.
Item {
    id: row
    property string key
    property int output: -1
    property string text
    signal showTune()

    readonly property bool modern: LayoutController.modern
    // The volume a measurement may start at (preflight.rs VOL_USER_MAX_DB)
    readonly property real volumeMaxDb: -12
    readonly property bool fixed: key === "measure.volume_high" && SidebarController.userVolumeDb <= volumeMaxDb
    readonly property string fixText: key === "measure.volume_high" ? qsTr("Lower to −12 dB")
                                    : key === "measure.tweeter_highpass" && output >= 0 ? qsTr("Open the Crossover")
                                    : key === "measure.guard_pending" && TuneController.applied ? qsTr("Show the Auto-tune")
                                    : ""
    function fix() {
        if (key === "measure.volume_high") {
            SidebarController.setUserVolume(volumeMaxDb)
        } else if (key === "measure.tweeter_highpass") {
            // The output chosen; Modern shows its crossover, Classic's main
            // window its detail (the crossover with it)
            SidebarController.select(ChannelController.outputChannel(output))
            if (modern && Shell.pageHost) Shell.pageHost.go("equaliser", "crossover")
        } else if (key === "measure.guard_pending") {
            row.showTune()
        }
    }

    visible: !fixed
    implicitWidth: modern ? notice.implicitWidth : line.implicitWidth
    implicitHeight: modern ? notice.implicitHeight : line.implicitHeight

    NoticeRow {
        id: notice
        visible: row.modern
        anchors.fill: parent
        objectName: row.modern ? row.objectName + ".notice" : ""
        kind: "error"
        text: row.text
        actionText: row.fixText
        onAction: row.fix()
    }
    RowLayout {
        id: line
        visible: !row.modern
        anchors.fill: parent
        spacing: Theme.spacingSmall
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: row.text
            font: Theme.fontBody
            color: Theme.dangerText
            wrapMode: Text.WordWrap
            Accessible.role: Accessible.AlertMessage
            Accessible.name: text
        }
        AppButton {
            objectName: row.modern ? "" : row.objectName + ".notice.action"
            visible: row.fixText.length > 0
            text: row.fixText
            onClicked: row.fix()
        }
    }
}
