import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// The main window: the system's frame, the menu bar, the banners, and the
// sidebar beside the right pane (mockups section 1). A 1000×660 minimum (the
// Mac's is 1000×780; -- WP QTTWEAK -- 660 fits a 1366×768 screen with a
// title bar and a taskbar).
ApplicationWindow {
    id: window
    objectName: "MainWindow"
    width: 1100
    height: 800
    minimumWidth: 1000
    minimumHeight: 660
    visible: true
    title: AppIdentity.displayName
    color: Theme.window

    palette.window: Theme.window
    palette.windowText: Theme.text
    palette.base: Theme.control
    palette.alternateBase: Theme.panel
    palette.text: Theme.text
    palette.button: Theme.control
    palette.buttonText: Theme.text
    palette.highlight: Theme.accent
    palette.highlightedText: Theme.textOnAccent
    palette.placeholderText: Theme.textMuted
    palette.toolTipBase: Theme.tooltip
    palette.toolTipText: Theme.tooltipText
    font: Theme.fontBody

    // The channel on the right pane: -1 for the overview, else a wire channel
    property int selectedChannel: -1

    menuBar: Menus {
        window: window
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
                selectedChannel: window.selectedChannel
                onChannelSelected: channel => window.selectedChannel = channel
            }

            RightPane {
                SplitView.fillWidth: true
                selectedChannel: window.selectedChannel
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

    // -- WP QP17 -- the Help menu's messages
    HelpHost {}
}
