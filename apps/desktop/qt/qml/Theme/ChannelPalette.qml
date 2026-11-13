pragma Singleton
import QtQuick
import UConsole

// The one source of every per-channel colour: sidebar pills, graph curves,
// matrix rows, dashboard cards and the output assignment list read from here.
// The values are the Mac console's (ChannelPalette.swift), unchanged, in both
// colour schemes.
QtObject {
    // Input channels 0..7; 0 and 1 are the stereo pair's blue and red
    readonly property var inputs: [
        Qt.rgba(0.29, 0.56, 0.89, 1),  // 0 FL, blue
        Qt.rgba(0.96, 0.45, 0.45, 1),  // 1 FR, red
        Qt.rgba(0.45, 0.78, 0.55, 1),  // 2 FC, green
        Qt.rgba(0.93, 0.70, 0.30, 1),  // 3 LFE, amber
        Qt.rgba(0.60, 0.55, 0.92, 1),  // 4 BL, violet
        Qt.rgba(0.90, 0.55, 0.78, 1),  // 5 BR, pink
        Qt.rgba(0.40, 0.78, 0.82, 1),  // 6 SL, teal
        Qt.rgba(0.80, 0.72, 0.42, 1)   // 7 SR, olive
    ]

    // Output channels 0..7, the four S/PDIF or I2S stereo pairs
    readonly property var outputs: [
        Qt.rgba(0.27, 0.76, 0.64, 1),  // 0 teal
        Qt.rgba(0.35, 0.82, 0.50, 1),  // 1 green
        Qt.rgba(0.94, 0.77, 0.35, 1),  // 2 amber
        Qt.rgba(0.95, 0.65, 0.30, 1),  // 3 orange
        Qt.rgba(0.35, 0.55, 0.95, 1),  // 4 blue
        Qt.rgba(0.55, 0.70, 0.95, 1),  // 5 light blue
        Qt.rgba(0.85, 0.45, 0.55, 1),  // 6 rose
        Qt.rgba(0.95, 0.60, 0.65, 1)   // 7 pink
    ]

    // The PDM subwoofer output: one tone, it has no stereo partner
    readonly property color pdm: Qt.rgba(0.73, 0.53, 0.95, 1)

    // An input's colour; the accent past the eight inputs
    function input(index) {
        return index >= 0 && index < inputs.length ? inputs[index] : Theme.accent
    }

    // An output's colour by matrix output number; the PDM output is asked for
    // with `pdm` (output 8 on the RP2350, 4 on the RP2040)
    function output(index, isPdm) {
        if (isPdm) return pdm
        return index >= 0 && index < outputs.length ? outputs[index] : Theme.accent
    }

    // A ChannelListModel row's colour
    function forRow(kind, index, isPdm) {
        return kind === "input" ? input(index) : output(index, isPdm)
    }
}
