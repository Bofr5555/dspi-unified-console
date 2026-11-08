import QtQuick 2.15
import "../"
import "../../components"

// Graph preferences live on the main window (and persist there).
SettingsPage {
    writesToDevice: false   // -- WP QD6 --
    id: page
    title: qsTr("Graphing")
    subtitle: qsTr("How the response graph looks. These preferences are kept here, not on the DSPi, and take effect immediately.")
    readonly property var app: ctx ? ctx.app : null

    SettingsSection {
        title: qsTr("Curves")
        SettingsSwitchRow {
            title: qsTr("Graph Line Glow")
            detail: qsTr("A soft glow behind the response curves.")
            checked: app ? app.graphShowGlow : false
            onToggled: app.graphShowGlow = checked
        }
        SettingsSliderRow {
            title: qsTr("Line Width")
            from: 1; to: 4; stepSize: 0.5
            value: app ? app.graphLineWidth : 1.5
            format: function (v) { return qsTr("%1 pt").arg(v.toFixed(1)) }
            onMoved: app.graphLineWidth = value
            onCommitted: app.graphLineWidth = value
        }
        SettingsSwitchRow {
            title: qsTr("Show Phase Response")
            detail: qsTr("Overlay the selected channel's phase (degrees) as a dotted line.")
            checked: app ? app.graphShowPhase : false
            onToggled: app.graphShowPhase = checked
        }
        SettingsSwitchRow {
            title: qsTr("Unwrap Phase")
            detail: qsTr("Show continuous phase instead of wrapping at ±180°.")
            enabled: app ? app.graphShowPhase : false
            checked: app ? app.graphPhaseUnwrapped : false
            onToggled: app.graphPhaseUnwrapped = checked
        }
    }

    SettingsSection {
        title: qsTr("Pop-Out Graph")
        SettingsSwitchRow {
            title: qsTr("Follow Channel Selection")
            detail: qsTr("The pop-out graph shows the channels the main window shows. Off, it keeps its own, picked with the pills under it.")
            checked: app ? app.graphPopOutFollows : true
            onToggled: app.graphPopOutFollows = checked
        }
    }

    SettingsSection {
        title: qsTr("Editing")
        footnote: qsTr("Drag a band's dot to change it; Shift for fine steps, %1 to keep to one axis, %2-drag or the wheel for its width. Double-click empty graph to add a band.")
                  .arg(isMacOS ? qsTr("Option") : qsTr("Alt")).arg(MenuStyle.ctrlName)
        SettingsSwitchRow {
            title: qsTr("Show Frequency Readout")
            detail: qsTr("While editing bands, label the frequency under the pointer along the bottom of the graph.")
            checked: app ? app.graphFreqReadout : true
            onToggled: app.graphFreqReadout = checked
        }
        SettingsSwitchRow {
            title: qsTr("Show Gain Readout")
            detail: qsTr("Label the level under the pointer along the left edge: the gain a new band takes there.")
            checked: app ? app.graphLevelReadout : true
            onToggled: app.graphLevelReadout = checked
        }
    }

    SettingsSection {
        title: qsTr("Grid and Labels")
        SettingsSwitchRow { title: qsTr("Frequency Grid"); checked: app ? app.graphShowFreqGrid : true; onToggled: app.graphShowFreqGrid = checked }
        SettingsSwitchRow { title: qsTr("Frequency Labels"); checked: app ? app.graphShowFreqLabels : true; onToggled: app.graphShowFreqLabels = checked }
        SettingsSwitchRow { title: qsTr("dB Grid"); checked: app ? app.graphShowDbGrid : true; onToggled: app.graphShowDbGrid = checked }
        SettingsSwitchRow { title: qsTr("dB Labels"); checked: app ? app.graphShowDbLabels : true; onToggled: app.graphShowDbLabels = checked }
        SettingsSliderRow {
            title: qsTr("Grid Opacity")
            enabled: app ? app.graphShowFreqGrid || app.graphShowDbGrid : true
            from: 0; to: 2; stepSize: 0.05
            value: app ? app.graphGridOpacity : 0.5
            format: function (v) { return qsTr("%1%").arg(Math.round(v * 100)) }
            onMoved: app.graphGridOpacity = value
            onCommitted: app.graphGridOpacity = value
        }
    }

    SettingsSection {
        title: qsTr("Range")
        SettingsSliderRow {
            title: qsTr("Vertical Range")
            from: 10; to: 100; stepSize: 1
            value: app ? app.graphDbRange : 50
            format: function (v) { return qsTr("%1 dB").arg(Math.round(v)) }
            onMoved: app.graphDbRange = Math.round(value)
            onCommitted: app.graphDbRange = Math.round(value)
        }
        SettingsSliderRow {
            title: qsTr("Center")
            detail: app ? qsTr("Shows +%1 to %2 dB").arg(String(Math.round(app.graphDbCenter) + app.graphDbRange / 2))
                          .arg(String(Math.round(app.graphDbCenter) - app.graphDbRange / 2)) : ""
            from: -40; to: 20; stepSize: 1
            value: app ? app.graphDbCenter : 0
            format: function (v) { return qsTr("%1 dB").arg((v > 0 ? "+" : "") + Math.round(v)) }
            onMoved: app.graphDbCenter = Math.round(value)
            onCommitted: app.graphDbCenter = Math.round(value)
        }
        SettingsChoiceRow {
            title: qsTr("Min Frequency")
            options: [{ text: qsTr("10 Hz"), value: 10 }, { text: qsTr("15 Hz"), value: 15 }, { text: qsTr("20 Hz"), value: 20 },
                      { text: qsTr("50 Hz"), value: 50 }, { text: qsTr("100 Hz"), value: 100 }]
            value: app ? app.graphMinFreq : 15
            onChosen: app.graphMinFreq = value
        }
        SettingsChoiceRow {
            title: qsTr("Max Frequency")
            options: [{ text: qsTr("5 kHz"), value: 5000 }, { text: qsTr("10 kHz"), value: 10000 }, { text: qsTr("20 kHz"), value: 20000 }]
            value: app ? app.graphMaxFreq : 20000
            onChosen: app.graphMaxFreq = value
        }
    }
}
