import QtQuick
import UConsole

// -- WP QP3 -- The pop-out graph's legend (the Mac's GraphLegend): a pill per
// input and per enabled output, wrapping onto more rows rather than widening
// the window. A click shows or hides that curve: the main graph's set while the
// pop-out follows the page, its own set otherwise. Faded without a device.
Flow {
    id: legend
    objectName: "GraphLegend"
    spacing: 8
    topPadding: 6
    bottomPadding: 2
    opacity: DeviceModel.connected ? 1 : 0
    enabled: DeviceModel.connected
    Behavior on opacity { OpacityAnimator { duration: 300 } }

    Repeater {
        model: DeviceModel.channels
        delegate: Pill {
            id: pill
            required property string kind
            required property int number
            required property int channel
            required property string descriptor
            required property bool outputEnabled
            required property bool pdm

            objectName: "GraphLegend.channel." + channel
            visible: kind === "input" || outputEnabled
            text: descriptor
            checkable: true
            tint: ChannelPalette.forRow(kind, number, pdm)
            checked: shown()
            accessibleName: qsTr("%1 curve").arg(descriptor)
            function shown() { return GraphController.popOutVisibleChannels.indexOf(channel) >= 0 }
            // The click's own toggle is undone: the controller decides
            onClicked: {
                GraphController.togglePopOutCurve(channel)
                checked = Qt.binding(shown)
            }
        }
    }
}
