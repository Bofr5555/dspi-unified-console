import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- Headphone crossfeed (the Mac's CrossfeedView, 780×470, at least
// 740×460): the header with the switch; on the left the response of the
// direct and opposite paths over the presets that set it; on the right the
// custom parameters, the interaural delay and the output pairs, with the
// pair map on firmware that has it. While a preset other than Custom is
// chosen the parameter rows are dimmed and show that preset's own values;
// moving one makes the preset Custom.
// -- WP DL1 -- Modern (DESIGN-LANGUAGE): the intro row, the graph in a
// GraphFrame, no lines between the parts, the pairs as option chips, the
// two columns at most 1200 wide from the left of the page.
// -- WP LAY1a -- A panel (Panel.qml): PanelTable's row for
// tools.crossfeed gives its window's title and sizes.
Panel {
    id: panel
    objectName: "CrossfeedPanel"
    // -- WP QTPOP --
    fitBodies: [body]

    readonly property bool writable: EffectsController.writable
    readonly property bool custom: EffectsController.crossfeedPreset === EffectsController.crossfeedCustomPreset
    readonly property bool showPairMap: ExtController.hasPairMap
    readonly property bool showPairs: EffectsController.crossfeedMaskSupported || showPairMap
    // For the smoke driver
    readonly property alias pairMap: pairMap
    readonly property alias editingPairs: pairMap.editing

    // "1" without a pair map, "1: Out 3 / Out 1" with one
    function pairLabel(p) {
        if (!showPairMap) return String(p + 1)
        const pr = ExtController.pairs[p]
        return pr ? qsTr("%1: Out %2 / Out %3").arg(p + 1).arg(pr.left + 1).arg(pr.right + 1) : String(p + 1)
    }
    function pairTip(p) {
        EffectsController.revision
        const pr = showPairMap ? ExtController.pairs[p] : null
        const l = pr ? pr.left : 2 * p, r = pr ? pr.right : 2 * p + 1
        return EffectsController.outputName(l) + " / " + EffectsController.outputName(r)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        EffectHeader {
            objectName: "CrossfeedWindow.header"
            touchPath: "cf."   // -- WP AIVIS --
            Layout.fillWidth: true
            iconName: "headphones"
            title: qsTr("Crossfeed")
            subtitle: qsTr("BS2B Bauer Stereophonic-to-Binaural")
            on: EffectsController.crossfeedOn
            switchEnabled: panel.writable
            onToggled: on => EffectsController.setEnabled("crossfeed", on)
        }
        EffectRule {}

        Flickable {
            id: body
            Layout.fillWidth: true
            Layout.fillHeight: true
            readonly property int topGap: LayoutController.modern ? Theme.spacingSection : Theme.spacingLarge   // -- WP DL1 --
            contentHeight: columns.implicitHeight + topGap + Theme.spacingLarge
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}

            // Opened again, the panel starts at the top, as the Mac's does
            Connections {
                target: panel
                function onShownChanged() { if (panel.shown) body.contentY = 0 }
            }

            RowLayout {
                id: columns
                x: 0
                y: body.topGap
                width: LayoutController.modern ? Math.min(parent.width, 1200) : parent.width   // -- WP DL1 --
                spacing: 0

                // ---- Left: the response and the presets ----
                ColumnLayout {
                    Layout.preferredWidth: columns.width / 2
                    Layout.alignment: Qt.AlignTop
                    Layout.leftMargin: Theme.spacingLarge
                    Layout.rightMargin: Theme.spacingLarge
                    spacing: 14

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 6
                        SectionHeading { text: qsTr("Frequency Response") }
                        EffectGraph {
                            id: graph
                            objectName: "CrossfeedWindow.graph"
                            Layout.fillWidth: true
                            Layout.preferredHeight: LayoutController.modern ? 200 : 160   // -- WP DL1 --
                            title: qsTr("Frequency Response")
                            active: EffectsController.crossfeedOn
                            readonly property var graphData: EffectsController.crossfeedGraph(
                                isNaN(freqRow.liveValue) ? EffectsController.crossfeedShownFreq : freqRow.liveValue,
                                isNaN(feedRow.liveValue) ? EffectsController.crossfeedShownFeed : feedRow.liveValue)
                            curves: [
                                { points: graphData.crossfeed, color: Theme.warning },
                                { points: graphData.direct, color: Theme.accent }
                            ]
                            gridDb: graphData.gridDb
                            gridY: graphData.gridY
                            legend: [
                                { color: Theme.accent, label: qsTr("Direct") },
                                { color: Theme.warning, label: qsTr("Crossfeed") }
                            ]
                        }
                    }
                    EffectRule {}
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 10
                        SectionHeading { text: qsTr("Preset") }
                        Repeater {
                            model: EffectsController.crossfeedPresets
                            AbstractButton {
                                id: presetRow
                                required property int index
                                required property var modelData
                                readonly property bool selected: EffectsController.crossfeedPreset === index
                                objectName: "CrossfeedWindow.preset." + index
                                Layout.fillWidth: true
                                implicitHeight: presetContent.implicitHeight
                                enabled: panel.writable
                                hoverEnabled: true
                                focusPolicy: Qt.StrongFocus
                                onClicked: EffectsController.setCrossfeedPreset(index)
                                contentItem: RowLayout {
                                    id: presetContent
                                    spacing: 10
                                    Rectangle {
                                        Layout.alignment: Qt.AlignVCenter
                                        width: 14
                                        height: 14
                                        radius: 7
                                        color: "transparent"
                                        border.width: presetRow.visualFocus ? 2 : 1.5
                                        border.color: presetRow.visualFocus ? Theme.focusRing
                                                      : presetRow.selected ? Theme.accent : Theme.textMuted
                                        Rectangle {
                                            anchors.centerIn: parent
                                            visible: presetRow.selected
                                            width: 8
                                            height: 8
                                            radius: 4
                                            color: Theme.accent
                                        }
                                    }
                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 1
                                        Text {
                                            text: presetRow.modelData.name
                                            font: Theme.fontBody
                                            color: presetRow.enabled ? Theme.text : Theme.textDisabled
                                        }
                                        Text {
                                            Layout.fillWidth: true
                                            text: presetRow.modelData.description
                                            font: Theme.fontCaption
                                            color: Theme.textMuted
                                            elide: Text.ElideRight
                                        }
                                    }
                                }
                                background: Item {}
                                Accessible.role: Accessible.RadioButton
                                Accessible.name: modelData.name
                                Accessible.description: modelData.description
                                Accessible.checkable: true
                                Accessible.checked: selected
                            }
                        }
                    }
                }

                EffectRule { vertical: true }

                // ---- Right: parameters, the delay, the output pairs ----
                ColumnLayout {
                    Layout.preferredWidth: columns.width / 2
                    Layout.alignment: Qt.AlignTop
                    Layout.leftMargin: Theme.spacingLarge
                    Layout.rightMargin: Theme.spacingLarge
                    spacing: 14

                    ColumnLayout {
                        objectName: "CrossfeedWindow.parameters"
                        Layout.fillWidth: true
                        spacing: 14
                        opacity: panel.custom ? 1 : Theme.dim(0.5)
                        SectionHeading { text: qsTr("Parameters") }
                        EffectParameterRow {
                            id: freqRow
                            objectName: "CrossfeedWindow.freq"
                            Layout.fillWidth: true
                            enabled: panel.writable
                            title: qsTr("Cutoff Frequency")
                            unit: qsTr("Hz")
                            value: EffectsController.crossfeedShownFreq
                            from: 500
                            to: 2000
                            resetValue: 700
                            caption: qsTr("Simulates head shadow lowpass cutoff. Lower = more bass crossfeed. Typical: 650-700 Hz.")
                            onLive: v => EffectsController.setCrossfeedFreq(v, true)
                            onCommit: v => EffectsController.setCrossfeedFreq(v)
                        }
                        EffectRule {}
                        EffectParameterRow {
                            id: feedRow
                            objectName: "CrossfeedWindow.feed"
                            Layout.fillWidth: true
                            enabled: panel.writable
                            title: qsTr("Feed Level")
                            unit: qsTr("dB")
                            value: EffectsController.crossfeedShownFeed
                            from: 0
                            to: 15
                            resetValue: 4.5
                            caption: qsTr("Crossfeed attenuation below direct signal. Higher = more crossfeed. Typical: 4.5-9.5 dB.")
                            onLive: v => EffectsController.setCrossfeedFeed(v, true)
                            onCommit: v => EffectsController.setCrossfeedFeed(v)
                        }
                    }
                    EffectRule {}
                    RowLayout {
                        Layout.fillWidth: true
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2
                            Text {
                                text: qsTr("Interaural Time Delay")
                                font: Theme.fontBody
                                color: Theme.text
                            }
                            Text {
                                Layout.fillWidth: true
                                text: qsTr("Simulates ~220 µs path difference via all-pass filter")
                                font: Theme.fontCaption
                                color: Theme.textMuted
                                wrapMode: Text.Wrap
                            }
                        }
                        AppSwitch {
                            objectName: "CrossfeedWindow.itd"
                            enabled: panel.writable
                            checked: EffectsController.crossfeedItd
                            onToggled: EffectsController.setCrossfeedItd(checked)
                            Accessible.name: qsTr("Interaural Time Delay")
                        }
                    }

                    EffectRule { visible: panel.showPairs }
                    ColumnLayout {
                        objectName: "CrossfeedWindow.pairs"
                        visible: panel.showPairs
                        Layout.fillWidth: true
                        spacing: 10
                        EffectSection {
                            text: qsTr("Output Pairs")
                            EffectMenuButton {
                                objectName: "CrossfeedWindow.pairs.presets"
                                enabled: panel.writable && EffectsController.crossfeedMaskSupported
                                text: qsTr("Presets")
                                AppMenuItem {
                                    objectName: "CrossfeedWindow.pairs.all"
                                    text: qsTr("All pairs")
                                    onTriggered: EffectsController.setCrossfeedPairMask(EffectsController.allPairsMask())
                                }
                                AppMenuItem {
                                    objectName: "CrossfeedWindow.pairs.first"
                                    text: qsTr("Pair 1 only (Headphones)")
                                    onTriggered: EffectsController.setCrossfeedPairMask(0x01)
                                }
                                AppMenuItem {
                                    objectName: "CrossfeedWindow.pairs.none"
                                    text: qsTr("None")
                                    onTriggered: EffectsController.setCrossfeedPairMask(0x00)
                                }
                            }
                        }
                        Text {
                            Layout.fillWidth: true
                            text: qsTr("Crossfeed only the stereo output pairs feeding headphones. Speaker pairs stay bit-accurate. The mono sub is never crossfed.")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                            wrapMode: Text.Wrap
                        }
                        CrossfeedPairMap {
                            id: pairMap
                            visible: panel.showPairMap
                            Layout.fillWidth: true
                        }
                        // One chip per pair: two columns with the map, one row without
                        // (-- WP DL1 -- Modern: as wide as their labels, from the left)
                        EffectChipRow {
                            objectName: panel.showPairMap ? "CrossfeedWindow.pairChips" : "CrossfeedWindow.plainPairChips"
                            Layout.fillWidth: true
                            classicColumns: panel.showPairMap ? 2 : Math.max(1, EffectsController.crossfeedPairCount)
                            Repeater {
                                model: EffectsController.crossfeedPairCount
                                EffectChip {
                                    required property int index
                                    objectName: "CrossfeedWindow.pair." + index
                                    enabled: panel.writable && EffectsController.crossfeedMaskSupported
                                    on: (EffectsController.crossfeedPairMask & (1 << index)) !== 0
                                    text: panel.pairLabel(index)
                                    toolTip: panel.pairTip(index)
                                    onClicked: EffectsController.toggleCrossfeedPair(index)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
