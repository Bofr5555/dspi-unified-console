import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP9 -- The Spectrum Analyser window (the Mac's SpectrumAnalyserWindow,
// 760×560, at least 620×420): the page's spectrum (the dashboard's or the
// open channel page's) as curves, bars or both, a legend that hides channels
// here only, and the analyser's status bar with "Following another console"
// and Take control. It shows only what the page already shows; it asks for
// the analyser while it is on screen.
ToolWindow {
    id: window
    objectName: "SpectrumWindow"
    windowId: "spectrum"
    defaultWidth: 760
    defaultHeight: 560
    minimumWidth: 620
    minimumHeight: 420
    title: qsTr("Spectrum Analyser")

    // 0 Curves, 1 Bars, 2 Both: taken from the page on every show
    property int mode: 0
    // Channels hidden here, as "tap:channel"
    property var hidden: ({})
    readonly property bool available: RtaController.available && (RtaController.showGraph || RtaController.showBars)
                                      && (RtaController.ownChannels.length > 0 || RtaController.following)
    readonly property var drawn: RtaController.drawnChannels
    readonly property var visibleChannels: drawn.filter(c => !hidden[RtaController.drawnTap + ":" + c])
    readonly property bool rendering: visible && visibility !== Window.Minimized
    readonly property bool curvesOn: rendering && available && mode !== 1 && visibleChannels.length > 0
    readonly property bool barsOn: rendering && available && mode !== 0 && visibleChannels.length > 0

    onVisibleChanged: {
        if (!visible) return
        mode = RtaController.showGraph && RtaController.showBars ? 2 : RtaController.showBars ? 1 : 0
        RtaController.openAnalyser()
    }
    onCurvesOnChanged: RtaController.setView("window.curves", RtaController.Curves, curvesOn)
    onBarsOnChanged: RtaController.setView("window.bars", RtaController.Bars, barsOn)
    // Hidden channels no longer drawn are forgotten
    onDrawnChanged: {
        const keep = {}
        for (const c of drawn) {
            const k = RtaController.drawnTap + ":" + c
            if (hidden[k]) keep[k] = true
        }
        hidden = keep
    }

    function infoOf(ch) { DeviceModel.revision; return RtaController.channelInfo(RtaController.drawnTap, ch) }
    function colorOf(ch) { const i = infoOf(ch); return ChannelPalette.forRow(i.kind, i.number, i.pdm) }
    function toggleHidden(ch) {
        const k = RtaController.drawnTap + ":" + ch
        const next = Object.assign({}, hidden)
        if (next[k]) delete next[k]
        else next[k] = true
        hidden = next
    }

    component Notice: ColumnLayout {
        property string icon
        property string headline
        property string caption
        spacing: Theme.spacingSmall
        Item { Layout.fillHeight: true }
        Icon {
            Layout.alignment: Qt.AlignHCenter
            name: parent.icon
            size: 30
            color: Theme.textMuted
        }
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: parent.headline
            font: Theme.fontTitle
            color: Theme.text
        }
        Text {
            Layout.alignment: Qt.AlignHCenter
            Layout.maximumWidth: 360
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            text: parent.caption
            font: Theme.fontSmall
            color: Theme.textMuted
        }
        Item { Layout.fillHeight: true }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Notice {
            objectName: "SpectrumWindow.unavailable"
            visible: !RtaController.available
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: 40
            icon: "triangle-alert"
            headline: qsTr("Spectrum analyser unavailable")
            caption: !RtaController.ready ? qsTr("Connect a DSPi to use the analyser.")
                     : RtaController.readOnlyRefused ? qsTr("The analyser is not available while the device is read-only. Update the firmware to use it.")
                     : qsTr("The connected firmware does not provide a compatible analyser. Update the firmware to use it.")
        }

        // ---- Header ----
        RowLayout {
            visible: RtaController.available
            Layout.fillWidth: true
            Layout.preferredHeight: 32
            Layout.leftMargin: Theme.spacing
            Layout.rightMargin: Theme.spacing
            spacing: 10
            Text {
                objectName: "SpectrumWindow.page"
                property string shownName: text
                text: { DeviceModel.revision; return RtaController.pageTitle }
                font: Qt.font({ pixelSize: Theme.fontSmall.pixelSize, weight: Font.DemiBold })
                color: Theme.text
            }
            Text {
                visible: window.available
                text: RtaController.drawnTap === 0 ? qsTr("Inputs") : qsTr("Outputs")
                font: Theme.fontSmall
                color: Theme.textMuted
            }
            Rectangle {
                visible: window.available
                implicitWidth: 1
                implicitHeight: 12
                color: Theme.separator
            }
            // The legend: a click hides or shows a channel here only
            Flickable {
                visible: window.available
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                Layout.preferredWidth: 0
                Layout.preferredHeight: 20
                contentWidth: legend.implicitWidth
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                Row {
                    id: legend
                    spacing: 10
                    anchors.verticalCenter: parent.verticalCenter
                    Repeater {
                        model: window.drawn
                        AbstractButton {
                            id: entry
                            required property int modelData
                            readonly property bool isHidden: window.hidden[RtaController.drawnTap + ":" + modelData] === true
                            objectName: "SpectrumWindow.legend." + modelData
                            implicitWidth: entryRow.implicitWidth
                            implicitHeight: 18
                            hoverEnabled: true
                            focusPolicy: Qt.TabFocus
                            contentItem: Row {
                                id: entryRow
                                spacing: 4
                                Rectangle {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 6
                                    height: 6
                                    radius: 3
                                    color: window.colorOf(entry.modelData)
                                    opacity: entry.isHidden ? 0.25 : 1
                                }
                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: window.infoOf(entry.modelData).name
                                    font: Qt.font({ pixelSize: 10, strikeout: entry.isHidden })
                                    color: Theme.textMuted
                                    opacity: entry.isHidden ? 0.4 : 1
                                }
                            }
                            background: Rectangle {
                                color: "transparent"
                                border.width: entry.visualFocus ? 1 : 0
                                border.color: Theme.focusRing
                                radius: 3
                            }
                            HelpTip {
                                text: entry.isHidden ? qsTr("Show in this window") : qsTr("Hide in this window")
                                visible: entry.hovered
                            }
                            Accessible.role: Accessible.CheckBox
                            Accessible.name: window.infoOf(entry.modelData).name
                            Accessible.checked: !isHidden
                            onClicked: window.toggleHidden(modelData)
                        }
                    }
                }
            }
            Item { visible: !window.available; Layout.fillWidth: true }
            AppTabBar {
                objectName: "SpectrumWindow.mode"
                label: qsTr("View")
                enabled: window.available
                currentIndex: window.mode
                ToolTip.visible: hovered
                ToolTip.text: qsTr("How this window draws the page's spectrum. It does not change the main window.")
                AppTabButton { text: qsTr("Curves"); onClicked: window.mode = 0 }
                AppTabButton { text: qsTr("Bars"); onClicked: window.mode = 1 }
                AppTabButton { text: qsTr("Both"); onClicked: window.mode = 2 }
            }
        }
        Rectangle { visible: RtaController.available; Layout.fillWidth: true; implicitHeight: 1; color: Theme.separator }

        // ---- Content ----
        Notice {
            objectName: "SpectrumWindow.hidden"
            visible: RtaController.available && !window.available
            Layout.fillWidth: true
            Layout.fillHeight: true
            icon: "eye-off"
            headline: RtaController.ownChannels.length === 0 ? qsTr("No channels selected") : qsTr("Spectrum switched off")
            caption: RtaController.onDashboard
                     ? qsTr("This window shows the spectrum of the dashboard. Choose channels and switch the spectrum on from the gear on the response graph.")
                     : qsTr("This window shows the spectrum of the open channel page. Choose channels and switch the spectrum on from the gear on the response graph.")
        }
        Rectangle {
            visible: RtaController.available && window.available
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Theme.panel

            Notice {
                objectName: "SpectrumWindow.allHidden"
                anchors.fill: parent
                visible: window.visibleChannels.length === 0
                icon: "eye-off"
                headline: qsTr("Every channel is hidden in this window")
                caption: qsTr("Click a channel name above to show it again.")
            }

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.spacing
                spacing: Theme.spacing
                visible: window.visibleChannels.length > 0

                RtaSurface {
                    objectName: "SpectrumWindow.curves"
                    visible: window.mode !== 1
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredHeight: 1
                    active: window.curvesOn
                    grid: true
                    tap: RtaController.drawnTap
                    channels: window.visibleChannels
                    colors: window.visibleChannels.map(c => window.colorOf(c))
                    minFreq: GraphController.minFreq
                    maxFreq: GraphController.maxFreq
                    glow: GraphController.showGraphGlow
                    gridColor: Theme.text
                    labelColor: Theme.textMuted
                    labelFont: Qt.font({ family: Theme.fontMono.family, pixelSize: 8 })
                }

                GridLayout {
                    id: barGrid
                    objectName: "SpectrumWindow.bars"
                    readonly property int n: window.visibleChannels.length
                    visible: window.mode !== 0
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredHeight: 1
                    columns: Math.min(Math.max(n, 1), Math.max(1, Math.min(4, RtaController.prefs.barColumns)))
                    columnSpacing: Theme.spacing
                    rowSpacing: Theme.spacing
                    Repeater {
                        model: window.visibleChannels
                        ColumnLayout {
                            id: barCell
                            required property int modelData
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.preferredWidth: 1
                            spacing: 3
                            RowLayout {
                                spacing: 4
                                Rectangle {
                                    width: 6
                                    height: 6
                                    radius: 3
                                    color: window.colorOf(barCell.modelData)
                                }
                                Text {
                                    text: window.infoOf(barCell.modelData).name
                                    font: Qt.font({ pixelSize: 10, weight: Font.DemiBold })
                                    color: Theme.textMuted
                                    elide: Text.ElideRight
                                }
                            }
                            RtaBars {
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                active: window.barsOn
                                tap: RtaController.drawnTap
                                channel: barCell.modelData
                                tint: window.colorOf(barCell.modelData)
                                frequencyLabels: true
                                dbLabels: true
                                gridColor: Theme.text
                                labelColor: Theme.textMuted
                                labelFont: Qt.font({ family: Theme.fontMono.family, pixelSize: 8 })
                            }
                        }
                    }
                    Repeater {
                        model: barGrid.n > 0 ? (barGrid.columns - barGrid.n % barGrid.columns) % barGrid.columns : 0
                        Item { Layout.fillWidth: true; Layout.preferredWidth: 1 }
                    }
                }
            }
        }

        // ---- Status bar ----
        Rectangle { visible: RtaController.available; Layout.fillWidth: true; implicitHeight: 1; color: Theme.separator }
        RowLayout {
            objectName: "SpectrumWindow.status"
            visible: RtaController.available
            Layout.fillWidth: true
            Layout.leftMargin: Theme.spacing
            Layout.rightMargin: Theme.spacing
            Layout.topMargin: 6
            Layout.bottomMargin: 6
            spacing: 14
            readonly property font mono: Qt.font({ family: Theme.fontMono.family, pixelSize: 10 })

            Row {
                spacing: 4
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 6
                    height: 6
                    radius: 3
                    color: RtaController.analysing ? Theme.success : Qt.alpha(Theme.textMuted, 0.5)
                }
                Text {
                    objectName: "SpectrumWindow.running"
                    property string shownName: text
                    text: RtaController.analysing ? qsTr("Running") : qsTr("Idle")
                    font: parent.parent.mono
                    color: Theme.textMuted
                }
            }
            Text {
                objectName: "SpectrumWindow.telemetry"
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                text: RtaController.statusParts.concat(RtaController.bassText !== "" ? [RtaController.bassText] : []).join("   ")
                font: parent.mono
                color: Theme.textMuted
                elide: Text.ElideRight
                HoverHandler { id: bassHover }
                HelpTip {
                    text: qsTr("Bass processing time summed across both cores. Already included in the audio CPU meters; ≥ means the counter is saturated.")
                    visible: bassHover.hovered && RtaController.bassText !== ""
                }
            }
            // The first that applies: refused, following, the low bands, the range
            Text {
                objectName: "SpectrumWindow.rejected"
                visible: RtaController.refused
                text: qsTr("Device refused this configuration")
                font: parent.mono
                color: Theme.warning
            }
            RowLayout {
                visible: !RtaController.refused && RtaController.following
                spacing: 6
                Icon {
                    name: "users"
                    size: 11
                    color: Theme.textMuted
                }
                Text {
                    objectName: "SpectrumWindow.following"
                    property string shownName: text
                    text: qsTr("Following another console")
                    font: parent.parent.mono
                    color: Theme.textMuted
                }
                AppButton {
                    objectName: "SpectrumWindow.takeControl"
                    text: qsTr("Take control")
                    onClicked: RtaController.takeControl()
                }
            }
            Text {
                objectName: "SpectrumWindow.lowBands"
                visible: !RtaController.refused && !RtaController.following && window.available && window.mode !== 0
                         && RtaController.lowBandText !== ""
                text: RtaController.lowBandText
                font: parent.mono
                color: Theme.warning
            }
            Text {
                objectName: "SpectrumWindow.range"
                visible: !RtaController.refused && !RtaController.following
                         && !(window.available && window.mode !== 0 && RtaController.lowBandText !== "")
                         && RtaController.rangeText !== ""
                text: RtaController.rangeText
                font: parent.mono
                color: Theme.textMuted
                HoverHandler { id: rangeHover }
                HelpTip {
                    text: qsTr("FFT / bass usable dynamic range. Bass bands use overlapping filters calibrated for tones.")
                    visible: rangeHover.hovered
                }
            }
        }
    }
}
