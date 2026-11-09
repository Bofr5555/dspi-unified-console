import QtQuick 2.15
import QtQuick.Controls 2.15

// The app menu opened from the titlebar's menu button, laid out like the
// macOS menu bar: the device summary, then Device, File, AutoEQ, Effects,
// Tools and Help, each opening its items in a submenu beside it, then Settings.
// Keyboard: Up/Down, Right or Enter opens a submenu, Esc closes.
Popup {
    id: menu
    width: Math.ceil(MenuStyle.sideInset + MenuStyle.iconSize + 10 + labelWidest.advanceWidth
                     + 40 + MenuStyle.sideInset + 2 * MenuStyle.padding)
    padding: MenuStyle.padding
    modal: false
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    TextMetrics {
        id: labelWidest
        font.pixelSize: MenuStyle.fontSize
        text: qsTr("No device connected") + "   "
    }

    // Actions are supplied by the window that hosts the menu
    signal commitRequested()
    signal revertRequested()
    signal factoryResetRequested()
    signal openWindow(string name)
    // importFilters / exportFilters / importConfig / exportConfig
    signal fileAction(string name)
    signal autoeqUpdateRequested()

    // While the Getting Started wizard has the window: only Settings and Help
    property bool restricted: false

    // The menu button toggles the menu. Pressing the button while the menu is
    // open closes it as an outside press before the click arrives, so a click
    // right after such a close must not reopen it.
    property real closedAt: 0
    onClosed: { closedAt = Date.now(); sub.close(); openGroup = -1 }

    function toggleAt(anchorItem) {
        if (visible) { close(); return }
        if (Date.now() - closedAt < 300) return
        openAt(anchorItem)
    }

    function openAt(anchorItem) {
        var p = anchorItem.mapToItem(parent, 0, anchorItem.height + 6)
        x = Math.max(6, p.x - 4)
        y = p.y
        current = -1
        open()
    }

    // ── Menus ──
    // Items: { icon, text, shortcut, action | window | autoeq, danger,
    //          help (allowed during setup), always (needs no device),
    //          enabled: false } or { sep: true }
    readonly property var groups: [
        { text: qsTr("Device"), icon: "chip", items: [
            { icon: "link", text: qsTr("Devices…"), window: "devices" },   // -- WP QD9 --
            { sep: true },
            { icon: "save", text: qsTr("Commit Parameters"), shortcut: "Ctrl+S", action: "commit" },
            { icon: "revert", text: qsTr("Revert to Saved"), action: "revert" },
            { sep: true },
            { icon: "speaker", text: qsTr("Save Master Volume"), action: "saveMasterVolume" },
            { icon: "chip", text: qsTr("Save Output Config"), action: "saveOutputConfig" },
            { sep: true },
            { icon: "chip", text: qsTr("Firmware Update…"), window: "firmware", firmwareUpdate: true },
            { icon: "warning", text: qsTr("Factory Reset…"), action: "factoryReset", danger: true }
        // -- WP QD11 -- No UF2 bootloader on this board (the STM32H723), so no entry
        ].filter(function (r) { return !r.firmwareUpdate || menu.firmwareUpdatable })},
        { text: qsTr("File"), icon: "input", items: [
            { icon: "input", text: qsTr("Import Filters…"), shortcut: "Ctrl+I", action: "importFilters" },
            { icon: "output", text: qsTr("Export Filters…"), shortcut: "Ctrl+E", action: "exportFilters" },
            { sep: true },
            { icon: "input", text: qsTr("Import Device Configuration…"), action: "importConfig" },
            { icon: "output", text: qsTr("Export Device Configuration…"), action: "exportConfig" }
        ]},
        { text: qsTr("AutoEQ"), icon: "headphones", items: autoeqItems },
        { text: qsTr("Effects"), icon: "sliders", items: [
            { icon: "loudness", text: qsTr("Loudness Compensation"), shortcut: "Ctrl+Shift+L", window: "loudness" },
            { icon: "headphones", text: qsTr("Headphone Crossfeed"), shortcut: "Ctrl+Shift+X", window: "crossfeed" },
            { icon: "waveform", text: qsTr("Volume Leveller"), shortcut: "Ctrl+Shift+V", window: "leveller" },
            { icon: "bassclef", text: qsTr("Psychoacoustic Bass"), shortcut: "Ctrl+Shift+P", window: "psybass", feature: true },
            { icon: "subwave", text: qsTr("Subharmonic Synthesizer"), shortcut: "Ctrl+Shift+S", window: "subharm", feature: true },
            { icon: "tube", text: qsTr("Tube Modeller"), shortcut: "Ctrl+Shift+D", window: "tube", feature: true },
            { icon: "upmix", text: qsTr("Stereo Upmixer"), shortcut: "Ctrl+Shift+U", window: "upmix", feature: true }
        ].filter(function (r) { menu.firmwareFeatures; return !r.feature || bridge.firmwareHas(r.window) })},
        { text: qsTr("Tools"), icon: "wrench", items: [
            { icon: "sliders", text: qsTr("Matrix Mixer"), shortcut: "Ctrl+Shift+M", window: "matrix" },
            { sep: true },
            { icon: "spectrum", text: qsTr("Spectrum Analyser"), shortcut: "Ctrl+Shift+A", window: "spectrum" },
            { icon: "signal", text: qsTr("Signal Generator"), shortcut: "Ctrl+Shift+G", window: "siggen" },
            { icon: "info", text: qsTr("Stats for Nerds"), shortcut: "Ctrl+Shift+T", window: "stats" },
            { icon: "identify", text: qsTr("Interrupt Monitor"), shortcut: "Ctrl+Shift+I", window: "monitor" }
        ]},
        { text: qsTr("Help"), icon: "question-circle", help: true, items: [
            { icon: "cap", text: qsTr("Getting Started…"), window: "gettingStarted", help: true },
            { icon: "sparkles", text: qsTr("What's New in DSPi uConsole"), window: "whatsNew", help: true },
            { sep: true },
            // -- WP REL013M2 -- this app's own project (was Weeb Labs' Linux console, WP QD40)
            { icon: "github", text: qsTr("DSPi uConsole on GitHub"), window: "url:https://github.com/Bofr5555/dspi-unified-console", help: true },
            { icon: "github", text: qsTr("DSPi Firmware on GitHub"), window: "url:https://github.com/WeebLabs/DSPi", help: true }
        ]}
    ]
    // Browse, the favourites (one click applies), and the database
    readonly property var autoeqItems: {
        var list = [{ icon: "search", text: qsTr("Browse Profiles…"), shortcut: "Ctrl+Shift+B", window: "autoeq" }, { sep: true }]
        var favs = autoeq.favorites
        if (favs.length === 0) list.push({ icon: "heart", text: qsTr("No favorites yet"), enabled: false })
        for (var i = 0; i < favs.length; i++) list.push({ icon: "heart", text: favs[i].name, autoeq: favs[i].id })
        list.push({ icon: "xmark", text: qsTr("Clear Favorites"), action: "clearFavorites", always: true, enabled: favs.length > 0 })
        list.push({ sep: true })
        list.push({ icon: "revert", text: qsTr("Update Database…"), action: "autoeqUpdate", always: true })
        return list
    }
    readonly property var settingsItem: ({ icon: "gear", text: qsTr("Settings"), shortcut: "Ctrl+,", window: "settings" })

    // -- WP QD6 -- Changes with the device: `groups` hides what its firmware lacks
    readonly property int firmwareFeatures: bridge.wireFeatures
    // -- WP QD11 --
    readonly property bool firmwareUpdatable: firmware.supported
    readonly property var writeActions: ["commit", "revert", "saveMasterVolume", "saveOutputConfig", "factoryReset", "importFilters", "importConfig"]
    function rowEnabled(r) {
        if (r.sep || r.enabled === false) return false
        if (restricted && !r.help && r.window !== "settings") return false
        if (r.window || r.always) return true
        // -- WP QD6 -- Export and the firmware update stay; whatever writes doesn't
        if (r.autoeq || writeActions.indexOf(r.action) >= 0) return bridge.writable
        return bridge.connected
    }
    function groupEnabled(g) {
        for (var i = 0; i < g.items.length; i++) if (rowEnabled(g.items[i])) return true
        return false
    }

    function activateItem(r) {
        if (!r || !rowEnabled(r)) return
        close()
        if (r.window) { openWindow(r.window); return }
        if (r.autoeq) { autoeq.apply(r.autoeq); return }
        switch (r.action) {
        case "commit": commitRequested(); break
        case "revert": revertRequested(); break
        case "saveMasterVolume": bridge.saveMasterVolume(); break
        case "saveOutputConfig": bridge.saveOutputConfig(); break
        case "factoryReset": factoryResetRequested(); break
        case "clearFavorites": autoeq.clearFavorites(); break
        case "autoeqUpdate": autoeqUpdateRequested(); break
        default: fileAction(r.action)
        }
    }

    // ── Top level: groups, then Settings ──
    property int current: -1          // 0..groups.length-1, groups.length = Settings
    property int openGroup: -1
    readonly property int rowCount: groups.length + 1

    function openSubmenu(i, focusIt) {
        if (i < 0 || i >= groups.length || !groupEnabled(groups[i])) { sub.close(); openGroup = -1; return }
        current = i
        openGroup = i
        var row = groupRows.itemAt(i)
        sub.items = groups[i].items.map(function (r, k) {
            return r.sep ? { separator: true }
                         : { key: "" + k, text: r.text, icon: r.icon, shortcut: r.shortcut || "",
                             enabled: rowEnabled(r), danger: r.danger === true }
        })
        sub.openAt(row, row.width + MenuStyle.padding + 2, -MenuStyle.padding)
        if (focusIt) { sub.contentItem.forceActiveFocus(); sub.step(1) }
    }
    function step(dir) {
        current = (current < 0 ? (dir > 0 ? -1 : 0) : current) + dir
        current = (current + rowCount) % rowCount
    }

    ActionMenu {
        id: sub
        parent: menu.parent
        onTriggered: {
            var g = menu.groups[menu.openGroup]
            if (g) menu.activateItem(g.items[parseInt(key)])
        }
        onClosed: if (menu.visible && menu.openGroup >= 0) { menu.openGroup = -1; menu.contentItem.forceActiveFocus() }
    }

    // ── Open / close animation ──
    enter: Transition {
        ParallelAnimation {
            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 120; easing.type: Easing.OutCubic }
            NumberAnimation { property: "scale"; from: 0.96; to: 1; duration: 140; easing.type: Easing.OutCubic }
        }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; from: 1; to: 0; duration: 90 }
    }
    transformOrigin: Popup.TopLeft

    // ── Card with a soft layered shadow (no shader effects) ──
    background: Item {
        Repeater {
            model: 6
            Rectangle {
                anchors.fill: parent
                anchors.margins: -(index + 1) * 2
                anchors.topMargin: -(index + 1) * 2 + 4
                radius: MenuStyle.radius + (index + 1) * 2
                color: "transparent"
                border.width: 2
                border.color: Qt.rgba(0, 0, 0, 0.10 - index * 0.015)
            }
        }
        Rectangle {
            anchors.fill: parent
            radius: MenuStyle.radius
            color: MenuStyle.background
            border.color: MenuStyle.border
        }
    }

    contentItem: Column {
        spacing: 0
        focus: true

        Keys.onUpPressed: menu.step(-1)
        Keys.onDownPressed: menu.step(1)
        Keys.onRightPressed: menu.openSubmenu(menu.current, true)
        Keys.onReturnPressed: menu.current === menu.groups.length ? menu.activateItem(menu.settingsItem) : menu.openSubmenu(menu.current, true)
        Keys.onEnterPressed: menu.current === menu.groups.length ? menu.activateItem(menu.settingsItem) : menu.openSubmenu(menu.current, true)

        // Device summary: one line; the serial number is in the tooltip
        Item {
            width: parent.width
            height: MenuStyle.rowHeight + 2
            Rectangle {
                id: dot
                x: MenuStyle.sideInset + 3
                anchors.verticalCenter: parent.verticalCenter
                width: 8; height: 8; radius: 4
                color: bridge.connected ? (bridge.compat >= 2 ? "#ff9f0a" : "#32d74b") : "#ff453a"
            }
            Text {
                anchors.left: dot.right
                anchors.leftMargin: 10
                anchors.right: parent.right
                anchors.rightMargin: MenuStyle.sideInset
                anchors.verticalCenter: parent.verticalCenter
                elide: Text.ElideRight
                textFormat: Text.StyledText
                text: bridge.connected
                      ? (bridge.firmwareVersion ? qsTr("<b>DSPi %1</b><font color='#8c8c90'>  ·  %2</font>").arg(bridge.platformName).arg(bridge.firmwareVersion)
                                                : qsTr("<b>DSPi %1</b>").arg(bridge.platformName))
                      : qsTr("No device connected")
                font.pixelSize: MenuStyle.fontSize - 1
                color: "white"
            }
            MouseArea {
                id: deviceMouse
                anchors.fill: parent
                hoverEnabled: true
                acceptedButtons: Qt.NoButton
                onEntered: { sub.close(); menu.current = -1 }
            }
            ToolTip.visible: deviceMouse.containsMouse && bridge.connected
            ToolTip.delay: 500
            ToolTip.text: qsTr("Serial %1").arg(bridge.selectedSerial)
        }
        Rectangle { width: parent.width; height: 1; color: isMacOS ? Qt.rgba(1, 1, 1, 0.15) : MenuStyle.separator }
        Item { width: 1; height: MenuStyle.padding }

        Repeater {
            id: groupRows
            model: menu.groups
            Item {
                id: row
                width: parent.width
                height: MenuStyle.rowHeight
                readonly property bool usable: menu.groupEnabled(modelData)
                readonly property bool hot: (menu.current === index || menu.openGroup === index) && usable
                Rectangle {
                    anchors.fill: parent
                    anchors.leftMargin: 2
                    anchors.rightMargin: 2
                    radius: MenuStyle.rowRadius
                    color: row.hot ? MenuStyle.highlight : "transparent"
                }
                Icon {
                    id: groupIcon
                    x: MenuStyle.sideInset
                    anchors.verticalCenter: parent.verticalCenter
                    name: modelData.icon
                    size: MenuStyle.iconSize
                    color: isMacOS ? (row.hot ? "white" : row.usable ? MenuStyle.iconColor : MacColors.tertiaryLabel) : row.hot ? "white" : row.usable ? MenuStyle.iconColor : Qt.rgba(1, 1, 1, 0.25)
                }
                Text {
                    anchors.left: groupIcon.right
                    anchors.leftMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    text: modelData.text
                    font.pixelSize: MenuStyle.fontSize
                    color: isMacOS ? (!row.usable ? MacColors.tertiaryLabel : row.hot ? "white" : MenuStyle.text) : !row.usable ? Qt.rgba(1, 1, 1, 0.3) : row.hot ? "white" : MenuStyle.text
                }
                Icon {
                    anchors.right: parent.right
                    anchors.rightMargin: MenuStyle.sideInset
                    anchors.verticalCenter: parent.verticalCenter
                    name: "chev-right"
                    size: 12
                    color: isMacOS ? (row.hot ? "white" : Qt.rgba(1, 1, 1, 0.75)) : row.hot ? "white" : MenuStyle.dimText
                }
                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    onEntered: menu.openSubmenu(index, false)
                    onClicked: menu.openSubmenu(index, false)
                    // -- WP QD22 -- The menu keeps the keys; a row is its item
                    Accessible.role: Accessible.ButtonMenu
                    Accessible.name: modelData.text
                    Accessible.onPressAction: menu.openSubmenu(index, true)
                }
            }
        }

        Item {
            width: parent.width
            height: 7
            Rectangle { anchors.verticalCenter: parent.verticalCenter; x: 8; width: parent.width - 16; height: 1; color: isMacOS ? Qt.rgba(1, 1, 1, 0.15) : MenuStyle.separator }
        }

        // Settings
        Item {
            id: settingsRow
            width: parent.width
            height: MenuStyle.rowHeight
            readonly property bool hot: menu.current === menu.groups.length
            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 2
                anchors.rightMargin: 2
                radius: MenuStyle.rowRadius
                color: settingsRow.hot ? MenuStyle.highlight : "transparent"
            }
            Icon {
                id: settingsIcon
                x: MenuStyle.sideInset
                anchors.verticalCenter: parent.verticalCenter
                name: "gear"
                size: MenuStyle.iconSize
                color: settingsRow.hot ? "white" : MenuStyle.iconColor
            }
            Text {
                anchors.left: settingsIcon.right
                anchors.leftMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Settings")
                font.pixelSize: MenuStyle.fontSize
                color: settingsRow.hot ? "white" : MenuStyle.text
            }
            Text {
                anchors.right: parent.right
                anchors.rightMargin: MenuStyle.sideInset
                anchors.verticalCenter: parent.verticalCenter
                text: MenuStyle.shortcutLabel("Ctrl+,")
                font.pixelSize: MenuStyle.smallFontSize
                color: isMacOS ? (settingsRow.hot ? "white" : MenuStyle.dimText) : settingsRow.hot ? Qt.rgba(1, 1, 1, 0.8) : MenuStyle.dimText
            }
            MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onEntered: { sub.close(); menu.openGroup = -1; menu.current = menu.groups.length }
                onClicked: menu.activateItem(menu.settingsItem)
                Accessible.role: Accessible.MenuItem
                Accessible.name: menu.settingsItem.text
                Accessible.onPressAction: menu.activateItem(menu.settingsItem)
            }
        }
    }
}
