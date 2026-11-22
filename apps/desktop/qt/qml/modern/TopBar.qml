import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The Modern top bar (MODLAY 2): the app button, the device tabs
// (with one device; several get a row of their own above, ModernShell), the
// preset picker, A/B, the search, the AI button, the theme, undo and redo, Save
// to device, the installer's lock and the notifications. Actions are MenuTable
// ids, so a row greyed in Classic is greyed here.
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

    implicitHeight: 48
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
        AppComboBox {
            id: presetBox
            objectName: "TopBar.preset"
            readonly property string shownName: displayText
            visible: !DeviceInfoController.sender
            implicitWidth: bar.compact ? 124 : 150
            label: qsTr("Preset")
            model: SidebarController.presetTitles
            displayText: count > 0 && currentIndex >= 0 ? currentText : "–"
            currentIndex: bar.activeSlot
            enabled: bar.writable && !SidebarController.presetBusy
            onActivated: index => {
                SidebarController.requestPresetSwitch(index)
                // The device says which slot is active, not the click
                currentIndex = Qt.binding(() => bar.activeSlot)
            }
            // -- WP LAY3b -- a right-click on a slot in the list: that slot's menu
            rowMenus: true
            onRowMenu: (index, row, x, y) => {
                const at = row.mapToItem(bar, x, y)
                popup.close()
                ContextMenus.slot(index, bar.dialogs, bar, at.x, at.y)
            }
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: point => {
                    ContextMenus.slot(bar.activeSlot, bar.dialogs, presetBox, point.position.x, point.position.y)   // -- WP LAY3b --
                }
            }
        }
        IconButton {
            id: slotButton
            objectName: "TopBar.presetMenu"
            visible: presetBox.visible
            iconName: "save"
            iconSize: 14
            toolTip: qsTr("Preset Slot…")
            enabled: bar.writable && !SidebarController.presetBusy && bar.activeSlot >= 0
            onClicked: ContextMenus.slot(bar.activeSlot, bar.dialogs, slotButton, 0, slotButton.height + 2)   // -- WP LAY3b --
        }
        ToolSegments {
            objectName: "TopBar.ab"
            visible: !DeviceInfoController.sender
            label: qsTr("A/B compare")
            model: ["A", "B"]
            current: !SnapshotsController.abActive ? -1 : SnapshotsController.abSide === "b" ? 1 : 0
            enabled: DeviceModel.connected
            // With no compare running it is started in Library › Snapshots
            onChosen: index => {
                if (!SnapshotsController.abActive) Shell.trigger("device.snapshots")
                else if (index !== current && SnapshotsController.canToggle) Shell.trigger("device.abToggle")
            }
        }

        Item { Layout.fillWidth: true; Layout.preferredWidth: 1 }

        // ---- Search (the command palette is WP LAY6's; until then the settings' search)
        AbstractButton {
            id: search
            objectName: "TopBar.search"
            Layout.fillWidth: !bar.compact
            Layout.maximumWidth: bar.compact ? 32 : 300
            Layout.preferredWidth: bar.compact ? 32 : 240
            implicitHeight: 28
            text: qsTr("Search settings…")
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
                        text: search.text
                        font: Theme.fontBody
                        color: Theme.textMuted
                    }
                }
            }
            background: Rectangle {
                radius: Theme.radiusSmall
                color: search.hovered ? Theme.controlHover : Theme.control
                border.width: search.visualFocus ? 2 : 0
                border.color: Theme.focusRing
            }
            HelpTip {
                text: search.text
                visible: bar.compact && search.hovered
            }
            Accessible.role: Accessible.Button
            Accessible.name: text
        }

        // -- WP LAY3 -- the AI button; -- WP MCP4 -- with the agents' state, opening
        // Settings > AI Agents. WP LAY8 puts AiButton here in this block's place
        AbstractButton {
            id: ai
            objectName: "TopBar.ai"
            text: "AI"
            readonly property bool attention: AiController.state === "asking" || AiController.state === "error"
            implicitHeight: 28
            implicitWidth: aiFace.implicitWidth + 18
            hoverEnabled: true
            focusPolicy: Qt.TabFocus
            onClicked: AiController.openSettings()
            contentItem: Item {
                Row {
                    id: aiFace
                    anchors.centerIn: parent
                    spacing: 6
                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 8
                        height: 8
                        radius: 4
                        color: ai.attention ? Theme.warning : AiController.state === "active" ? Theme.connected
                             : AiController.state === "on" || AiController.state === "paused" ? Theme.offline : "transparent"
                        border.width: AiController.state === "off" ? 1.5 : 0
                        border.color: Theme.textMuted
                    }
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: ai.text
                        font: Theme.fontBody
                        color: Theme.text
                    }
                }
            }
            background: Rectangle {
                radius: Theme.radiusSmall
                color: ai.hovered ? Theme.controlHover : Theme.control
                border.width: ai.visualFocus ? 2 : 0
                border.color: Theme.focusRing
            }
            Accessible.role: Accessible.Button
            Accessible.name: AiController.statusLine
            HelpTip {
                text: AiController.statusLine
                visible: ai.hovered
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

    component ThemeItem: AppMenuItem {
        required property string value
        objectName: "TopBar.theme." + value
        checkable: true
        checked: LayoutController.theme === value
        onTriggered: LayoutController.theme = value
    }
}
