pragma Singleton
import QtQuick
import UConsole

// -- WP LAY3 -- The Modern layout's sections (MODLAY 2) and their tabs. A
// section is a place in the rail; its tabs are its parts. A panel's tab is its
// PanelTable row (section, tab, tabTitle): a new panel needs no line here.
//
// A section: {
//   id, title, icon
//   foot     at the rail's foot (Devices, Settings)
//   tabs     tabs that are no panel: { tab, title, icon, content }, `content`
//            a QML type of module UConsole: an Item with `property bool shown`
//            (false while the person cannot see it) and, when it wants them,
//            `required property Item host` (PageHost)
//   order    the tabs' order, by tab id; one not listed comes after, in
//            PanelTable's order (optional)
//   quiet    tab ids with no tab of their own in the tab bar: reached by their
//            action only (optional)
//   sender   works on an alink sender, which has no DSP (optional)
// }
// LAY4 and LAY5 replace a section's content by naming their own types here.
QtObject {
    readonly property var sections: [
        {
            id: "overview", title: qsTr("Overview"), icon: "layout-dashboard", sender: true,
            tabs: [{ tab: "overview", title: qsTr("Overview"), icon: "layout-dashboard", content: "OverviewSection" }]
        },
        {
            id: "equaliser", title: qsTr("Equaliser"), icon: "sliders-vertical",
            tabs: [{ tab: "peq", title: qsTr("Equaliser"), icon: "sliders-vertical", content: "EqualiserSection" }]
        },
        { id: "routing", title: qsTr("Routing"), icon: "waypoints" },
        { id: "effects", title: qsTr("Effects"), icon: "headphones" },
        { id: "measure", title: qsTr("Measure"), icon: "mic" },
        {
            id: "analyse", title: qsTr("Analyse"), icon: "activity",
            order: ["spectrum", "signalGenerator", "stats", "interruptMonitor"]
        },
        { id: "library", title: qsTr("Library"), icon: "library" },
        { id: "system", title: qsTr("System"), icon: "network", sender: true },
        {
            id: "devices", title: qsTr("Devices"), icon: "plug", foot: true, sender: true,
            tabs: [{ tab: "devices", title: qsTr("All Devices"), icon: "plug", content: "DevicesSection" }]
        },
        {
            id: "settings", title: qsTr("Settings"), icon: "settings", foot: true, sender: true,
            order: ["settings"], quiet: ["whatsNew", "aiAgents"]   // -- WP MCP4 -- aiAgents
        }
    ]

    function section(id) { return sections.find(s => s.id === id) || null }

    // The tabs of section `id`, in order: [{ key, section, tab, title, icon,
    // content ("" for a panel's), row (null for content), quiet }]
    function tabs(id) {
        const s = section(id)
        if (!s) return []
        const quiet = s.quiet || []
        const list = (s.tabs || []).map(t => ({
            key: id + "/" + t.tab, section: id, tab: t.tab, title: t.title, icon: t.icon, content: t.content, row: null,
            quiet: quiet.indexOf(t.tab) >= 0
        }))
        for (const r of PanelTable.panels) {
            if (r.section !== id) continue
            list.push({
                key: id + "/" + r.tab, section: id, tab: r.tab, title: r.tabTitle || r.title, icon: r.icon, content: "", row: r,
                quiet: quiet.indexOf(r.tab) >= 0
            })
        }
        const order = s.order || []
        const rank = t => order.indexOf(t.tab) < 0 ? order.length : order.indexOf(t.tab)
        return list.map((t, i) => ({ t: t, i: i })).sort((a, b) => rank(a.t) - rank(b.t) || a.i - b.i).map(x => x.t)
    }

    // One tab of a section, its first for "", null for none
    function tab(id, tab) {
        const list = tabs(id)
        if (tab === "") return list.length ? list[0] : null
        return list.find(t => t.tab === tab) || null
    }
}
