import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- The IR receiver's learned remote buttons (the Mac's
// irCommandsSection): each a button-shaped command fired by a learned code.
// Their edits go with the receiver's Apply.
ColumnLayout {
    id: section
    property var receiver: ({})
    readonly property var subs: receiver.remotes || []

    Layout.fillWidth: true
    spacing: Theme.spacing

    Rectangle {
        Layout.fillWidth: true
        implicitHeight: 1
        color: Theme.separator
    }
    RowLayout {
        Layout.fillWidth: true
        HwRow {
            title: qsTr("Remote Buttons")
            caption: qsTr("Learn buttons on any remote and bind each to a function.")
        }
        Text {
            objectName: "Cs.remotes.count"
            textFormat: Text.PlainText
            text: qsTr("%1/%2").arg(section.subs.length).arg(section.receiver.maxRemotes || 0)
            font: Theme.fontSmall
            color: Theme.textMuted
        }
    }
    Text {
        visible: section.receiver.receiverLive !== true
        Layout.fillWidth: true
        text: qsTr("Apply the receiver above before learning remote buttons.")
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
    Repeater {
        model: section.subs
        delegate: CsRemoteButton {
            required property string modelData
            key: modelData
            receiverLive: section.receiver.receiverLive === true
        }
    }
}
