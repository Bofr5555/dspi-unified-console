pragma Singleton
import QtQuick
import UConsole

// Every colour, size and font the screens use. Nothing in a screen spells a
// colour literal: it names a token here (or a ChannelPalette entry). Light and
// dark follow the system; dark is also the look when the system says nothing
// (offscreen, some desktops).
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
    readonly property color selection: dark ? "#2d4a7a" : "#cfe0fb"
    readonly property color menu: dark ? "#2a2b2f" : "#ffffff"
    readonly property color tooltip: dark ? "#3a3b40" : "#1c1c1e"
    readonly property color tooltipText: "#f2f2f2"

    // Lines
    readonly property color separator: dark ? "#35363a" : "#d9d9de"
    readonly property color border: dark ? "#44454b" : "#c9c9cf"
    readonly property color focusRing: accent
    readonly property color graphGrid: dark ? "#3a3b40" : "#d0d0d5"

    // Text
    readonly property color text: dark ? "#e6e6e6" : "#1c1c1e"
    readonly property color textMuted: dark ? "#9a9aa0" : "#6e6e73"
    readonly property color textDisabled: dark ? "#6a6a70" : "#a8a8ad"
    readonly property color textOnAccent: "#ffffff"

    // Meaning
    readonly property color accent: "#3b82f6"
    readonly property color danger: "#c42b1c"
    readonly property color warning: dark ? "#e0a030" : "#b26b00"
    readonly property color success: dark ? "#34c759" : "#248a3d"
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

    // Fonts: the system's UI font; numbers in the system's monospace
    readonly property font fontBody: Qt.font({ pixelSize: 13 })
    readonly property font fontSmall: Qt.font({ pixelSize: 11 })
    readonly property font fontCaption: Qt.font({ pixelSize: 10 })
    readonly property font fontHeading: Qt.font({ pixelSize: 11, weight: Font.DemiBold, capitalization: Font.AllUppercase })
    readonly property font fontTitle: Qt.font({ pixelSize: 15, weight: Font.DemiBold })
    readonly property font fontMono: Qt.font({ family: "monospace", pixelSize: 12 })
}
