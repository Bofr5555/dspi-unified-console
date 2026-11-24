pragma Singleton
import QtQuick
import UConsole

// Every colour, size and font the screens use. Nothing in a screen spells a
// colour literal: it names a token here (or a ChannelPalette entry). Light and
// dark follow the system; dark is also the look when the system says nothing
// (offscreen, some desktops).
//
// Contrast (WCAG 2 AA, 4.5:1 for text): text, textMuted, accentText,
// dangerText, warning and success on every surface; textOnAccent on accent
// and danger; textOnTint on every ChannelPalette colour. textDisabled stays at
// 3:1 or more (disabled controls are outside 1.4.3). Checked by
// tools/contrast.py (ci/check.sh --step qt); keep it true when changing a value.
QtObject {
    id: theme

    readonly property bool dark: Shell.darkScheme

    // -- WP LAY4 -- Modern's neutral greys (MODLAY 2, 7: areas apart by tone, the
    // accent only for what is on or chosen): Grey, or Darker grey
    // (LayoutController.arrangement.tone). Classic keeps the colours below; a
    // colour a tone does not name is the same in every one.
    readonly property var toned: !LayoutController.modern ? ({})
        : greys[(LayoutController.arrangement.tone === "darker" ? "darker " : "grey ") + (dark ? "dark" : "light")]
    readonly property var greys: ({
        "grey dark": {
            window: "#202020", panel: "#2a2a2a", sidebar: "#1c1c1c", control: "#343434", controlHover: "#3c3c3c",
            controlPressed: "#464646", selection: "#3b3b3b", menu: "#2c2c2c", separator: "#303030", border: "#484848",
            graphGrid: "#3a3a3a", text: "#e8e8e8", textMuted: "#a7a7a7", textDisabled: "#868686", meterTrack: "#141414",
            pinFree: "#343434"
        },
        "grey light": {
            window: "#f3f3f3", panel: "#fbfbfb", sidebar: "#ebebeb", control: "#e8e8e8", controlHover: "#e2e2e2",
            controlPressed: "#d8d8d8", selection: "#e1e1e1", menu: "#fbfbfb", separator: "#e2e2e2", border: "#cdcdcd",
            graphGrid: "#d3d3d3", text: "#1b1b1b", textMuted: "#5e5e5e", textDisabled: "#7d7d7d", success: "#1a6b2e",
            meterTrack: "#dadada", pinFree: "#e4e4e4"
        },
        "darker dark": {
            window: "#151515", panel: "#1e1e1e", sidebar: "#111111", control: "#2a2a2a", controlHover: "#323232",
            controlPressed: "#3b3b3b", selection: "#303030", menu: "#222222", separator: "#262626", border: "#404040",
            graphGrid: "#303030", text: "#e8e8e8", textMuted: "#a3a3a3", textDisabled: "#858585", meterTrack: "#0c0c0c",
            pinFree: "#2a2a2a"
        },
        "darker light": {
            window: "#e2e2e2", panel: "#eeeeee", sidebar: "#d9d9d9", control: "#d9d9d9", controlHover: "#d2d2d2",
            controlPressed: "#c8c8c8", selection: "#cfcfcf", menu: "#eeeeee", separator: "#d2d2d2", border: "#b9b9b9",
            graphGrid: "#c6c6c6", text: "#1b1b1b", textMuted: "#545454", textDisabled: "#707070", accentText: "#174ba8",
            dangerText: "#98190f", warning: "#6e4300", success: "#14602a", meterTrack: "#cbcbcb", meterLevel: "#4c5e7e",
            meterMid: "#8a5700", alarmRaised: "#98190f", alarmAcknowledged: "#6e4300", pinFree: "#d4d4d4"
        }
    })

    // Surfaces
    readonly property color window: toned.window || (dark ? "#1e1f22" : "#f4f4f5")
    readonly property color panel: toned.panel || (dark ? "#26272b" : "#ffffff")
    readonly property color sidebar: toned.sidebar || (dark ? "#222326" : "#ececee")
    readonly property color control: toned.control || (dark ? "#303136" : "#ffffff")
    readonly property color controlHover: toned.controlHover || (dark ? "#3a3b40" : "#f0f0f3")
    readonly property color controlPressed: toned.controlPressed || (dark ? "#45464c" : "#e2e2e7")
    readonly property color selection: toned.selection || (dark ? "#203a63" : "#dbe8fc")
    // -- WP LAY4 -- selected text in a field, which stays blue where `selection` is a grey
    readonly property color textSelection: dark ? "#203a63" : "#dbe8fc"
    readonly property color menu: toned.menu || (dark ? "#2a2b2f" : "#ffffff")
    readonly property color tooltip: dark ? "#3a3b40" : "#1c1c1e"
    readonly property color tooltipText: "#f2f2f2"
    // Behind a modal question: the window dimmed
    readonly property color scrim: dark ? "#80000000" : "#59000000"

    // Lines
    readonly property color separator: toned.separator || (dark ? "#35363a" : "#d9d9de")
    readonly property color border: toned.border || (dark ? "#44454b" : "#c9c9cf")
    readonly property color focusRing: accent
    readonly property color graphGrid: toned.graphGrid || (dark ? "#3a3b40" : "#d0d0d5")

    // Text
    readonly property color text: toned.text || (dark ? "#e6e6e6" : "#1c1c1e")
    readonly property color textMuted: toned.textMuted || (dark ? "#a4a4aa" : "#5f5f64")
    readonly property color textDisabled: toned.textDisabled || (dark ? "#88888e" : "#808085")
    readonly property color textOnAccent: "#ffffff"
    // Text on a ChannelPalette colour (pills): every palette colour reaches
    // at least 6.3:1 with it, white none
    readonly property color textOnTint: "#111214"

    // Meaning
    readonly property color accent: "#2563eb"
    readonly property color danger: "#c42b1c"
    // The accent and danger as text or icons on a surface (fills use the above)
    readonly property color accentText: toned.accentText || (dark ? "#7aaeff" : "#1d5bd0")
    readonly property color dangerText: toned.dangerText || (dark ? "#ff7468" : "#b42318")
    readonly property color warning: toned.warning || (dark ? "#e0a030" : "#875200")
    readonly property color success: toned.success || (dark ? "#34c759" : "#1c7532")
    readonly property color connected: "#34c759"
    readonly property color disconnected: "#ff3b30"

    // -- WP LAY7 -- The opacity of what is off but can be read and changed (a
    // disabled output's cells, an Off band): Classic's is the Mac's, in Modern it
    // stays at 3:1 or over (smoke section lay7-contrast)
    function dim(opacity) { return LayoutController.modern ? Math.max(0.75, opacity) : opacity }

    // Banners: background and edge per kind ("info", "warning", "error")
    function bannerFill(kind) {
        if (kind === "error") return dark ? "#4a1f1c" : "#fde8e6"
        if (kind === "warning") return dark ? "#45381a" : "#fff4dc"
        // -- INTC (DL5 fold) -- Modern's info is the mockup's calm grey, so
        // a NoticeRow and the status strip read the same; Classic keeps its blue
        if (LayoutController.modern) return panel
        return dark ? "#1f3350" : "#e6f0fd"
    }
    function bannerEdge(kind) {
        if (kind === "error") return danger
        if (kind === "warning") return warning
        return accent
    }

    // Geometry
    readonly property int radius: 8
    readonly property int radiusSmall: 4
    readonly property int spacingSmall: 8
    readonly property int spacing: 12
    readonly property int spacingLarge: 16
    readonly property int rowHeight: LayoutController.modern ? 28 : 26   // -- WP DL0 -- Modern's table rows
    readonly property int controlHeight: 24
    readonly property int sidebarWidth: 240
    readonly property int popupMaxHeight: 320
    // -- WP QTPOP -- Room a dialog leaves inside the window it opens over, on
    // each side (Shell::kFitMargin for tool windows)
    readonly property int fitMargin: 48

    // Fonts: the system's UI font; numbers in the system's monospace
    readonly property font fontBody: Qt.font({ pixelSize: 13 })
    readonly property font fontSmall: Qt.font({ pixelSize: 11 })
    readonly property font fontCaption: Qt.font({ pixelSize: 10 })
    readonly property font fontHeading: Qt.font({ pixelSize: 11, weight: Font.DemiBold, capitalization: Font.AllUppercase })
    readonly property font fontTitle: Qt.font({ pixelSize: 15, weight: Font.DemiBold })
    readonly property font fontMono: Qt.font({ family: Shell.monospaceFamily, pixelSize: 12 })

    // -- WP QP10 -- Settings: the sidebar badge of each page (the Mac's
    // SettingsCategory tints, by page id) and the Overview's pin roles (outputs,
    // clocks, inputs, control, other). Each carries textOnAccent at 4.5:1 or
    // more and clears 3:1 on the dark window, so one value serves both schemes.
    readonly property var settingsTints: ({
        "overview": "#627080", "about": "#646971", "updates": "#5d6a7a", "advanced": "#727880",
        "graphing": "#587a89", "networking": "#4a7499", "ai": "#6a62a8", "spectrum": "#4c808a",
        "inputs": "#04856f", "outputs": "#0278c7", "i2s": "#ba3822", "global": "#807701",
        "surfaces": "#8354a0", "interfaces": "#8f60ad", "groups": "#6c64b7", "macros": "#9e528d", "aux": "#7a5aac"
    })
    readonly property color settingsTintDefault: "#646971"
    function settingsTint(id) { return settingsTints[id] !== undefined ? settingsTints[id] : settingsTintDefault }
    readonly property var pinRoleTints: ["#0278c7", "#ba3822", "#04856f", "#9543a7", "#807701"]
    // A free pin on the Overview map
    readonly property color pinFree: toned.pinFree || (dark ? "#303136" : "#e4e4e8")

    // -- WP QP15 -- AutoEQ: the tag behind each measurement source's name (the
    // Mac's source colours), drawn faint under Theme.text
    readonly property var autoEqSourceTints: ({
        "oratory1990": "#e8890c", "crinacle": "#9b51e0", "rtings": "#2f80ed", "innerfidelity": "#27ae60"
    })
    readonly property color autoEqSourceTintDefault: dark ? "#88888e" : "#808085"
    function autoEqSourceTint(source) {
        return autoEqSourceTints[source] !== undefined ? autoEqSourceTints[source] : autoEqSourceTintDefault
    }

    // -- WP QP12 -- Control surfaces: the badge of each component type, by the
    // firmware's type number (the Mac's typeTint: cool hues for the controls,
    // warm ones for the LEDs), and the badge of a type without one
    readonly property var csTypeTints: ["#75879e", "#339ebd", "#267d9e", "#367dd1", "#575ecc", "#eda12e",
                                        "#e67333", "#8c59b8", "#04856f", "#7a5aac", "#9e669e"]
    function csTypeTint(type) { return type > 0 && type < csTypeTints.length ? csTypeTints[type] : csTypeTints[0] }

    // -- WP QP18 -- The Pro layer's tokens (docs/UI-MODES.md §7): level
    // meters in one neutral colour on the classic screens, the segmented
    // meter's three bands and hold line, the offline and alarm states, a
    // density scale (classic and compact) and the status bar's height. Meter
    // colours are graphics, not text: each reaches 3:1 on meterTrack (WCAG
    // 1.4.11), checked by tools/contrast.py.
    readonly property color meterLevel: toned.meterLevel || (dark ? "#8fa3bf" : "#56698a")
    readonly property color meterTrack: toned.meterTrack || (dark ? "#16171a" : "#dcdce1")
    readonly property color meterLow: dark ? "#3ea65a" : "#2b7d43"
    readonly property color meterMid: toned.meterMid || (dark ? "#e0a030" : "#9a6200")
    readonly property color meterHigh: dark ? "#ff3b30" : "#c42b1c"
    readonly property color meterHold: dark ? "#f2f2f2" : "#1c1c1e"
    // Where the segments change, in dBFS, and the bottom of a meter's scale
    readonly property real meterMidDb: -18
    readonly property real meterHighDb: -6
    readonly property real meterFloorDb: -60
    readonly property color offline: dark ? "#6b6c72" : "#76767c"
    readonly property color alarmRaised: toned.alarmRaised || (dark ? "#ff7468" : "#b42318")
    readonly property color alarmAcknowledged: toned.alarmAcknowledged || (dark ? "#e0a030" : "#875200")
    readonly property color alarmCleared: textMuted
    // Row height and font size per density: the classic screens' and the
    // Pro tables' compact one
    readonly property var densities: ({
        "classic": { rowHeight: 26, fontSize: 13 },
        "compact": { rowHeight: 22, fontSize: 12 }
    })
    function density(name) { return densities[name] !== undefined ? densities[name] : densities["classic"] }
    readonly property int statusBarHeight: 24

    // -- WP DL0 -- The Modern design language (DESREV's DESIGN-LANGUAGE.md):
    // surfaces apart by tone and spacing, not by lines; the accent only for
    // what is on, chosen or the one primary action. Tokens that would change
    // Classic are gated on LayoutController.modern, so Classic keeps its look.
    //
    // The outline of a card, button, field, combo box or segment bar: none in
    // Modern (the control tone separates them), Classic's 1 px otherwise.
    // Keyboard focus is the one outline that stays: focusRing, focusRingWidth.
    readonly property int lineWidth: LayoutController.modern ? 0 : 1
    readonly property int focusRingWidth: 2
    // Every second row of a table or list, instead of row rules
    readonly property color stripe: dark ? "#06ffffff" : "#09000000"
    // Floating surfaces (popovers, menus, dialogs): the shadow's colour, its
    // downward offset and its blur radius
    readonly property color shadowColor: dark ? "#80000000" : "#29000000"
    readonly property int shadowOffset: dark ? 12 : 10
    readonly property int shadowBlur: dark ? 34 : 30
    // Buttons, fields, segments, toggle chips (Classic keeps radiusSmall); option
    // chips are rounder
    readonly property int radiusControl: LayoutController.modern ? 6 : radiusSmall
    readonly property int radiusChip: 12
    // The gap between a page's sections, and under an intro row
    readonly property int spacingSection: 24
    // The shell's bars, a form row and a tool's intro row (ToolHeader)
    readonly property int topBarHeight: 40
    readonly property int tabBarHeight: 36
    readonly property int dockHeight: 44
    readonly property int formRowHeight: 36
    readonly property int introRowHeight: 44
    // Forms and lists in Modern: left-aligned, no wider than this
    readonly property int contentMaxWidth: 760
    // The page title at the left of the tab row; table cells and list rows
    // (the spec's 12.5 px, whole pixels here)
    readonly property font fontPageTitle: Qt.font({ pixelSize: 15, weight: Font.DemiBold })
    readonly property font fontTable: Qt.font({ pixelSize: 12 })
}
