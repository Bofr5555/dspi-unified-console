import QtQuick
import UConsole

// -- WP QP18 -- The notice on show (NoticeCenter): a finished action or a
// change someone else made, in the banner stack instead of a modal alert.
// "info" ones go by themselves; the others stay until dismissed.
// -- WP P2QT -- A hub alarm's notice offers Acknowledge (the hub's alarm.ack)
// to a Control or Admin session; dismissing it only hides it here.
Banner {
    objectName: "NoticeBanner"
    readonly property var notice: NoticeCenter.current
    kind: notice.kind !== undefined ? notice.kind : "info"
    iconName: kind === "info" ? "circle-check" : kind === "error" ? "circle-alert" : "triangle-alert"
    text: notice.text !== undefined ? notice.text : ""
    textFormat: Text.PlainText   // -- WP MDQT2c -- it can hold a device's name
    visible: text.length > 0
    dismissable: true
    onDismissed: NoticeCenter.dismiss()

    // -- WP P2QT --
    AppButton {
        objectName: "NoticeBanner.acknowledge"
        visible: notice.alarmId !== undefined && notice.alarmId.length > 0 && notice.canAck === true
        text: qsTr("Acknowledge")
        toolTip: qsTr("Tell the hub and every console that you have seen this alarm")
        onClicked: NoticeCenter.acknowledge(notice.alarmId)
    }
}
