import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The Modern top bar (MODLAY 2): the app button, the device tabs
// (with one device; several get a row of their own above, ModernShell), the
// preset picker, A/B, the search, the AI button, the theme, undo and redo, Save
// to device, the installer's lock and the notifications. Actions are MenuTable
// ids, so a row greyed in Classic is greyed here.
// -- WP DL5 -- The preset picker is one button and its popover (PresetPopover).
// -- WP UA3 -- Save to Device: its count explained, greyed with no device,
// Save All with several devices, no Factory Reset; A/B only while a compare runs.
// -- WP UB34 -- A/B also from the first edit: as saved against now.
Rectangle {
    id: bar
    objectName: "TopBar"
    property Window window: null
    // ModernShell's: the slot menu, and the questions a preset switch asks
    property Item dialogs: null
    property bool showDevices: true
    property int notifications: 0
    readonly property bool writable: DeviceModel.connected && !DeviceModel.readOnly
    readonly property int activeSlot: DeviceModel.presets.activeSlot
    // Room runs out in this order: the search's words, Save's words
    readonly property bool compact: width < 1240
    readonly property bool tight: width < 1120
    // Where the device switcher opens from
    readonly property Item deviceAnchor: devices
    signal bellClicked()
    signal searchRequested()

    // -- WP UA3 -- Save to Device's count, explained: what is unsaved, as the
    // Unsaved Changes question lists it (15 lines, then how many more)
    function unsavedTip(lines) {
        const shown = lines.slice(0, 15).map(l => "• " + l)
        const more = lines.length - shown.length
        if (more === 1) shown.push(qsTr("…and 1 more change"))
        else if (more > 1) shown.push(qsTr("…and %1 more changes").arg(more))
        const head = lines.length === 1 ? qsTr("1 unsaved change on this device:") : qsTr("%1 unsaved changes on this device:").arg(lines.length)
        //: %1 is the shortcut, Ctrl+S
        return head + "\n" + shown.join("\n") + "\n\n" + qsTr("Save to Device (%1)").arg(MenuTable.shortcutOf("device.commit"))
    }
    // -- WP UA3 -- With several devices open: Save All, one question for every
    // device with unsaved changes (UnsavedGuard)
    readonly property int unsavedDevices: DeviceRegistry.devices.filter(r => r.dirty === true).length
    readonly property var saveAllRows: DeviceRegistry.devices.length < 2 ? [] : [{
        name: "saveAll",
        text: unsavedDevices === 0 ? qsTr("Save All…")
            : unsavedDevices === 1 ? qsTr("Save All (1 device)…")
            //: %1 is how many devices have unsaved changes, 2 or more
            : qsTr("Save All (%1 devices)…").arg(unsavedDevices),
        enabled: unsavedDevices > 0 && !UnsavedGuard.asking,
        run: () => UnsavedGuard.askSaveAll()
    }]

    implicitHeight: Theme.topBarHeight   // -- INTC (DL5 fold) -- 40, the token (was 48)
    color: Theme.sidebar
    Accessible.role: Accessible.ToolBar
    Accessible.name: AppIdentity.displayName

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.spacingSmall
        anchors.rightMargin: Theme.spacingSmall
        spacing: 6

        AppMenuButton {
            window: bar.window
        }
        DeviceTabs {
            id: devices
            visible: bar.showDevices
            Layout.preferredWidth: 250
            Layout.minimumWidth: 150
            Layout.fillWidth: true
            Layout.maximumWidth: 270
        }

        // ---- Presets and A/B
        // -- WP DL5 -- One control (DESREV §5): the active preset's name, and its
        // popover with every slot and what can be done to it (PresetPopover), in
        // place of the combo box and the slot menu beside it. The name is the
        // active slot's title, with its "*" while unsaved (WP USR1); Up and Down
        // load the slot next to it, as the combo box did; a right-click opens the
        // active slot's menu (WP LAY3b).
        AbstractButton {
            id: presetBox
            AiTouched { path: "preset." }   // -- WP AIVIS --
            AiTouched { path: "bulk" }
            objectName: "TopBar.preset"
            readonly property var titles: SidebarController.presetTitles
            readonly property string shownName: bar.activeSlot >= 0 && bar.activeSlot < titles.length ? titles[bar.activeSlot] : "–"
            // The popover, as the combo box's list was (the smoke driver, gallery.py)
            readonly property alias popup: presetPopover
            visible: !DeviceInfoController.sender
            implicitWidth: bar.compact ? 124 : 150
            implicitHeight: 28
            enabled: bar.writable && !SidebarController.presetBusy
            hoverEnabled: true
            focusPolicy: Qt.StrongFocus
            text: shownName
            // From the keyboard, the popover's keyboard row starts on the active slot
            function toggle(keyboard) {
                if (presetPopover.opened) {
                    presetPopover.close()
                    return
                }
                presetPopover.fromKeyboard = keyboard
                presetPopover.open()
            }
            onClicked: toggle(false)
            function step(by) {
                const to = bar.activeSlot + by
                if (bar.activeSlot >= 0 && to >= 0 && to < titles.length) SidebarController.requestPresetSwitch(to)
            }
            Keys.onDownPressed: step(1)
            Keys.onUpPressed: step(-1)
            Keys.onSpacePressed: toggle(true)
            Keys.onReturnPressed: toggle(true)
            Keys.onEnterPressed: toggle(true)
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: point => {
                    ContextMenus.slot(bar.activeSlot, bar.dialogs, presetBox, point.position.x, point.position.y)   // -- WP LAY3b --
                }
            }
            contentItem: RowLayout {
                spacing: 4
                Text {
                    Layout.fillWidth: true
                    leftPadding: 8
                    textFormat: Text.PlainText
                    text: presetBox.shownName
                    font: Theme.fontBody
                    color: presetBox.enabled ? Theme.text : Theme.textDisabled
                    elide: Text.ElideRight
                }
                Icon {
                    Layout.rightMargin: 6
                    name: "chevrons-up-down"
                    size: 12
                    color: presetBox.enabled ? Theme.textMuted : Theme.textDisabled
                }
            }
            background: Rectangle {
                radius: Theme.radiusControl
                color: presetBox.down || presetPopover.opened ? Theme.controlPressed : presetBox.hovered ? Theme.barControlHover : Theme.barControl   // -- WP REVDFIX -- D1
                border.width: presetBox.visualFocus ? Theme.focusRingWidth : 0
                border.color: Theme.focusRing
            }
            PresetPopover {
                id: presetPopover
                y: presetBox.height + 4
                dialogs: bar.dialogs
                maximumHeight: bar.window ? bar.window.height - presetBox.mapToItem(null, 0, presetBox.height).y - 16 : 600
            }
            Accessible.role: Accessible.ComboBox
            Accessible.name: qsTr("Preset")
            Accessible.description: shownName
            Accessible.onPressAction: toggle(true)
        }
        // -- WP UA3 -- (UXP C2) Shown while a compare runs, and then it says what
        // A and B are; never a way to a page.
        // -- WP UB34 -- (UXP B3, owner D7) Also from the first edit: A is the
        // preset as saved on the device, B is now (SnapshotsController's
        // abStartSaved); a click on A, or the B key, starts that compare. A
        // compare of two snapshots (Library › Snapshots) shows its names.
        ToolSegments {
            id: ab
            objectName: "TopBar.ab"
            readonly property bool abShown: SnapshotsController.abActive || SnapshotsController.canStartSaved
            readonly property bool saved: SnapshotsController.abSaved || !SnapshotsController.abActive
            track: Theme.barControl   // -- WP REVDFIX -- D1
            visible: !DeviceInfoController.sender && abShown
            label: qsTr("A/B compare")
            model: ["A", "B"]
            // B, now, is what plays until a compare starts
            current: !SnapshotsController.abActive ? (abShown ? 1 : -1) : SnapshotsController.abSide === "b" ? 1 : 0
            enabled: DeviceModel.connected
            readonly property string toolTip: !abShown ? ""
                : !SnapshotsController.abActive
                //: B is the key that switches
                ? qsTr("A/B compare: A is the preset as saved on the device, B is now, with your unsaved changes. Click A, or press B, to hear the saved settings.")
                : saved && SnapshotsController.abSide === "a"
                //: B is the key that switches
                ? qsTr("A/B compare: you hear A, the preset as saved on the device. B is now, with your changes. Click B, or press B, to go back.")
                : saved
                //: B is the key that switches
                ? qsTr("A/B compare: you hear B, now, with your changes. A is the preset as saved on the device. Click A, or press B, to switch.")
                //: %1 and %2 are snapshots' names; B is the key that switches
                : qsTr("A/B compare: A is “%1”, B is “%2”. Click the other one, or press B, to switch.")
                      .arg(SnapshotsController.abAName).arg(SnapshotsController.abBName)
            onChosen: index => {
                if (index === current) return
                if (!SnapshotsController.abActive) {
                    if (index === 0) SnapshotsController.abStartSaved()
                } else if (SnapshotsController.canToggle) Shell.trigger("device.abToggle")
            }
            HoverHandler { id: abHover }
            HelpTip {
                objectName: "TopBar.ab.tip"
                text: ab.toolTip
                visible: abHover.hovered && text.length > 0
            }
        }

        Item { Layout.fillWidth: true; Layout.preferredWidth: 1 }

        // ---- Search: the command palette (-- WP LAY6 --)
        AbstractButton {
            id: search
            objectName: "TopBar.search"
            Layout.fillWidth: !bar.compact
            Layout.maximumWidth: bar.compact ? 32 : 300
            Layout.preferredWidth: bar.compact ? 32 : 240
            implicitHeight: 28
            text: qsTr("Search settings, channels, commands…")
            hoverEnabled: true
            focusPolicy: Qt.TabFocus
            onClicked: bar.searchRequested()
            contentItem: Item {
                Row {
                    anchors.verticalCenter: parent.verticalCenter
                    x: bar.compact ? (search.width - 14) / 2 : 8
                    spacing: 6
                    Icon {
                        anchors.verticalCenter: parent.verticalCenter
                        name: "search"
                        size: 14
                        color: Theme.textMuted
                    }
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: !bar.compact
                        // -- WP LAY6 -- up to the shortcut
                        width: Math.min(implicitWidth, search.width - 14 - keys.implicitWidth - 8 - 3 * parent.spacing - 8)
                        elide: Text.ElideRight
                        text: search.text
                        font: Theme.fontBody
                        color: Theme.textMuted
                    }
                }
                // -- WP LAY6 --
                Text {
                    id: keys
                    objectName: "TopBar.search.keys"
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    visible: !bar.compact
                    text: PaletteController.shortcut
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
            }
            background: Rectangle {
                radius: Theme.radiusSmall
                color: search.hovered ? Theme.barControlHover : Theme.barControl   // -- WP REVDFIX -- D1
                border.width: search.visualFocus ? 2 : 0
                border.color: Theme.focusRing
            }
            HelpTip {
                text: search.text + " (" + PaletteController.shortcut + ")"   // -- WP LAY6 --
                visible: bar.compact && search.hovered
            }
            Accessible.role: Accessible.Button
            Accessible.name: text
        }

        // -- WP LAY8 -- the AI button, with the agents' state and its popover
        AiButton {
            id: aiButton
            objectName: "TopBar.ai"
            // -- WP DL5 -- the AI strip's way to its popover (grants and Revoke)
            Connections {
                target: ContextMenus
                function onAiRequested() { if (!aiButton.popover.opened) aiButton.popover.open() }
            }
        }

        // ---- The theme: the system's, light or dark
        IconButton {
            id: themeButton
            objectName: "TopBar.theme"
            iconName: Theme.dark ? "moon" : "sun"
            toolTip: qsTr("Theme")
            onClicked: themeMenu.popup(themeButton, themeButton.width - themeMenu.width, themeButton.height + 2)
            AppMenu {
                id: themeMenu
                objectName: "TopBar.themeMenu"
                ThemeItem { value: "system"; text: qsTr("System") }
                ThemeItem { value: "light"; text: qsTr("Light") }
                ThemeItem { value: "dark"; text: qsTr("Dark") }
            }
        }

        // ---- Undo, redo, save
        // -- WP LAY3b -- Undo ▾: what was changed last, and History
        SplitButton {
            objectName: "TopBar.undo"
            window: bar.window
            action: "edit.undo"
            place: "history"
            iconName: "undo-2"
            toolTip: MenuTable.textOf("edit.undo", false)
            handlers: ({
                "history.recent": {
                    rows: () => HistoryController.entries.slice(0, 8).map((e, i) => ({
                        name: i, text: e.detail.length ? e.title + "  " + e.detail : e.title, detail: e.time, value: i
                    })),
                    run: () => Shell.trigger("edit.history")
                }
            })
        }
        IconButton {
            objectName: "TopBar.redo"
            iconName: "redo-2"
            toolTip: MenuTable.textOf("edit.redo", false)
            enabled: HistoryController.canRedo && MenuTable.available("edit.redo")
            onClicked: MenuTable.trigger(bar.window, "edit.redo")
        }
        SplitButton {
            id: save
            objectName: "TopBar.save"
            visible: !DeviceInfoController.sender
            window: bar.window
            action: "device.commit"
            place: "save"   // -- WP LAY3b --
            iconName: "hard-drive"
            text: bar.tight ? "" : qsTr("Save to Device")
            toolTip: qsTr("Save to Device")
            primary: SidebarController.unsaved
            count: SidebarController.unsaved ? SidebarController.unsavedChanges.length : 0
            // -- WP UA3 -- the count explained: the Unsaved Changes list
            countToolTip: count === 0 ? "" : bar.unsavedTip(SidebarController.unsavedChanges)
            actionEnabled: DeviceModel.connected
            // -- WP UA3 -- (UXP C1) Factory Reset is no save: it is on Devices ›
            // This Computer and the device menu, not here
            handlers: ({ "device.factoryReset": { shown: () => false } })
            leadRows: bar.saveAllRows
        }

        // ---- The installer's lock (only with a PIN set) and the notifications
        IconButton {
            objectName: "TopBar.lock"
            visible: LimitsController.pinSet
            iconName: LimitsController.locked ? "lock" : "lock-open"
            iconColor: LimitsController.locked ? Theme.warning : Theme.text
            toolTip: LimitsController.locked ? qsTr("Locked by the installer. Unlock…") : qsTr("Lock Now")
            onClicked: LimitsController.locked ? unlockDialog.ask() : LimitsController.lockNow()
            UnlockDialog {
                id: unlockDialog
                parent: Overlay.overlay
            }
        }
        IconButton {
            objectName: "TopBar.bell"
            readonly property int shownCount: bar.notifications
            iconName: "bell"
            toolTip: bar.notifications > 0 ? qsTr("%n notification(s)", "", bar.notifications) : qsTr("Notifications")
            onClicked: bar.bellClicked()
            Rectangle {
                visible: bar.notifications > 0
                anchors.right: parent.right
                anchors.top: parent.top
                width: Math.max(14, badge.implicitWidth + 6)
                height: 14
                radius: 7
                color: Theme.danger
                Text {
                    id: badge
                    anchors.centerIn: parent
                    text: bar.notifications
                    font: Theme.fontCaption
                    color: Theme.textOnAccent
                }
            }
        }
    }

    // -- WP UA3 -- Save All's question
    QuestionDialog {
        id: saveAllQuestion
        objectName: "TopBar.saveAllQuestion"
        parent: Overlay.overlay
        acceptText: qsTr("Save All")
        onAnswered: answer => UnsavedGuard.answerSaveAll(answer)
    }
    Connections {
        target: UnsavedGuard
        function onSaveAllQuestion(title, text) {
            saveAllQuestion.title = title
            saveAllQuestion.text = text
            saveAllQuestion.open()
        }
    }

    component ThemeItem: AppMenuItem {
        required property string value
        objectName: "TopBar.theme." + value
        checkable: true
        checked: LayoutController.theme === value
        onTriggered: LayoutController.theme = value
    }
}
