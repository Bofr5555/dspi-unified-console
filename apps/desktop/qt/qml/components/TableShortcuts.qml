import QtQuick
import UConsole

// The menu table's shortcuts for a window without the menu bar: every
// ToolWindow has one, so Ctrl+Shift+M opens the matrix from any window, as the
// Mac's global menu bar does. The main window's come from its menus.
Item {
    id: shortcuts
    required property Window window
    visible: false

    Instantiator {
        model: MenuTable.shortcutRows()
        delegate: Shortcut {
            required property var modelData
            sequences: [modelData.shortcut]
            context: Qt.WindowShortcut
            enabled: modelData.enabled ? modelData.enabled() : true   // -- WP QP18 --
            onActivated: MenuTable.trigger(shortcuts.window, modelData.id)
        }
    }
}
