import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The menus and dialogs of the presets and the source (moved out
// of GlobalControls, names unchanged): the slot menu and its questions, the
// unsaved-changes question of a preset switch, a preset operation's failure,
// the source's menu, and the rename sheets. Classic's sidebar and Modern's
// shell have one each, whichever is loaded.
Item {
    id: dialogs
    objectName: "PresetDialogs"
    // Only popups: no item of its own
    visible: false
    readonly property int activeSlot: DeviceModel.presets.activeSlot

    function popupPresetMenu(item, x, y) { presetMenu.popup(item, x, y) }
    function popupSourceMenu(item, x, y) { sourceMenu.popup(item, x, y) }

    component Line: MenuSeparator {
        contentItem: Rectangle { implicitHeight: 1; color: Theme.separator }
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
                presetRename.slot = dialogs.activeSlot
                presetRename.open()
            }
        }
        AppMenuItem {
            text: qsTr("Set as Default")
            enabled: !(DeviceModel.presets.startupMode === 0 && DeviceModel.presets.defaultSlot === dialogs.activeSlot)
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
                    visible: index !== dialogs.activeSlot
                    height: visible ? implicitHeight : 0
                    onTriggered: SidebarController.requestPresetCopy(index)
                }
                onObjectAdded: (index, object) => copyMenu.insertItem(index, object)
                onObjectRemoved: (index, object) => copyMenu.removeItem(object)
            }
        }
        Line {
            visible: SidebarController.presetOccupied(dialogs.activeSlot)
            height: visible ? implicitHeight : 0
        }
        AppMenuItem {
            objectName: "GlobalControls.presetClear"
            //: %1 is a slot's label, "3: Living Room"
            text: qsTr("Clear \"%1\"…").arg(SidebarController.presetLabel(dialogs.activeSlot))
            visible: SidebarController.presetOccupied(dialogs.activeSlot)
            height: visible ? implicitHeight : 0
            onTriggered: clearOne.ask(dialogs.activeSlot)
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
                textFormat: Text.PlainText
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
                textFormat: Text.PlainText
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
                textFormat: Text.PlainText
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
