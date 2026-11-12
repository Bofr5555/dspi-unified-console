import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP9 -- The analyser's part of the graph options (the Mac's popover
// main page above Graph Setup), in GraphController's RtaOptions slot: who
// drives the analyser, the side and channels of the page's spectrum, Clear,
// and the page's "FFT Graph" and "RTA Bars" switches. A choice here takes the
// analyser back from a console being followed.
ColumnLayout {
    id: options
    property bool popOut: false
    readonly property string prefix: popOut ? "GraphWindow.options.rta" : "GraphHost.options.rta"
    readonly property var shown: RtaController.channels(RtaController.drawnTap)
    readonly property int count: RtaController.drawnChannels.length

    objectName: prefix
    spacing: 0

    function colorOf(info) { return ChannelPalette.forRow(info.kind, info.number, info.pdm) }

    // Without an analyser: one line
    Text {
        objectName: options.prefix + ".unavailable"
        visible: !RtaController.available
        Layout.fillWidth: true
        Layout.margins: Theme.spacing
        wrapMode: Text.WordWrap
        text: !RtaController.ready ? qsTr("Connect a DSPi to show its spectrum.")
              : DeviceModel.readOnly ? qsTr("The analyser is not available while the device is read-only.")
              : qsTr("This firmware has no spectrum analyser.")
        font: Theme.fontSmall
        color: Theme.textMuted
    }

    // Following another console
    RowLayout {
        objectName: options.prefix + ".following"
        visible: RtaController.available && RtaController.following
        Layout.fillWidth: true
        Layout.leftMargin: Theme.spacing
        Layout.rightMargin: Theme.spacing
        Layout.topMargin: 10
        spacing: 6
        Icon {
            name: "users"
            size: 12
            color: Theme.textMuted
        }
        Text {
            text: qsTr("Following another console")
            font: Theme.fontSmall
            color: Theme.textMuted
            elide: Text.ElideRight
            Layout.fillWidth: true
        }
        AppButton {
            objectName: options.prefix + ".takeControl"
            text: qsTr("Take control")
            onClicked: RtaController.takeControl()
        }
    }

    ColumnLayout {
        visible: RtaController.available
        Layout.fillWidth: true
        Layout.margins: Theme.spacing
        spacing: 10

        RowLayout {
            Layout.fillWidth: true
            Text {
                text: qsTr("SPECTRUM")
                font: Theme.fontHeading
                color: Theme.textMuted
                Layout.fillWidth: true
            }
            AppTabBar {
                objectName: options.prefix + ".side"
                label: qsTr("Spectrum side")
                currentIndex: RtaController.drawnTap === 0 ? 0 : 1
                AppTabButton {
                    objectName: options.prefix + ".inputs"
                    text: qsTr("Inputs")
                    onClicked: RtaController.switchSide(0)
                }
                AppTabButton {
                    objectName: options.prefix + ".outputs"
                    text: qsTr("Outputs")
                    onClicked: RtaController.switchSide(1)
                }
            }
        }
        Text {
            visible: options.shown.length === 0
            text: RtaController.drawnTap === 0 ? qsTr("No active inputs.") : qsTr("No enabled outputs.")
            font: Theme.fontSmall
            color: Theme.textMuted
        }
        GridLayout {
            visible: options.shown.length > 0
            Layout.fillWidth: true
            columns: 2
            rowSpacing: 6
            columnSpacing: 6
            Repeater {
                model: options.shown
                RtaChip {
                    required property var modelData
                    objectName: options.prefix + ".channel." + modelData.channel
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    text: modelData.name
                    tint: options.colorOf(modelData)
                    on: RtaController.drawnChannels.indexOf(modelData.channel) >= 0
                    onClicked: RtaController.toggleChannel(modelData.channel)
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Text {
                objectName: options.prefix + ".summary"
                property string shownName: text
                text: options.count === 0 ? qsTr("Spectrum hidden")
                    : options.count === 1 ? qsTr("1 channel") : qsTr("%1 channels").arg(options.count)
                font: Theme.fontCaption
                color: Theme.textMuted
                Layout.fillWidth: true
            }
            AppButton {
                objectName: options.prefix + ".clear"
                visible: options.count > 0
                text: qsTr("Clear")
                onClicked: RtaController.clearSelection()
            }
        }
    }

    Rectangle {
        visible: RtaController.available
        color: Theme.separator
        implicitHeight: 1
        Layout.fillWidth: true
    }
    AppSwitch {
        objectName: options.prefix + ".graph"
        visible: RtaController.available
        implicitHeight: 26
        Layout.fillWidth: true
        Layout.leftMargin: 10
        Layout.rightMargin: 10
        Layout.topMargin: 6
        text: qsTr("FFT Graph")
        checked: RtaController.showGraph
        onToggled: RtaController.showGraph = checked
    }
    AppSwitch {
        objectName: options.prefix + ".bars"
        visible: RtaController.available
        implicitHeight: 26
        Layout.fillWidth: true
        Layout.leftMargin: 10
        Layout.rightMargin: 10
        Layout.bottomMargin: 6
        text: qsTr("RTA Bars")
        checked: RtaController.showBars
        onToggled: RtaController.showBars = checked
    }
}
