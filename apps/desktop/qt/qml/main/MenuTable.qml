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
// enabled (optional: a function, bound to the row's and shortcut's enabled) }.
// { separator: true } is a line; { submenu: "<title>", rows: [...] } nests.
// Work packages add rows under their own marker; ids are "<menu>.<name>".
QtObject {
    readonly property var menus: [
        {
            title: qsTr("&File"),
            rows: [
                { id: "file.importFilters", text: qsTr("Import Filters…"), shortcut: "Ctrl+I" },
                { id: "file.exportFilters", text: qsTr("Export Filters…"), shortcut: "Ctrl+E" },
                { separator: true },
                { id: "file.importConfiguration", text: qsTr("Import Device Configuration…") },
                { id: "file.exportConfiguration", text: qsTr("Export Device Configuration…") },
                // -- WP P1QT1 -- offline documents and templates (FilesController, P1OF)
                { separator: true },
                { id: "file.openFile", text: qsTr("Open File…"), shortcut: "Ctrl+O" },
                { id: "file.newFromTemplate", text: qsTr("New from Template…") },
                { id: "file.saveFile", text: qsTr("Save File"), enabled: () => FilesController.offline },
                { id: "file.saveAsTemplate", text: qsTr("Save as Template…") },
                { separator: true },
                { id: "file.saveMasterVolume", text: qsTr("Save Master Volume") },
                { id: "file.saveOutputConfiguration", text: qsTr("Save Output Configuration") },
                { separator: true },
                { id: "file.quit", text: qsTr("Quit"), shortcut: "Ctrl+Q" }
            ]
        },
        {
            title: qsTr("&Edit"),
            rows: [
                // -- WP QP18 -- the core's change journal (HistoryController)
                { id: "edit.undo", text: qsTr("Undo"), shortcut: "Ctrl+Z", enabled: () => HistoryController.canUndo },
                { id: "edit.redo", text: qsTr("Redo"), shortcut: "Ctrl+Shift+Z", enabled: () => HistoryController.canRedo },
                { id: "edit.history", text: qsTr("History…"), shortcut: "Ctrl+Shift+H" },
                { separator: true },
                { id: "edit.cut", text: qsTr("Cut"), shortcut: "Ctrl+X" },
                { id: "edit.copy", text: qsTr("Copy"), shortcut: "Ctrl+C" },
                { id: "edit.paste", text: qsTr("Paste"), shortcut: "Ctrl+V" },
                { id: "edit.delete", text: qsTr("Delete") },
                { id: "edit.selectAll", text: qsTr("Select All"), shortcut: "Ctrl+A" },
                { separator: true },
                { id: "edit.settings", text: qsTr("Settings…"), shortcut: "Ctrl+," }
            ]
        },
        {
            title: qsTr("&Device"),
            rows: [
                // -- WP QP18 -- Commit and Revert get shortcuts (owner, 2026-11-12)
                { id: "device.commit", text: qsTr("Commit Parameters…"), shortcut: "Ctrl+S" },
                { id: "device.revert", text: qsTr("Revert to Saved…"), shortcut: "Ctrl+Shift+R" },
                { id: "device.factoryReset", text: qsTr("Factory Reset…") },
                { separator: true },
                // -- WP QP18 -- presets 1-9, mute and bypass from the keyboard (ShortcutController)
                {
                    submenu: qsTr("Recall Preset"),
                    rows: [1, 2, 3, 4, 5, 6, 7, 8, 9].map(n => ({
                        //: %1 is a preset slot's number
                        id: "device.preset" + n, text: qsTr("Preset %1").arg(n), shortcut: "Ctrl+" + n
                    }))
                },
                { id: "device.mute", text: qsTr("Mute"), shortcut: "Ctrl+M" },
                { id: "device.bypass", text: qsTr("Bypass Master EQ"), shortcut: "Ctrl+B" },
                // -- WP P1QT1 -- snapshots and the A/B key (SnapshotsController)
                { separator: true },
                { id: "device.snapshots", text: qsTr("Snapshots…") },
                // -- WP P1DCFIX -- a bare B (never while typing or while a dialog asks)
                { id: "device.abToggle", text: qsTr("Toggle A/B"), shortcut: "B", enabled: () => SnapshotsController.canToggle },
                { separator: true },
                { id: "device.firmwareUpdate", text: qsTr("Firmware Update…"), platformHidden: "STM32H723" },
                { separator: true },
                { id: "device.reconnect", text: qsTr("Reconnect") },
                { id: "device.devices", text: qsTr("Devices…") }
            ]
        },
        {
            title: qsTr("&AutoEQ"),
            rows: [
                { id: "autoeq.browse", text: qsTr("Browse Profiles…"), shortcut: "Ctrl+Shift+B" },
                {
                    submenu: qsTr("Favorite Profiles"),
                    rows: [
                        { id: "autoeq.clearFavorites", text: qsTr("Clear Favorites") }
                    ]
                },
                { separator: true },
                { id: "autoeq.updateDatabase", text: qsTr("Update Database…") }
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
                { separator: true },
                { id: "tools.interruptMonitor", text: qsTr("Interrupt Monitor…"), shortcut: "Ctrl+Shift+I" }
            ]
        },
        {
            title: qsTr("&Window"),
            rows: [
                { id: "window.minimiseToTray", text: qsTr("Minimise to Tray"), shortcut: "Ctrl+Alt+M" },
                { id: "window.popOutGraph", text: qsTr("Pop Out Graph") },
                // -- WP LKUI1 -- the hub's audio link installation (SystemController)
                { separator: true },
                { id: "window.system", text: qsTr("System…"), optional: true }
                // -- WP QP18 -- docs/UI-MODES.md §5 (ModeController)
                // -- WP QPI5 -- hidden until the modes come (owner, 2026-11-13);
                // ModeController and ModeWindow stay, and the rows come back as
                // { separator: true }, { id: "window.mode", text: qsTr("Mode…") }
            ]
        },
        {
            title: qsTr("&Help"),
            rows: [
                { id: "help.gettingStarted", text: qsTr("Getting Started…") },
                { separator: true },
                //: %1 is the app's name
                { id: "help.whatsNew", text: qsTr("What's New in %1").arg(AppIdentity.displayName) },
                { separator: true },
                //: %1 is the app's name
                { id: "help.projectGithub", text: qsTr("%1 on GitHub").arg(AppIdentity.displayName) },
                { id: "help.firmwareGithub", text: qsTr("DSPi Firmware on GitHub") },
                { separator: true },
                // -- WP QP17 -- shown once the core can write the bundle (PRO1 E2)
                { id: "help.saveDiagnostics", text: qsTr("Save Diagnostics…"), optional: true },
                { id: "help.checkForUpdates", text: qsTr("Check for Updates…"), optional: true },
                //: %1 is the app's name
                { id: "help.about", text: qsTr("About %1").arg(AppIdentity.displayName) }
            ]
        }
    ]

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
