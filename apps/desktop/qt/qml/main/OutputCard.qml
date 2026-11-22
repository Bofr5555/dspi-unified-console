import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP2 -- An output's settings card (the Mac's ChannelSettingsView), four
// panels in one row: routing of the first two inputs, GAIN, DELAY, and mute
// over the output limiter.
//
// Sliders send live while dragged and once on release; the delay slider moves
// in whole milliseconds and a typed delay is kept as typed (rule SL1), up to
// the device's line (ChannelController.maxDelayMs). A right-click on a slider
// or a field resets it; elsewhere on the card it opens the card's menu with
// Identify, as the output's row in the sidebar has (QPI4).
//
// -- WP QTTWEAK -- Where the card is wide enough (`oneRowWidth`) its panels
// take one row each: both inputs side by side, a title, slider and value on
// one line, mute beside the limiter; narrower, the two rows as before.
Rectangle {
    id: card
    property int output: 0
    readonly property bool oneRow: width >= oneRowWidth
    // The narrowest the one-row card can be: what its panels need with
    // sliders of 120 px
    readonly property real oneRowWidth: {
        let routes = 0
        for (let i = 0; i < routeRepeater.count; ++i) {
            const row = routeRepeater.itemAt(i)
            if (row) routes += row.implicitWidth
        }
        routes += Math.max(0, routeRepeater.count - 1) * Theme.spacing
        const value = 60 + 4 + 20   // a ChannelValueField of 60 with its unit
        const panel = title => title + 120 + value + 2 * Theme.spacingSmall
        return routes + panel(gainTitle.implicitWidth) + panel(delayTitle.implicitWidth)
             + 6 * Theme.spacing + muteButton.implicitWidth + (limiterButton.visible ? limiterButton.implicitWidth + 4 : 0)
             + 2 * Theme.spacingSmall + 3
    }

    readonly property var out: (ChannelController.revision < 0 ? undefined : ChannelController.output(output))
    readonly property string outputName: {
        const row = DeviceModel.revision < 0 ? -1 : DeviceModel.channels.rowOfChannel(ChannelController.outputChannel(output))
        const label = row >= 0 ? DeviceModel.channels.get(row, "label") : ""
        return label && label.length ? label : qsTr("Out %1").arg(output + 1)
    }

    objectName: "OutputCard"
    implicitHeight: oneRow ? Theme.controlHeight + 2 * Theme.spacingSmall : 76
    radius: Theme.radius
    color: Theme.panel
    border.width: 1
    border.color: Theme.separator
    enabled: ChannelController.writable

    component PanelTitle: Text {
        font: Theme.fontHeading
        color: Theme.textMuted
    }
    component Divider: Rectangle {
        Layout.fillHeight: true
        Layout.preferredWidth: 1
        color: Theme.separator
    }

    // -- WP QPI4 -- under the panels: what they don't take reaches here
    readonly property bool canIdentify: DeviceModel.connected && SidebarController.identifySupported
    MouseArea {
        anchors.fill: parent
        acceptedButtons: card.canIdentify ? Qt.RightButton : Qt.NoButton
        onClicked: mouse => cardMenu.popup(mouse.x, mouse.y)
    }
    AppMenu {
        id: cardMenu
        objectName: "OutputCard.menu"
        AppMenuItem {
            objectName: "OutputCard.menu.identify"
            text: qsTr("Identify")
            enabled: card.canIdentify && !DeviceModel.readOnly
            onTriggered: SiggenController.playIdent(card.output)
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // ---- Routing: the first two inputs into this output ----
        GridLayout {
            objectName: "OutputCard.routing"
            Layout.margins: Theme.spacingSmall
            Layout.leftMargin: Theme.spacing
            Layout.rightMargin: Theme.spacing
            columns: card.oneRow ? 2 : 1
            columnSpacing: Theme.spacing
            rowSpacing: 4
            Repeater {
                id: routeRepeater
                model: Math.min(ChannelController.numInputs, 2)
                RowLayout {
                    id: routeRow
                    required property int index
                    readonly property var r: (ChannelController.revision < 0 ? undefined : ChannelController.route(index, card.output))
                    readonly property bool on: r.enabled === true
                    readonly property color tint: ChannelPalette.input(index)
                    readonly property string inputName: {
                        const row = DeviceModel.revision < 0 ? -1 : DeviceModel.channels.rowOfChannel(index)
                        return row >= 0 ? DeviceModel.channels.get(row, "label") : qsTr("USB %1").arg(index + 1)
                    }
                    spacing: Theme.spacingSmall

                    AbstractButton {
                        id: connectButton
                        objectName: "OutputCard.route." + routeRow.index
                        hoverEnabled: true
                        focusPolicy: Qt.TabFocus
                        onClicked: ChannelController.setRoute(routeRow.index, card.output, !routeRow.on, routeRow.r.gain, routeRow.r.invert)
                        contentItem: Row {
                            spacing: 6
                            Rectangle {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 9
                                height: 9
                                radius: 4.5
                                color: routeRow.on ? routeRow.tint : "transparent"
                                border.width: 1
                                border.color: routeRow.on ? routeRow.tint : Theme.textDisabled
                            }
                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                width: Math.min(implicitWidth, 90)
                                textFormat: Text.PlainText
                                text: routeRow.inputName
                                font.pixelSize: Theme.fontSmall.pixelSize
                                font.weight: routeRow.on ? Font.DemiBold : Font.Normal
                                color: routeRow.on ? routeRow.tint : Theme.textMuted
                                elide: Text.ElideRight
                            }
                        }
                        background: Rectangle {
                            radius: Theme.radiusSmall
                            color: connectButton.hovered ? Theme.controlHover : "transparent"
                            border.width: connectButton.visualFocus ? 2 : 0
                            border.color: Theme.focusRing
                        }
                        HelpTip {
                            text: routeRow.on ? qsTr("Disconnect from %1").arg(card.outputName) : qsTr("Connect to %1").arg(card.outputName)
                            visible: connectButton.hovered
                        }
                        Accessible.role: Accessible.CheckBox
                        Accessible.name: qsTr("%1 to %2").arg(routeRow.inputName).arg(card.outputName)
                        Accessible.checkable: true
                        Accessible.checked: routeRow.on
                    }
                    // Greyed while disconnected, but editable: a gain can be set before connecting
                    ChannelValueField {
                        objectName: "OutputCard.routeGain." + routeRow.index
                        opacity: routeRow.on ? 1 : 0.55
                        label: qsTr("%1 gain into %2").arg(routeRow.inputName).arg(card.outputName)
                        unit: qsTr("dB")
                        fieldWidth: 48
                        maxDecimals: 2
                        scrollStep: 0.5
                        resetValue: 0
                        value: routeRow.r.gain !== undefined ? routeRow.r.gain : 0
                        onCommitted: v => ChannelController.setRoute(routeRow.index, card.output, routeRow.on, v, routeRow.r.invert)
                    }
                    AbstractButton {
                        id: invertButton
                        objectName: "OutputCard.routeInvert." + routeRow.index
                        enabled: routeRow.on && card.enabled
                        hoverEnabled: true
                        focusPolicy: Qt.TabFocus
                        readonly property bool inverted: routeRow.r.invert === true
                        onClicked: ChannelController.setRoute(routeRow.index, card.output, routeRow.on, routeRow.r.gain, !inverted)
                        contentItem: Text {
                            text: qsTr("INV")
                            font.pixelSize: Theme.fontCaption.pixelSize
                            font.weight: invertButton.inverted ? Font.Bold : Font.Medium
                            color: invertButton.inverted ? Theme.warning : Theme.textDisabled
                        }
                        background: Rectangle {
                            color: "transparent"
                            border.width: invertButton.visualFocus ? 2 : 0
                            border.color: Theme.focusRing
                        }
                        HelpTip {
                            text: invertButton.inverted ? qsTr("Phase inverted") : qsTr("Normal phase")
                            visible: invertButton.hovered
                        }
                        Accessible.role: Accessible.CheckBox
                        Accessible.name: qsTr("Invert %1").arg(routeRow.inputName)
                        Accessible.checkable: true
                        Accessible.checked: inverted
                    }
                }
            }
        }
        Divider {}

        // ---- GAIN ----
        GridLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.spacingSmall
            Layout.leftMargin: Theme.spacing
            Layout.rightMargin: Theme.spacing
            columnSpacing: Theme.spacingSmall
            rowSpacing: 4
            PanelTitle {
                id: gainTitle
                text: qsTr("GAIN")
                Layout.row: 0
                Layout.column: 0
                Layout.fillWidth: !card.oneRow
            }
            ChannelValueField {
                objectName: "OutputCard.gainValue"
                Layout.row: 0
                Layout.column: card.oneRow ? 2 : 1
                label: qsTr("Output gain")
                unit: qsTr("dB")
                fieldWidth: 60
                resetValue: 0
                value: gainSlider.pressed ? gainSlider.value : (card.out.gain || 0)
                onCommitted: v => ChannelController.setOutputGain(card.output, v)
            }
            CustomSlider {
                id: gainSlider
                objectName: "OutputCard.gain"
                Layout.row: card.oneRow ? 0 : 1
                Layout.column: card.oneRow ? 1 : 0
                Layout.columnSpan: card.oneRow ? 1 : 2
                Layout.fillWidth: true
                label: qsTr("Output gain")
                from: -60
                to: 20
                stepSize: 0.1
                resetValue: 0
                Binding on value {
                    when: !gainSlider.pressed
                    value: card.out.gain || 0
                    restoreMode: Binding.RestoreNone
                }
                onValueMoved: v => ChannelController.setOutputGain(card.output, v)
                onCommitted: v => {
                    GraphController.endOutputGainPreview(card.output)
                    ChannelController.setOutputGain(card.output, v)
                }
                // -- WP QPI1 -- the graph follows the thumb, not the write
                onMoved: if (pressed) GraphController.previewOutputGain(card.output, value)
            }
        }
        Divider {}

        // ---- DELAY ----
        GridLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.spacingSmall
            Layout.leftMargin: Theme.spacing
            Layout.rightMargin: Theme.spacing
            columnSpacing: Theme.spacingSmall
            rowSpacing: 4
            PanelTitle {
                id: delayTitle
                text: qsTr("DELAY")
                Layout.row: 0
                Layout.column: 0
                Layout.fillWidth: !card.oneRow
            }
            // While the slider drags, the whole ms it sends; otherwise the delay exactly
            ChannelValueField {
                objectName: "OutputCard.delayValue"
                Layout.row: 0
                Layout.column: card.oneRow ? 2 : 1
                label: qsTr("Output delay")
                unit: qsTr("ms")
                fieldWidth: 60
                maxDecimals: 4
                stripTrailingZeros: true
                scrollStep: 1
                wholeSteps: true
                minValue: 0
                maxValue: ChannelController.maxDelayMs
                resetValue: 0
                value: delaySlider.pressed ? Math.round(delaySlider.value) : (card.out.delay || 0)
                onCommitted: v => ChannelController.setOutputDelay(card.output, v)
            }
            CustomSlider {
                id: delaySlider
                objectName: "OutputCard.delay"
                Layout.row: card.oneRow ? 0 : 1
                Layout.column: card.oneRow ? 1 : 0
                Layout.columnSpan: card.oneRow ? 1 : 2
                Layout.fillWidth: true
                label: qsTr("Output delay")
                from: 0
                to: ChannelController.maxDelayMs
                stepSize: 1
                resetValue: 0
                Binding on value {
                    when: !delaySlider.pressed
                    value: card.out.delay || 0
                    restoreMode: Binding.RestoreNone
                }
                onValueMoved: v => ChannelController.setOutputDelay(card.output, Math.round(v))
                onCommitted: v => ChannelController.setOutputDelay(card.output, Math.round(v))
            }
        }
        Divider {}

        // ---- Mute over the limiter (beside it in one row) ----
        GridLayout {
            Layout.preferredWidth: card.oneRow ? implicitWidth : 60
            Layout.leftMargin: card.oneRow ? Theme.spacingSmall : 0
            Layout.rightMargin: card.oneRow ? Theme.spacingSmall : 0
            Layout.alignment: Qt.AlignVCenter
            columns: card.oneRow ? 2 : 1
            columnSpacing: 4
            rowSpacing: 6
            readonly property bool split: ChannelController.limiterSupported
            IconButton {
                id: muteButton
                objectName: "OutputCard.mute"
                Layout.alignment: Qt.AlignHCenter
                readonly property bool muted: card.out.muted === true
                iconName: muted ? "volume-x" : "volume-2"
                iconSize: 20
                iconColor: muted ? Theme.dangerText : Theme.textMuted
                checkable: false
                toolTip: muted ? qsTr("Unmute") : qsTr("Mute")
                onClicked: ChannelController.setOutputMute(card.output, !muted)
                Accessible.role: Accessible.CheckBox
                Accessible.checkable: true
                Accessible.checked: muted
            }
            IconButton {
                id: limiterButton
                objectName: "OutputCard.limiter"
                visible: parent.split
                Layout.alignment: Qt.AlignHCenter
                readonly property bool on: card.out.limiter === true
                readonly property real reduction: {
                    const gr = ChannelController.limiterReduction
                    return on && card.output < gr.length ? gr[card.output] : 0
                }
                iconName: "gauge"
                iconSize: 19
                // Orange while it reduces gain, accent while on, grey while off
                iconColor: reduction >= 0.05 ? Theme.warning : on ? Theme.accentText : Theme.textMuted
                toolTip: on ? qsTr("Output limiter on, ceiling %1 dBFS. Click to switch off, right-click for settings.").arg(Number(card.out.limiterThreshold).toFixed(1))
                            : qsTr("Output limiter off. Click to switch on, right-click for settings.")
                onClicked: ChannelController.setLimiterEnabled(card.output, !on)
                onSecondaryClicked: ChannelController.requestLimiterSettings(card.output)
                Accessible.role: Accessible.CheckBox
                Accessible.checkable: true
                Accessible.checked: on
            }
        }
    }

    // -- WP QP8 -- The limiter's settings, a popover over its cell, opened by a right-click
    LimiterSettings {
        id: limiterSettings
        output: card.output
        parent: limiterButton
        x: limiterButton.width - width
        y: limiterButton.height + 4
    }
    Connections {
        target: ChannelController
        function onLimiterSettingsRequested(output) {
            if (output === card.output && limiterButton.visible) limiterSettings.open()
        }
    }

    Component.onCompleted: ChannelController.watchLimiter(true)
    Component.onDestruction: ChannelController.watchLimiter(false)

    Accessible.role: Accessible.Grouping
    Accessible.name: qsTr("%1 settings").arg(outputName)
}
