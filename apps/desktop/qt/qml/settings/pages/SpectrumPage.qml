import QtQuick 2.15
import "../"

// Spectrum analyser preferences. Display options live on the main window
// (and persist there); the engine options are sent to the device whenever
// the analyser runs and are not stored on it.
SettingsPage {
    writesToDevice: false   // -- WP QD6 --
    id: page
    title: qsTr("Spectrum Analyser")
    subtitle: qsTr("How the device's spectrum analyser looks and measures. Pick the channels with the gear on the response graph.")
    readonly property var app: ctx ? ctx.app : null

    SettingsSection {
        title: qsTr("Display")
        SettingsSliderRow {
            title: qsTr("Spectrum Strength")
            from: 0.3; to: 1.0; stepSize: 0.05
            value: app ? app.rtaGraphOpacity : 1.0
            format: function (v) { return qsTr("%1%").arg(Math.round(v * 100)) }
            onMoved: app.rtaGraphOpacity = value
            onCommitted: app.rtaGraphOpacity = value
        }
        SettingsSwitchRow {
            title: qsTr("Peak Hold")
            detail: qsTr("A thin line (or cap on the bars) at each band's recent peak.")
            checked: app ? app.rtaShowPeakHold : true
            onToggled: app.rtaShowPeakHold = checked
        }
        SettingsSwitchRow {
            title: qsTr("Smoothing")
            detail: qsTr("Glide between the device's frames instead of stepping.")
            checked: app ? app.rtaSmoothing : true
            onToggled: app.rtaSmoothing = checked
        }
        SettingsChoiceRow {
            title: qsTr("Floor")
            options: [{ text: qsTr("−60 dBFS"), value: -60 }, { text: qsTr("−90 dBFS"), value: -90 }, { text: qsTr("−120 dBFS"), value: -120 }]
            value: app ? app.rtaFloorDb : -90
            onChosen: app.rtaFloorDb = value
        }
        SettingsChoiceRow {
            title: qsTr("Ceiling")
            options: [{ text: qsTr("0 dBFS"), value: 0 }, { text: qsTr("+6 dBFS"), value: 6 }, { text: qsTr("+12 dBFS"), value: 12 }]
            value: app ? app.rtaCeilingDb : 6
            onChosen: app.rtaCeilingDb = value
        }
    }

    SettingsSection {
        title: qsTr("Engine")
        footnote: !rta.supported ? (bridge.connected ? qsTr("This firmware has no spectrum analyser.") : qsTr("Connect a DSPi to see what its analyser can do."))
                  : qsTr("This device reports %1 dB of usable range and transforms up to %2 points.")
                    .arg(String(rta.dynamicRange)).arg(String(Math.pow(2, rta.orderMax)))
        SettingsSegmentedRow {
            title: qsTr("Transform Size")
            detail: qsTr("Larger transforms resolve lower bands but refresh each channel less often.")
            options: ["256", "512", "1024"]
            controlWidth: 210
            value: app ? app.rtaFftOrder - 8 : 2
            onChosen: app.rtaFftOrder = index + 8
        }
        SettingsChoiceRow {
            title: qsTr("Averaging")
            options: [{ text: qsTr("Off"), value: 0 }, { text: qsTr("50 ms"), value: 50 }, { text: qsTr("125 ms"), value: 125 },
                      { text: qsTr("300 ms"), value: 300 }, { text: qsTr("1 s"), value: 1000 }, { text: qsTr("3 s"), value: 3000 }]
            value: app ? app.rtaAvgMs : 300
            onChosen: app.rtaAvgMs = value
        }
        SettingsChoiceRow {
            title: qsTr("Peak Decay")
            enabled: app ? app.rtaShowPeakHold : true
            options: [{ text: qsTr("Off"), value: 0 }, { text: qsTr("4 dB/s"), value: 4 }, { text: qsTr("12 dB/s"), value: 12 }, { text: qsTr("30 dB/s"), value: 30 }]
            value: app ? app.rtaPeakDecay : 12
            onChosen: app.rtaPeakDecay = value
        }
    }
}
