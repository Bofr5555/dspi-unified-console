import QtQuick
import QtQuick.Layouts
import UConsole

// Settings > Graphing (the Mac's GraphingSettingsTab): how the response graph
// draws. App preferences, applied at once (SettingsController.graph).
SettingsPage {
    id: page
    objectName: "GraphingPage"
    readonly property var prefs: SettingsController.graph

    SettingsGroup {
        title: qsTr("Graph Appearance")
        iconName: "chart-line"
        SettingsToggle {
            objectName: "Graphing.showGlow"
            title: qsTr("Graph Line Glow")
            caption: qsTr("Add a neon glow effect to frequency response curves")
            checked: page.prefs.showGlow
            onToggled: on => page.prefs.showGlow = on
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
            label: qsTr("Line Width: %1pt").arg(Number(page.prefs.lineWidth).toFixed(1))
            from: 1.0
            to: 4.0
            stepSize: 0.5
            resetValue: 1.5
            value: page.prefs.lineWidth
            onMoved: v => page.prefs.lineWidth = v
        }
    }

    SettingsGroup {
        title: qsTr("Scale & Grid")
        iconName: "layout-grid"
        SettingsToggle {
            objectName: "Graphing.showFreqGrid"
            title: qsTr("Show Frequency Grid")
            checked: page.prefs.showFreqGrid
            onToggled: on => page.prefs.showFreqGrid = on
        }
        SettingsToggle {
            objectName: "Graphing.showFreqLabels"
            title: qsTr("Show Frequency Labels")
            checked: page.prefs.showFreqLabels
            onToggled: on => page.prefs.showFreqLabels = on
        }
        SettingsToggle {
            objectName: "Graphing.showDbGrid"
            title: qsTr("Show dB Grid")
            checked: page.prefs.showDbGrid
            onToggled: on => page.prefs.showDbGrid = on
        }
        SettingsToggle {
            objectName: "Graphing.showDbLabels"
            title: qsTr("Show dB Labels")
            checked: page.prefs.showDbLabels
            onToggled: on => page.prefs.showDbLabels = on
        }
        SettingsToggle {
            objectName: "Graphing.freqReadout"
            title: qsTr("Show Frequency Readout")
            checked: page.prefs.freqReadout
            onToggled: on => page.prefs.freqReadout = on
        }
        SettingsToggle {
            objectName: "Graphing.levelReadout"
            title: qsTr("Show Gain Readout")
            checked: page.prefs.levelReadout
            onToggled: on => page.prefs.levelReadout = on
        }
        SettingsSliderRow {
            objectName: "Graphing.gridOpacity"
            label: qsTr("Grid Opacity: %1%").arg(Math.round(page.prefs.gridOpacity * 100))
            enabled: page.prefs.showFreqGrid || page.prefs.showDbGrid
            from: 0
            to: 2
            resetValue: 0.5
            value: page.prefs.gridOpacity
            onMoved: v => page.prefs.gridOpacity = v
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }
        SettingsSliderRow {
            objectName: "Graphing.dbRange"
            label: qsTr("Vertical Range: %1 dB").arg(Math.round(page.prefs.dbRange))
            from: 10
            to: 100
            resetValue: 50
            value: page.prefs.dbRange
            onMoved: v => page.prefs.dbRange = Math.round(v)
        }
        SettingsSliderRow {
            objectName: "Graphing.dbCenter"
            readonly property real dbTop: page.prefs.dbCenter + page.prefs.dbRange / 2
            readonly property real dbBottom: page.prefs.dbCenter - page.prefs.dbRange / 2
            function signedDb(v) { return (v > 0 ? "+" : "") + Math.round(v) }
            label: qsTr("Center: %1 dB → %2 to %3").arg(Math.round(page.prefs.dbCenter)).arg(signedDb(dbTop)).arg(signedDb(dbBottom))
            from: -40
            to: 20
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
                options: [{ text: qsTr("10 Hz"), value: 10 }, { text: qsTr("15 Hz"), value: 15 },
                          { text: qsTr("20 Hz"), value: 20 }, { text: qsTr("50 Hz"), value: 50 },
                          { text: qsTr("100 Hz"), value: 100 }]
                value: Number(page.prefs.minFreq)
                onChosen: v => page.prefs.minFreq = v
            }
            SettingsPickerRow {
                objectName: "Graphing.maxFreq"
                label: qsTr("Max Frequency")
                options: [{ text: qsTr("5 kHz"), value: 5000 }, { text: qsTr("10 kHz"), value: 10000 },
                          { text: qsTr("20 kHz"), value: 20000 }]
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
            checked: page.prefs.popOutFollows
            onToggled: on => page.prefs.popOutFollows = on
        }
    }
}
