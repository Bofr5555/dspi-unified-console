import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- The note a tool window shows instead of its body on firmware
// that lacks the effect (the Mac's unsupportedNote).
ColumnLayout {
    property string text
    property string detail

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
        text: parent.text
        font: Theme.fontBody
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
    Text {
        Layout.fillWidth: true
        horizontalAlignment: Text.AlignHCenter
        text: parent.detail
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
    Item { Layout.fillHeight: true }
}
