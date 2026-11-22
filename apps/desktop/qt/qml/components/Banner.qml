import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// One banner under the menu bar (BannerHost stacks them): a coloured strip
// with an icon, a message and its buttons. A banner type registered with
// Registrar::Banner is a Banner with its own `visible` binding:
//
//     Banner {
//         kind: "warning"
//         text: qsTr("...")
//         visible: SomeController.shouldShow
//         Button { text: qsTr("Update..."); onClicked: ... }
//     }
Rectangle {
    id: banner
    property string kind: "info"          // "info", "warning" or "error"
    property string text
    property string iconName: kind === "info" ? "info" : "triangle-alert"
    property bool dismissable: false
    property int textFormat: Text.PlainText   // -- WP MDQT2c -- (WP RBFIX: plain unless a banner says otherwise)
    default property alias actions: buttons.data
    signal dismissed()

    implicitHeight: row.implicitHeight + 2 * Theme.spacingSmall
    color: Theme.bannerFill(kind)
    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 1
        color: Theme.bannerEdge(banner.kind)
    }

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.leftMargin: Theme.spacing
        anchors.rightMargin: Theme.spacing
        spacing: Theme.spacingSmall
        Icon {
            name: banner.iconName
            color: Theme.bannerEdge(banner.kind)
        }
        Text {
            Layout.fillWidth: true
            text: banner.text
            textFormat: banner.textFormat
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        RowLayout {
            id: buttons
            spacing: Theme.spacingSmall
        }
        IconButton {
            visible: banner.dismissable
            iconName: "x"
            toolTip: qsTr("Dismiss")
            onClicked: banner.dismissed()
        }
    }

    Accessible.role: Accessible.AlertMessage
    Accessible.name: text
}
