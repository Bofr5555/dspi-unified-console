import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP9 -- Stats for nerbs (the Mac's StatsView in its "System Statistics"
// window, 960×660, at least 900×480): device and system facts, the audio
// paths' counters, ring health, PDM, DMA starvation and the buffers' fill
// with their history; a third column for the interfaces the device has; and
// the desktop's clock diagnostics when the firmware was built with them.
ToolWindow {
    id: window
    objectName: "StatsWindow"
    windowId: "stats"
    defaultWidth: 960
    defaultHeight: 660
    minimumWidth: 900
    minimumHeight: 480
    title: qsTr("System Statistics")

    readonly property bool thirdColumn: StatsController.spdifInput.length > 0 || StatsController.lgSoundSync.length > 0
                                        || StatsController.adatOutput.length > 0 || StatsController.i2sSlave.length > 0
                                        || StatsController.clockDiagnostics.length > 0
                                        || StatsController.usbErrors.length > 0

    onVisibleChanged: StatsController.windowShown = visible
    Component.onCompleted: StatsController.windowShown = visible

    function bandColor(band) {
        if (band === "good") return Theme.success
        if (band === "warn") return Theme.warning
        if (band === "bad") return Theme.dangerText
        if (band === "off") return Theme.textMuted
        return Theme.text
    }
    function seriesColor(name) {
        if (name === "pdm") return ChannelPalette.pdm
        return ChannelPalette.output(parseInt(name.split(" ")[1]), false)
    }

    // A section's heading
    component Heading: Text {
        font: Theme.fontHeading
        color: Theme.textMuted
        Layout.topMargin: 2
    }
    component Rule: Rectangle {
        Layout.fillWidth: true
        implicitHeight: 1
        color: Theme.separator
    }
    // A label and a value (the Mac's SystemInfoRow), or with a state dot
    component InfoRow: RowLayout {
        id: info
        property string label
        property string value
        property string band
        property bool dot: false
        property string tip
        property string shownName: value
        Layout.fillWidth: true
        spacing: 4
        Text {
            text: info.label
            font: Qt.font({ pixelSize: Theme.fontSmall.pixelSize, weight: Font.Medium })
            color: Theme.text
            Layout.fillWidth: true
            elide: Text.ElideRight
        }
        Rectangle {
            visible: info.dot
            width: 6
            height: 6
            radius: 3
            color: window.bandColor(info.band)
        }
        Text {
            text: info.value
            font: info.dot ? Theme.fontSmall : Qt.font({ family: Theme.fontMono.family, pixelSize: 12, weight: Font.Bold })
            color: info.dot ? Theme.textMuted : window.bandColor(info.band)
        }
        HoverHandler { id: infoHover }
        HelpTip {
            text: info.tip
            visible: infoHover.hovered && info.tip.length > 0
        }
    }
    // Rows from the controller's lists
    component Rows: ColumnLayout {
        id: rowsBlock
        property var model: []
        property string prefix
        Layout.fillWidth: true
        spacing: 6
        Repeater {
            model: rowsBlock.model
            InfoRow {
                required property var modelData
                required property int index
                objectName: rowsBlock.prefix + "." + index
                label: modelData.label
                value: modelData.text
                band: modelData.band
                dot: modelData.dot === true
                tip: modelData.tip || ""
            }
        }
    }
    // Over and under counters (the Mac's StatRow)
    component CounterRow: RowLayout {
        id: counter
        property string label
        property string subtitle
        property var counts: ({ over: -1, under: -1 })
        Layout.fillWidth: true
        spacing: Theme.spacing
        ColumnLayout {
            spacing: 2
            Layout.fillWidth: true
            Text {
                text: counter.label
                font: Qt.font({ pixelSize: Theme.fontSmall.pixelSize, weight: Font.Medium })
                color: Theme.text
            }
            Text {
                text: counter.subtitle
                font: Theme.fontCaption
                color: Theme.textMuted
            }
        }
        Repeater {
            model: [{ key: "over", caption: qsTr("over"), band: "warn" }, { key: "under", caption: qsTr("under"), band: "bad" }]
            ColumnLayout {
                required property var modelData
                readonly property real count: counter.counts ? counter.counts[modelData.key] : -1
                visible: count >= 0
                spacing: 0
                Text {
                    Layout.alignment: Qt.AlignHCenter
                    text: String(parent.count)
                    font: Qt.font({ family: Theme.fontMono.family, pixelSize: 14, weight: Font.Bold })
                    color: parent.count > 0 ? window.bandColor(modelData.band) : Theme.text
                }
                Text {
                    Layout.alignment: Qt.AlignHCenter
                    text: modelData.caption
                    font: Qt.font({ pixelSize: 9 })
                    color: Theme.textMuted
                }
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Flickable {
            id: flick
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: width
            contentHeight: columns.implicitHeight + 32
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}

            RowLayout {
                id: columns
                y: 16
                width: flick.width
                spacing: 0

                // ---- Column 1 ----
                ColumnLayout {
                    Layout.preferredWidth: 1
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignTop
                    Layout.leftMargin: Theme.spacingLarge
                    Layout.rightMargin: Theme.spacingLarge
                    spacing: Theme.spacing

                    Heading { text: qsTr("DEVICE INFORMATION") }
                    InfoRow { objectName: "StatsWindow.platform"; label: qsTr("Platform"); value: StatsController.device.platform }
                    InfoRow { objectName: "StatsWindow.firmware"; label: qsTr("Firmware"); value: StatsController.device.firmware }
                    InfoRow { objectName: "StatsWindow.serial"; label: qsTr("Serial"); value: StatsController.device.serial }
                    InfoRow { objectName: "StatsWindow.reconnects"; label: qsTr("Reconnects"); value: StatsController.device.reconnects }
                    Rule {}
                    Heading { text: qsTr("SYSTEM INFORMATION") }
                    InfoRow { objectName: "StatsWindow.clock"; label: qsTr("Clock Frequency"); value: StatsController.system.clock }
                    InfoRow { objectName: "StatsWindow.coreVoltage"; label: qsTr("Core Voltage"); value: StatsController.system.coreVoltage }
                    InfoRow { objectName: "StatsWindow.sampleRate"; label: qsTr("Sample Rate"); value: StatsController.system.sampleRate }
                    InfoRow { objectName: "StatsWindow.temperature"; label: qsTr("Temperature"); value: StatsController.system.temperature }
                    Rule {}
                    Heading { text: qsTr("AUDIO OUTPUT") }
                    CounterRow {
                        objectName: "StatsWindow.usbRing"
                        label: qsTr("USB Ring")
                        subtitle: qsTr("ISR → Main Loop")
                        counts: StatsController.counters.usbRing
                    }
                    CounterRow {
                        objectName: "StatsWindow.bufferPool"
                        label: qsTr("Buffer Pool")
                        subtitle: qsTr("USB → DMA")
                        counts: StatsController.counters.spdif
                    }
                    Rule { visible: StatsController.ringHealth.length > 0 }
                    Heading {
                        objectName: "StatsWindow.ringHeading"
                        visible: StatsController.ringHealth.length > 0
                        text: qsTr("OUTPUT RING HEALTH")
                    }
                    Rows {
                        prefix: "StatsWindow.ring"
                        visible: StatsController.ringHealth.length > 0
                        model: StatsController.ringHealth
                    }
                    Rule {}
                    Heading { text: qsTr("PDM (SUBWOOFER)") }
                    CounterRow {
                        label: qsTr("Ring Buffer")
                        subtitle: qsTr("Core 0 → Core 1")
                        counts: StatsController.counters.pdmRing
                    }
                    CounterRow {
                        label: qsTr("DMA Buffer")
                        subtitle: qsTr("Core 1 → PIO")
                        counts: StatsController.counters.pdmDma
                    }
                }

                Rectangle { Layout.fillHeight: true; implicitWidth: 1; color: Theme.separator }

                // ---- Column 2 ----
                ColumnLayout {
                    Layout.preferredWidth: 1
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignTop
                    Layout.leftMargin: Theme.spacingLarge
                    Layout.rightMargin: Theme.spacingLarge
                    spacing: Theme.spacing

                    Heading { text: qsTr("SPDIF DMA STARVATION") }
                    RowLayout {
                        objectName: "StatsWindow.starvation"
                        readonly property real total: StatsController.starvation.total
                        readonly property real delta: StatsController.starvation.delta
                        property string shownName: String(total)
                        Layout.fillWidth: true
                        Text {
                            text: qsTr("Total")
                            font: Qt.font({ pixelSize: Theme.fontSmall.pixelSize, weight: Font.Medium })
                            color: Theme.text
                            Layout.fillWidth: true
                        }
                        Rectangle {
                            visible: parent.delta > 0
                            implicitWidth: deltaText.implicitWidth + 8
                            implicitHeight: deltaText.implicitHeight + 2
                            radius: 3
                            color: Qt.alpha(Theme.danger, 0.15)
                            Text {
                                id: deltaText
                                anchors.centerIn: parent
                                text: "+" + parent.parent.delta
                                font: Qt.font({ family: Theme.fontMono.family, pixelSize: 10, weight: Font.Bold })
                                color: Theme.dangerText
                            }
                        }
                        Text {
                            text: String(parent.total)
                            font: Qt.font({ family: Theme.fontMono.family, pixelSize: 14, weight: Font.Bold })
                            color: parent.total > 0 ? Theme.dangerText : Theme.text
                        }
                    }
                    Repeater {
                        model: StatsController.starvation.slots
                        RowLayout {
                            required property var modelData
                            Layout.fillWidth: true
                            Text {
                                text: modelData.label
                                font: Theme.fontSmall
                                color: Theme.textMuted
                                Layout.fillWidth: true
                            }
                            Text {
                                text: String(modelData.count)
                                font: Qt.font({ family: Theme.fontMono.family, pixelSize: 12, weight: Font.Medium })
                                color: modelData.count > 0 ? Theme.dangerText : Theme.text
                            }
                        }
                    }
                    // The event times: "since" ticks every second while shown
                    RowLayout {
                        Layout.fillWidth: true
                        property real now: Date.now()
                        Timer {
                            interval: 1000
                            repeat: true
                            running: window.visible && StatsController.starvation.lastEventMs > 0
                            onTriggered: parent.now = Date.now()
                        }
                        Text {
                            text: qsTr("Time since last event")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                            Layout.fillWidth: true
                        }
                        Text {
                            readonly property real last: StatsController.starvation.lastEventMs
                            text: last > 0 ? StatsController.formatElapsed(parent.now - last) : "-"
                            font: Qt.font({ family: Theme.fontMono.family, pixelSize: 10 })
                            color: Theme.textMuted
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        Text {
                            text: qsTr("Time between last two")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                            Layout.fillWidth: true
                        }
                        Text {
                            readonly property real last: StatsController.starvation.lastEventMs
                            readonly property real previous: StatsController.starvation.previousEventMs
                            text: last > 0 && previous > 0 ? StatsController.formatElapsed(last - previous) : "-"
                            font: Qt.font({ family: Theme.fontMono.family, pixelSize: 10 })
                            color: Theme.textMuted
                        }
                    }

                    Rule {}
                    RowLayout {
                        Layout.fillWidth: true
                        Heading { text: qsTr("BUFFER FILL LEVELS") }
                        Item { Layout.fillWidth: true }
                        AppButton {
                            objectName: "StatsWindow.reset"
                            text: qsTr("Reset Watermarks")
                            enabled: StatsController.writable
                            onClicked: StatsController.resetWatermarks()
                        }
                    }
                    RowLayout {
                        spacing: Theme.spacing
                        Repeater {
                            model: [{ text: qsTr("Audio Streaming"), on: StatsController.streaming },
                                    { text: qsTr("PDM Active"), on: StatsController.pdmActive }]
                            RowLayout {
                                required property var modelData
                                spacing: 4
                                Rectangle {
                                    width: 6
                                    height: 6
                                    radius: 3
                                    color: modelData.on ? Theme.success : Theme.textMuted
                                }
                                Text {
                                    text: modelData.text
                                    font: Theme.fontCaption
                                    color: Theme.textMuted
                                }
                            }
                        }
                    }
                    Repeater {
                        model: StatsController.fillRows
                        StatsFillRow {
                            required property var modelData
                            required property int index
                            objectName: "StatsWindow.fill." + index
                            Layout.fillWidth: true
                            entry: modelData
                            tint: window.seriesColor(modelData.colour)
                            valueColor: window.bandColor(modelData.band)
                        }
                    }
                }

                Rectangle { visible: window.thirdColumn; Layout.fillHeight: true; implicitWidth: 1; color: Theme.separator }

                // ---- Column 3: the interfaces the device has, and the diagnostics ----
                ColumnLayout {
                    visible: window.thirdColumn
                    Layout.preferredWidth: 1
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignTop
                    Layout.leftMargin: Theme.spacingLarge
                    Layout.rightMargin: Theme.spacingLarge
                    spacing: Theme.spacing

                    ColumnLayout {
                        objectName: "StatsWindow.spdifInput"
                        visible: StatsController.spdifInput.length > 0
                        Layout.fillWidth: true
                        spacing: Theme.spacing
                        Heading { text: qsTr("S/PDIF INPUT") }
                        Rows { prefix: "StatsWindow.spdif"; model: StatsController.spdifInput }
                        Rule { visible: StatsController.channelStatus.length > 0 }
                        Heading { visible: StatsController.channelStatus.length > 0; text: qsTr("CHANNEL STATUS") }
                        Rows { model: StatsController.channelStatus }
                        Rule { visible: StatsController.spdifDebug.length > 0 }
                        Heading { visible: StatsController.spdifDebug.length > 0; text: qsTr("DEBUG") }
                        Rows { model: StatsController.spdifDebug }
                    }
                    ColumnLayout {
                        objectName: "StatsWindow.lg"
                        visible: StatsController.lgSoundSync.length > 0
                        Layout.fillWidth: true
                        spacing: Theme.spacing
                        Rule { visible: StatsController.spdifInput.length > 0 }
                        Heading { text: qsTr("LG SOUND SYNC") }
                        Rows { model: StatsController.lgSoundSync }
                    }
                    ColumnLayout {
                        objectName: "StatsWindow.adat"
                        visible: StatsController.adatOutput.length > 0
                        Layout.fillWidth: true
                        spacing: Theme.spacing
                        Rule { visible: StatsController.spdifInput.length > 0 || StatsController.lgSoundSync.length > 0 }
                        Heading { text: qsTr("ADAT BULK OUTPUT") }
                        Rows { model: StatsController.adatOutput }
                    }
                    ColumnLayout {
                        objectName: "StatsWindow.i2s"
                        visible: StatsController.i2sSlave.length > 0
                        Layout.fillWidth: true
                        spacing: Theme.spacing
                        Rule {
                            visible: StatsController.spdifInput.length > 0 || StatsController.lgSoundSync.length > 0
                                     || StatsController.adatOutput.length > 0
                        }
                        Heading { text: qsTr("I2S INPUT (SLAVE CLOCK)") }
                        Rows { model: StatsController.i2sSlave }
                    }
                    // Desktop additions: shown only when the firmware has them
                    ColumnLayout {
                        objectName: "StatsWindow.clockDiagnostics"
                        visible: StatsController.clockDiagnostics.length > 0
                        Layout.fillWidth: true
                        spacing: Theme.spacing
                        Rule {
                            visible: StatsController.spdifInput.length > 0 || StatsController.lgSoundSync.length > 0
                                     || StatsController.adatOutput.length > 0 || StatsController.i2sSlave.length > 0
                        }
                        Heading { text: qsTr("CLOCK DIAGNOSTICS") }
                        Text {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: qsTr("Shown because this firmware was built with clock diagnostics. Values as the device reports them, latched once a second.")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                        }
                        Repeater {
                            model: StatsController.clockDiagnostics
                            InfoRow {
                                required property var modelData
                                label: modelData.name
                                value: modelData.value
                            }
                        }
                    }
                    ColumnLayout {
                        objectName: "StatsWindow.usbErrors"
                        visible: StatsController.usbErrors.length > 0
                        Layout.fillWidth: true
                        spacing: Theme.spacing
                        Rule {}
                        Heading { text: qsTr("USB ERRORS") }
                        Text {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: qsTr("Shown because the device has counted USB errors.")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                        }
                        Rows { model: StatsController.usbErrors }
                    }
                }
            }
        }

        Rule {}
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
            Layout.topMargin: Theme.spacingSmall
            Layout.bottomMargin: Theme.spacingSmall
            spacing: 4
            Rectangle {
                width: 6
                height: 6
                radius: 3
                color: StatsController.connected ? Theme.connected : Theme.disconnected
            }
            Text {
                objectName: "StatsWindow.connection"
                property string shownName: text
                text: StatsController.connected ? qsTr("Connected") : qsTr("Disconnected")
                font: Theme.fontCaption
                color: Theme.textMuted
            }
            Item { Layout.fillWidth: true }
            Text {
                text: qsTr("Updated every 2 seconds")
                font: Theme.fontCaption
                color: Theme.textMuted
            }
        }
    }
}
