import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// Every DSPi this app can open, in one list (the Mac's device picker and its
// iOS Devices sheet, with the Networking page's "Other Hubs"). USB and the demo
// first, then DSPi Link: the session's state in the core's words, the hub's
// devices, the hubs found on this network and a field for a hub's address.
//
// -- WP LAY2 -- The content alone: Classic shows it in the Devices popover
// (DevicesPopup), Modern in its device switcher. `done` is its Done button.
// -- WP LAY5b -- Its two parts are DevicesLocal and DevicesLink, which Modern's
// Devices section shows on a page each.
ColumnLayout {
    id: content
    property string title: qsTr("Devices")
    signal done()

    // Done takes the focus as the list shows
    function takeFocus() { doneButton.forceActiveFocus() }

    Layout.fillWidth: true
    spacing: Theme.spacingSmall

    RowLayout {
        Layout.fillWidth: true
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: content.title
            font: Theme.fontTitle
            color: Theme.text
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        AppButton {
            id: doneButton
            objectName: "DevicesPopup.done"
            text: qsTr("Done")
            primary: true
            onClicked: content.done()
        }
    }

    Flickable {
        id: flick
        Layout.fillWidth: true
        Layout.fillHeight: true
        implicitHeight: body.implicitHeight
        contentHeight: body.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: AppScrollBar {}

        ColumnLayout {
            id: body
            width: flick.width
            spacing: 4

            DevicesLocal { Layout.fillWidth: true }   // -- WP LAY5b --
            DevicesLink { Layout.fillWidth: true }
        }
    }
}
