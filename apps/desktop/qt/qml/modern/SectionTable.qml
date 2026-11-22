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
// -- WP LAY5b --
//   parts    { a panel's tab: [{ tab, title, icon }, ...] }: the panel's parts
//            (Panel.part), each a tab of the section; they share its one page
//   shows    a function of a tab id: whether its tab is there now (optional)
//   now      a function: the tab to go to when none is asked for, "" for the
//            one the section was on (optional)
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
        // -- WP LAY5b -- a connection that may not play has the passive RTA
        {
            id: "measure", title: qsTr("Measure"), icon: "mic",
            parts: {
                measure: [
                    { tab: "measure", title: qsTr("Measure"), icon: "mic" },
                    { tab: "results", title: qsTr("Results"), icon: "chart-line" },
                    { tab: "live", title: qsTr("Live"), icon: "activity" },
                    { tab: "tune", title: qsTr("Auto-tune"), icon: "sliders-vertical" },
                    { tab: "documents", title: qsTr("Documents"), icon: "file-text" },
                    { tab: "passive", title: qsTr("Passive RTA"), icon: "chart-column" },
                    { tab: "units", title: qsTr("Several units"), icon: "network" }
                ]
            },
            shows: tab => tab === "results" || tab === "documents" || (tab === "passive") === (MeasureController.playBlocker.length > 0),
            dot: tab => tab === "measure" && MeasureController.measuring || tab === "live" && MeasureController.liveActive ? true : null
        },
        {
            id: "analyse", title: qsTr("Analyse"), icon: "activity",
            order: ["spectrum", "signalGenerator", "stats", "interruptMonitor"],
            // -- WP LAY5a -- the generator plays, whatever page shows
            tools: "AnalyseTools", dot: tab => tab === "signalGenerator" && SiggenController.playing ? true : null
        },
        // -- WP LAY5b --
        {
            id: "library", title: qsTr("Library"), icon: "library",
            order: ["presets", "snapshots", "history", "files", "autoeq"],
            tabs: [
                { tab: "presets", title: qsTr("Presets"), icon: "save", content: "LibraryPresets" },
                { tab: "files", title: qsTr("Files"), icon: "folder-open", content: "LibraryFiles" }
            ]
        },
        // Simple has the status only, as in Classic
        {
            id: "system", title: qsTr("System"), icon: "network", sender: true,
            parts: {
                status: [
                    { tab: "status", title: qsTr("Units"), icon: "network" },
                    { tab: "clock", title: qsTr("Clock and Links"), icon: "timer" },
                    { tab: "installation", title: qsTr("Installation"), icon: "group" }
                ]
            },
            shows: tab => (tab !== "clock" && tab !== "installation") || ModeController.mode !== "simple"
        },
        {
            id: "devices", title: qsTr("Devices"), icon: "plug", foot: true, sender: true,
            // -- WP LAY5b --
            tabs: [
                { tab: "devices", title: qsTr("This Computer"), icon: "plug", content: "DevicesSection" },
                { tab: "hubs", title: qsTr("Hubs"), icon: "wifi", content: "DevicesHubs" }
            ]
        },
        {
            id: "settings", title: qsTr("Settings"), icon: "settings", foot: true, sender: true,
            quiet: ["whatsNew", "aiAgents"],   // -- WP MCP4 -- aiAgents
            // -- WP LAY5b -- the groups, in the order of Classic's list
            order: ["application", "display", "device", "control"],
            parts: {
                settings: [
                    { tab: "application", title: qsTr("Application"), icon: "app-window" },
                    { tab: "display", title: qsTr("Display"), icon: "monitor" },
                    { tab: "device", title: qsTr("Device"), icon: "cpu" },
                    { tab: "control", title: qsTr("Control"), icon: "sliders-horizontal" }
                ]
            },
            shows: tab => (tab !== "device" && tab !== "control") || SettingsController.groupShown(tab, SettingsController.revision),
            now: () => SettingsController.groupOf(SettingsController.currentPage)
        }
    ]

    function section(id) { return sections.find(s => s.id === id) || null }

    // The tabs of section `id`, in order: [{ key, section, tab, title, icon,
    // content ("" for a panel's), row (null for content), quiet,
    // -- WP LAY5b -- page (the key of the page that shows it), part }]
    function tabs(id) {
        const s = section(id)
        if (!s) return []
        const quiet = s.quiet || []
        const list = (s.tabs || []).map(t => ({
            key: id + "/" + t.tab, section: id, tab: t.tab, title: t.title, icon: t.icon, content: t.content, row: null,
            page: id + "/" + t.tab, part: "",   // -- WP LAY5b --
            tools: t.tools || "", // -- WP LAY4 --
            aside: "", // -- WP LAY5a -- (content has its own)
            quiet: quiet.indexOf(t.tab) >= 0
        }))
        for (const r of PanelTable.panels) {
            if (r.section !== id) continue
            // -- WP LAY5b -- a tab for each part
            const parts = s.parts && s.parts[r.tab] ? s.parts[r.tab] : [{ tab: r.tab, title: r.tabTitle || r.title, icon: r.icon, whole: true }]
            for (const p of parts) list.push({
                key: id + "/" + p.tab, section: id, tab: p.tab, title: p.title, icon: p.icon, content: "", row: r,
                page: id + "/" + r.tab, part: p.whole ? "" : p.tab,
                tools: s.tools || "", // -- WP LAY4 -- (-- WP LAY5a --)
                aside: s.aside || "", // -- WP LAY5a --
                quiet: quiet.indexOf(p.tab) >= 0
            })
        }
        const order = s.order || []
        const rank = t => order.indexOf(t.tab) < 0 ? order.length : order.indexOf(t.tab)
        return list.map((t, i) => ({ t: t, i: i })).sort((a, b) => rank(a.t) - rank(b.t) || a.i - b.i).map(x => x.t)
    }

    // One tab of a section, its first for "" (-- WP LAY5b -- that is there
    // now), null for none. A panel's own tab (PanelTable's) whose parts have
    // the tabs is the part to go to.
    function tab(id, tab) {
        const list = tabs(id)
        if (tab === "") return list.find(t => shows(id, t.tab)) || list[0] || null
        return list.find(t => t.tab === tab) || part(list.filter(t => t.page === id + "/" + tab)) || null
    }
    // -- WP LAY5b -- Of a panel's parts `list`, the section's `now`, else the
    // one it was on (`last`), else the first; null for an empty list
    function part(list, last) {
        if (!list.length) return null
        const want = now(list[0].section)
        return list.find(t => t.tab === want) || list.find(t => t.tab === last && shows(t.section, t.tab))
            || list.find(t => shows(t.section, t.tab)) || list[0]
    }
    function now(id) {
        const s = section(id)
        return s && s.now ? s.now() : ""
    }
    // Whether a tab is there now. Read in a binding, it follows what the
    // section's function reads.
    function shows(id, tab) {
        const s = section(id)
        return !s || !s.shows || s.shows(tab)
    }

    // -- WP LAY5a -- A tab's dot (true: on, false: off, null: none). Read in a
    // binding, it follows what the section's function reads.
    function dot(id, tab) {
        const s = section(id)
        return s && s.dot ? s.dot(tab) : null
    }
}
