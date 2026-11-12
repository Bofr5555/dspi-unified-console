import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// The text viewer of a third-party notice (behaviour.md §27): a modal card
// nearly the Settings window's size, the notice's name at the top left, Close
// at the top right, the text below in the monospaced font, unwrapped as in the
// file and selectable. Close or Escape returns to the same place on About.
Popup {
    id: viewer
    property string title: AboutController.noticeTitle
    property string body: AboutController.noticeText
    property bool loading: AboutController.noticeLoading

    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    readonly property real hostWidth: parent ? parent.width : 800
    readonly property real hostHeight: parent ? parent.height : 600
    x: Theme.spacingLarge
    y: Theme.spacingLarge
    width: hostWidth - 2 * Theme.spacingLarge
    height: hostHeight - 2 * Theme.spacingLarge
    padding: 0

    onOpened: text.forceActiveFocus()
    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.color: Theme.border
    }

    contentItem: ColumnLayout {
        spacing: 0
        Accessible.role: Accessible.Dialog
        Accessible.name: viewer.title

        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.spacing
            Text {
                objectName: "About.viewerTitle"
                Layout.fillWidth: true
                text: viewer.title
                font: Theme.fontTitle
                color: Theme.text
                elide: Text.ElideRight
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            AppButton {
                objectName: "About.viewerClose"
                text: qsTr("Close")
                onClicked: viewer.close()
            }
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }
        Flickable {
            id: flick
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            contentWidth: Math.max(width, text.implicitWidth)
            contentHeight: text.implicitHeight
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}
            ScrollBar.horizontal: AppScrollBar {}

            TextEdit {
                id: text
                objectName: "About.viewerText"
                width: Math.max(flick.width, implicitWidth)
                padding: Theme.spacing
                text: viewer.loading ? qsTr("Reading…") : viewer.body
                textFormat: TextEdit.PlainText
                readOnly: true
                selectByMouse: true
                selectByKeyboard: true
                wrapMode: TextEdit.NoWrap
                font: Theme.fontMono
                color: Theme.text
                selectionColor: Theme.selection
                selectedTextColor: Theme.text
                Keys.onEscapePressed: viewer.close()
                Accessible.role: Accessible.EditableText
                Accessible.readOnly: true
                Accessible.name: viewer.title
            }
        }
    }
}
