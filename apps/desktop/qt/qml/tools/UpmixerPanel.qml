import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- The Stereo Upmixer (the Mac's UpmixerView, 400×720, 400 wide,
// 400 to 1200 tall): one scrolling column with the live status, the two
// engines, the centre's and the surround's controls (each only while its
// engine runs), and where the derived channels appear in the matrix. The
// status polls about 16 times a second while the window shows. RP2350 with
// wire V26 or newer only; anything else gets a note.
// -- WP DL1 -- Modern: with room for them (a page), two columns from the left:
// the status, the engines and the routing note on the left, the centre's and
// the surround's controls on the right; segments as wide as their labels,
// the gauges only while it runs, no lines between the parts. A narrow host
// keeps the one column.
// -- WP LAY1a -- A panel (Panel.qml): PanelTable's row for
// tools.upmixer gives its window's title and sizes.
Panel {
    id: panel
    objectName: "UpmixerPanel"
    // -- WP QTPOP --
    fitBodies: [body]

    readonly property bool writable: ToolsController.writable && ToolsController.upmixSupported
    readonly property bool unsupported: ToolsController.connected && !ToolsController.upmixSupported
    // Wire values: centre 0 Sinner, 1 Logician, 2 Off; surround 0 Off, 1 Sinner, 2 Logician
    readonly property int centerMode: value("center")
    readonly property int surroundMode: value("surround")
    readonly property bool centreOff: centerMode === 2
    readonly property bool surroundOn: surroundMode > 0
    readonly property var status: ToolsController.upmixStatus
    readonly property bool active: ToolsController.connected && status.active === true
    // -- WP DL1 -- Modern with room: two columns
    readonly property bool twoColumns: LayoutController.modern && body.width >= 720

    function value(key) { return ToolsController.revision < 0 ? 0 : ToolsController.upmix(key) }

    // -- WP QPI3 -- its polls run while it is on screen, not while minimised
    readonly property bool polling: onScreen
    onPollingChanged: ToolsController.windowShown("upmixer", polling)
    Component.onDestruction: if (shown) ToolsController.windowShown("upmixer", false)   // -- WP LAY1a -- a page host unloaded

    component UpmixRow: ToolParameterRow {
        property string key
        Layout.fillWidth: true
        enabled: panel.writable
        value: panel.value(key)
        onLive: v => ToolsController.setUpmix(key, v)
        onCommit: v => ToolsController.setUpmix(key, v)
    }

    // One live gauge: its name, the bar, the figure
    component Gauge: RowLayout {
        id: gauge
        property string label
        property real level: 0
        property string figure
        property color color: Theme.accent
        spacing: 8
        Text {
            Layout.preferredWidth: 76
            text: gauge.label
            font: Theme.fontCaption
            color: Theme.textMuted
        }
        Meter {
            objectName: gauge.objectName.length ? gauge.objectName + ".bar" : ""
            Layout.fillWidth: true
            Layout.preferredHeight: 6
            color: gauge.color
            trackColor: Theme.control
            level: gauge.level
        }
        Text {
            Layout.preferredWidth: 44
            horizontalAlignment: Text.AlignRight
            text: gauge.figure
            font: Theme.fontMono
            color: Theme.textMuted
        }
    }

    // Where the derived channels go
    component Routing: ColumnLayout {
        Layout.fillWidth: true
        spacing: 6
        SectionHeading { text: qsTr("Routing") }
        Text {
            Layout.fillWidth: true
            text: qsTr("The derived channels appear as matrix source rows: row 2 = Centre, row 3 = Left Surround, row 4 = Right Surround. Open the Matrix Mixer to route them to your output slots (a centre crosspoint gain of -3 dB is a safe start, since the centre row can reach +3 dBFS).")
            font: Theme.fontCaption
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        ToolHeader {
            objectName: "UpmixerWindow.header"
            touchPath: "up."   // -- WP AIVIS --
            Layout.fillWidth: true
            iconName: "layout-grid"
            title: qsTr("Stereo Upmixer")
            subtitle: qsTr("Derive Centre and Surround from stereo")
            on: panel.value("enabled") !== 0
            switchEnabled: panel.writable
            onToggled: on => ToolsController.setUpmix("enabled", on ? 1 : 0)
        }
        EffectRule {}

        ToolUnsupported {
            objectName: "UpmixerWindow.unsupported"
            visible: panel.unsupported
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: 24
            // -- WP QPI3 -- the Mac names V25, but the upmixer came with wire 26: plain words
            text: qsTr("Requires an RP2350 device with firmware that includes the upmixer.")
            detail: qsTr("The upmixer runs on stereo input at 48 kHz or below.")
        }

        Flickable {
            id: body
            objectName: "UpmixerWindow.body"
            visible: !panel.unsupported
            Layout.fillWidth: true
            Layout.fillHeight: true
            readonly property int topGap: LayoutController.modern ? Theme.spacingSection : Theme.spacingLarge   // -- WP DL1 --
            contentHeight: column.implicitHeight + topGap + Theme.spacingLarge
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}

            // -- WP DL1 -- One column (the window), or two (Modern with room)
            GridLayout {
                id: column
                readonly property real columnWidth: (width - columnSpacing) / 2
                x: Theme.spacingLarge
                y: body.topGap
                width: (LayoutController.modern ? Math.min(parent.width, 1200) : parent.width) - 2 * Theme.spacingLarge
                columns: panel.twoColumns ? 2 : 1
                columnSpacing: panel.twoColumns ? 2 * Theme.spacingLarge : 0
                rowSpacing: 20

                ColumnLayout {
                    id: leftColumn
                    Layout.row: 0
                    Layout.column: 0
                    Layout.fillWidth: true
                    Layout.preferredWidth: panel.twoColumns ? column.columnWidth : -1
                    Layout.alignment: Qt.AlignTop
                    spacing: 20

                    // ---- Status: why it is idle, or the live gauges ----
                    ColumnLayout {
                        objectName: "UpmixerWindow.status"
                        Layout.fillWidth: true
                        spacing: 10
                        SectionHeading { text: qsTr("Status") }
                        RowLayout {
                            spacing: 8
                            Rectangle {
                                width: 8
                                height: 8
                                radius: 4
                                color: !ToolsController.connected ? Theme.textMuted : panel.active ? Theme.success : Theme.warning
                            }
                            Text {
                                objectName: "UpmixerWindow.status.text"
                                text: !ToolsController.connected ? qsTr("No device connected")
                                    : panel.active ? qsTr("Active - processing audio")
                                    : panel.status.parked === 1 ? qsTr("Idle: upmixer disabled")
                                    : panel.status.parked === 2 ? qsTr("Idle: input is not stereo")
                                    : panel.status.parked === 3 ? qsTr("Idle: sample rate above 48 kHz")
                                    : qsTr("Idle")
                                font: Theme.fontSmall
                                color: panel.active ? Theme.text : Theme.textMuted
                            }
                        }
                        // Kept in place at zero opacity while unused, so the height never changes
                        Gauge {
                            objectName: "UpmixerWindow.correlation"
                            visible: !LayoutController.modern || panel.active   // -- WP DL1 -- Modern: only while it runs
                            Layout.fillWidth: true
                            opacity: panel.active ? 1 : 0
                            label: qsTr("Correlation")
                            level: ((panel.status.correlation || 0) + 1) / 2
                            figure: ((panel.status.correlation || 0) >= 0 ? "+" : "") + (panel.status.correlation || 0).toFixed(2)
                            color: Theme.accent
                        }
                        Gauge {
                            objectName: "UpmixerWindow.centerGain"
                            visible: !LayoutController.modern || panel.active   // -- WP DL1 -- Modern: only while it runs
                            Layout.fillWidth: true
                            opacity: panel.active && !panel.centreOff ? 1 : 0
                            label: qsTr("Centre gain")
                            level: panel.status.center || 0
                            figure: qsTr("%1%").arg(Math.round((panel.status.center || 0) * 100))
                            color: Theme.success
                        }
                        Gauge {
                            objectName: "UpmixerWindow.lsGain"
                            visible: !LayoutController.modern || panel.active   // -- WP DL1 -- Modern: only while it runs
                            Layout.fillWidth: true
                            opacity: panel.active && panel.surroundOn ? 1 : 0
                            label: qsTr("Ls gain")
                            level: panel.status.ls || 0
                            figure: qsTr("%1%").arg(Math.round((panel.status.ls || 0) * 100))
                            color: Theme.warning
                        }
                        Gauge {
                            objectName: "UpmixerWindow.rsGain"
                            visible: !LayoutController.modern || panel.active   // -- WP DL1 -- Modern: only while it runs
                            Layout.fillWidth: true
                            opacity: panel.active && panel.surroundOn ? 1 : 0
                            label: qsTr("Rs gain")
                            level: panel.status.rs || 0
                            figure: qsTr("%1%").arg(Math.round((panel.status.rs || 0) * 100))
                            color: Theme.danger
                        }
                    }
                    EffectRule {}

                    // ---- Engines ----
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 14
                        SectionHeading { text: qsTr("Engines") }
                        RowLayout {
                            Layout.fillWidth: true
                            Text {
                                Layout.preferredWidth: 90
                                text: qsTr("Centre")
                                font: Theme.fontBody
                                color: panel.writable ? Theme.text : Theme.textDisabled
                            }
                            // Off is wire value 2 but sits first, to line up with Surround
                            ToolSegments {
                                objectName: "UpmixerWindow.centre"
                                Layout.fillWidth: true
                                enabled: panel.writable
                                label: qsTr("Centre")
                                model: [qsTr("Off"), qsTr("Sinner"), qsTr("Logician")]
                                current: panel.centerMode === 2 ? 0 : panel.centerMode + 1
                                onChosen: index => ToolsController.setUpmix("center", index === 0 ? 2 : index - 1)
                            }
                        }
                        RowLayout {
                            Layout.fillWidth: true
                            Text {
                                Layout.preferredWidth: 90
                                text: qsTr("Surround")
                                font: Theme.fontBody
                                color: panel.writable ? Theme.text : Theme.textDisabled
                            }
                            ToolSegments {
                                objectName: "UpmixerWindow.surround"
                                Layout.fillWidth: true
                                enabled: panel.writable
                                label: qsTr("Surround")
                                model: [qsTr("Off"), qsTr("Sinner"), qsTr("Logician")]
                                current: panel.surroundMode
                                onChosen: index => ToolsController.setUpmix("surround", index)
                            }
                        }
                        Text {
                            Layout.fillWidth: true
                            text: qsTr("Logician centre gates extraction on running L/R correlation; Logician surround uses a Pro Logic II-style matrix decoder. Sinner modes are fixed (C = 0.7071(L+R), surround = L-R) - a Hafler-style passive matrix like the one in the Schiit Syn.")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                            wrapMode: Text.Wrap
                        }
                    }
                    // -- WP DL1 -- two columns: the routing note under the engines
                    Routing { visible: panel.twoColumns }
                }

                ColumnLayout {
                    id: rightColumn
                    visible: !panel.centreOff || panel.surroundOn
                    Layout.row: panel.twoColumns ? 0 : 1
                    Layout.column: panel.twoColumns ? 1 : 0
                    Layout.fillWidth: true
                    Layout.preferredWidth: panel.twoColumns ? column.columnWidth : -1
                    Layout.alignment: Qt.AlignTop
                    spacing: 20

                    // ---- Centre: gone with its engine ----
                    EffectRule { visible: !panel.centreOff && !panel.twoColumns }
                    ColumnLayout {
                        objectName: "UpmixerWindow.centreSection"
                        visible: !panel.centreOff
                        Layout.fillWidth: true
                        spacing: 14
                        SectionHeading { text: qsTr("Centre") }
                        UpmixRow {
                            objectName: "UpmixerWindow.strength"
                            key: "strength"
                            title: qsTr("Strength")
                            unit: qsTr("%")
                            from: 0
                            to: 100
                            decimals: 0
                            scrollStep: 1
                            caption: qsTr("Centre extraction strength; scales both the C output and how much centre energy is removed from L/R. In Sinner mode this is the fixed centre gain.")
                        }
                        EffectRule {}
                        UpmixRow {
                            objectName: "UpmixerWindow.width"
                            key: "width"
                            title: qsTr("Centre Width")
                            unit: qsTr("%")
                            from: 0
                            to: 100
                            decimals: 0
                            scrollStep: 1
                            caption: qsTr("How much extracted centre stays in L/R. 0 = full removal (discrete centre); 100 = L/R untouched (expect combing if a real centre speaker plays).")
                        }
                        EffectRule {}
                        UpmixRow {
                            objectName: "UpmixerWindow.presence"
                            key: "presence"
                            title: qsTr("Presence")
                            unit: qsTr("dB")
                            from: -12
                            to: 12
                            scrollStep: 0.5
                            caption: qsTr("Voice presence bell at 3 kHz (Q 0.6). Positive brings voices forward, negative pushes them back (Syn-style). Stored in 0.5 dB steps.")
                        }
                        // The steering controls drive the Logician centre only
                        EffectRule { visible: panel.centerMode === 1 }
                        UpmixRow {
                            objectName: "UpmixerWindow.threshold"
                            visible: panel.centerMode === 1
                            key: "threshold"
                            title: qsTr("Correlation Threshold")
                            unit: qsTr("%")
                            from: 0
                            to: 95
                            decimals: 0
                            scrollStep: 1
                            caption: qsTr("Correlation gate. Below this, nothing is extracted; above it, extraction scales up to full. Raise to extract only strongly-correlated content.")
                        }
                        EffectRule { visible: panel.centerMode === 1 }
                        UpmixRow {
                            objectName: "UpmixerWindow.attack"
                            visible: panel.centerMode === 1
                            key: "attack"
                            title: qsTr("Attack")
                            unit: qsTr("ms")
                            from: 1
                            to: 500
                            decimals: 0
                            scrollStep: 1
                            caption: qsTr("Centre gain rise time (Logician mode).")
                        }
                        EffectRule { visible: panel.centerMode === 1 }
                        UpmixRow {
                            objectName: "UpmixerWindow.release"
                            visible: panel.centerMode === 1
                            key: "release"
                            title: qsTr("Release")
                            unit: qsTr("ms")
                            from: 5
                            to: 2000
                            decimals: 0
                            scrollStep: 5
                            caption: qsTr("Centre gain fall time (Logician mode).")
                        }
                        EffectRule { visible: panel.centerMode === 1 }
                        UpmixRow {
                            objectName: "UpmixerWindow.detector"
                            visible: panel.centerMode === 1
                            key: "detector"
                            title: qsTr("Detector HPF")
                            unit: qsTr("Hz")
                            from: 20
                            to: 1000
                            decimals: 0
                            scrollStep: 5
                            caption: qsTr("Detector bass-cut corner. Content below this is ignored by the steering detector (the audio itself is not filtered) so bass does not pump the centre.")
                        }
                    }

                    // ---- Surround: gone with its engine ----
                    EffectRule { visible: panel.surroundOn && !(panel.twoColumns && panel.centreOff) }
                    ColumnLayout {
                        objectName: "UpmixerWindow.surroundSection"
                        visible: panel.surroundOn
                        Layout.fillWidth: true
                        spacing: 14
                        SectionHeading { text: qsTr("Surround") }
                        UpmixRow {
                            objectName: "UpmixerWindow.delay"
                            key: "delay"
                            title: qsTr("Delay")
                            unit: qsTr("ms")
                            from: 0
                            to: 20
                            scrollStep: 0.5
                            caption: qsTr("Haas delay on Ls/Rs (precedence effect). Rule of thumb ~1 ms per foot of listener distance.")
                        }
                        EffectRule {}
                        UpmixRow {
                            objectName: "UpmixerWindow.hpf"
                            key: "hpf"
                            title: qsTr("Band-limit HPF")
                            unit: qsTr("Hz")
                            from: 20
                            to: 2000
                            decimals: 0
                            scrollStep: 5
                            caption: qsTr("Surround high-pass; keeps rumble out of the rears.")
                        }
                        EffectRule {}
                        UpmixRow {
                            objectName: "UpmixerWindow.lpf"
                            key: "lpf"
                            title: qsTr("Band-limit LPF")
                            unit: qsTr("Hz")
                            from: 1000
                            to: 20000
                            decimals: 0
                            scrollStep: 100
                            caption: qsTr("Surround low-pass. 7 kHz is the classic surround voicing; raise for full-band rears.")
                        }
                        EffectRule {}
                        UpmixRow {
                            objectName: "UpmixerWindow.decorrelation"
                            key: "decorrelation"
                            title: qsTr("Decorrelation")
                            unit: qsTr("%")
                            from: 0
                            to: 100
                            decimals: 0
                            scrollStep: 1
                            caption: qsTr("Schroeder allpass decorrelator amount. 0 disables decorrelation.")
                        }
                    }
                }

                // ---- Where the derived channels go ----
                EffectRule {
                    visible: !panel.twoColumns
                    Layout.row: 2
                    Layout.column: 0
                }
                Routing {
                    visible: !panel.twoColumns
                    Layout.row: 3
                    Layout.column: 0
                }
            }
        }
    }
}
