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
//            `required property Item host` (PageHost); -- WP LAY4 -- `tools`, a
//            QML type for the row over the page, beside the tabs (optional)
//   order    the tabs' order, by tab id; one not listed comes after, in
//            PanelTable's order (optional)
//   quiet    tab ids with no tab of their own in the tab bar: reached by their
//            action only (optional)
//   sender   works on an alink sender, which has no DSP (optional)
// -- WP LAY5a -- for a section's panels:
//   tools    a QML type for the row over their pages (optional)
//   aside    a QML type at the right of their pages, an Inspector: an Item
//            with `property bool shown` and, when it wants them, `host` and
//            `tab`, the panel's (optional)
//   dot      a function of a tab id: true or false for a dot on its tab, on
//            or off; null for none (optional)
// }
// LAY4 and LAY5 replace a section's content by naming their own types here.
QtObject {
    readonly property var sections: [
        {
            id: "overview", title: qsTr("Overview"), icon: "layout-dashboard", sender: true,
            tabs: [{ tab: "overview", title: qsTr("Overview"), icon: "layout-dashboard", content: "OverviewSection", tools: "OverviewTools" }]
        },
        {
            id: "equaliser", title: qsTr("Equaliser"), icon: "sliders-vertical",
            // -- WP LAY4 --
            tabs: [
                { tab: "peq", title: qsTr("PEQ"), icon: "sliders-vertical", content: "EqualiserPeq", tools: "EqualiserTools" },
                { tab: "crossover", title: qsTr("Crossover"), icon: "chart-line", content: "EqualiserCrossover", tools: "EqualiserTools" },
                { tab: "delay", title: qsTr("Delay & Gain"), icon: "timer", content: "EqualiserDelay", tools: "EqualiserTools" }
            ]
        },
        // -- WP LAY5a --
        { id: "routing", title: qsTr("Routing"), icon: "waypoints", tools: "RoutingTools", aside: "RoutingInspector" },
        {
            id: "effects", title: qsTr("Effects"), icon: "headphones", tools: "EffectsTools", aside: "EffectsChain",
            dot: tab => EffectsState.on(tab)
        },
        { id: "measure", title: qsTr("Measure"), icon: "mic" },
        {
            id: "analyse", title: qsTr("Analyse"), icon: "activity",
            order: ["spectrum", "signalGenerator", "stats", "interruptMonitor"],
            // -- WP LAY5a -- the generator plays, whatever page shows
            tools: "AnalyseTools", dot: tab => tab === "signalGenerator" && SiggenController.playing ? true : null
        },
        { id: "library", title: qsTr("Library"), icon: "library" },
        { id: "system", title: qsTr("System"), icon: "network", sender: true },
        {
            id: "devices", title: qsTr("Devices"), icon: "plug", foot: true, sender: true,
            tabs: [{ tab: "devices", title: qsTr("All Devices"), icon: "plug", content: "DevicesSection" }]
        },
        {
            id: "settings", title: qsTr("Settings"), icon: "settings", foot: true, sender: true,
            order: ["settings"], quiet: ["whatsNew"]
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
            tools: t.tools || "", // -- WP LAY4 --
            aside: "", // -- WP LAY5a -- (content has its own)
            quiet: quiet.indexOf(t.tab) >= 0
        }))
        for (const r of PanelTable.panels) {
            if (r.section !== id) continue
            list.push({
                key: id + "/" + r.tab, section: id, tab: r.tab, title: r.tabTitle || r.title, icon: r.icon, content: "", row: r,
                tools: s.tools || "", // -- WP LAY4 -- (-- WP LAY5a --)
                aside: s.aside || "", // -- WP LAY5a --
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

    // -- WP LAY5a -- A tab's dot (true: on, false: off, null: none). Read in a
    // binding, it follows what the section's function reads.
    function dot(id, tab) {
        const s = section(id)
        return s && s.dot ? s.dot(tab) : null
    }
}
