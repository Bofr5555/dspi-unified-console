import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP P1QT1 -- The safe apply's countdown bar (P1SA): a guarded import or
// recall waits for Keep; when the time runs out the settings from before come
// back. After a lost connection it says so, and offers Revert once the same
// device is back (the core's rule: nothing is replayed, a revert is never
// queued).
Banner {
    id: banner
    objectName: "SafeApplyBanner"
    readonly property bool waiting: SnapshotsController.guardWaiting
    readonly property bool lost: SnapshotsController.guardPhase === "lost"
    readonly property int seconds: Math.ceil(SnapshotsController.guardRemainingMs / 1000)
    kind: "warning"
    iconName: "clock"
    visible: waiting || lost
    text: lost ? SnapshotsController.guardMessage
          //: %1 what was applied (a file's or snapshot's name), %2 the seconds left
          : qsTr("“%1” is applied. Keep it, or the settings from before come back in %2 s.")
                .arg(SnapshotsController.guardLabel).arg(seconds)

    // The time left, as a bar that runs down
    Rectangle {
        objectName: "SafeApplyBanner.countdown"
        visible: banner.waiting
        implicitWidth: 120
        implicitHeight: 6
        radius: 3
        color: Theme.meterTrack
        Rectangle {
            height: parent.height
            radius: parent.radius
            width: SnapshotsController.guardTimeoutMs > 0
                   ? parent.width * SnapshotsController.guardRemainingMs / SnapshotsController.guardTimeoutMs : 0
            color: Theme.warning
            Behavior on width { NumberAnimation { duration: 250 } }
        }
        Accessible.role: Accessible.ProgressBar
        Accessible.name: qsTr("Time left: %1 s").arg(banner.seconds)
    }
    AppButton {
        objectName: "SafeApplyBanner.revert"
        text: qsTr("Revert")
        enabled: !SnapshotsController.busy
        onClicked: SnapshotsController.revert()
    }
    AppButton {
        objectName: "SafeApplyBanner.keep"
        visible: banner.waiting
        text: qsTr("Keep")
        primary: true
        enabled: !SnapshotsController.busy
        onClicked: SnapshotsController.keep()
    }
}
