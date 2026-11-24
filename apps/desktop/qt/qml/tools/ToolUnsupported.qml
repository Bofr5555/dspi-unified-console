import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- The note a tool window shows instead of its body on firmware
// that lacks the effect (the Mac's unsupportedNote).
// -- WP DL1 fold -- In Modern it is an EmptyState (DESIGN-LANGUAGE §16): the
// requirement as its title, the way out as its sentence. Classic keeps the
// note as it was.
Item {
    id: note
    property string text
    property string detail

    implicitWidth: LayoutController.modern ? empty.implicitWidth : classic.implicitWidth
    implicitHeight: LayoutController.modern ? empty.implicitHeight : classic.implicitHeight

    ColumnLayout {
        id: classic
        anchors.fill: parent
        visible: !LayoutController.modern
        spacing: 10
        Item { Layout.fillHeight: true }
        Icon {
            Layout.alignment: Qt.AlignHCenter
            name: "triangle-alert"
            size: 28
            color: Theme.textMuted
        }
        Text {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: note.text
            font: Theme.fontBody
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        Text {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: note.detail
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        Item { Layout.fillHeight: true }
    }

    EmptyState {
        id: empty
        objectName: note.objectName.length ? note.objectName + ".empty" : ""
        anchors.fill: parent
        visible: LayoutController.modern
        iconName: "cpu"
        title: note.text
        text: note.detail
    }
}
