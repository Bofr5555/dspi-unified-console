import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP LAY0 -- Settings > Appearance: the layout (Classic or Modern) and how
// Modern is arranged (LayoutController). Applied at once: the main window
// swaps its shell and keeps the device and the selection. -- WP LAY3 -- And the
// theme (both layouts).
SettingsPage {
    id: page
    objectName: "AppearancePage"

    SettingsGroup {
        title: qsTr("Layout")
        iconName: "layout-grid"
        // -- WP RBFIX -- why it is not on offer now
        footer: LayoutController.switchBlocker.length ? LayoutController.switchBlocker :
            qsTr("Classic is the Mac console's layout: a menu bar and tool windows. Modern puts every tool on a page of one window.")
        SettingsPickerRow {
            objectName: "Appearance.layout"
            enabled: LayoutController.switchBlocker.length === 0
            inline: true
            label: qsTr("Layout")
            values: ["classic", "modern"]
            texts: [qsTr("Classic"), qsTr("Modern")]
            value: LayoutController.layout
            onChosen: v => LayoutController.layout = v
        }
    }

    // -- WP LAY3 --
    SettingsGroup {
        title: qsTr("Theme")
        iconName: "sun"
        SettingsPickerRow {
            objectName: "Appearance.theme"
            inline: true
            label: qsTr("Theme")
            values: ["system", "light", "dark"]
            texts: [qsTr("System"), qsTr("Light"), qsTr("Dark")]
            value: LayoutController.theme
            onChosen: v => LayoutController.theme = v
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
        // -- WP LAY4 --
        SettingsPickerRow {
            objectName: "Appearance.tone"
            inline: true
            label: qsTr("Colour")
            values: ["grey", "darker"]
            texts: [qsTr("Grey"), qsTr("Darker grey")]
            value: LayoutController.arrangement.tone
            onChosen: v => LayoutController.arrangement.tone = v
        }
        SettingsPickerRow {
            objectName: "Appearance.inspector"
            inline: true
            label: qsTr("Inspector")
            values: ["shown", "hidden"]
            texts: [qsTr("Beside the Equaliser"), qsTr("Hidden")]
            value: LayoutController.arrangement.inspector
            onChosen: v => LayoutController.arrangement.inspector = v
        }
        SettingsPickerRow {
            objectName: "Appearance.bandBar"
            inline: true
            label: qsTr("Band bar")
            values: ["hidden", "shown"]
            texts: [qsTr("Off"), qsTr("Under the graph")]
            value: LayoutController.arrangement.bandBar
            onChosen: v => LayoutController.arrangement.bandBar = v
        }
    }
}
