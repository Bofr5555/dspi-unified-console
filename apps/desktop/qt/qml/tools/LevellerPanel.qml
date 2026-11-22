import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- The volume leveller (the Mac's VolumeLevellerView, 380×540, at
// least 380×400): the header with the switch, then one scrolling column: the
// channel masks (with more than two inputs, on firmware with them, wire V18),
// amount, speed, largest gain, gate and lookahead.
// -- WP LAY1a -- A panel (Panel.qml): PanelTable's row for
// tools.leveller gives its window's title and sizes.
Panel {
    id: panel
    objectName: "LevellerPanel"
    // -- WP QTPOP --
    fitBodies: [body]

    readonly property bool writable: EffectsController.writable
    readonly property var speedTexts: [
        qsTr("Slow — Gentle response for music and wide dynamic range content."),
        qsTr("Medium — Balanced response for general purpose use."),
        qsTr("Fast — Tight response for speech, dialogue, and podcasts.")
    ]

    // One mask's chips: Detector or Apply
    component MaskRow: ColumnLayout {
        id: maskRow
        required property bool detector
        property string label
        property string hint
        readonly property int mask: detector ? EffectsController.levellerDetectorMask : EffectsController.levellerApplyMask
        Layout.fillWidth: true
        spacing: 4
        RowLayout {
            spacing: 6
            Text {
                textFormat: Text.PlainText
                text: maskRow.label
                font: Theme.fontBody
                color: Theme.text
            }
            Text {
                textFormat: Text.PlainText
                text: maskRow.hint
                font: Theme.fontCaption
                color: Theme.textMuted
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            Repeater {
                model: EffectsController.levellerChannels
                EffectChip {
                    required property int index
                    objectName: "LevellerWindow." + (maskRow.detector ? "detector." : "apply.") + index
                    enabled: panel.writable
                    on: (maskRow.mask & (1 << index)) !== 0
                    text: String(index + 1)
                    toolTip: { EffectsController.revision; return EffectsController.inputName(index) }
                    onClicked: EffectsController.toggleLevellerChannel(maskRow.detector, index)
                }
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        EffectHeader {
            objectName: "LevellerWindow.header"
            Layout.fillWidth: true
            iconName: "activity"
            title: qsTr("Volume Leveller")
            subtitle: qsTr("Upward Dynamic Range Compression")
            on: EffectsController.levellerOn
            switchEnabled: panel.writable
            onToggled: on => EffectsController.setEnabled("leveller", on)
        }
        EffectRule {}

        Flickable {
            id: body
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentHeight: column.implicitHeight + 2 * Theme.spacingLarge
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}

            ColumnLayout {
                id: column
                x: Theme.spacingLarge
                y: Theme.spacingLarge
                width: body.width - 2 * Theme.spacingLarge
                spacing: 14

                ColumnLayout {
                    objectName: "LevellerWindow.channels"
                    visible: EffectsController.levellerMasksShown
                    Layout.fillWidth: true
                    spacing: 10
                    EffectSection {
                        text: qsTr("Channels")
                        EffectMenuButton {
                            objectName: "LevellerWindow.channels.presets"
                            enabled: panel.writable
                            text: qsTr("Presets")
                            AppMenuItem {
                                objectName: "LevellerWindow.channels.all"
                                text: qsTr("All channels (Night mode)")
                                onTriggered: EffectsController.setLevellerMasks(0xFF, 0xFF)
                            }
                            AppMenuItem {
                                objectName: "LevellerWindow.channels.center"
                                text: qsTr("Center only (Dialog boost)")
                                onTriggered: EffectsController.setLevellerMasks(0x04, 0x04)
                            }
                            AppMenuItem {
                                objectName: "LevellerWindow.channels.front"
                                text: qsTr("Front L / R only")
                                onTriggered: EffectsController.setLevellerMasks(0x03, 0x03)
                            }
                        }
                    }
                    MaskRow {
                        detector: true
                        label: qsTr("Detector")
                        hint: qsTr("sets the shared gain")
                    }
                    MaskRow {
                        detector: false
                        label: qsTr("Apply")
                        hint: qsTr("receives the gain")
                    }
                }
                EffectRule { visible: EffectsController.levellerMasksShown }

                SectionHeading { text: qsTr("Parameters") }
                EffectParameterRow {
                    objectName: "LevellerWindow.amount"
                    Layout.fillWidth: true
                    enabled: panel.writable
                    title: qsTr("Amount")
                    unit: qsTr("%")
                    value: EffectsController.levellerAmount
                    from: 0
                    to: 100
                    resetValue: 50
                    caption: qsTr("Compression strength. Higher values reduce dynamic range more aggressively.")
                    onLive: v => EffectsController.setLevellerAmount(v)
                    onCommit: v => EffectsController.setLevellerAmount(v)
                }
                EffectRule {}
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    Text {
                        text: qsTr("Speed")
                        font: Theme.fontBody
                        color: Theme.text
                    }
                    AppTabBar {
                        objectName: "LevellerWindow.speed"
                        Layout.fillWidth: true
                        label: qsTr("Speed")
                        enabled: panel.writable
                        currentIndex: EffectsController.levellerSpeed
                        AppTabButton {
                            objectName: "LevellerWindow.speed.0"
                            text: qsTr("Slow")
                            onClicked: EffectsController.setLevellerSpeed(0)
                        }
                        AppTabButton {
                            objectName: "LevellerWindow.speed.1"
                            text: qsTr("Medium")
                            onClicked: EffectsController.setLevellerSpeed(1)
                        }
                        AppTabButton {
                            objectName: "LevellerWindow.speed.2"
                            text: qsTr("Fast")
                            onClicked: EffectsController.setLevellerSpeed(2)
                        }
                    }
                    Text {
                        objectName: "LevellerWindow.speedText"
                        Layout.fillWidth: true
                        text: panel.speedTexts[EffectsController.levellerSpeed] || panel.speedTexts[1]
                        font: Theme.fontCaption
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                }
                EffectRule {}
                EffectParameterRow {
                    objectName: "LevellerWindow.maxGain"
                    Layout.fillWidth: true
                    enabled: panel.writable
                    title: qsTr("Max Gain")
                    unit: qsTr("dB")
                    value: EffectsController.levellerMaxGain
                    from: 0
                    to: 35
                    resetValue: 15
                    caption: qsTr("Maximum boost for quiet passages. Higher values risk amplifying noise.")
                    onLive: v => EffectsController.setLevellerMaxGain(v)
                    onCommit: v => EffectsController.setLevellerMaxGain(v)
                }
                EffectRule {}
                EffectParameterRow {
                    objectName: "LevellerWindow.gate"
                    Layout.fillWidth: true
                    enabled: panel.writable
                    title: qsTr("Gate Threshold")
                    unit: qsTr("dB")
                    value: EffectsController.levellerGate
                    from: -96
                    to: 0
                    resetValue: -96
                    caption: qsTr("Silence gate. Signals below this level are not boosted, preventing noise amplification.")
                    onLive: v => EffectsController.setLevellerGate(v)
                    onCommit: v => EffectsController.setLevellerGate(v)
                }
                EffectRule {}
                RowLayout {
                    Layout.fillWidth: true
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        Text {
                            text: qsTr("Lookahead")
                            font: Theme.fontBody
                            color: Theme.text
                        }
                        Text {
                            Layout.fillWidth: true
                            text: qsTr("Adds 5ms latency. Improves transient handling.")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                            wrapMode: Text.Wrap
                        }
                    }
                    AppSwitch {
                        objectName: "LevellerWindow.lookahead"
                        enabled: panel.writable
                        checked: EffectsController.levellerLookahead
                        onToggled: EffectsController.setLevellerLookahead(checked)
                        Accessible.name: qsTr("Lookahead")
                    }
                }
            }
        }
    }
}
