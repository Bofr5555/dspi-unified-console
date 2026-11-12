import QtQuick
import QtQuick.Layouts
import UConsole

// Settings > Spectrum Analyser (the Mac's SpectrumSettingsTab): how the
// analyser draws and what it asks the device for. App preferences, applied at
// once (SettingsController.spectrum); the analyser (QP9) follows them.
SettingsPage {
    id: page
    objectName: "SpectrumPage"
    readonly property var prefs: SettingsController.spectrum
    readonly property var caps: SettingsController.analyser
    readonly property bool supported: caps.supported === true

    // The sizes this device offers, from its caps, else the protocol's range
    // (the Mac's RTA_ORDER_MIN...RTA_ORDER_MAX), so no size the device would refuse
    readonly property var orders: {
        let lo = 8, hi = 10
        if (supported && caps.orderMax >= caps.orderMin) {
            lo = caps.orderMin
            hi = caps.orderMax
        }
        let list = []
        for (let o = lo; o <= hi; ++o) list.push({ text: qsTr("%1 points").arg(1 << o), value: o })
        return list
    }
    // The offered size nearest the stored one
    readonly property int selectedOrder: orders.length
        ? Math.min(Math.max(page.prefs.fftOrder, orders[0].value), orders[orders.length - 1].value)
        : page.prefs.fftOrder

    Component.onCompleted: SettingsController.refreshDeviceFacts()

    SettingsGroup {
        title: qsTr("Display")
        iconName: "layout-grid"
        footer: qsTr("Choose which channels to show, and whether to draw them on the response graph, as bars, or both, from the gear button at the top right of the response graph. The analyser only runs while something is watching it, so with the spectrum hidden and the analyser window closed the device stops it and spends nothing.")
        SettingsSliderRow {
            objectName: "Spectrum.graphOpacity"
            label: qsTr("Spectrum Strength: %1%").arg(Math.round(page.prefs.graphOpacity * 100))
            from: 0.3
            to: 1.0
            stepSize: 0.05
            resetValue: 1.0
            value: page.prefs.graphOpacity
            onMoved: v => page.prefs.graphOpacity = v
        }
        Text {
            Layout.fillWidth: true
            text: qsTr("How far the fill comes forward behind the curves when the spectrum is drawn on the response graph.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        SettingsToggle {
            objectName: "Spectrum.showPeakHold"
            title: qsTr("Peak Hold")
            caption: qsTr("A cap above each band marking its recent maximum")
            checked: page.prefs.showPeakHold
            onToggled: on => page.prefs.showPeakHold = on
        }
        SettingsToggle {
            objectName: "Spectrum.smoothing"
            title: qsTr("Smoothing")
            caption: qsTr("Glides the spectrum between device frames the way the peak meters glide between polls. It matters most with several channels selected, where one channel refreshes only every few hundred milliseconds and the picture would otherwise step.")
            checked: page.prefs.smoothing
            onToggled: on => page.prefs.smoothing = on
        }
    }

    SettingsGroup {
        title: qsTr("Vertical Scale")
        iconName: "sliders-vertical"
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingLarge
            SettingsPickerRow {
                objectName: "Spectrum.floorDb"
                label: qsTr("Floor")
                options: [{ text: qsTr("-60 dB"), value: -60 }, { text: qsTr("-90 dB"), value: -90 },
                          { text: qsTr("-120 dB"), value: -120 }]
                value: Number(page.prefs.floorDb)
                onChosen: v => page.prefs.floorDb = v
            }
            SettingsPickerRow {
                objectName: "Spectrum.ceilingDb"
                label: qsTr("Ceiling")
                options: [{ text: qsTr("0 dBFS"), value: 0 }, { text: qsTr("+6 dBFS"), value: 6 },
                          { text: qsTr("+12 dBFS"), value: 12 }]
                value: Number(page.prefs.ceilingDb)
                onChosen: v => page.prefs.ceilingDb = v
            }
        }
        Text {
            Layout.fillWidth: true
            text: qsTr("Six decibels of headroom above full scale is the default because upmix-derived rows and hot EQ can legitimately exceed 0 dBFS.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    SettingsGroup {
        title: qsTr("Engine")
        iconName: "cpu"
        footer: DeviceModel.connected && !page.supported && page.caps.supported !== undefined
                ? qsTr("The connected firmware has no spectrum analyser, so these settings have nothing to apply to until it is updated.")
                : page.supported
                  ? qsTr("This device reports %1 dB of usable range and transforms up to %2 points.").arg(page.caps.dynamicRangeDb).arg(1 << page.caps.orderMax)
                  : ""
        footerColor: page.supported ? Theme.textMuted : Theme.warning
        SettingsPickerRow {
            objectName: "Spectrum.fftOrder"
            label: qsTr("Transform Size")
            options: page.orders
            value: page.selectedOrder
            onChosen: v => page.prefs.fftOrder = v
        }
        Text {
            Layout.fillWidth: true
            text: qsTr("More points resolve lower frequencies, but a frame takes longer to fill, so each channel refreshes less often.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
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
                objectName: "Spectrum.avgMs"
                label: qsTr("Averaging")
                options: [{ text: qsTr("Off"), value: 0 }, { text: qsTr("50 ms"), value: 50 },
                          { text: qsTr("125 ms"), value: 125 }, { text: qsTr("300 ms"), value: 300 },
                          { text: qsTr("1 s"), value: 1000 }, { text: qsTr("3 s"), value: 3000 }]
                value: Number(page.prefs.avgMs)
                onChosen: v => page.prefs.avgMs = v
            }
            SettingsPickerRow {
                objectName: "Spectrum.peakDecay"
                label: qsTr("Peak Decay")
                enabled: page.prefs.showPeakHold
                options: [{ text: qsTr("Off"), value: 0 }, { text: qsTr("4 dB/s"), value: 4 },
                          { text: qsTr("12 dB/s"), value: 12 }, { text: qsTr("30 dB/s"), value: 30 }]
                value: Number(page.prefs.peakDecay)
                onChosen: v => page.prefs.peakDecay = v
            }
        }
    }
}
