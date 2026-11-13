import QtQuick
import UConsole

// -- WP QP18 -- The notice on show (NoticeCenter): a finished action or a
// change someone else made, in the banner stack instead of a modal alert.
// "info" ones go by themselves; the others stay until dismissed.
Banner {
    objectName: "NoticeBanner"
    readonly property var notice: NoticeCenter.current
    kind: notice.kind !== undefined ? notice.kind : "info"
    iconName: kind === "info" ? "circle-check" : kind === "error" ? "circle-alert" : "triangle-alert"
    text: notice.text !== undefined ? notice.text : ""
    visible: text.length > 0
    dismissable: true
    onDismissed: NoticeCenter.dismiss()
}
