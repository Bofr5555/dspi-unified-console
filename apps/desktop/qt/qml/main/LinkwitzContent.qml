import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP2 -- The Linkwitz Transform panel (the Mac's linkwitzPopover): the
// driver's alignment (f0, Q0) moved to a target (fp, Qp), with the DC boost
// that implies. Edits stay in a draft until Apply, because a mistyped fp is
// tens of dB of boost into a driver; Revert goes back to the band as applied,
// and hiding the panel drops the draft.
//
// -- WP LAY2 -- The content alone, draft and all: Classic shows it in a popover
// from the band row (Linkwitz), Modern in its inspector (InspectorSection).
// The host sets `active` as it shows the content, which starts the draft afresh
// from the band as applied.
ColumnLayout {
    id: content
    property int channel: -1
    property int band: 0
    property bool active: false

    // The band as applied (fp is carried in the gain field)
    readonly property var applied: (ChannelController.revision < 0 ? undefined : ChannelController.band(channel, band, false))
    property real f0: 0
    property real q0: 0
    property real fp: 0
    property real qp: 0
    readonly property real boost: ChannelController.linkwitzBoost(f0, fp)
    // Compared with the round trip's grid: fp to 0.05 Hz, Qp to 1/512
    readonly property bool dirty: Math.abs(f0 - applied.freq) >= 0.05 || Math.abs(q0 - applied.q) >= 0.0005
                                  || Math.abs(fp - applied.gain) >= 0.05 || Math.abs(qp - applied.qp) >= 1 / 512

    function revert() {
        f0 = applied.freq
        q0 = applied.q
        fp = applied.gain
        qp = applied.qp
    }
    function applyDraft() { ChannelController.setLinkwitz(channel, band, f0, q0, fp, qp) }

    onActiveChanged: if (active) revert()
    Connections {   // -- WP MDQT2a -- the draft is the device's it was typed for
        target: content.active ? DeviceRegistry : null
        function onActiveChanged() { content.revert() }
    }

    Layout.fillWidth: true
    spacing: Theme.spacing

    ColumnLayout {
        spacing: 2
        Text {
            text: qsTr("Linkwitz Transform")
            font: Theme.fontTitle
            color: Theme.text
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Text {
            Layout.fillWidth: true
            text: qsTr("Re-align a sealed woofer's rolloff from the driver's (f0, Q0) to a target (fp, Qp).")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    GridLayout {
        columns: 5
        columnSpacing: Theme.spacingSmall
        rowSpacing: Theme.spacingSmall

        Text { text: qsTr("Driver"); font: Theme.fontSmall; color: Theme.textMuted }
        Text { text: "f0"; font: Theme.fontSmall; color: Theme.textMuted }
        ChannelValueField {
            objectName: "Linkwitz.f0"
            label: qsTr("Driver resonance f0")
            unit: qsTr("Hz")
            fieldWidth: 64
            scrollStep: 1
            minValue: 10
            stripTrailingZeros: true
            value: content.f0
            onCommitted: v => content.f0 = Math.max(10, v)
        }
        Text { text: "Q0"; font: Theme.fontSmall; color: Theme.textMuted }
        ChannelValueField {
            objectName: "Linkwitz.q0"
            label: qsTr("Driver Q0")
            fieldWidth: 56
            maxDecimals: 3
            scrollStep: 0.01
            minValue: 0.1
            stripTrailingZeros: true
            value: content.q0
            onCommitted: v => content.q0 = Math.min(20, Math.max(0.1, v))
        }

        Text { text: qsTr("Target"); font: Theme.fontSmall; color: Theme.textMuted }
        Text { text: "fp"; font: Theme.fontSmall; color: Theme.textMuted }
        ChannelValueField {
            objectName: "Linkwitz.fp"
            label: qsTr("Target frequency fp")
            unit: qsTr("Hz")
            fieldWidth: 64
            scrollStep: 1
            minValue: 10
            stripTrailingZeros: true
            value: content.fp
            onCommitted: v => content.fp = Math.max(10, v)
        }
        Text { text: "Qp"; font: Theme.fontSmall; color: Theme.textMuted }
        ChannelValueField {
            objectName: "Linkwitz.qp"
            label: qsTr("Target Qp")
            fieldWidth: 56
            maxDecimals: 3
            scrollStep: 0.01
            minValue: 0.1
            stripTrailingZeros: true
            value: content.qp
            onCommitted: v => content.qp = Math.min(20, Math.max(0.1, v))
        }
    }

    Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.separator }

    RowLayout {
        spacing: 6
        Text { text: qsTr("DC boost"); font: Theme.fontSmall; color: Theme.textMuted }
        Text {
            objectName: "Linkwitz.boost"
            text: (content.boost >= 0 ? "+" : "") + content.boost.toFixed(1) + " " + qsTr("dB")
            font: Theme.fontMono
            color: content.boost > 15 ? Theme.warning : Theme.text
            Accessible.role: Accessible.StaticText
            Accessible.name: qsTr("DC boost %1").arg(text)
        }
        Icon {
            visible: content.boost > 15
            name: "triangle-alert"
            size: 12
            color: Theme.warning
            accessibleName: qsTr("High boost")
        }
    }
    Text {
        Layout.fillWidth: true
        text: qsTr("Real low-frequency gain (40 x log10(f0/fp)). It uses driver excursion and amp headroom - reduce preamp or master volume to match.")
        font: Theme.fontCaption
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }

    Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.separator }

    RowLayout {
        spacing: Theme.spacingSmall
        Text {
            objectName: "Linkwitz.state"
            text: content.dirty ? qsTr("Not applied yet") : qsTr("Applied")
            font: Theme.fontSmall
            color: content.dirty ? Theme.warning : Theme.textMuted
        }
        Item { Layout.fillWidth: true }
        AppButton {
            objectName: "Linkwitz.revert"
            text: qsTr("Revert")
            enabled: content.dirty
            onClicked: content.revert()
        }
        AppButton {
            objectName: "Linkwitz.apply"
            text: qsTr("Apply")
            primary: true
            enabled: content.dirty && ChannelController.writable
            onClicked: content.applyDraft()
            Keys.onReturnPressed: content.applyDraft()
        }
    }
}
