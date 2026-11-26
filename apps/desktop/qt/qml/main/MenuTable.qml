pragma Singleton
import QtQuick
import UConsole

// Every menu of the main window and its rows, in order. A row triggers its
// action id through Shell.trigger(); whoever handles the id (a controller's
// handler or a registered tool window) does the work, and until then the row
// does nothing. The wording and the shortcuts are the Mac console's, with Ctrl
// for Command and Alt for Option.
//
// A row: { id, text, shortcut (optional), optional (shown only while
// Shell.handles(id)), platformHidden (a platform name it is hidden on),
// enabled (optional: a function, bound to the row's and shortcut's enabled),
// modern (optional: its places in the Modern layout, see `places`) }.
// { separator: true } is a line; { submenu: "<title>", rows: [...] } nests.
// Work packages add rows under their own marker; ids are "<menu>.<name>".
QtObject {
    readonly property var menus: [
        {
            title: qsTr("&File"),
            rows: [
                { id: "file.importFilters", text: qsTr("Import Filters…"), shortcut: "Ctrl+I", modern: ["files.filters", "appMenu.transfer"] },   // -- INTC -- Files ▾ (was toolbar.eq)
                { id: "file.exportFilters", text: qsTr("Export Filters…"), shortcut: "Ctrl+E", modern: ["files.filters", "appMenu.transfer"] },   // -- INTC -- Files ▾ (was toolbar.eq)
                { separator: true },
                { id: "file.importConfiguration", text: qsTr("Import Device Configuration…"), modern: ["appMenu.transfer"] },
                { id: "file.exportConfiguration", text: qsTr("Export Device Configuration…"), modern: ["appMenu.transfer"] },
                // -- WP P1QT1 -- offline documents and templates (FilesController, P1OF)
                { separator: true },
                { id: "file.openFile", text: qsTr("Open File…"), shortcut: "Ctrl+O", modern: ["appMenu.file"] },
                { id: "file.newFromTemplate", text: qsTr("New from Template…"), modern: ["appMenu.file"] },
                { id: "file.saveFile", text: qsTr("Save File"), enabled: () => FilesController.offline, modern: ["appMenu.file"] },
                { id: "file.saveAsTemplate", text: qsTr("Save as Template…"), modern: ["appMenu.file"] },
                { separator: true },
                { id: "file.saveMasterVolume", text: qsTr("Save Master Volume"), modern: ["save.store"] },
                { id: "file.saveOutputConfiguration", text: qsTr("Save Output Configuration"), modern: ["save.store"] },
                { separator: true },
                { id: "file.quit", text: qsTr("Quit"), shortcut: "Ctrl+Q", modern: ["appMenu.quit"] }
            ]
        },
        {
            title: qsTr("&Edit"),
            rows: [
                // -- WP QP18 -- the core's change journal (HistoryController)
                { id: "edit.undo", text: qsTr("Undo"), shortcut: "Ctrl+Z", enabled: () => HistoryController.canUndo, modern: ["topBar", "history.step"] },
                { id: "edit.redo", text: qsTr("Redo"), shortcut: "Ctrl+Shift+Z", enabled: () => HistoryController.canRedo, modern: ["topBar", "history.step"] },
                { id: "edit.history", text: qsTr("History…"), shortcut: "Ctrl+Shift+H", modern: ["history.all"] },
                { separator: true },
                { id: "edit.cut", text: qsTr("Cut"), shortcut: "Ctrl+X", modern: ["keyboard"] },
                { id: "edit.copy", text: qsTr("Copy"), shortcut: "Ctrl+C", modern: ["keyboard"] },
                { id: "edit.paste", text: qsTr("Paste"), shortcut: "Ctrl+V", modern: ["keyboard"] },
                { id: "edit.delete", text: qsTr("Delete"), modern: ["keyboard"] },
                { id: "edit.selectAll", text: qsTr("Select All"), shortcut: "Ctrl+A", modern: ["keyboard"] },
                { separator: true },
                { id: "edit.settings", text: qsTr("Settings…"), shortcut: "Ctrl+,", modern: ["appMenu.settings"] }
            ]
        },
        {
            title: qsTr("&Device"),
            rows: [
                // -- WP QP18 -- Commit and Revert get shortcuts (owner, 2026-11-12)
                // -- WP UFW -- (UXP A10, D1) one word on every layout and client:
                // Save to Device, as the top bar's button; nothing to save
                // without a device. Factory Reset is the device's (its tab's
                // menu), not one slip away from Save (UXP A3, C1)
                { id: "device.commit", text: qsTr("Save to Device…"), shortcut: "Ctrl+S", enabled: () => DeviceModel.connected, modern: ["topBar"] },
                { id: "device.revert", text: qsTr("Revert to Saved…"), shortcut: "Ctrl+Shift+R", modern: ["save.revert"] },
                { id: "device.factoryReset", text: qsTr("Factory Reset…"), modern: ["device.reset"] },
                { separator: true },
                // -- WP QP18 -- presets 1-9, mute and bypass from the keyboard (ShortcutController)
                {
                    submenu: qsTr("Recall Preset"),
                    rows: [1, 2, 3, 4, 5, 6, 7, 8, 9].map(n => ({
                        //: %1 is a preset slot's number
                        id: "device.preset" + n, text: qsTr("Preset %1").arg(n), shortcut: "Ctrl+" + n, modern: ["preset"]
                    }))
                },
                { id: "device.mute", text: qsTr("Mute"), shortcut: "Ctrl+M", modern: ["dock"] },
                { id: "device.bypass", text: qsTr("Bypass Master EQ"), shortcut: "Ctrl+B", modern: ["dock"] },
                // -- WP P1QT1 -- snapshots and the A/B key (SnapshotsController)
                { separator: true },
                { id: "device.snapshots", text: qsTr("Snapshots…") },
                // -- WP P1DCFIX -- a bare B (never while typing or while a dialog asks)
                { id: "device.abToggle", text: qsTr("Toggle A/B"), shortcut: "B", enabled: () => SnapshotsController.canToggle, modern: ["topBar"] },
                // -- WP P2QT -- the device's events (this console's and the hub's) and Identify
                { id: "device.events", text: qsTr("Events…") },
                { id: "device.identify", text: qsTr("Identify"), enabled: () => IdentifyController.available, modern: ["device.act"] },
                // -- WP MCP4 -- what AI agents do, with Pause and Stop (AiController)
                { id: "device.aiAgents", text: qsTr("AI Agents…") },
                { separator: true },
                { id: "device.firmwareUpdate", text: qsTr("Firmware Update…"), platformHidden: "STM32H723", modern: ["device.act"] },
                { separator: true },
                { id: "device.reconnect", text: qsTr("Reconnect"), modern: ["device.act"] },
                { id: "device.devices", text: qsTr("Devices…"), modern: ["topBar"] }
            ]
        },
        {
            title: qsTr("&AutoEQ"),
            rows: [
                { id: "autoeq.browse", text: qsTr("Browse Profiles…"), shortcut: "Ctrl+Shift+B", modern: ["toolbar.eq"] },
                {
                    submenu: qsTr("Favorite Profiles"),
                    rows: [
                        { id: "autoeq.clearFavorites", text: qsTr("Clear Favorites"), modern: ["autoeq.favorites"] }
                    ]
                },
                { separator: true },
                { id: "autoeq.updateDatabase", text: qsTr("Update Database…"), modern: ["autoeq.database"] }
            ]
        },
        {
            title: qsTr("&Tools"),
            rows: [
                { id: "tools.matrix", text: qsTr("Matrix Mixer…"), shortcut: "Ctrl+Shift+M" },
                { id: "tools.loudness", text: qsTr("Loudness Compensation…"), shortcut: "Ctrl+Shift+L" },
                { id: "tools.crossfeed", text: qsTr("Headphone Crossfeed…"), shortcut: "Ctrl+Shift+X" },
                { id: "tools.psybass", text: qsTr("Psychoacoustic Bass…"), shortcut: "Ctrl+Shift+P" },
                { id: "tools.subharmonic", text: qsTr("Subharmonic Synthesizer…"), shortcut: "Ctrl+Shift+S" },
                { id: "tools.tube", text: qsTr("Tube Modeller…"), shortcut: "Ctrl+Shift+D" },
                { id: "tools.upmixer", text: qsTr("Stereo Upmixer…"), shortcut: "Ctrl+Shift+U" },
                { id: "tools.leveller", text: qsTr("Volume Leveller…"), shortcut: "Ctrl+Shift+V" },
                { id: "tools.signalGenerator", text: qsTr("Signal Generator…"), shortcut: "Ctrl+Shift+G" },
                { id: "tools.spectrum", text: qsTr("Spectrum Analyser…"), shortcut: "Ctrl+Shift+A" },
                { id: "tools.stats", text: qsTr("Stats for nerbs"), shortcut: "Ctrl+Shift+T" },
                // -- WP P5QUI -- the measurement, live view and auto-tune (MeasureController)
                { id: "tools.measure", text: qsTr("Measure…") },
                { separator: true },
                { id: "tools.interruptMonitor", text: qsTr("Interrupt Monitor…"), shortcut: "Ctrl+Shift+I" }
            ]
        },
        {
            title: qsTr("&Window"),
            rows: [
                { id: "window.minimiseToTray", text: qsTr("Minimise to Tray"), shortcut: "Ctrl+Alt+M", modern: ["appMenu.end"] },
                { id: "window.popOutGraph", text: qsTr("Pop Out Graph"), modern: ["graph.view"] },
                // -- WP LKUI1 -- the hub's audio link installation (SystemController)
                { separator: true },
                { id: "window.system", text: qsTr("System…"), optional: true },
                // -- WP UB2 -- (UXP B2, D4) the modes have come: Simple is Home, a remote, first
                { separator: true },
                { id: "window.mode", text: qsTr("Mode…"), modern: ["appMenu.settings"] }
                // -- WP QP18 -- docs/UI-MODES.md §5 (ModeController)
                // -- WP QPI5 -- hidden until the modes come (owner, 2026-11-13);
                // ModeController and ModeWindow stay, and the rows come back as
                // { separator: true }, { id: "window.mode", text: qsTr("Mode…") }
            ]
        },
        {
            title: qsTr("&Help"),
            rows: [
                { id: "help.gettingStarted", text: qsTr("Getting Started…"), modern: ["appMenu.help"] },
                // -- WP UFW -- (UXP A8) the manual, and the shortcut list (its
                // dialog is UXP A5's: the row shows once it handles the id)
                { id: "help.manual", text: qsTr("User Manual"), modern: ["appMenu.help"] },
                { id: "help.shortcuts", text: qsTr("Keyboard Shortcuts"), optional: true, modern: ["appMenu.help"] },
                { separator: true },
                //: %1 is the app's name
                { id: "help.whatsNew", text: qsTr("What's New in %1").arg(AppIdentity.displayName), modern: ["appMenu.help"] },
                { separator: true },
                //: %1 is the app's name
                { id: "help.projectGithub", text: qsTr("%1 on GitHub").arg(AppIdentity.displayName), modern: ["appMenu.help"] },
                { id: "help.firmwareGithub", text: qsTr("DSPi Firmware on GitHub"), modern: ["appMenu.help"] },
                { separator: true },
                // -- WP QP17 -- shown once the core can write the bundle (PRO1 E2)
                { id: "help.saveDiagnostics", text: qsTr("Save Diagnostics…"), optional: true, modern: ["appMenu.help"] },
                { id: "help.checkForUpdates", text: qsTr("Check for Updates…"), optional: true, modern: ["appMenu.help"] },
                //: %1 is the app's name
                { id: "help.about", text: qsTr("About %1").arg(AppIdentity.displayName), modern: ["appMenu.help"] }
            ]
        }
    ]

    // ---- -- WP LAY3b -- Where the actions are in the Modern layout, which has no
    // menu bar (MODLAY 3.1): they act where they are. A row's `modern` lists its
    // places. A row whose tool is a panel (PanelTable) has its page besides, and
    // needs no `modern`; every other row needs one (the smoke section
    // lay3b-actions checks it). assets/actions.json is made from this table.
    //
    // A place is "<place>" or "<place>.<group>". Those listed here are menus,
    // which show their groups in this order, with a line between two:
    //   appMenu  the app button's menu: the few global actions
    //   save     Save to Device ▾
    //   history  Undo ▾
    //   device   the device switcher and a device tab's right-click menu (the
    //            switcher and Devices › This Computer show its first group)
    //   autoeq   Library > AutoEQ's ▾
    //   files    the Equaliser's Files ▾ (-- INTC (DL8 fold) --)
    //   channel, band, graph, slot   the right-click menu of a channel, a band
    //            (its row or its dot), the graph and a preset slot
    readonly property var places: ({
        appMenu: ["file", "transfer", "settings", "help", "end", "quit"],
        save: ["revert", "store", "reset"],
        history: ["step", "recent", "all"],
        device: ["act", "reset", "tab"],   // -- WP UFW -- reset: Factory Reset, under a line
        autoeq: ["favorites", "database"],
        files: ["filters"],   // -- INTC --
        channel: ["name", "copy", "act", "clear"],
        band: ["shape", "play", "copy", "reset"],
        graph: ["add", "select", "view"],
        slot: ["use", "name", "copy", "clear"]
    })
    // The others are no menus:
    //   topBar, dock  a control there     preset  the preset picker
    //   toolbar.<page>  a button in the row over that page ("eq": the Equaliser)
    //   keyboard  text editing, by its shortcut in a text field
    //   palette   the command palette only
    readonly property var otherPlaces: ["topBar", "dock", "preset", "toolbar.eq", "keyboard", "palette"]

    // The actions on one thing (a channel, a band, ...), which the menu bar does
    // not have; ActionMenu's handlers run them. `of`: the menu row it is there
    // (its shortcut and, without a handler of its own, its work are that row's);
    // `on`: the text while what it switches is on; `submenu`, `dynamic`: rows
    // the handler gives; `sender`: it works on an alink sender too.
    readonly property var itemRows: [
        { id: "app.classicLayout", text: qsTr("Switch to the Classic Layout"), sender: true, modern: ["appMenu.settings"] },

        { id: "history.recent", text: qsTr("Recent Changes"), dynamic: true, modern: ["history.recent"] },

        { id: "device.close", text: qsTr("Close"), sender: true, modern: ["device.tab"] },

        { id: "channel.rename", text: qsTr("Rename"), modern: ["channel.name"] },
        { id: "channel.curve", text: qsTr("Show Curve"), on: qsTr("Hide Curve"), modern: ["channel.name"] },
        { id: "channel.copy", of: "edit.copy", text: qsTr("Copy Parameters"), modern: ["channel.copy"] },
        { id: "channel.paste", of: "edit.paste", text: qsTr("Paste Parameters"), modern: ["channel.copy"] },
        { id: "channel.identify", text: qsTr("Identify"), modern: ["channel.act"] },
        { id: "channel.mute", text: qsTr("Mute"), on: qsTr("Unmute"), modern: ["channel.act"] },
        { id: "channel.routing", of: "tools.matrix", text: qsTr("Edit Routing…"), modern: ["channel.act"] },
        { id: "channel.headphone", of: "autoeq.browse", text: qsTr("Load Headphone Profile…"), modern: ["channel.act"] },
        { id: "channel.clear", text: qsTr("Clear PEQ…"), modern: ["channel.clear"] },

        { id: "band.shape", text: qsTr("Shape"), submenu: true, modern: ["band.shape"] },
        { id: "band.slope", text: qsTr("Slope"), submenu: true, modern: ["band.shape"] },
        { id: "band.bypass", text: qsTr("Bypass"), on: qsTr("Enable"), modern: ["band.play"] },
        { id: "band.invert", text: qsTr("Invert Gain"), modern: ["band.play"] },
        { id: "band.copy", text: qsTr("Copy Band"), modern: ["band.copy"] },
        { id: "band.paste", text: qsTr("Paste Band"), modern: ["band.copy"] },
        { id: "band.reset", text: qsTr("Reset Band"), modern: ["band.reset"] },

        { id: "graph.addBand", text: qsTr("Add a Band Here"), modern: ["graph.add"] },
        { id: "graph.selectAll", of: "edit.selectAll", text: qsTr("Select All Bands"), modern: ["graph.select"] },
        { id: "graph.deselect", text: qsTr("Deselect All"), modern: ["graph.select"] },
        { id: "graph.delete", of: "edit.delete", text: qsTr("Delete Selected Bands"), modern: ["graph.select"] },
        { id: "graph.rta", text: qsTr("Show the Spectrum"), on: qsTr("Hide the Spectrum"), modern: ["graph.view"] },
        { id: "graph.phase", text: qsTr("Show the Phase"), on: qsTr("Hide the Phase"), modern: ["graph.view"] },
        { id: "graph.settings", text: qsTr("Graph Settings…"), modern: ["graph.view"] },

        { id: "slot.recall", text: qsTr("Recall"), modern: ["slot.use"] },
        { id: "slot.save", text: qsTr("Save Current Settings Here"), modern: ["slot.use"] },
        { id: "slot.rename", text: qsTr("Rename…"), modern: ["slot.name"] },
        { id: "slot.startup", text: qsTr("Set as Startup Preset"), modern: ["slot.name"] },
        { id: "slot.copyTo", text: qsTr("Copy to…"), submenu: true, modern: ["slot.copy"] },
        { id: "slot.clear", text: qsTr("Clear…"), modern: ["slot.clear"] },
        { id: "slot.clearAll", text: qsTr("Clear All Slots…"), modern: ["slot.clear"] }
    ]

    // Every row of the menus, flat: the row with its menu's title and its
    // submenu's (`menu`, `submenu`)
    function flatRows() {
        const rows = []
        for (const m of menus) {
            const walk = (list, submenu) => {
                for (const r of list) {
                    if (r.rows) walk(r.rows, r.submenu)
                    else if (r.id) rows.push(Object.assign({ menu: m.title.replace("&", ""), submenuTitle: submenu }, r))
                }
            }
            walk(m.rows, "")
        }
        return rows
    }
    // The row `id`, of the menus or of one thing, or null
    function row(id) { return flatRows().find(r => r.id === id) || itemRows.find(r => r.id === id) || null }
    // The rows of the menu at `place`, by group: [[row, ...], ...], none empty
    function placeRows(place) {
        const all = flatRows().concat(itemRows)
        return (places[place] || []).map(g => all.filter(r => (r.modern || []).indexOf(place + "." + g) >= 0)).filter(g => g.length > 0)
    }
    // Where the Modern layout has row `id`: its places, "page.<section>/<tab>" for
    // a panel's, and the places of the rows that are it on one thing (`of`)
    function homes(id) {
        const r = row(id)
        if (!r) return []
        let out = (r.modern || []).slice()
        const panel = PanelTable.row(id)
        if (panel) out.push("page." + panel.section + "/" + panel.tab)
        for (const i of itemRows) if (i.of === id) out = out.concat(i.modern)
        return out
    }
    // Whether row `id` does something now (a menu greys it otherwise)
    function can(id) {
        const r = row(id)
        if (!r) return false
        return (r.sender || available(r.of || id)) && (!r.enabled || r.enabled()) && (!r.platformHidden || DeviceModel.platformName !== r.platformHidden)
    }
    // The table for the other clients (assets/actions.json, which
    // tools/actions.sh writes): the app's name is the placeholder {app}
    function exported() {
        const named = t => t.replace(AppIdentity.displayName, "{app}")
        const actions = flatRows().map(r => {
            const a = { id: r.id, text: named(r.text), menu: r.menu }
            if (r.submenuTitle) a.submenu = r.submenuTitle
            if (r.shortcut) a.shortcut = r.shortcut
            if (r.optional) a.optional = true
            if (r.platformHidden) a.hiddenOn = r.platformHidden
            a.places = homes(r.id)
            return a
        })
        const items = itemRows.map(r => {
            const a = { id: r.id, text: r.text }
            if (r.on) a.textOn = r.on
            if (r.of) a.of = r.of
            if (r.submenu) a.rows = "submenu"
            if (r.dynamic) a.rows = "dynamic"
            a.places = r.modern
            return a
        })
        const menusOf = {}
        for (const p in places) menusOf[p] = placeRows(p).map(g => g.map(r => r.id))
        return JSON.stringify({ version: 1, actions: actions, itemActions: items, menus: menusOf }, null, 2) + "\n"
    }

    // The rows with a shortcut, as [{ id, shortcut }]: TableShortcuts gives
    // them to every tool window, so a shortcut works whichever window is active
    function shortcutRows() {
        const rows = []
        const walk = list => {
            for (const r of list) {
                if (r.rows) walk(r.rows)
                else if (r.id && r.shortcut) rows.push({ id: r.id, shortcut: r.shortcut, enabled: r.enabled })
            }
        }
        for (const m of menus) walk(m.rows)
        return rows
    }

    // Edit's rows act on the focused text field first, as the Mac sends them
    // to the first responder; otherwise they go to whoever handles the id (the
    // channel clipboard). Every other id goes to the shell.
    readonly property var textEdits: ({
        "edit.cut": "cut",
        "edit.copy": "copy",
        "edit.paste": "paste",
        "edit.selectAll": "selectAll",
        "edit.delete": "delete"
    })
    // -- WP QTSENDER -- Whether row `id` does something on the open device: on
    // the alink sender only DeviceInfoController.senderKeeps' rows and the
    // Edit rows that act on a focused text field. Menus and TableShortcuts
    // bind a row's enabled to it (greyed, not hidden); Shell refuses the rest.
    function available(id) {
        return !DeviceInfoController.sender || textEdits[id] !== undefined || DeviceInfoController.senderKeeps(id)
    }

    function trigger(window, id) {
        const target = window ? window.activeFocusItem : null
        // -- WP QP18 -- Undo and Redo: the focused field's own typing first
        if (target && ((id === "edit.undo" && target.canUndo) || (id === "edit.redo" && target.canRedo))) {
            if (id === "edit.undo") target.undo()
            else target.redo()
            return
        }
        const op = textEdits[id]
        if (op && target && target.selectionStart !== undefined && typeof target.selectAll === "function") {
            if (op === "delete") target.remove(target.selectionStart, target.selectionEnd)
            else target[op]()
            return
        }
        Shell.trigger(id)
    }

    // -- WP QP18 -- A row's text by its id, or a submenu's title by its first
    // row's id; Menus binds to it, so a new translation reaches the menus
    function textOf(id, submenu) {
        let found = ""
        const walk = rows => {
            for (const r of rows) {
                if (found) return
                if (r.rows) {
                    if (submenu && r.rows.length && r.rows[0].id === id) found = r.submenu
                    else walk(r.rows)
                } else if (!submenu && r.id === id) {
                    found = r.text
                }
            }
        }
        for (const m of menus) walk(m.rows)
        return found
    }

    // -- WP LAY3 -- A row's shortcut by its id, "" for none (menus that only show it)
    function shortcutOf(id) {
        const row = shortcutRows().find(r => r.id === id)
        return row ? row.shortcut : ""
    }

    // Every action id in the table, in menu order (the smoke driver checks them)
    function allIds() {
        const ids = []
        const walk = rows => {
            for (const r of rows) {
                if (r.rows) walk(r.rows)
                else if (r.id) ids.push(r.id)
            }
        }
        for (const m of menus) walk(m.rows)
        return ids
    }
}
