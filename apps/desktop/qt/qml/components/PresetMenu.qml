import QtQuick 2.15
import QtQuick.Controls 2.15

// Right-click menu of the preset picker (as on the macOS Console): Save,
// Rename, Set as Default, Copy to, Clear the slot, Clear All Slots. Acts on
// the active slot. openAt(item, x, y) opens it at a point in `item`; the
// Copy to slot list opens under `anchorItem`.
Item {
    id: pm
    property Item anchorItem

    readonly property int slot: bridge.activePresetSlot

    function slotName(i) {
        if (!bridge.isPresetOccupied(i)) return qsTr("Empty")
        var n = bridge.presetName(i)
        return n === "" ? qsTr("Preset %1").arg(i + 1) : n
    }
    function slotLabel(i) { return (i + 1) + ": " + slotName(i) }
    // Saving an unnamed slot gives it its default name, so the picker
    // doesn't keep showing it as "Empty"
    function ensureName(i) {
        if (bridge.presetName(i) === "") bridge.setPresetName(i, "Preset " + (i + 1))
    }
    function report(status, what) {
        if (status === 0) return
        errorDialog.title = what
        errorDialog.message = qsTr("The device reported error %1.").arg(status)
        errorDialog.open()
    }

    function openAt(item, x, y) {
        var anyOccupied = bridge.presetOccupied !== 0
        var items = [
            { key: "save", text: qsTr("Save"), icon: "save" },
            { key: "rename", text: qsTr("Rename…"), icon: "pencil" },
            { key: "default", text: qsTr("Set as Default"),
              enabled: !(bridge.presetStartupMode === 0 && bridge.presetDefaultSlot === slot) },
            { key: "copy", text: qsTr("Copy to…"), icon: "copy" }
        ]
        if (bridge.isPresetOccupied(slot))
            items.push({ separator: true },
                       { key: "clear", text: qsTr("Clear “%1”…").arg(slotLabel(slot)), danger: true })
        if (anyOccupied)
            items.push({ separator: true },
                       { key: "clearAll", text: qsTr("Clear All Slots…"), danger: true })
        menu.items = items
        menu.openAt(item, x, y)
    }

    ActionMenu {
        id: menu
        parent: Overlay.overlay
        onTriggered: {
            if (key === "save") {
                pm.ensureName(pm.slot)
                pm.report(bridge.savePreset(pm.slot), qsTr("Save Failed"))
            } else if (key === "rename") {
                renameDialog.renameSlot = pm.slot
                renameDialog.inputText = pm.slotName(pm.slot) === qsTr("Empty") ? "" : pm.slotName(pm.slot)
                renameDialog.open()
            } else if (key === "default") {
                bridge.setPresetStartup(0, pm.slot)
            } else if (key === "copy") {
                var o = []
                for (var i = 0; i < 10; i++)
                    if (i !== pm.slot)
                        o.push({ value: i, prefix: String(i + 1), text: pm.slotName(i) })
                copyMenu.options = o
                copyMenu.openAt(pm.anchorItem)
            } else if (key === "clear") {
                clearDialog.clearSlot = pm.slot
                clearDialog.open()
            } else if (key === "clearAll") {
                clearAllDialog.open()
            }
        }
    }

    ChoiceMenu {
        id: copyMenu
        parent: Overlay.overlay
        alignRight: true
        currentValue: -1
        onChosen: {
            if (bridge.isPresetOccupied(value)) {
                replaceDialog.destSlot = value
                replaceDialog.open()
            } else pm.copyTo(value)
        }
    }

    // Copies the current settings (saving them to the active slot as well),
    // after asking about unsaved changes: Save keeps them, Discard reloads
    // the active slot first
    function copyTo(dest) {
        var src = slot
        root.withUnsaved(function (choice) {
            if (choice === "discard") bridge.loadPreset(src)
            ensureName(src)
            ensureName(dest)
            report(bridge.copyPreset(src, dest), qsTr("Copy Failed"))
        })
    }

    AppDialog {
        id: renameDialog
        property int renameSlot: 0
        icon: "pencil"
        title: qsTr("Rename Preset %1").arg(renameSlot + 1)
        hasInput: true
        placeholder: qsTr("Preset %1").arg(renameSlot + 1)
        buttons: [
            { key: "cancel", text: qsTr("Cancel") },
            { key: "rename", text: qsTr("Rename"), role: "primary" }
        ]
        onChosen: {
            var name = inputText.trim()
            if (key === "rename" && name !== "") bridge.setPresetName(renameSlot, name)
        }
    }

    AppDialog {
        id: replaceDialog
        property int destSlot: 0
        icon: "copy"
        title: qsTr("Replace “%1”?").arg(pm.slotLabel(destSlot))
        message: qsTr("Its settings will be replaced with the current ones. Its name stays the same.")
        buttons: [
            { key: "cancel", text: qsTr("Cancel") },
            { key: "replace", text: qsTr("Replace"), role: "destructive" }
        ]
        onChosen: if (key === "replace") pm.copyTo(destSlot)
    }

    AppDialog {
        id: clearDialog
        property int clearSlot: 0
        icon: "warning"
        iconTint: "#ff6961"
        title: qsTr("Clear Preset?")
        message: qsTr("Clear “%1” and restore factory defaults? This cannot be undone.").arg(pm.slotLabel(clearSlot))
        buttons: [
            { key: "cancel", text: qsTr("Cancel") },
            { key: "clear", text: qsTr("Clear"), role: "destructive" }
        ]
        onChosen: if (key === "clear") pm.report(bridge.deletePreset(clearSlot), qsTr("Clear Failed"))
    }

    AppDialog {
        id: clearAllDialog
        icon: "warning"
        iconTint: "#ff6961"
        title: qsTr("Clear All Presets?")
        message: qsTr("This erases all preset data and names, restoring every slot to factory defaults. This cannot be undone.")
        buttons: [
            { key: "cancel", text: qsTr("Cancel") },
            { key: "clearAll", text: qsTr("Clear All"), role: "destructive" }
        ]
        onChosen: {
            if (key !== "clearAll") return
            for (var i = 0; i < 10; i++) {
                if (!bridge.isPresetOccupied(i)) continue
                var status = bridge.deletePreset(i)
                if (status !== 0) { pm.report(status, qsTr("Clear Failed")); return }
            }
        }
    }

    AppDialog {
        id: errorDialog
        icon: "warning"
        iconTint: "#ff9f0a"
        buttons: [{ key: "ok", text: qsTr("OK"), role: "primary" }]
    }
}
