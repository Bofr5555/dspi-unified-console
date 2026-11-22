import QtQuick
import UConsole

// -- WP LAY5a -- The buttons in the row over the analysis pages: the live
// spectrum on the Equaliser's graph, and a stop for the signal generator,
// which goes on playing whatever page shows.
Row {
    id: tools
    objectName: "AnalyseTools"
    spacing: 6
    readonly property string tab: Shell.pageHost ? Shell.pageHost.tab : ""

    AppButton {
        objectName: "AnalyseTools.stop"
        visible: SiggenController.playing && tools.tab !== "signalGenerator"
        iconName: "square"
        text: qsTr("Stop Generator")
        toolTip: SiggenController.transportTitle
        onClicked: SiggenController.stopPlaying()
    }
    AppButton {
        objectName: "AnalyseTools.graph"
        visible: RtaController.available && tools.tab === "spectrum"
        iconName: RtaController.showGraph ? "eye" : "eye-off"
        text: qsTr("On the Equaliser's Graph")
        toolTip: RtaController.showGraph ? qsTr("The Equaliser's graph shows the spectrum. Click to hide it there.")
                                         : qsTr("Show the spectrum on the Equaliser's graph")
        onClicked: RtaController.showGraph = !RtaController.showGraph
        Accessible.role: Accessible.CheckBox
        Accessible.checkable: true
        Accessible.checked: RtaController.showGraph
    }
}
