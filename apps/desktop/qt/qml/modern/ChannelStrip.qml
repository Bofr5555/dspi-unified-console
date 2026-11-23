import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- The selected channel in one row over the Equaliser's graph
// (MODLAY 2): its tag and name, and for an output its Enabled switch, the mix
// of the first two inputs, gain, delay, and mute, polarity and limiter; for an
// input the pair link and the preamp. It wraps where the page is narrow. The
// edits are Classic's (OutputCard, InputDetail).
Rectangle {
    id: strip
    property int channel: -1
    readonly property bool isOutput: info.isOutput
    readonly property int output: isOutput ? info.number : -1
    readonly property var out: ChannelController.revision < 0 || output < 0 ? ({}) : ChannelController.output(output)
    // A click on the limiter's settings (right-click): the inspector has them
    signal limiterWanted()

    objectName: "ChannelStrip"
    implicitHeight: flow.implicitHeight + 2 * 6
    radius: Theme.radius
    color: Theme.panel
    enabled: ChannelController.writable

    ChannelInfo { id: info; channel: strip.channel }

    component Caption: Text {
        anchors.verticalCenter: parent ? parent.verticalCenter : undefined
        font: Theme.fontHeading
        color: Theme.textMuted
    }
    // A button of the strip's that is on or off
    component Toggle: AbstractButton {
        id: toggle
        property bool on: false
        property string toolTip
        property color ink: on ? Theme.accentText : Theme.textMuted
        property string glyph: ""
        property string iconName: ""
        signal secondaryClicked()
        anchors.verticalCenter: parent ? parent.verticalCenter : undefined
        implicitWidth: 28
        implicitHeight: 26
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        contentItem: Item {
            Icon {
                anchors.centerIn: parent
                visible: toggle.iconName.length > 0
                name: toggle.iconName
                size: 16
                color: toggle.enabled ? toggle.ink : Theme.textDisabled
            }
            Text {
                anchors.centerIn: parent
                visible: toggle.glyph.length > 0
                text: toggle.glyph
                font: Theme.fontTitle
                color: toggle.enabled ? toggle.ink : Theme.textDisabled
            }
        }
        background: Rectangle {
            radius: Theme.radiusSmall
            color: toggle.down ? Theme.controlPressed : toggle.hovered ? Theme.controlHover : Theme.control
            border.width: toggle.visualFocus ? 2 : 0
            border.color: Theme.focusRing
        }
        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: toggle.secondaryClicked()
        }
        HelpTip {
            text: toggle.toolTip
            visible: toggle.hovered && toggle.toolTip.length > 0
        }
        Accessible.role: Accessible.CheckBox
        Accessible.name: toolTip
        Accessible.checkable: true
        Accessible.checked: on
        Accessible.onPressAction: toggle.clicked()
    }

    Flow {
        id: flow
        x: Theme.spacing
        y: 6
        width: strip.width - 2 * Theme.spacing
        spacing: Theme.spacing

        // ---- Who ----
        Row {
            height: 26
            spacing: Theme.spacingSmall
            Pill {
                anchors.verticalCenter: parent.verticalCenter
                text: info.descriptor
                tint: info.tint
            }
            Text {
                objectName: "ChannelStrip.name"
                anchors.verticalCenter: parent.verticalCenter
                width: Math.min(implicitWidth, 170)
                textFormat: Text.PlainText
                text: info.label
                font: Theme.fontTitle
                color: Theme.text
                elide: Text.ElideRight
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            AppSwitch {
                id: enableSwitch
                objectName: "ChannelStrip.enable"
                anchors.verticalCenter: parent.verticalCenter
                visible: strip.isOutput
                checked: strip.out.enabled === true
                onToggled: {
                    // A refused or asked-about enable shows the device's state again
                    ChannelController.requestOutputEnable(strip.output, checked)
                    checked = Qt.binding(() => strip.out.enabled === true)
                }
                Accessible.name: qsTr("%1 enabled").arg(info.label)
                HelpTip {
                    text: qsTr("Enabled")
                    visible: enableSwitch.hovered
                }
            }
        }

        // ---- An output: the mix of the first two inputs ----
        Row {
            visible: strip.isOutput
            height: 26
            spacing: Theme.spacing
            Caption { text: qsTr("MIX") }
            Repeater {
                model: strip.isOutput ? Math.min(ChannelController.numInputs, 2) : 0
                Row {
                    id: mix
                    required property int index
                    readonly property var r: ChannelController.revision < 0 ? ({}) : ChannelController.route(index, strip.output)
                    readonly property bool on: r.enabled === true
                    anchors.verticalCenter: parent ? parent.verticalCenter : undefined
                    spacing: 6
                    ChannelInfo { id: source; channel: mix.index }
                    AbstractButton {
                        id: connect
                        objectName: "ChannelStrip.route." + mix.index
                        anchors.verticalCenter: parent.verticalCenter
                        padding: 4
                        hoverEnabled: true
                        focusPolicy: Qt.TabFocus
                        onClicked: ChannelController.setRoute(mix.index, strip.output, !mix.on, mix.r.gain, mix.r.invert)
                        contentItem: Row {
                            spacing: 6
                            Rectangle {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 10
                                height: 10
                                radius: 5
                                color: mix.on ? ChannelPalette.input(mix.index) : "transparent"
                                border.width: 1
                                border.color: mix.on ? ChannelPalette.input(mix.index) : Theme.textDisabled
                            }
                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                width: Math.min(implicitWidth, 64)
                                textFormat: Text.PlainText
                                text: source.label.length ? source.label : qsTr("USB %1").arg(mix.index + 1)
                                font: Theme.fontSmall
                                color: mix.on ? Theme.text : Theme.textMuted
                                elide: Text.ElideRight
                            }
                        }
                        background: Rectangle {
                            radius: Theme.radiusSmall
                            color: connect.hovered ? Theme.controlHover : "transparent"
                            border.width: connect.visualFocus ? 2 : 0
                            border.color: Theme.focusRing
                        }
                        HelpTip {
                            text: mix.on ? qsTr("Disconnect from %1").arg(info.label) : qsTr("Connect to %1").arg(info.label)
                            visible: connect.hovered
                        }
                        Accessible.role: Accessible.CheckBox
                        Accessible.name: qsTr("%1 to %2").arg(source.label).arg(info.label)
                        Accessible.checkable: true
                        Accessible.checked: mix.on
                    }
                    // Greyed while disconnected, but editable: a gain can be set before connecting
                    ChannelValueField {
                        objectName: "ChannelStrip.routeGain." + mix.index
                        anchors.verticalCenter: parent.verticalCenter
                        opacity: mix.on ? 1 : 0.55
                        label: qsTr("%1 gain into %2").arg(source.label).arg(info.label)
                        unit: qsTr("dB")
                        fieldWidth: 46
                        maxDecimals: 2
                        scrollStep: 0.5
                        resetValue: 0
                        value: mix.r.gain !== undefined ? mix.r.gain : 0
                        onCommitted: v => ChannelController.setRoute(mix.index, strip.output, mix.on, v, mix.r.invert)
                    }
                }
            }
        }

        // ---- An output: gain ----
        Row {
            visible: strip.isOutput
            height: 26
            spacing: Theme.spacingSmall
            Caption { text: qsTr("GAIN") }
            ChannelValueField {
                objectName: "ChannelStrip.gain"
                anchors.verticalCenter: parent.verticalCenter
                label: qsTr("Output gain")
                unit: qsTr("dB")
                fieldWidth: 50
                resetValue: 0
                value: gainSlider.pressed ? gainSlider.value : (strip.out.gain || 0)
                onCommitted: v => ChannelController.setOutputGain(strip.output, v)
            }
            CustomSlider {
                id: gainSlider
                objectName: "ChannelStrip.gainSlider"
                anchors.verticalCenter: parent.verticalCenter
                width: 100
                label: qsTr("Output gain")
                from: -60
                to: 20
                stepSize: 0.1
                resetValue: 0
                Binding on value {
                    when: !gainSlider.pressed
                    value: strip.out.gain || 0
                    restoreMode: Binding.RestoreNone
                }
                onValueMoved: v => ChannelController.setOutputGain(strip.output, v)
                onCommitted: v => {
                    GraphController.endOutputGainPreview(strip.output)
                    ChannelController.setOutputGain(strip.output, v)
                }
                // The graph follows the thumb, not the write
                onMoved: if (pressed) GraphController.previewOutputGain(strip.output, value)
            }
        }

        // ---- An output: delay ----
        Row {
            visible: strip.isOutput
            height: 26
            spacing: Theme.spacingSmall
            Caption { text: qsTr("DELAY") }
            ChannelValueField {
                objectName: "ChannelStrip.delay"
                anchors.verticalCenter: parent.verticalCenter
                label: qsTr("Output delay")
                unit: qsTr("ms")
                fieldWidth: 56
                maxDecimals: 4
                stripTrailingZeros: true
                scrollStep: 1
                wholeSteps: true
                minValue: 0
                maxValue: ChannelController.maxDelayMs
                resetValue: 0
                value: strip.out.delay || 0
                onCommitted: v => ChannelController.setOutputDelay(strip.output, v)
            }
        }

        // ---- An output: mute, polarity, limiter ----
        Row {
            visible: strip.isOutput
            height: 26
            spacing: 4
            Toggle {
                objectName: "ChannelStrip.mute"
                on: strip.out.muted === true
                ink: on ? Theme.dangerText : Theme.textMuted
                iconName: on ? "volume-x" : "volume-2"
                toolTip: on ? qsTr("Unmute") : qsTr("Mute")
                onClicked: ChannelController.setOutputMute(strip.output, !on)
            }
            Toggle {
                objectName: "ChannelStrip.polarity"
                readonly property int polarity: ChannelController.revision < 0 ? -1 : ChannelController.outputPolarity(strip.output)
                on: polarity > 0
                enabled: polarity >= 0 && strip.enabled
                ink: on ? Theme.warning : Theme.textMuted
                //: The sign for inverted polarity
                glyph: qsTr("ø")
                toolTip: polarity < 0 ? qsTr("No input is connected")
                       : polarity === 0 ? qsTr("Invert polarity")
                       : polarity === 1 ? qsTr("Polarity inverted")
                       : qsTr("Some inputs are inverted")
                onClicked: ChannelController.setOutputPolarity(strip.output, polarity === 0)
            }
            Toggle {
                objectName: "ChannelStrip.limiter"
                visible: ChannelController.limiterSupported
                readonly property real reduction: {
                    const gr = ChannelController.limiterReduction
                    return on && strip.output < gr.length ? gr[strip.output] : 0
                }
                on: strip.out.limiter === true
                // Orange while it reduces gain, accent while on, grey while off
                ink: reduction >= 0.05 ? Theme.warning : on ? Theme.accentText : Theme.textMuted
                iconName: "gauge"
                toolTip: on ? qsTr("Output limiter on, ceiling %1 dBFS. Click to switch off; its settings are in the inspector (right-click).").arg(Number(strip.out.limiterThreshold).toFixed(1))
                            : qsTr("Output limiter off. Click to switch on; its settings are in the inspector (right-click).")
                onClicked: ChannelController.setLimiterEnabled(strip.output, !on)
                onSecondaryClicked: strip.limiterWanted()
            }
        }

        // ---- An input: the pair link and the preamp ----
        PairLink {
            objectName: "ChannelStrip.link"
            visible: info.known && !strip.isOutput && available
            height: 26
            input: strip.isOutput ? -1 : strip.channel
        }
        Row {
            visible: info.known && !strip.isOutput
            height: 26
            spacing: Theme.spacingSmall
            Caption { text: qsTr("PREAMP") }
            ChannelValueField {
                objectName: "ChannelStrip.preamp"
                anchors.verticalCenter: parent.verticalCenter
                label: qsTr("Preamp")
                unit: qsTr("dB")
                fieldWidth: 50
                resetValue: 0
                value: preampSlider.pressed ? preampSlider.value : preampSlider.device
                onCommitted: v => ChannelController.setPreamp(strip.channel, v)
            }
            CustomSlider {
                id: preampSlider
                objectName: "ChannelStrip.preampSlider"
                readonly property real device: ChannelController.revision < 0 || strip.isOutput || strip.channel < 0 ? 0 : ChannelController.preamp(strip.channel)
                anchors.verticalCenter: parent.verticalCenter
                width: 160
                label: qsTr("Preamp")
                from: -60
                to: ChannelController.preampMaxDb
                stepSize: 0.1
                resetValue: 0
                Binding on value {
                    when: !preampSlider.pressed
                    value: preampSlider.device
                    restoreMode: Binding.RestoreNone
                }
                onValueMoved: v => ChannelController.setPreamp(strip.channel, v)
                onCommitted: v => ChannelController.setPreamp(strip.channel, v)
            }
        }
    }

    Accessible.role: Accessible.Grouping
    Accessible.name: qsTr("%1 settings").arg(info.label)

    // -- WP AIVIS -- an agent changed it just now
    AiTouched { objectName: "ChannelStrip.touched"; path: "out."; indices: [strip.output]; visible: lit && strip.isOutput }
}
