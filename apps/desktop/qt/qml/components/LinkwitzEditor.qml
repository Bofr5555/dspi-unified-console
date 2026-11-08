import QtQuick 2.15
import QtQuick.Controls 2.15

// Linkwitz Transform editor: driver (f0, Q0) -> target (fp, Qp). Edits are
// sent only on Apply (or Return); Revert discards them, and closing the
// editor drops anything unapplied.
Popup {
    id: lt
    width: 300
    padding: 14
    background: Rectangle {
        color: isMacOS ? "#353535" : nativeAltBaseColor
        border.color: Qt.rgba(1, 1, 1, 0.15)
        radius: 8
    }

    property int channelId: 0
    property int bandIndex: 0
    property real f0: 50
    property real q0: 0.707
    property real fp: 50
    property real qp: 0.707

    function load() {
        f0 = bridge.filterFreq(channelId, bandIndex)
        q0 = bridge.filterQ(channelId, bandIndex)
        fp = bridge.filterGain(channelId, bandIndex)
        qp = bridge.filterQp(channelId, bandIndex)
    }
    function openFor(ch, band) {
        channelId = ch
        bandIndex = band
        load()
        open()
    }
    function apply() { bridge.setLinkwitzTransform(channelId, bandIndex, f0, q0, fp, qp) }

    readonly property bool applied:
        Math.abs(f0 - bridge.filterFreq(channelId, bandIndex)) < 0.01
        && Math.abs(q0 - bridge.filterQ(channelId, bandIndex)) < 0.0005
        && Math.abs(fp - bridge.filterGain(channelId, bandIndex)) < 0.01
        && Math.abs(qp - bridge.filterQp(channelId, bandIndex)) < 0.0005
    property int rev: 0
    Connections { target: bridge; function onStateChanged() { lt.rev++ } }

    // Low-frequency boost of the transform: (f0 / fp)^2, in dB
    readonly property real dcBoost: fp > 0 ? 40 * Math.log(f0 / fp) / Math.LN10 : 0

    component Field: Row {
        property alias label: lbl.text
        property alias value: vf.value
        property alias suffix: vf.suffix
        property alias decimals: vf.decimals
        property alias minValue: vf.minValue
        property alias maxValue: vf.maxValue
        property alias wheelStep: vf.wheelStep
        signal edited(real v)
        spacing: 8
        Text { id: lbl; width: 34; font.pixelSize: 12; color: isMacOS ? MacColors.secondaryLabel : "white"; anchors.verticalCenter: parent.verticalCenter }
        ValueField { id: vf; fieldWidth: 64; onValueEdited: parent.edited(newValue) }
    }

    Column {
        width: parent.width
        spacing: 10

        Text { text: qsTr("Linkwitz Transform"); font.pixelSize: 13; font.weight: Font.DemiBold; color: isMacOS ? MacColors.label : "white" }

        Row {
            spacing: 20
            Column {
                spacing: 6
                Text { text: qsTr("DRIVER"); font.pixelSize: 9; font.weight: Font.Bold; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45) }
                Field { label: qsTr("f0"); suffix: qsTr("Hz"); decimals: 1; wheelStep: 1; minValue: 10; maxValue: 500; value: lt.f0; onEdited: lt.f0 = v }
                Field { label: qsTr("Q0"); suffix: ""; decimals: 3; wheelStep: 0.1; minValue: 0.1; maxValue: 20; value: lt.q0; onEdited: lt.q0 = v }
            }
            Column {
                spacing: 6
                Text { text: qsTr("TARGET"); font.pixelSize: 9; font.weight: Font.Bold; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45) }
                Field { label: qsTr("fp"); suffix: qsTr("Hz"); decimals: 1; wheelStep: 1; minValue: 10; maxValue: 500; value: lt.fp; onEdited: lt.fp = v }
                Field { label: qsTr("Qp"); suffix: ""; decimals: 3; wheelStep: 0.1; minValue: 0.1; maxValue: 20; value: lt.qp; onEdited: lt.qp = v }
            }
        }

        Text {
            width: parent.width
            wrapMode: Text.WordWrap
            font.pixelSize: 11
            color: isMacOS ? (lt.dcBoost > 15 ? MacColors.orange : MacColors.label) : lt.dcBoost > 15 ? "#ff9800" : Qt.rgba(1, 1, 1, 0.7)
            text: lt.dcBoost > 15 ? qsTr("DC boost %1 dB — large boosts cost headroom and cone excursion").arg((lt.dcBoost >= 0 ? "+" : "") + lt.dcBoost.toFixed(1))
                                  : qsTr("DC boost %1 dB").arg((lt.dcBoost >= 0 ? "+" : "") + lt.dcBoost.toFixed(1))
        }

        Row {
            spacing: 8
            Button { text: qsTr("Revert"); enabled: { lt.rev; return !lt.applied } onClicked: lt.load() }
            Button { text: qsTr("Apply"); highlighted: true; enabled: { lt.rev; return !lt.applied } onClicked: lt.apply() }
            Text {
                text: { lt.rev; return lt.applied ? qsTr("Applied") : qsTr("Not applied yet") }
                font.pixelSize: 11
                color: isMacOS ? (lt.rev >= 0 && lt.applied ? MacColors.secondaryLabel : MacColors.orange) : Qt.rgba(1, 1, 1, 0.5)
                anchors.verticalCenter: parent.verticalCenter
            }
        }
    }

    Shortcut {
        sequences: ["Return", "Enter"]
        enabled: lt.visible
        // Take focus off the field first so a value being typed commits.
        onActivated: { lt.contentItem.forceActiveFocus(); Qt.callLater(lt.apply) }
    }
}
