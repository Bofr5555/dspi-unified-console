import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY4 -- The few switches that float in the corner of the Equaliser's
// graph (MODLAY 2): the spectrum, the phase, every channel's curve, and the
// graph in its own window. The rest is behind the gear beside them.
Row {
    id: toggles
    // The page's channel, and the other half of its linked pair (-1: none)
    property int channel: -1
    property int partner: -1
    property string prefix: "EqualiserGraph"
    readonly property var all: {
        const out = []
        if (DeviceModel.revision < 0) return out
        for (let r = 0; r < DeviceModel.channels.count; ++r)
            if (DeviceModel.channels.get(r, "kind") === "input" || DeviceModel.channels.get(r, "outputEnabled"))
                out.push(DeviceModel.channels.get(r, "channel"))
        return out
    }
    readonly property bool allShown: all.every(ch => GraphController.visibleChannels.indexOf(ch) >= 0)
    spacing: 4

    component Toggle: AbstractButton {
        id: toggle
        property string toolTip
        implicitHeight: 22
        leftPadding: 8
        rightPadding: 8
        checkable: false
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        contentItem: Text {
            text: toggle.text
            font: Theme.fontSmall
            color: toggle.checked ? Theme.accentText : Theme.text
            verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            radius: Theme.radiusSmall
            color: toggle.hovered ? Theme.controlHover : Theme.control
            opacity: 0.92
            border.width: toggle.visualFocus ? 2 : 0
            border.color: Theme.focusRing
        }
        HelpTip {
            text: toggle.toolTip
            visible: toggle.hovered
        }
        Accessible.role: Accessible.CheckBox
        Accessible.name: toolTip
        Accessible.checkable: true
        Accessible.checked: checked
        Accessible.onPressAction: toggle.clicked()
    }

    Toggle {
        objectName: toggles.prefix + ".rta"
        visible: RtaController.available
        text: qsTr("RTA")
        toolTip: qsTr("Show the spectrum on the graph")
        checked: RtaController.showGraph
        onClicked: RtaController.showGraph = !RtaController.showGraph
    }
    Toggle {
        objectName: toggles.prefix + ".phase"
        text: qsTr("Phase")
        toolTip: qsTr("Show the phase")
        checked: GraphController.showPhase
        onClicked: GraphController.showPhase = !GraphController.showPhase
    }
    Toggle {
        objectName: toggles.prefix + ".all"
        text: qsTr("All Curves")
        toolTip: qsTr("Show every channel's curve")
        checked: toggles.allShown
        onClicked: {
            const on = !toggles.allShown
            for (const ch of toggles.all)
                GraphController.setCurveVisible(ch, on || ch === toggles.channel || ch === toggles.partner)
        }
    }
    AbstractButton {
        id: pop
        objectName: toggles.prefix + ".popOut"
        implicitWidth: 26
        implicitHeight: 22
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        onClicked: Shell.trigger("window.popOutGraph")
        contentItem: Item {
            Icon {
                anchors.centerIn: parent
                name: "external-link"
                size: 13
            }
        }
        background: Rectangle {
            radius: Theme.radiusSmall
            color: pop.hovered ? Theme.controlHover : Theme.control
            opacity: 0.92
            border.width: pop.visualFocus ? 2 : 0
            border.color: Theme.focusRing
        }
        HelpTip {
            text: qsTr("Pop Out Graph")
            visible: pop.hovered
        }
        Accessible.role: Accessible.Button
        Accessible.name: qsTr("Pop Out Graph")
        Accessible.onPressAction: pop.clicked()
    }
}
