import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// One entry of the device list: an icon, the device's name with a line under
// it, and a check mark on the open one (the iOS Devices sheet's rows). A click
// chooses it.
AbstractButton {
    id: row
    property string iconName
    property string title
    property string subtitle
    property bool current: false

    implicitHeight: Math.max(36, lines.implicitHeight + 8)
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    contentItem: RowLayout {
        spacing: Theme.spacingSmall
        Icon {
            Layout.preferredWidth: 24
            name: row.iconName
            size: 16
            color: row.enabled ? Theme.textMuted : Theme.textDisabled
        }
        ColumnLayout {
            id: lines
            Layout.fillWidth: true
            spacing: 1
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: row.title
                font: Theme.fontBody
                color: row.enabled ? Theme.text : Theme.textDisabled
                elide: Text.ElideRight
            }
            Text {
                Layout.fillWidth: true
                visible: row.subtitle.length > 0
                textFormat: Text.PlainText
                text: row.subtitle
                font: Theme.fontSmall
                color: row.enabled ? Theme.textMuted : Theme.textDisabled
                elide: Text.ElideRight
            }
        }
        Icon {
            visible: row.current
            name: "circle-check"
            size: 16
            color: Theme.success
            accessibleName: qsTr("Open")
        }
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        color: row.down ? Theme.controlPressed : row.hovered && row.enabled ? Theme.controlHover : "transparent"
        border.width: row.visualFocus ? 2 : 0
        border.color: Theme.focusRing
    }

    Accessible.role: Accessible.Button
    Accessible.name: title
    Accessible.description: current ? qsTr("Open") + ". " + subtitle : subtitle
    Accessible.onPressAction: row.clicked()
}
