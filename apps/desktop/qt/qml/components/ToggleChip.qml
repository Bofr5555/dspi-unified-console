import QtQuick
import UConsole

// -- WP DL0 -- An on/off toggle as a chip (DESIGN-LANGUAGE §7, §12): the
// graph overlays (RTA, Phase, All Curves), Solo, a mode that is on or off.
// The control tone with radius 6; on, the selection tone and accent text
// (and icon). It flips itself on a click and emits `toggled()`: bind
// `checked` to the state and write it in onToggled. Over a graph, set
// `overlay: true` for the 0.92 opacity of the graph's tools.
//
//     ToggleChip {
//         objectName: "Graph.rta"
//         text: qsTr("RTA")
//         iconName: "audio-lines"          // optional
//         toolTip: qsTr("Show the analyser behind the curves")
//         checked: GraphController.showRta
//         onToggled: GraphController.showRta = checked
//     }
//
// Everything else is OptionChip's (text, iconName, toolTip, checked).
OptionChip {
    id: toggle
    property bool overlay: false

    checkable: true
    chipRadius: Theme.radiusControl
    opacity: overlay ? 0.92 : 1
    Accessible.role: Accessible.Button
    Accessible.checkable: true
}
