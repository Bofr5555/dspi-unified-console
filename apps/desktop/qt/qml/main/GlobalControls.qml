import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP1 -- The controls under the quick strip (ContentView.swift's
// "Global Controls"): the preset popup with its unsaved marker, save button and
// slot menu; the source popup with its rename menu; the volume slider in User
// or Master mode (UserVolumeSection, MasterModeSection). Every change goes
// through SidebarController.
ColumnLayout {
    id: controls
    objectName: "GlobalControls"
    spacing: Theme.spacing

    readonly property bool writable: DeviceModel.connected && !DeviceModel.readOnly
    readonly property int activeSlot: DeviceModel.presets.activeSlot

    // A caption in the sidebar's small grey ("Preset", "Source")
    component Caption: Text {
        font: Theme.fontSmall
        color: Theme.textMuted
    }

    // ---- Preset ----
    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        Caption {
            text: qsTr("Preset")
            Layout.fillWidth: true
        }
        AppComboBox {
            id: presetBox
            AiTouched { path: "preset." }   // -- WP AIVIS --
            AiTouched { path: "bulk" }
            objectName: "GlobalControls.preset"
            // What the popup shows, for the smoke driver
            readonly property string shownName: displayText
            Layout.minimumWidth: 80
            Layout.maximumWidth: 150
            implicitWidth: 130
            label: qsTr("Preset")
            model: SidebarController.presetTitles
            displayText: count > 0 && currentIndex >= 0 ? currentText : "–"
            currentIndex: controls.activeSlot
            enabled: controls.writable && !SidebarController.presetBusy
            onActivated: index => {
                SidebarController.requestPresetSwitch(index)
                // The device says which slot is active, not the click
                currentIndex = Qt.binding(() => controls.activeSlot)
            }
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: point => { if (controls.writable) dialogs.popupPresetMenu(presetBox, point.position.x, point.position.y) }
            }
        }
        IconButton {
            objectName: "GlobalControls.presetSave"
            iconName: "save"
            iconSize: 14
            toolTip: qsTr("Save")
            enabled: controls.writable && !SidebarController.presetBusy && controls.activeSlot >= 0
            onClicked: SidebarController.savePreset()
        }
    }

    // ---- Source (hidden on firmware without input switching) ----
    RowLayout {
        Layout.fillWidth: true
        visible: SidebarController.sourceSupported
        spacing: Theme.spacingSmall
        Caption {
            text: qsTr("Source")
            Layout.fillWidth: true
        }
        AppComboBox {
            id: sourceBox
            AiTouched { path: "in." }   // -- WP AIVIS --
            objectName: "GlobalControls.source"
            readonly property string shownName: displayText
            Layout.minimumWidth: 80
            Layout.maximumWidth: 150
            implicitWidth: 130
            label: qsTr("Source")
            model: SidebarController.sources
            textRole: "title"
            valueRole: "value"
            currentIndex: indexOfValue(SidebarController.source)
            enabled: controls.writable
            onActivated: index => {
                SidebarController.setSource(valueAt(index))
                currentIndex = Qt.binding(() => indexOfValue(SidebarController.source))
            }
            // -- WP FEATUI -- (FEAT3) where the sound comes from, in the words agents get too
            HoverHandler { id: sourceHover }
            ToolTip.visible: sourceHover.hovered && !sourceBox.popup.visible
            ToolTip.delay: 600
            //: %1 the input chosen ("USB", "S/PDIF")
            ToolTip.text: qsTr("A DSPi has no player: it plays what %1 receives.").arg(sourceBox.displayText)
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: point => {
                    if (controls.writable && SidebarController.sourceNamesSupported)
                        dialogs.popupSourceMenu(sourceBox, point.position.x, point.position.y)
                }
            }
        }
    }
    // ---- Volume: User (-60..0 dB, square-root taper) or Master (-128..0 dB) ----
    ColumnLayout {
        id: volume
        Layout.fillWidth: true
        spacing: 4
        readonly property bool master: SidebarController.volumeMode === "master"
        readonly property real deviceDb: master ? SidebarController.masterVolumeDb : SidebarController.userVolumeDb
        function toPosition(db) { return master ? SidebarController.masterDbToPosition(db) : SidebarController.userDbToPosition(db) }
        function toDb(position) { return master ? SidebarController.masterPositionToDb(position) : SidebarController.userPositionToDb(position) }
        function send(db) {
            if (master) SidebarController.setMasterVolume(db)
            else SidebarController.setUserVolume(db)
        }

        RowLayout {
            Layout.fillWidth: true
            // The heading is the mode chooser ("User Volume ▾")
            AbstractButton {
                id: modeButton
                objectName: "GlobalControls.volumeMode"
                readonly property string shownName: modeText.text
                hoverEnabled: true
                focusPolicy: Qt.TabFocus
                implicitWidth: modeText.implicitWidth + 4
                implicitHeight: modeText.implicitHeight + 4
                onClicked: modeMenu.popup(modeButton, 0, modeButton.height)
                contentItem: Text {
                    id: modeText
                    text: (volume.master ? qsTr("Master Volume") : qsTr("User Volume")) + " ▾"
                    font: Theme.fontSmall
                    color: modeButton.hovered ? Theme.text : Theme.textMuted
                }
                background: Rectangle {
                    color: "transparent"
                    radius: Theme.radiusSmall
                    border.width: modeButton.visualFocus ? 2 : 0
                    border.color: Theme.focusRing
                }
                Accessible.role: Accessible.ButtonMenu
                Accessible.name: qsTr("Volume mode")
                Accessible.description: modeText.text
            }
            Item { Layout.fillWidth: true }
            Text {
                objectName: "GlobalControls.volumeReadout"
                readonly property string shownName: text
                text: !DeviceModel.connected ? "–"
                      : volume.master ? SidebarController.formatMasterDb(volume.toDb(slider.value))
                                      : SidebarController.formatUserDb(volume.toDb(slider.value))
                font: Theme.fontMono
                color: Theme.text
            }
        }
        AppMenu {
            id: modeMenu
            AppMenuItem {
                objectName: "GlobalControls.volumeMode.user"
                text: qsTr("User Volume")
                checkable: true
                checked: !volume.master
                onTriggered: SidebarController.volumeMode = "user"
            }
            AppMenuItem {
                objectName: "GlobalControls.volumeMode.master"
                text: qsTr("Master Volume")
                checkable: true
                checked: volume.master
                onTriggered: SidebarController.volumeMode = "master"
            }
        }
        CustomSlider {
            id: slider
            AiTouched { objectName: "GlobalControls.volume.touched"; path: "vol." }   // -- WP AIVIS --
            objectName: "GlobalControls.volumeSlider"
            Layout.fillWidth: true
            from: 0
            to: 1
            label: volume.master ? qsTr("Master Volume") : qsTr("User Volume")
            enabled: controls.writable
            // Right-click: back to 0 dB
            resetValue: volume.toPosition(0)
            // Follows the device except while the person holds it
            Binding on value {
                when: !slider.pressed
                value: volume.toPosition(volume.deviceDb)
                restoreMode: Binding.RestoreNone
            }
            onValueMoved: position => volume.send(volume.toDb(position))
            onCommitted: position => volume.send(volume.toDb(position))
            // Master mode tints the track red, as on the Mac
            background: Rectangle {
                x: slider.leftPadding
                y: slider.topPadding + slider.availableHeight / 2 - height / 2
                width: slider.availableWidth
                height: 4
                radius: 2
                color: Theme.border
                Rectangle {
                    width: slider.visualPosition * parent.width
                    height: parent.height
                    radius: 2
                    color: !slider.enabled ? Theme.textDisabled : volume.master ? Theme.danger : Theme.accent
                }
            }
            Accessible.description: volume.master ? SidebarController.formatMasterDb(volume.toDb(value))
                                                  : SidebarController.formatUserDb(volume.toDb(value))
        }
    }

    // -- WP LAY3 -- The slot and source menus and the questions and alerts
    PresetDialogs {
        id: dialogs
    }
}
