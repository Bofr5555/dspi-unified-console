import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- One output in Delay & Gain. The columns are EqualiserDelay's.
Rectangle {
    id: row
    property int output: 0
    property real latest: 0.5
    property bool selected: false
    signal limiterWanted()

    readonly property int channel: ChannelController.revision < 0 ? -1 : ChannelController.outputChannel(output)
    readonly property var out: ChannelController.revision < 0 ? ({}) : ChannelController.output(output)
    readonly property bool enabledOut: out.enabled === true
    readonly property int polarity: ChannelController.revision < 0 ? -1 : ChannelController.outputPolarity(output)
    readonly property bool writable: ChannelController.writable
    // Sound takes a millisecond for this many centimetres
    readonly property real cmPerMs: 34.3

    objectName: "EqualiserDelay.row." + output
    height: 32
    color: selected ? Theme.selection : "transparent"
    Rectangle {
        anchors.fill: parent
        visible: row.output % 2 === 0 && !row.selected
        color: Theme.text
        opacity: 0.03
    }
    TapHandler { onTapped: SidebarController.select(row.channel) }
    ChannelInfo { id: info; channel: row.channel }

    // A value that is a button: what it is now; a click switches it
    component Flip: AbstractButton {
        id: flip
        property color ink: Theme.textMuted
        property string toolTip
        property bool on: false
        signal secondaryClicked()
        implicitHeight: 24
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        contentItem: Text {
            text: flip.text
            font: Theme.fontBody
            color: flip.enabled ? flip.ink : Theme.textDisabled
            elide: Text.ElideRight
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            radius: Theme.radiusSmall
            color: flip.down ? Theme.controlPressed : flip.hovered ? Theme.controlHover : Theme.control
            border.width: flip.visualFocus ? 2 : 0
            border.color: Theme.focusRing
        }
        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: flip.secondaryClicked()
        }
        HelpTip {
            text: flip.toolTip
            visible: flip.hovered && flip.toolTip.length > 0
        }
        Accessible.role: Accessible.CheckBox
        Accessible.name: toolTip
        Accessible.description: text
        Accessible.checkable: true
        Accessible.checked: on
        Accessible.onPressAction: flip.clicked()
    }

    RowLayout {
        x: Theme.spacing
        width: parent.width - 2 * Theme.spacing
        height: parent.height
        spacing: Theme.spacingSmall
        opacity: row.enabledOut ? 1 : 0.6

        RowLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 110
            Layout.preferredWidth: 150
            Layout.maximumWidth: 260
            spacing: Theme.spacingSmall
            Pill {
                Layout.preferredWidth: 44
                text: info.descriptor
                tint: info.tint
            }
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: info.label
                font: Theme.fontBody
                color: Theme.text
                elide: Text.ElideRight
            }
        }
        AppSwitch {
            objectName: row.objectName + ".enable"
            Layout.preferredWidth: 40
            enabled: row.writable
            checked: row.enabledOut
            onToggled: {
                ChannelController.requestOutputEnable(row.output, checked)
                checked = Qt.binding(() => row.enabledOut)
            }
            Accessible.name: qsTr("%1 enabled").arg(info.label)
        }
        ChannelValueField {
            objectName: row.objectName + ".gain"
            Layout.preferredWidth: 84
            label: qsTr("%1 gain").arg(info.label)
            enabled: row.writable
            unit: qsTr("dB")
            fieldWidth: 56
            resetValue: 0
            value: row.out.gain || 0
            onCommitted: v => ChannelController.setOutputGain(row.output, v)
        }
        ChannelValueField {
            objectName: row.objectName + ".delay"
            Layout.preferredWidth: 88
            label: qsTr("%1 delay").arg(info.label)
            enabled: row.writable
            unit: qsTr("ms")
            fieldWidth: 60
            maxDecimals: 4
            stripTrailingZeros: true
            minValue: 0
            maxValue: ChannelController.maxDelayMs
            resetValue: 0
            value: row.out.delay || 0
            onCommitted: v => ChannelController.setOutputDelay(row.output, v)
        }
        ChannelValueField {
            objectName: row.objectName + ".distance"
            Layout.preferredWidth: 88
            label: qsTr("%1 distance").arg(info.label)
            enabled: row.writable
            unit: qsTr("cm")
            fieldWidth: 60
            scrollStep: 1
            minValue: 0
            maxValue: ChannelController.maxDelayMs * row.cmPerMs
            resetValue: 0
            value: (row.out.delay || 0) * row.cmPerMs
            onCommitted: v => ChannelController.setOutputDelay(row.output, v / row.cmPerMs)
        }
        Item {
            objectName: row.objectName + ".arrival"
            // What it shows: the bar's share of the longest
            readonly property real share: row.enabledOut ? Math.min(1, (row.out.delay || 0) / row.latest) : 0
            Layout.fillWidth: true
            Layout.minimumWidth: 50
            Layout.preferredWidth: 160
            implicitHeight: 8
            Rectangle {
                anchors.fill: parent
                radius: 3
                color: Theme.meterTrack
            }
            Rectangle {
                width: Math.max(row.enabledOut ? 2 : 0, parent.width * parent.share)
                height: parent.height
                radius: 3
                color: info.tint
            }
            Accessible.role: Accessible.Graphic
            Accessible.name: qsTr("%1 arrives %2 ms late").arg(info.label).arg(Number((row.out.delay || 0).toFixed(3)))
        }
        Flip {
            objectName: row.objectName + ".polarity"
            Layout.preferredWidth: 84
            enabled: row.writable && row.polarity >= 0
            on: row.polarity > 0
            text: row.polarity < 0 ? qsTr("–") : row.polarity === 0 ? qsTr("Normal") : row.polarity === 1 ? qsTr("Inverted") : qsTr("Mixed")
            ink: row.polarity > 0 ? Theme.warning : Theme.textMuted
            toolTip: row.polarity < 0 ? qsTr("No input is connected to %1").arg(info.label) : qsTr("Invert %1").arg(info.label)
            onClicked: ChannelController.setOutputPolarity(row.output, row.polarity === 0)
        }
        IconButton {
            objectName: row.objectName + ".mute"
            readonly property bool muted: row.out.muted === true
            Layout.preferredWidth: 36
            enabled: row.writable
            iconName: muted ? "volume-x" : "volume-2"
            iconColor: muted ? Theme.dangerText : Theme.textMuted
            toolTip: muted ? qsTr("Unmute %1").arg(info.label) : qsTr("Mute %1").arg(info.label)
            onClicked: ChannelController.setOutputMute(row.output, !muted)
            Accessible.role: Accessible.CheckBox
            Accessible.checkable: true
            Accessible.checked: muted
        }
        Flip {
            objectName: row.objectName + ".limiter"
            readonly property bool limited: row.out.limiter === true
            Layout.preferredWidth: 96
            visible: ChannelController.limiterSupported
            enabled: row.writable
            on: limited
            text: limited ? qsTr("%1 dBFS").arg(Number(row.out.limiterThreshold).toFixed(1)) : qsTr("Off")
            ink: limited ? Theme.accentText : Theme.textMuted
            toolTip: qsTr("%1 limiter; right-click for its settings").arg(info.label)
            onClicked: ChannelController.setLimiterEnabled(row.output, !limited)
            onSecondaryClicked: row.limiterWanted()
        }
    }

    Accessible.role: Accessible.Row
    Accessible.name: info.label
}
