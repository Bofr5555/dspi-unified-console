import QtQuick
import QtQuick.Layouts
import UConsole

// The sections other packages register for page `page`
// (SettingsRegistry::Section), in their order: the place a page keeps for them.
ColumnLayout {
    id: slot
    required property string page

    Layout.fillWidth: true
    spacing: Theme.spacingLarge
    visible: children.length > 1

    Repeater {
        model: SettingsController.sectionTypes(slot.page)
        delegate: Loader {
            required property string modelData
            Layout.fillWidth: true
            sourceComponent: Qt.createComponent("UConsole", modelData)
        }
    }
}
