import QtQuick
import QtQuick.Layouts
import UConsole

// Settings > Graphing (the Mac's GraphingSettingsTab): how the response graph
// draws. App preferences, applied at once. GraphController owns them (and
// stores them under graph/*), so the graph and its own setup page see a change
// here at once.
SettingsPage {
    id: page
    objectName: "GraphingPage"
    readonly property var prefs: GraphController

    SettingsGroup {
        title: qsTr("Graph Appearance")
        iconName: "chart-line"
        SettingsToggle {
            objectName: "Graphing.showGlow"
            title: qsTr("Graph Line Glow")
            caption: qsTr("Add a neon glow effect to frequency response curves")
            checked: page.prefs.showGraphGlow
            onToggled: on => page.prefs.showGraphGlow = on
        }
        SettingsToggle {
            objectName: "Graphing.showPhase"
            title: qsTr("Show Phase Response")
            caption: qsTr("Overlay the selected channel's phase (degrees) as a dotted line")
            checked: page.prefs.showPhase
            onToggled: on => page.prefs.showPhase = on
        }
        SettingsToggle {
            objectName: "Graphing.phaseUnwrapped"
            title: qsTr("Unwrap Phase")
            caption: qsTr("Show continuous phase instead of wrapping at ±180°")
            enabled: page.prefs.showPhase
            checked: page.prefs.phaseUnwrapped
            onToggled: on => page.prefs.phaseUnwrapped = on
        }
    }

    SettingsGroup {
        title: qsTr("Response Curve")
        iconName: "activity"
        SettingsSliderRow {
            objectName: "Graphing.lineWidth"
            label: qsTr("Line Width: %1pt").arg(Number(page.prefs.graphLineWidth).toFixed(1))
            minimum: 1.0
            maximum: 4.0
            step: 0.5
            resetValue: 1.5
            value: page.prefs.graphLineWidth
            onMoved: v => page.prefs.graphLineWidth = v
        }
    }

    SettingsGroup {
        title: qsTr("Scale & Grid")
        iconName: "layout-grid"
        SettingsToggle {
            objectName: "Graphing.showFreqGrid"
            title: qsTr("Show Frequency Grid")
            checked: page.prefs.showFrequencyGrid
            onToggled: on => page.prefs.showFrequencyGrid = on
        }
        SettingsToggle {
            objectName: "Graphing.showFreqLabels"
            title: qsTr("Show Frequency Labels")
            checked: page.prefs.showFrequencyLabels
            onToggled: on => page.prefs.showFrequencyLabels = on
        }
        SettingsToggle {
            objectName: "Graphing.showDbGrid"
            title: qsTr("Show dB Grid")
            checked: page.prefs.showDBGrid
            onToggled: on => page.prefs.showDBGrid = on
        }
        SettingsToggle {
            objectName: "Graphing.showDbLabels"
            title: qsTr("Show dB Labels")
            checked: page.prefs.showDBLabels
            onToggled: on => page.prefs.showDBLabels = on
        }
        SettingsToggle {
            objectName: "Graphing.freqReadout"
            title: qsTr("Show Frequency Readout")
            checked: page.prefs.showFrequencyReadout
            onToggled: on => page.prefs.showFrequencyReadout = on
        }
        SettingsToggle {
            objectName: "Graphing.levelReadout"
            title: qsTr("Show Gain Readout")
            checked: page.prefs.showLevelReadout
            onToggled: on => page.prefs.showLevelReadout = on
        }
        SettingsSliderRow {
            objectName: "Graphing.gridOpacity"
            label: qsTr("Grid Opacity: %1%").arg(Math.round(page.prefs.graphGridOpacity * 100))
            enabled: page.prefs.showFrequencyGrid || page.prefs.showDBGrid
            minimum: 0
            maximum: 2
            resetValue: 0.5
            value: page.prefs.graphGridOpacity
            onMoved: v => page.prefs.graphGridOpacity = v
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }
        SettingsSliderRow {
            objectName: "Graphing.dbRange"
            label: qsTr("Vertical Range: %1 dB").arg(Math.round(page.prefs.dbRange))
            minimum: 10
            maximum: 100
            resetValue: 50
            value: page.prefs.dbRange
            onMoved: v => page.prefs.dbRange = Math.round(v)
        }
        SettingsSliderRow {
            objectName: "Graphing.dbCenter"
            readonly property real dbTop: page.prefs.dbCenter + page.prefs.dbRange / 2
            readonly property real dbBottom: page.prefs.dbCenter - page.prefs.dbRange / 2
            function signedDb(db) { const n = Math.round(db); return n > 0 ? "+" + n : String(n) }
            label: qsTr("Center: %1 dB → %2 to %3").arg(Math.round(page.prefs.dbCenter)).arg(signedDb(dbTop)).arg(signedDb(dbBottom))
            minimum: -40
            maximum: 20
            resetValue: 0
            value: page.prefs.dbCenter
            onMoved: v => page.prefs.dbCenter = Math.round(v)
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingLarge
            SettingsPickerRow {
                objectName: "Graphing.minFreq"
                label: qsTr("Min Frequency")
                values: [10, 15, 20, 50, 100]
                texts: [qsTr("10 Hz"), qsTr("15 Hz"), qsTr("20 Hz"), qsTr("50 Hz"), qsTr("100 Hz")]
                value: Number(page.prefs.minFreq)
                onChosen: v => page.prefs.minFreq = v
            }
            SettingsPickerRow {
                objectName: "Graphing.maxFreq"
                label: qsTr("Max Frequency")
                values: [5000, 10000, 20000]
                texts: [qsTr("5 kHz"), qsTr("10 kHz"), qsTr("20 kHz")]
                value: Number(page.prefs.maxFreq)
                onChosen: v => page.prefs.maxFreq = v
            }
        }
    }

    SettingsGroup {
        title: qsTr("Pop-out Window")
        iconName: "external-link"
        SettingsToggle {
            objectName: "Graphing.popOutFollows"
            title: qsTr("Pop-out graph follows channel selection")
            checked: page.prefs.popoutFollowsSelection
            onToggled: on => page.prefs.popoutFollowsSelection = on
        }
    }
}
