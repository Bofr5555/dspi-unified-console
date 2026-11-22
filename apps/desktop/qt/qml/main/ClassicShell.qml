import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY0 -- The Classic layout (the Mac console's), as MainWindow showed
// it before the layout switch: the menu bar, the banners, and the sidebar
// beside the right pane (mockups section 1), or the alink sender's page, or
// Getting Started. MainWindow loads it (LayoutController) and takes its
// `menuBar`; the selection is the window's, so it outlives a switch.
Item {
    id: shell
    objectName: "ClassicShell"
    required property ApplicationWindow mainWindow

    readonly property Item menuBar: Menus {
        window: shell.mainWindow
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        BannerHost {
            Layout.fillWidth: true
        }

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: !OnboardingController.wizardShown   // -- WP QP17 -- the wizard takes its place
                     && !DeviceInfoController.sender      // -- WP PGQT -- the sender has no DSP screens
            orientation: Qt.Horizontal
            handle: Rectangle {
                implicitWidth: 1
                color: Theme.separator
            }

            Sidebar {
                // -- WP QPI1 -- the width the person left it at (sidebar/width)
                SplitView.preferredWidth: SidebarController.sidebarWidth > 0 ? SidebarController.sidebarWidth : Theme.sidebarWidth
                SplitView.minimumWidth: 220
                SplitView.maximumWidth: 320
                onWidthChanged: if (SplitView.view && SplitView.view.resizing) SidebarController.sidebarWidth = Math.round(width)
                selectedChannel: shell.mainWindow.selectedChannel
                onChannelSelected: channel => shell.mainWindow.selectedChannel = channel
            }

            RightPane {
                SplitView.fillWidth: true
                selectedChannel: shell.mainWindow.selectedChannel
            }
        }

        // -- WP PGQT -- the alink sender's page in place of the DSP screens
        Loader {
            objectName: "MainWindow.sender"
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: DeviceInfoController.sender && !OnboardingController.wizardShown
            visible: active
            sourceComponent: SenderPage {}
        }

        // -- WP QP17 -- Getting Started replaces the console while it runs
        Loader {
            objectName: "MainWindow.wizard"
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: OnboardingController.wizardShown
            visible: active
            sourceComponent: GettingStarted {}
        }
    }
}
