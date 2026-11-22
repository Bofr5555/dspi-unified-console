import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY6 -- The command palette (MODLAY 3.1): everything the Modern layout
// can do or show, found by typing. Every MenuTable row (a panel's goes to its
// page), the sections and their other pages (SectionTable), the channels, the
// Settings pages by what is on them, and the other open devices. Nothing is
// listed here by hand but the few names Modern has besides a row's own
// (`aliases`). PaletteController finds; the rows run what a menu would.
//
// It is modal, so no shortcut of the window works under it, and its field has
// the keyboard: a bare key types. What was chosen runs once it is closed, with
// the focus back where it was (Cut, Copy and Undo are the focused field's).
//
// An entry: { id, kind, arg, title, where, icon, keys, aliases, keywords,
// enabled }; PaletteController.search() adds `matched`.
Popup {
    id: commands
    objectName: "CommandPalette"
    property Window window: null
    // ModernShell's PageHost
    property Item pages: null

    // Made when it opens, and again while it is open if the devices change
    property var entries: []
    property var rows: []
    // The entry to run once it is closed
    property var chosen: null

    readonly property real hostWidth: parent ? parent.width : 800
    readonly property real hostHeight: parent ? parent.height : 600
    parent: Overlay.overlay
    width: Math.min(640, hostWidth - 2 * Theme.spacingLarge)
    height: Math.min(implicitHeight, hostHeight - y - Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: Math.min(72, hostHeight / 10)
    padding: 0
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    Overlay.modal: Rectangle { color: Theme.scrim }

    onAboutToShow: {
        chosen = null
        field.text = ""
        fill()
        pick(next(-1, 1))
    }
    onOpened: field.forceActiveFocus()
    onClosed: {
        const entry = chosen
        chosen = null
        rows = []
        entries = []
        if (entry) run(entry)
    }
    Connections {
        target: PaletteController
        function onOpening() { commands.open() }
    }
    Connections {
        target: commands.visible ? DeviceRegistry : null
        function onDevicesChanged() { commands.refill() }
    }
    Connections {
        target: LayoutController
        function onLayoutChanging() { commands.close() }
    }

    // ---- What there is to find ------------------------------------------------

    // The names Modern shows for a row besides its own
    readonly property var aliases: ({
        "device.commit": [qsTr("Save to Device")],
        "device.devices": [qsTr("Open a Device…"), qsTr("Switch Device")],
        "autoeq.browse": [qsTr("Headphone Profile…"), qsTr("AutoEQ")],
        "window.minimiseToTray": [qsTr("Minimize to Tray")],
        "window.popOutGraph": [qsTr("Open Graph in Window")],
        "tools.spectrum": [qsTr("Spectrum Analyzer")]
    })
    // Rows that depend on more than MenuTable.can() says
    readonly property var conditions: ({
        "autoeq.clearFavorites": () => AutoEqController.favouriteList.length > 0,
        "device.reconnect": () => !DeviceModel.remote   // except during a hub session (behaviour.md §3)
    })

    function makeEntries() {
        const list = []
        const add = e => list.push(Object.assign({ arg: e.id, icon: "", keys: "", aliases: [], keywords: [], enabled: true }, e))
        const sender = DeviceInfoController.sender

        // ---- The other open devices
        for (const d of DeviceRegistry.devices) {
            if (d.active) continue
            add({
                id: "device:" + d.id, kind: "device", arg: d.id,
                //: %1 is a device's name
                title: qsTr("Switch to %1").arg(d.title),
                where: d.serial.length ? qsTr("Device") + " · " + d.serial : qsTr("Device"),
                icon: "plug"
            })
        }

        // ---- The sections, and their pages that no menu row opens
        const free = !OnboardingController.wizardShown
        for (const s of SectionTable.sections) {
            const open = free && (!sender || s.sender === true)
            const tabs = SectionTable.tabs(s.id).filter(t => !t.quiet)
            const only = tabs.length === 1 ? tabs[0] : null
            // Its only page is a tool of the same name: that row says it
            if (!only || !only.row || only.title !== s.title || !MenuTable.row(only.row.action))
                add({
                    id: "section:" + s.id, kind: "section", arg: s.id, icon: s.icon, enabled: open,
                    //: %1 is a section of the app (Equaliser, Effects…)
                    title: qsTr("Go to %1").arg(s.title), where: qsTr("Section"), aliases: [s.title]
                })
            if (only && !only.row) continue
            for (const t of tabs)
                if (!t.row || !MenuTable.row(t.row.action))
                    add({ id: "page:" + t.key, kind: "page", arg: t.key, icon: t.icon, enabled: open, title: t.title, where: s.title })
        }

        // ---- The menus' rows
        for (const r of MenuTable.flatRows()) {
            if (r.optional && !Shell.handles(r.id)) continue
            if (r.platformHidden && DeviceModel.platformName === r.platformHidden) continue
            const panel = PanelTable.row(r.id)
            const section = panel ? SectionTable.section(panel.section) : null
            const tab = panel ? panel.tabTitle || panel.title : ""
            const preset = r.id.startsWith("device.preset") ? Number(r.id.slice("device.preset".length)) : 0
            const slot = preset > 0 && preset <= SidebarController.presetTitles.length ? SidebarController.presetTitles[preset - 1] : ""
            add({
                id: r.id, kind: "action", keys: r.shortcut || "",
                title: preset > 0 ? r.submenuTitle + ": " + (slot.length ? slot : r.text)
                     : r.submenuTitle ? r.submenuTitle + ": " + r.text : r.text,
                where: !section ? r.menu : section.title === tab ? section.title : section.title + " › " + tab,
                icon: panel ? panel.icon : "",
                aliases: aliases[r.id] || [],
                keywords: [r.text, r.menu, r.shortcut || ""].concat(panel ? [panel.title, tab] : []),
                enabled: MenuTable.can(r.id) && (!conditions[r.id] || conditions[r.id]())
            })
        }
        const classic = MenuTable.row("app.classicLayout")
        add({ id: classic.id, kind: "action", title: classic.text, where: qsTr("Layout"), enabled: LayoutController.switchBlocker.length === 0 })

        // ---- The channels
        for (let i = 0; i < DeviceModel.channels.count; ++i) {
            const channel = DeviceModel.channels.get(i, "channel")
            const label = DeviceModel.channels.get(i, "label")
            const descriptor = DeviceModel.channels.get(i, "descriptor")
            add({
                id: "channel:" + channel, kind: "channel", arg: channel, icon: "sliders-vertical", enabled: free && !sender,
                title: label, where: descriptor.length && descriptor !== label ? qsTr("Channel") + " · " + descriptor : qsTr("Channel"),
                keywords: [descriptor]
            })
        }

        // ---- Settings, by what is on each page
        for (const s of SettingsController.searchEntries())
            add({
                id: "settings:" + s.id, kind: "settings", arg: s.id, icon: s.icon, enabled: s.enabled,
                title: s.title, where: qsTr("Settings") + " › " + s.group, keywords: s.terms.concat([qsTr("Settings")])
            })
        return list
    }

    function fill() {
        entries = makeEntries()
        rows = PaletteController.search(entries, field.text)
    }
    // Something changed under it: the same entry stays picked, and the rows stay
    // when they are the same (a click on one may be half done)
    function refill() {
        const id = list.currentIndex >= 0 && list.currentIndex < rows.length ? rows[list.currentIndex].id : ""
        entries = makeEntries()
        const now = PaletteController.search(entries, field.text)
        if (JSON.stringify(now) === JSON.stringify(rows)) return
        rows = now
        const at = rows.findIndex(r => r.id === id && r.enabled)
        pick(at >= 0 ? at : next(-1, 1))
    }

    // ---- Choosing ---------------------------------------------------------------

    // The next row from `from` that can be chosen, round the end; -1 for none
    function next(from, step) {
        const n = rows.length
        for (let i = 1; i <= n; ++i) {
            const at = ((from + i * step) % n + n) % n
            if (rows[at].enabled) return at
        }
        return -1
    }
    function pick(at) {
        list.currentIndex = at
        if (at >= 0) list.positionViewAtIndex(at, ListView.Contain)
    }
    function move(step) { pick(next(list.currentIndex < 0 && step < 0 ? 0 : list.currentIndex, step)) }
    function movePage(step) {
        const n = rows.length
        let at = Math.max(0, Math.min(n - 1, list.currentIndex + step * Math.max(1, Math.floor(list.height / Theme.rowHeight) - 1)))
        while (at >= 0 && at < n && !rows[at].enabled) at -= step
        if (at >= 0 && at < n) pick(at)
    }
    function choose(at) {
        if (at < 0 || at >= rows.length) return
        // As it is now, not when the list was made
        const id = rows[at].id
        const entry = makeEntries().find(e => e.id === id)
        if (!entry || !entry.enabled) {
            refill()
            return
        }
        chosen = entry
        close()
    }

    function run(entry) {
        if (entry.kind === "device") DevicesController.chooseOpen(entry.arg)
        else if (entry.kind === "section") pages.go(entry.arg, "")
        else if (entry.kind === "page") pages.go(entry.arg.split("/")[0], entry.arg.split("/")[1])
        else if (entry.kind === "settings") SettingsController.showPage(entry.arg)
        else if (entry.kind === "channel") {
            SidebarController.select(entry.arg)
            pages.go("equaliser", "")
        } else if (entry.id === "app.classicLayout") LayoutController.layout = "classic"
        // Not by its key, which is the field's while one has the focus
        else if (entry.id === "device.abToggle") SnapshotsController.abToggle()
        else MenuTable.trigger(window, entry.id)
    }

    // ---- What it looks like -------------------------------------------------------

    background: Rectangle {
        radius: Theme.radius
        color: Theme.menu
        border.width: 1
        border.color: Theme.border
        Accessible.role: Accessible.Dialog
        Accessible.name: qsTr("Command palette")
    }

    contentItem: ColumnLayout {
        spacing: 0

        TextField {
            id: field
            objectName: "CommandPalette.field"
            Layout.fillWidth: true
            implicitHeight: 42
            leftPadding: 40
            rightPadding: Theme.spacing
            font.pixelSize: 15
            color: Theme.text
            selectionColor: Theme.textSelection
            selectedTextColor: Theme.text
            placeholderText: qsTr("Type a command, page, setting, channel or device")
            placeholderTextColor: Theme.textMuted
            verticalAlignment: TextInput.AlignVCenter
            focus: true
            background: Item {
                Icon {
                    name: "search"
                    size: 16
                    x: 14
                    anchors.verticalCenter: parent.verticalCenter
                    color: Theme.textMuted
                }
            }
            onTextEdited: {
                commands.rows = PaletteController.search(commands.entries, text)
                commands.pick(commands.next(-1, 1))
                if (commands.rows.length) list.positionViewAtBeginning()
            }
            Keys.onPressed: event => {
                event.accepted = true
                if (event.key === Qt.Key_Down || event.key === Qt.Key_Tab) commands.move(1)
                else if (event.key === Qt.Key_Up || event.key === Qt.Key_Backtab) commands.move(-1)
                else if (event.key === Qt.Key_PageDown) commands.movePage(1)
                else if (event.key === Qt.Key_PageUp) commands.movePage(-1)
                else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) commands.choose(list.currentIndex)
                else event.accepted = false
            }
            Accessible.role: Accessible.EditableText
            Accessible.name: qsTr("Search settings, channels, commands…")
            // The row that Enter runs, for a screen reader: the focus stays here
            Accessible.description: list.currentItem ? list.currentItem.spoken : ""
        }
        // Its own key closes it; the windows' shortcuts do not work under it
        Shortcut {
            sequence: PaletteController.shortcut
            enabled: commands.opened
            onActivated: commands.close()
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }

        Text {
            objectName: "CommandPalette.none"
            Layout.fillWidth: true
            visible: commands.rows.length === 0
            text: qsTr("Nothing found")
            font: Theme.fontBody
            color: Theme.textMuted
            horizontalAlignment: Text.AlignHCenter
            topPadding: Theme.spacingLarge
            bottomPadding: Theme.spacingLarge
        }
        ListView {
            id: list
            objectName: "CommandPalette.list"
            visible: commands.rows.length > 0
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: 6
            implicitHeight: Math.min(420, contentHeight)
            model: commands.rows
            clip: true
            currentIndex: -1
            highlightMoveDuration: 0
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar { visible: list.contentHeight > list.height }
            Accessible.role: Accessible.List
            Accessible.name: qsTr("Commands")

            delegate: AbstractButton {
                id: row
                required property var modelData
                required property int index
                readonly property bool current: ListView.isCurrentItem
                // Where it is, and what it was found by when that is not its title
                readonly property bool unseen: modelData.matched.length > 0 && modelData.matched !== modelData.keys
                                               && modelData.title.indexOf(modelData.matched) < 0 && modelData.where.indexOf(modelData.matched) < 0
                readonly property string detail: unseen ? modelData.matched + " · " + modelData.where : modelData.where
                readonly property string spoken: modelData.title + ", " + detail + (modelData.keys.length ? ", " + modelData.keys : "")
                objectName: "CommandPalette.row." + modelData.id
                width: ListView.view.width - (list.contentHeight > list.height ? 12 : 0)
                height: Theme.rowHeight + 4
                enabled: modelData.enabled
                hoverEnabled: true
                focusPolicy: Qt.NoFocus
                leftPadding: 8
                rightPadding: 8
                text: modelData.title
                onClicked: commands.choose(index)
                Accessible.role: Accessible.ListItem
                Accessible.name: spoken
                Accessible.selected: current

                background: Rectangle {
                    radius: Theme.radiusSmall
                    color: row.current ? Theme.selection : row.hovered ? Theme.controlHover : "transparent"
                }
                contentItem: RowLayout {
                    spacing: Theme.spacingSmall
                    Icon {
                        name: row.modelData.icon.length ? row.modelData.icon : "chevron-right"
                        size: 14
                        color: row.enabled ? Theme.textMuted : Theme.textDisabled
                    }
                    Text {
                        objectName: row.objectName + ".title"
                        Layout.fillWidth: true
                        Layout.minimumWidth: 120
                        textFormat: Text.PlainText
                        text: row.modelData.title
                        font: Theme.fontBody
                        color: row.enabled ? Theme.text : Theme.textDisabled
                        elide: Text.ElideRight
                    }
                    Text {
                        objectName: row.objectName + ".where"
                        Layout.maximumWidth: row.width * 0.45
                        textFormat: Text.PlainText
                        text: row.detail
                        font: Theme.fontSmall
                        color: row.enabled ? Theme.textMuted : Theme.textDisabled
                        elide: Text.ElideRight
                    }
                    Rectangle {
                        visible: row.modelData.keys.length > 0
                        implicitWidth: keysText.implicitWidth + 10
                        implicitHeight: 18
                        radius: Theme.radiusSmall
                        color: Theme.control
                        Text {
                            id: keysText
                            objectName: row.objectName + ".keys"
                            anchors.centerIn: parent
                            textFormat: Text.PlainText
                            text: row.modelData.keys
                            font: Theme.fontSmall
                            color: row.enabled ? Theme.textMuted : Theme.textDisabled
                        }
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: 8
            Layout.leftMargin: 14
            spacing: Theme.spacingLarge
            Repeater {
                model: [qsTr("↑ ↓ choose"), qsTr("Enter run"), qsTr("Esc close")]
                delegate: Text {
                    required property string modelData
                    text: modelData
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
            }
            Item { Layout.fillWidth: true }
        }
    }
}
