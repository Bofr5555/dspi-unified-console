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
    component Line: MenuSeparator {
        contentItem: Rectangle { implicitHeight: 1; color: Theme.separator }
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
                onTapped: point => { if (controls.writable) presetMenu.popup(presetBox, point.position.x, point.position.y) }
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

    // The slot menu (PresetContextMenuItems)
    AppMenu {
        id: presetMenu
        objectName: "GlobalControls.presetMenu"
        AppMenuItem {
            text: qsTr("Save")
            onTriggered: SidebarController.savePreset()
        }
        AppMenuItem {
            text: qsTr("Rename…")
            onTriggered: {
                presetRename.slot = controls.activeSlot
                presetRename.open()
            }
        }
        AppMenuItem {
            text: qsTr("Set as Default")
            enabled: !(DeviceModel.presets.startupMode === 0 && DeviceModel.presets.defaultSlot === controls.activeSlot)
            onTriggered: SidebarController.setDefaultPreset()
        }
        AppMenu {
            id: copyMenu
            title: qsTr("Copy to…")
            enabled: DeviceModel.connected
            // Every slot but the active one
            Instantiator {
                model: SidebarController.presetTitles.length
                delegate: AppMenuItem {
                    required property int index
                    objectName: "GlobalControls.copyTo." + index
                    text: SidebarController.presetTitles.length ? SidebarController.presetLabel(index) : ""
                    visible: index !== controls.activeSlot
                    height: visible ? implicitHeight : 0
                    onTriggered: SidebarController.requestPresetCopy(index)
                }
                onObjectAdded: (index, object) => copyMenu.insertItem(index, object)
                onObjectRemoved: (index, object) => copyMenu.removeItem(object)
            }
        }
        Line {
            visible: SidebarController.presetOccupied(controls.activeSlot)
            height: visible ? implicitHeight : 0
        }
        AppMenuItem {
            objectName: "GlobalControls.presetClear"
            //: %1 is a slot's label, "3: Living Room"
            text: qsTr("Clear \"%1\"…").arg(SidebarController.presetLabel(controls.activeSlot))
            visible: SidebarController.presetOccupied(controls.activeSlot)
            height: visible ? implicitHeight : 0
            onTriggered: clearOne.ask(controls.activeSlot)
        }
        Line {
            visible: SidebarController.presetTitles.some((t, i) => SidebarController.presetOccupied(i))
            height: visible ? implicitHeight : 0
        }
        AppMenuItem {
            objectName: "GlobalControls.presetClearAll"
            text: qsTr("Clear All Slots…")
            visible: SidebarController.presetTitles.some((t, i) => SidebarController.presetOccupied(i))
            height: visible ? implicitHeight : 0
            onTriggered: clearAll.ask()
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
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: point => {
                    if (controls.writable && SidebarController.sourceNamesSupported)
                        sourceMenu.popup(sourceBox, point.position.x, point.position.y)
                }
            }
        }
    }
    AppMenu {
        id: sourceMenu
        objectName: "GlobalControls.sourceMenu"
        AppMenuItem {
            //: %1 is the source's name
            text: qsTr("Rename “%1”…").arg(SidebarController.sourceTitle(SidebarController.source))
            onTriggered: {
                sourceRename.source = SidebarController.source
                sourceRename.open()
            }
        }
        AppMenuItem {
            text: qsTr("Use Built-in Name")
            visible: SidebarController.sourceName(SidebarController.source).length > 0
            height: visible ? implicitHeight : 0
            onTriggered: SidebarController.renameSource(SidebarController.source, "")
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

    // ---- Questions and alerts ----
    QuestionDialog {
        id: unsavedQuestion
        objectName: "GlobalControls.unsavedQuestion"
        ofDevice: true   // -- WP MDQT2a --
        parent: Overlay.overlay
        property string op
        property int slot: -1
        title: qsTr("Unsaved Changes")
        acceptText: qsTr("Save")
        alternativeText: qsTr("Discard")
        rejectText: qsTr("Cancel")
        onAnswered: answer => SidebarController.resolveUnsaved(op, slot, answer)
    }
    Connections {
        target: SidebarController
        function onUnsavedQuestion(op, slot, summary) {
            unsavedQuestion.op = op
            unsavedQuestion.slot = slot
            unsavedQuestion.text = qsTr("The current preset has unsaved changes:\n\n%1\n\nSave before continuing?").arg(summary)
            unsavedQuestion.open()
        }
        function onPresetFailed(title, text) {
            failure.title = title
            failure.text = text
            failure.open()
        }
    }
    QuestionDialog {
        id: clearOne
        objectName: "GlobalControls.clearQuestion"
        ofDevice: true   // -- WP MDQT2a --
        parent: Overlay.overlay
        property int slot: -1
        function ask(s) {
            slot = s
            //: %1 is a slot's label, "3: Living Room"
            text = qsTr("Clear \"%1\" and restore factory defaults? This cannot be undone.").arg(SidebarController.presetLabel(s))
            open()
        }
        title: qsTr("Clear Preset?")
        acceptText: qsTr("Clear")
        destructive: true
        onAnswered: answer => { if (answer === "accept") SidebarController.clearPreset(slot) }
    }
    QuestionDialog {
        id: clearAll
        objectName: "GlobalControls.clearAllQuestion"
        ofDevice: true   // -- WP MDQT2a --
        parent: Overlay.overlay
        function ask() { open() }
        title: qsTr("Clear All Presets?")
        text: qsTr("This will erase all preset data and names, restoring every slot to factory defaults. This cannot be undone.")
        acceptText: qsTr("Clear All")
        destructive: true
        onAnswered: answer => { if (answer === "accept") SidebarController.clearAllPresets() }
    }

    // A small dialog with a text field (the Mac's rename sheets)
    component NameDialog: Dialog {
        id: nameDialog
        property alias fieldText: field.text
        property alias placeholder: field.placeholderText
        property string hint
        signal chosen(string text)
        parent: Overlay.overlay
        modal: true
        focus: true
        closePolicy: Popup.CloseOnEscape
        Connections {   // -- WP MDQT2a -- a name for the device that is left
            target: nameDialog.visible ? DeviceRegistry : null
            function onActiveLeaving() { nameDialog.close() }
        }
        width: 280
        x: (parent.width - width) / 2
        y: Math.max(Theme.spacingLarge, (parent.height - implicitHeight) / 3)
        padding: Theme.spacingLarge
        Overlay.modal: Rectangle { color: Theme.scrim }
        background: Rectangle {
            radius: Theme.radius
            color: Theme.panel
            border.color: Theme.border
        }
        onOpened: {
            field.forceActiveFocus()
            field.selectAll()
        }
        function finish() {
            chosen(field.text)
            close()
        }
        contentItem: ColumnLayout {
            spacing: Theme.spacing
            Text {
                Layout.alignment: Qt.AlignHCenter
                text: nameDialog.title
                font: Theme.fontTitle
                color: Theme.text
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            BorderedField {
                id: field
                objectName: nameDialog.objectName + ".field"
                Layout.fillWidth: true
                label: nameDialog.title
                onAccepted: nameDialog.finish()
            }
            Text {
                visible: nameDialog.hint.length > 0
                Layout.fillWidth: true
                text: nameDialog.hint
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            RowLayout {
                Layout.alignment: Qt.AlignHCenter
                spacing: Theme.spacingSmall
                AppButton {
                    text: qsTr("Cancel")
                    onClicked: nameDialog.close()
                }
                AppButton {
                    objectName: nameDialog.objectName + ".accept"
                    text: qsTr("Rename")
                    primary: true
                    onClicked: nameDialog.finish()
                }
            }
        }
    }
    NameDialog {
        id: presetRename
        objectName: "GlobalControls.presetRename"
        property int slot: -1
        title: qsTr("Rename Preset")
        placeholder: qsTr("Name")
        onAboutToShow: fieldText = slot >= 0 && SidebarController.presetOccupied(slot) ? SidebarController.presetTitle(slot) : ""
        onChosen: text => SidebarController.renamePreset(slot, text)
    }
    NameDialog {
        id: sourceRename
        objectName: "GlobalControls.sourceRename"
        property int source: 0
        title: qsTr("Rename Source")
        placeholder: SidebarController.builtInSourceTitle(source)
        //: %1 is the source's built-in name, "S/PDIF"
        hint: qsTr("Leave empty to use “%1”.").arg(SidebarController.builtInSourceTitle(source))
        onAboutToShow: fieldText = SidebarController.sourceName(source)
        onFieldTextChanged: {
            const clean = SidebarController.sanitizeSourceName(fieldText)
            if (clean !== fieldText) fieldText = clean
        }
        onChosen: text => SidebarController.renameSource(source, text)
    }
    // An alert with one answer
    Dialog {
        id: failure
        objectName: "GlobalControls.failure"
        property string text
        parent: Overlay.overlay
        modal: true
        closePolicy: Popup.CloseOnEscape
        width: 320
        x: (parent.width - width) / 2
        y: Math.max(Theme.spacingLarge, (parent.height - implicitHeight) / 3)
        padding: Theme.spacingLarge
        Overlay.modal: Rectangle { color: Theme.scrim }
        background: Rectangle {
            radius: Theme.radius
            color: Theme.panel
            border.color: Theme.border
        }
        contentItem: ColumnLayout {
            spacing: Theme.spacing
            Text {
                text: failure.title
                font: Theme.fontTitle
                textFormat: Text.PlainText   // -- WP MDQT2a -- it may hold a device's name
                color: Theme.text
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            Text {
                Layout.fillWidth: true
                text: failure.text
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.Wrap
                Accessible.role: Accessible.AlertMessage
                Accessible.name: failure.title
                Accessible.description: text
            }
            AppButton {
                id: failureOk
                Layout.alignment: Qt.AlignRight
                text: qsTr("OK")
                primary: true
                onClicked: failure.close()
            }
        }
        onOpened: failureOk.forceActiveFocus()
    }
}
