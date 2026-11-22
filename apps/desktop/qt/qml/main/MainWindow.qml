import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// The main window: the system's frame, the theme, and the shell of the layout
// in use (-- WP LAY0 -- ClassicShell: the menu bar, the banners, and the
// sidebar beside the right pane, mockups section 1; -- WP LAY3 -- ModernShell). A 1000×660 minimum (the
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
    // (-- WP LAY0 -- the window's, so it outlives a layout switch)
    property int selectedChannel: -1

    // -- WP LAY0 -- The shell of LayoutController.layout, its menu bar (Classic's)
    menuBar: shell.item ? shell.item.menuBar : null

    Loader {
        id: shell
        objectName: "MainWindow.shell"
        anchors.fill: parent
        sourceComponent: LayoutController.modern ? modernShell : classicShell
    }
    Component {
        id: classicShell
        ClassicShell {
            mainWindow: window
        }
    }
    // -- WP LAY3 --
    Component {
        id: modernShell
        ModernShell {
            mainWindow: window
        }
    }

    // -- WP QP17 -- the Help menu's messages
    HelpHost {}
}
