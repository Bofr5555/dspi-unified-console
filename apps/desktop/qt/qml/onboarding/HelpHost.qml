import QtQuick
import UConsole

// -- WP QP17 -- The main window's place for the Help menu's messages (Save
// Diagnostics' outcome), on QP15's alert.
Item {
    id: host
    objectName: "HelpHost"
    anchors.fill: parent

    NoticeDialog {
        id: alert
        objectName: "HelpHost.notice"
        parent: host.parent
    }
    Connections {
        target: OnboardingController
        function onNotice(title, text, error) { alert.show(title, text, error) }
    }
}
