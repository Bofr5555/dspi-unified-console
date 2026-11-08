import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Window 2.15
import "components"

// Volume Leveller: upward dynamic range compression.
AppWindow {
    id: win
    fitHeight: header.height + flick.contentHeight
    title: qsTr("Volume Leveller")
    visible: false
    width: 380
    height: 560 + titlebarHeight
    minimumWidth: 340
    minimumHeight: 320 + titlebarHeight

    // -- WP QD18 --
    readonly property int inputCount: hostAudio.inputCount
    readonly property var speedCaptions: [
        qsTr("Slow — Gentle response for music and wide dynamic range content."),
        qsTr("Medium — Balanced response for general purpose use."),
        qsTr("Fast — Tight response for speech, dialogue, and podcasts.")]

    function inputNames() {
        var n = []
        for (var i = 0; i < inputCount; i++) n.push(bridge.channelName(bridge.inputAppId(i)))
        return n
    }

    component SectionLabel: Text {
        font.pixelSize: 11
        font.weight: Font.Bold
        font.letterSpacing: 0.4
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
    }
    component Divider: Rectangle { width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.08) }
    // Row name with a grey note after it ("Detector  sets the shared gain")
    component RowTitle: Row {
        property string text: ""
        property string note: ""
        spacing: 6
        Text { id: rt; text: parent.text; font.pixelSize: 13; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9) }
        Text { text: parent.note; font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5); anchors.baseline: rt.baseline }
    }

    ToolHeader {
        id: header
        width: parent.width
        icon: "waveform"
        title: qsTr("Volume Leveller")
        subtitle: qsTr("Upward dynamic range compression")
        checked: bridge.levellerEnabled
        onToggled: bridge.setLevellerEnabled(enable)
    }

    Flickable {
        id: flick
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        width: parent.width
        contentHeight: body.height + 32
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {}

        Column {
            id: body
            x: 16
            y: 16
            width: win.width - 32
            spacing: 12
            enabled: bridge.connected

            // ── Channels (multichannel inputs only) ──
            Column {
                visible: inputCount > 2
                width: parent.width
                spacing: 8

                Item {
                    width: parent.width
                    height: 20
                    SectionLabel { text: qsTr("CHANNELS"); anchors.verticalCenter: parent.verticalCenter }
                    Item {
                        id: presetsLink
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        width: presetsRow.width
                        height: presetsRow.height
                        Row {
                            id: presetsRow
                            spacing: 4
                            Text {
                                text: qsTr("Presets")
                                font.pixelSize: 12
                                color: isMacOS ? MacColors.label : presetMouse.containsMouse ? "white" : Qt.rgba(1, 1, 1, 0.75)
                            }
                            Icon { name: "chev-down"; size: 11; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.6); anchors.verticalCenter: parent.verticalCenter }
                        }
                        ClickArea {
                            id: presetMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            Accessible.role: Accessible.ButtonMenu
                            Accessible.name: qsTr("Presets")
                            onActivated: presetMenu.openAt(presetsLink, 0, presetsLink.height + 4)
                        }
                    }
                }

                RowTitle { text: qsTr("Detector"); note: qsTr("sets the shared gain") }
                ChannelChips {
                    width: parent.width
                    count: inputCount
                    mask: bridge.levellerDetectorMask
                    names: inputNames()
                    onMaskEdited: bridge.setLevellerMasks(mask, bridge.levellerApplyMask)
                }

                RowTitle { text: qsTr("Apply"); note: qsTr("receives the gain") }
                ChannelChips {
                    width: parent.width
                    count: inputCount
                    mask: bridge.levellerApplyMask
                    names: inputNames()
                    onMaskEdited: bridge.setLevellerMasks(bridge.levellerDetectorMask, mask)
                }

                Item { width: 1; height: 2 }
                Divider {}
            }

            SectionLabel { text: qsTr("PARAMETERS") }

            ParamRow {
                label: qsTr("Amount"); unit: "%"; from: 0; to: 100; stepSize: 0.5
                value: bridge.levellerAmount
                caption: qsTr("Compression strength. Higher values reduce dynamic range more aggressively.")
                onLiveChanged: bridge.setLevellerAmount(v, true)
                onCommitted: bridge.setLevellerAmount(v)
            }
            Divider {}

            // Speed: segmented Slow / Medium / Fast
            Column {
                width: parent.width
                spacing: 6
                Text { text: qsTr("Speed"); font.pixelSize: 13; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9) }
                Rectangle {
                    width: parent.width
                    height: 26
                    radius: 7
                    color: Qt.rgba(1, 1, 1, 0.06)
                    border.color: Qt.rgba(1, 1, 1, 0.08)
                    Row {
                        anchors.fill: parent
                        anchors.margins: 2
                        Repeater {
                            model: [qsTr("Slow"), qsTr("Medium"), qsTr("Fast")]
                            Rectangle {
                                readonly property bool isCurrent: bridge.levellerSpeed === index
                                width: parent.width / 3
                                height: parent.height
                                radius: 5
                                color: isMacOS ? (isCurrent ? MacColors.control : "transparent") : isCurrent ? MenuStyle.highlight
                                     : speedMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.08) : "transparent"
                                Text {
                                    anchors.centerIn: parent
                                    text: modelData
                                    font.pixelSize: 12
                                    font.weight: parent.isCurrent ? Font.DemiBold : Font.Normal
                                    color: isMacOS ? (parent.isCurrent ? "white" : MacColors.label) : parent.isCurrent ? "white" : Qt.rgba(1, 1, 1, 0.75)
                                }
                                ClickArea {
                                    id: speedMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    focusRadius: 7
                                    Accessible.role: Accessible.RadioButton
                                    Accessible.name: qsTr("Speed %1").arg(modelData)
                                    Accessible.checkable: true
                                    Accessible.checked: parent.isCurrent
                                    onActivated: bridge.setLevellerSpeed(index)
                                }
                            }
                        }
                    }
                }
                Text {
                    width: parent.width
                    wrapMode: Text.WordWrap
                    text: speedCaptions[Math.max(0, Math.min(2, bridge.levellerSpeed))]
                    font.pixelSize: 11
                    color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
                }
            }
            Divider {}

            ParamRow {
                label: qsTr("Max Gain"); unit: qsTr("dB"); from: 0; to: 35; stepSize: 0.5
                value: bridge.levellerMaxGain
                caption: qsTr("Maximum boost for quiet passages. Higher values risk amplifying noise.")
                onLiveChanged: bridge.setLevellerMaxGain(v, true)
                onCommitted: bridge.setLevellerMaxGain(v)
            }
            Divider {}

            ParamRow {
                label: qsTr("Gate Threshold"); unit: qsTr("dB"); from: -96; to: 0; stepSize: 1; decimals: 1
                value: bridge.levellerGate
                caption: qsTr("Silence gate. Signals below this level are not boosted, preventing noise amplification.")
                onLiveChanged: bridge.setLevellerGate(v, true)
                onCommitted: bridge.setLevellerGate(v)
            }
            Divider {}

            Item {
                width: parent.width
                height: 34
                Column {
                    anchors.left: parent.left
                    anchors.right: lookSwitch.left
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 2
                    Text { text: qsTr("Lookahead"); font.pixelSize: 13; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9) }
                    Text { width: parent.width; wrapMode: Text.WordWrap; text: qsTr("Adds 5 ms latency. Improves transient handling."); font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5) }
                }
                ToggleSwitch {
                    id: lookSwitch
                    Accessible.name: qsTr("Lookahead")
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    checked: bridge.levellerLookahead
                    onToggled: bridge.setLevellerLookahead(checked)
                }
            }
        }
    }

    ActionMenu {
        id: presetMenu
        parent: Overlay.overlay
        items: [
            { key: "all", text: qsTr("All Channels (Night Mode)") },
            { key: "center", text: qsTr("Center Only (Dialogue Boost)") },
            { key: "front", text: qsTr("Front L / R Only") }
        ]
        onTriggered: {
            if (key === "all") bridge.setLevellerMasks(0xFF, 0xFF)
            else if (key === "center") bridge.setLevellerMasks(0x04, 0x04)
            else if (key === "front") bridge.setLevellerMasks(0x03, 0x03)
        }
    }
}
