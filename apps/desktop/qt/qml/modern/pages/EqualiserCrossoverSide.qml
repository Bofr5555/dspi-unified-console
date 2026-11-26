import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- An output's high pass or low pass: the family with its slope,
// or Off, and the frequency. An output has four crossover bands; this is the
// first of its kind, and a new one takes the first that is off. The page's
// list of all four is under the table (EqualiserCrossover). -- WP DL8 -- That
// list shows when this cannot say it all; "+1" is muted (a count, not a warning).
RowLayout {
    id: side
    property int channel: -1
    property bool highPass: true
    // Where it stands in a table, its name for assistive technology
    property string owner: ""

    readonly property var bands: {
        const out = []
        if (ChannelController.revision < 0 || channel < 0) return out
        for (let b = 0; b < 4; ++b) out.push(ChannelController.band(channel, b, true))
        return out
    }
    function mine(p) { return p.type !== 0 && ChannelController.isLowPass(p.type) !== highPass }
    readonly property int band: bands.findIndex(p => mine(p))
    readonly property int more: bands.filter(p => mine(p)).length - 1
    readonly property int free: bands.findIndex(p => p.type === 0)
    readonly property bool on: band >= 0
    readonly property var params: band >= 0 && band < bands.length ? bands[band] : null
    readonly property bool bypassed: params ? params.bypass : false
    readonly property string frequencyText: !params ? "" : params.freq >= 1000
        ? qsTr("%1 kHz").arg(Number((params.freq / 1000).toFixed(2))) : qsTr("%1 Hz").arg(Math.round(params.freq))
    // A table's short names for Linkwitz-Riley, Butterworth and Bessel
    property bool brief: false
    readonly property var briefNames: ["", "LR", "BW", "BS"]
    readonly property int choice: !params ? 0 : choices.findIndex(c => c.type === params.type)
    // Off, then every family at every slope: [{type, label}]
    readonly property var choices: {
        const list = [{ type: 0, label: qsTr("Off") }]
        const families = ChannelController.crossoverFamilies()
        for (let f = 1; f < families.length; ++f)
            for (const order of ChannelController.ordersOf(f))
                list.push({
                    type: ChannelController.crossoverType(f, order, !highPass),
                    //: A crossover family and its slope ("Linkwitz-Riley 24 dB/oct")
                    label: qsTr("%1 %2 dB/oct").arg(families[f]).arg(order * 6),
                    //: The same, short ("LR 24"), as the page's legend explains it -- WP UB1 -- (A10)
                    brief: qsTr("%1 %2").arg(briefNames[f]).arg(order * 6)
                })
        return list
    }

    spacing: Theme.spacingSmall
    opacity: bypassed ? 0.5 : 1

    AppComboBox {
        objectName: side.objectName + ".type"
        Layout.fillWidth: true
        Layout.minimumWidth: 90
        Layout.maximumWidth: 210
        label: (side.owner.length ? side.owner + ", " : "") + (side.highPass ? qsTr("High pass") : qsTr("Low pass"))
        enabled: ChannelController.writable && (side.on || side.free >= 0)
        model: side.choices.map(c => c.label)
        currentIndex: side.choice
        displayText: side.brief && side.choice > 0 ? side.choices[side.choice].brief : currentText
        popup.width: Math.max(width, 210)
        onActivated: index => {
            const at = side.on ? side.band : side.free
            // -- WP UB1 -- (UXP B1) a pass the output had none of opens at its
            // partner's frequency (a woofer's low pass at its tweeter's high pass)
            const partner = side.on || index === 0 ? 0 : LayoutSetupController.partnerFrequency(side.channel, side.highPass)
            if (at >= 0) ChannelController.setBandType(side.channel, at, true, side.choices[index].type, partner)
            // A refused choice shows the device's again
            currentIndex = Qt.binding(() => side.choice)
        }
        Accessible.description: currentText
    }
    ChannelValueField {
        objectName: side.objectName + ".freq"
        // Its place is kept, so a table's columns stand still
        opacity: side.on ? 1 : 0
        enabled: side.on && ChannelController.writable
        label: (side.owner.length ? side.owner + ", " : "") + (side.highPass ? qsTr("High pass frequency") : qsTr("Low pass frequency"))
        unit: qsTr("Hz")
        fieldWidth: 64
        Layout.fillWidth: false
        scrollStep: 10
        minValue: 10
        value: side.params ? side.params.freq : 1000
        onCommitted: v => { if (side.on) ChannelController.setBandValue(side.channel, side.band, true, "freq", v) }
    }
    Text {
        objectName: side.objectName + ".more"
        visible: side.more > 0
        //: More bands of the same kind than the one shown
        text: qsTr("+%1").arg(side.more)
        font: Theme.fontSmall
        color: Theme.textMuted   // -- WP DL8 -- never warning text
        Accessible.name: side.highPass ? qsTr("%1 more high passes").arg(side.more) : qsTr("%1 more low passes").arg(side.more)
    }
}
