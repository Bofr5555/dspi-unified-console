pragma Singleton
import QtQuick
import UConsole

// -- WP LAY5a -- Which effects the device has and which of them are on, for the
// Modern layout: the Effects tabs' dots, the signal chain and the Overview's
// chips. The ids are the Effects tabs' (PanelTable); the state and the writes
// are EffectsController's and ToolsController's, as in the effects' panels.
QtObject {
    // In the tabs' order: [{ tab, title, icon, action }]
    readonly property var effects: SectionTable.tabs("effects").filter(t => t.row !== null)
        .map(t => ({ tab: t.tab, title: t.title, icon: t.icon, action: t.row.action }))
    // Those the device has: made anew only when they change, not on every
    // reading of the device, so a list of them keeps its items and the focus
    readonly property string availableTabs: effects.filter(e => supported(e.tab)).map(e => e.tab).join(" ")
    readonly property var available: effects.filter(e => availableTabs.split(" ").indexOf(e.tab) >= 0)
    readonly property int count: effects.filter(e => on(e.tab) === true).length

    function supported(tab) {
        if (!DeviceModel.connected || EffectsController.revision < 0 || ToolsController.revision < 0) return false
        switch (tab) {
        case "crossfeed": case "loudness": case "leveller": return true
        case "psybass": return EffectsController.psybassSupported
        case "subharmonic": return ToolsController.subharmSupported
        case "tube": return ToolsController.tubeSupported
        case "upmixer": return ToolsController.upmixSupported
        }
        return false
    }
    // null: the device does not have it, or there is no device
    function on(tab) {
        if (!supported(tab)) return null
        switch (tab) {
        case "crossfeed": return EffectsController.crossfeedOn
        case "loudness": return EffectsController.loudnessOn
        case "leveller": return EffectsController.levellerOn
        case "psybass": return EffectsController.psybassOn
        case "subharmonic": return ToolsController.subharm("enabled") !== 0
        case "tube": return ToolsController.tube("enabled") !== 0
        case "upmixer": return ToolsController.upmix("enabled") !== 0
        }
        return null
    }
    // As the switch in the effect's panel
    function setOn(tab, value) {
        if (!supported(tab)) return
        if (tab === "subharmonic") ToolsController.setSubharm("enabled", value ? 1 : 0)
        else if (tab === "tube") ToolsController.setTube("enabled", value ? 1 : 0)
        else if (tab === "upmixer") ToolsController.setUpmix("enabled", value ? 1 : 0)
        else EffectsController.setEnabled(tab, value)
    }
}
