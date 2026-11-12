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

    // Surfaces
    readonly property color window: dark ? "#1e1f22" : "#f4f4f5"
    readonly property color panel: dark ? "#26272b" : "#ffffff"
    readonly property color sidebar: dark ? "#222326" : "#ececee"
    readonly property color control: dark ? "#303136" : "#ffffff"
    readonly property color controlHover: dark ? "#3a3b40" : "#f0f0f3"
    readonly property color controlPressed: dark ? "#45464c" : "#e2e2e7"
    readonly property color selection: dark ? "#203a63" : "#dbe8fc"
    readonly property color menu: dark ? "#2a2b2f" : "#ffffff"
    readonly property color tooltip: dark ? "#3a3b40" : "#1c1c1e"
    readonly property color tooltipText: "#f2f2f2"
    // Behind a modal question: the window dimmed
    readonly property color scrim: dark ? "#80000000" : "#59000000"

    // Lines
    readonly property color separator: dark ? "#35363a" : "#d9d9de"
    readonly property color border: dark ? "#44454b" : "#c9c9cf"
    readonly property color focusRing: accent
    readonly property color graphGrid: dark ? "#3a3b40" : "#d0d0d5"

    // Text
    readonly property color text: dark ? "#e6e6e6" : "#1c1c1e"
    readonly property color textMuted: dark ? "#a4a4aa" : "#5f5f64"
    readonly property color textDisabled: dark ? "#88888e" : "#808085"
    readonly property color textOnAccent: "#ffffff"
    // Text on a ChannelPalette colour (pills): every palette colour reaches
    // at least 6.3:1 with it, white none
    readonly property color textOnTint: "#111214"

    // Meaning
    readonly property color accent: "#2563eb"
    readonly property color danger: "#c42b1c"
    // The accent and danger as text or icons on a surface (fills use the above)
    readonly property color accentText: dark ? "#7aaeff" : "#1d5bd0"
    readonly property color dangerText: dark ? "#ff7468" : "#b42318"
    readonly property color warning: dark ? "#e0a030" : "#875200"
    readonly property color success: dark ? "#34c759" : "#1c7532"
    readonly property color connected: "#34c759"
    readonly property color disconnected: "#ff3b30"

    // Banners: background and edge per kind ("info", "warning", "error")
    function bannerFill(kind) {
        if (kind === "error") return dark ? "#4a1f1c" : "#fde8e6"
        if (kind === "warning") return dark ? "#45381a" : "#fff4dc"
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
    readonly property int rowHeight: 26
    readonly property int controlHeight: 24
    readonly property int sidebarWidth: 240
    readonly property int popupMaxHeight: 320

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
        "graphing": "#587a89", "networking": "#4a7499", "spectrum": "#4c808a",
        "inputs": "#04856f", "outputs": "#0278c7", "i2s": "#ba3822", "global": "#807701",
        "surfaces": "#8354a0", "interfaces": "#8f60ad", "groups": "#6c64b7", "macros": "#9e528d", "aux": "#7a5aac"
    })
    readonly property color settingsTintDefault: "#646971"
    function settingsTint(id) { return settingsTints[id] !== undefined ? settingsTints[id] : settingsTintDefault }
    readonly property var pinRoleTints: ["#0278c7", "#ba3822", "#04856f", "#9543a7", "#807701"]
    // A free pin on the Overview map
    readonly property color pinFree: dark ? "#303136" : "#e4e4e8"
}
