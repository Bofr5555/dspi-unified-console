import QtQuick
import QtQuick.Controls.Basic
import UConsole

// Where the device list and the pairing prompt live: DevicesController makes
// one in the main window once the window exists, so the prompt can come up
// whether the list is open or not.
Item {
    id: host
    objectName: "DevicesHost"
    anchors.fill: parent
    // The popups sit under the window's menu bar
    readonly property real topInset: ApplicationWindow.menuBar ? ApplicationWindow.menuBar.height : 0

    DevicesPopup {
        id: list
    }
    PairingDialog {
    }
    // -- WP SECQT --
    RePairDialog {
    }

    Connections {
        target: DevicesController
        function onListShownChanged() {
            if (DevicesController.listShown) list.open()
            else list.close()
        }
    }
}
