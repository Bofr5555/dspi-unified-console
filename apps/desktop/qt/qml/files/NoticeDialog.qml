import QtQuick
import QtQuick.Controls.Basic
import UConsole

// An alert with one button (the Mac's "Error" alerts after a file action).
// Alerts that come while one shows wait their turn. -- WP QP18 -- A success
// is no alert (PRO1 U5): it goes to the banner stack as a notice
// (NoticeCenter), which needs no click.
Dialog {
    id: notice
    property string text
    property bool error: false
    property var waiting: []

    function show(title, text, error) {
        if (!error) {
            NoticeCenter.post(text, "info")
            return
        }
        if (visible) {
            waiting.push({ title: title, text: text, error: error })
            return
        }
        notice.title = title
        notice.text = text
        notice.error = error
        open()
    }
    onClosed: {
        if (waiting.length === 0) return
        const next = waiting.shift()
        Qt.callLater(show, next.title, next.text, next.error)
    }
    onOpened: okButton.forceActiveFocus()

    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 320
    width: Math.min(420, hostWidth - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: Math.max(Theme.spacingLarge, (hostHeight - implicitHeight) / 3)
    padding: Theme.spacingLarge
    topPadding: Theme.spacingSmall

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.color: Theme.border
    }

    // Wrapped texts sit in Items sized by their laid-out height (see QuestionDialog)
    header: Item {
        implicitHeight: heading.height
        Row {
            id: heading
            x: Theme.spacingLarge
            topPadding: Theme.spacingLarge
            width: parent.width - 2 * Theme.spacingLarge
            spacing: Theme.spacingSmall
            Icon {
                id: mark
                name: notice.error ? "circle-alert" : "circle-check"
                color: notice.error ? Theme.dangerText : Theme.success
                size: 18
            }
            Text {
                width: heading.width - mark.width - heading.spacing
                textFormat: Text.PlainText
                text: notice.title
                font: Theme.fontTitle
                color: Theme.text
                wrapMode: Text.Wrap
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
        }
    }

    contentItem: Item {
        implicitHeight: body.height
        Accessible.role: Accessible.AlertMessage
        Accessible.name: notice.title
        Accessible.description: notice.text
        Text {
            id: body
            objectName: "NoticeDialog.text"
            width: parent.width
            textFormat: Text.PlainText
            text: notice.text
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
    }

    footer: Item {
        implicitHeight: okButton.implicitHeight + 2 * Theme.spacing
        AppButton {
            id: okButton
            objectName: "NoticeDialog.ok"
            x: parent.width - width - Theme.spacingLarge
            y: Theme.spacing
            text: qsTr("OK")
            primary: true
            onClicked: notice.close()
            Keys.onReturnPressed: notice.close()
            Keys.onEnterPressed: notice.close()
        }
    }
}
