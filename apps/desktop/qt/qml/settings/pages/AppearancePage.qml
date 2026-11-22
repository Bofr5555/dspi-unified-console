import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP LAY0 -- Settings > Appearance: the layout (Classic or Modern) and how
// Modern is arranged (LayoutController). Applied at once: the main window
// swaps its shell and keeps the device and the selection. Not in the sidebar
// until the Modern shell is there (WP LAY3).
SettingsPage {
    id: page
    objectName: "AppearancePage"

    SettingsGroup {
        title: qsTr("Layout")
        iconName: "layout-grid"
        footer: qsTr("Classic is the Mac console's layout: a menu bar and tool windows. Modern puts every tool on a page of one window.")
        SettingsPickerRow {
            objectName: "Appearance.layout"
            inline: true
            label: qsTr("Layout")
            values: ["classic", "modern"]
            texts: [qsTr("Classic"), qsTr("Modern")]
            value: LayoutController.layout
            onChosen: v => LayoutController.layout = v
        }
    }

    SettingsGroup {
        title: qsTr("Modern Arrangement")
        iconName: "layout-grid"
        enabled: LayoutController.modern
        SettingsPickerRow {
            objectName: "Appearance.navigation"
            inline: true
            label: qsTr("Sections")
            values: ["rail", "tabs"]
            texts: [qsTr("Rail with page tabs"), qsTr("Tabs along the top")]
            value: LayoutController.arrangement.navigation
            onChosen: v => LayoutController.arrangement.navigation = v
        }
        SettingsPickerRow {
            objectName: "Appearance.deviceTabs"
            inline: true
            label: qsTr("Device tabs")
            values: ["topBar", "rail"]
            texts: [qsTr("In the top bar"), qsTr("At the top of the rail")]
            value: LayoutController.arrangement.deviceTabs
            onChosen: v => LayoutController.arrangement.deviceTabs = v
        }
    }
}
