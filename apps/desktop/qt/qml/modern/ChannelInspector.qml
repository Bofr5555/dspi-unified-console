import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- The inspector's content for a channel (MODLAY 2): the selected
// band on the PEQ page; for an input what it feeds and its headphone profile;
// for an output its crossover, its limiter and its Linkwitz Transform (the
// same content as Classic's popovers). `host` is the page host.
ColumnLayout {
    id: inspector
    property int channel: -1
    property Item host: null
    // The PEQ page: the selected band (-1 for none)
    property bool bandShown: false
    property int band: -1
    property int bandsSelected: 0

    readonly property bool isOutput: info.isOutput
    readonly property int output: isOutput ? info.number : -1
    readonly property var out: ChannelController.revision < 0 || output < 0 ? ({}) : ChannelController.output(output)
    readonly property bool crossover: isOutput && ChannelController.crossoverSupported
    // The band that is the Linkwitz Transform: the selected one, else the first
    readonly property int linkwitzBand: {
        if (ChannelController.revision < 0 || !isOutput) return -1
        const is = b => ChannelController.band(channel, b, false).type === ChannelController.linkwitzType
        if (band >= 0 && is(band)) return band
        for (let b = 0; b < 10; ++b) if (is(b)) return b
        return -1
    }
    readonly property bool linkwitzSupported: ChannelController.revision >= 0 && isOutput
        && ChannelController.availableTypes(true).indexOf(ChannelController.linkwitzType) >= 0

    function typeValues(field) {
        bandSection.expanded = true
        bandInspector.typeValues(field)
    }

    objectName: "ChannelInspector"
    spacing: Theme.spacing

    ChannelInfo { id: info; channel: inspector.channel }

    component Section: InspectorSection {
        id: section
        // Folded as it was, over the pages and the channels
        required property string sectionId
        property bool openAtFirst: true
        objectName: "Inspector." + sectionId
        Layout.fillWidth: true
        Component.onCompleted: expanded = InspectorState.isExpanded(sectionId, openAtFirst)
        onExpandedChanged: InspectorState.setExpanded(sectionId, expanded)
        Connections {
            target: InspectorState
            function onExpandedChanged() {
                const want = InspectorState.isExpanded(section.sectionId, section.openAtFirst)
                if (section.expanded !== want) section.expanded = want
            }
        }
    }
    component Note: Text {
        Layout.fillWidth: true
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.rightMargin: 28   // the close button
        spacing: Theme.spacingSmall
        Pill {
            visible: info.known
            text: info.descriptor
            tint: info.tint
        }
        Text {
            objectName: "ChannelInspector.title"
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: info.label
            font: Theme.fontTitle
            color: Theme.text
            elide: Text.ElideRight
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
    }

    // ---- The band (PEQ) ----
    Section {
        id: bandSection
        sectionId: "band"
        visible: inspector.bandShown
        title: inspector.band >= 0 ? qsTr("Band %1").arg(inspector.band + 1) : qsTr("Band")
        BandInspector {
            id: bandInspector
            Layout.fillWidth: true
            channel: inspector.channel
            index: inspector.band
            includeLinkwitz: inspector.isOutput
            selected: inspector.bandsSelected
        }
    }

    // ---- An input ----
    Section {
        sectionId: "feeds"
        visible: info.known && !inspector.isOutput
        title: qsTr("Feeds")
        readonly property var outputs: {
            const list = []
            if (ChannelController.revision < 0 || inspector.isOutput || inspector.channel < 0) return list
            for (let o = 0; o < ChannelController.numOutputs; ++o) {
                const r = ChannelController.route(inspector.channel, o)
                if (r.enabled === true) list.push({ output: o, gain: r.gain, invert: r.invert === true })
            }
            return list
        }
        summary: outputs.length === 1 ? qsTr("1 output") : qsTr("%1 outputs").arg(outputs.length)
        id: feeds
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            Note {
                visible: feeds.outputs.length === 0
                text: qsTr("This input is connected to no output.")
            }
            Repeater {
                model: feeds.outputs
                AbstractButton {
                    id: fed
                    required property var modelData
                    objectName: "Inspector.feeds." + modelData.output
                    Layout.fillWidth: true
                    implicitHeight: 26
                    hoverEnabled: true
                    focusPolicy: Qt.TabFocus
                    onClicked: SidebarController.select(ChannelController.outputChannel(modelData.output))
                    ChannelInfo { id: target; channel: ChannelController.revision < 0 ? -1 : ChannelController.outputChannel(fed.modelData.output) }
                    contentItem: RowLayout {
                        spacing: Theme.spacingSmall
                        Pill {
                            Layout.leftMargin: 6
                            text: target.descriptor
                            tint: target.tint
                        }
                        Text {
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: target.label
                            font: Theme.fontBody
                            color: Theme.text
                            elide: Text.ElideRight
                        }
                        Text {
                            Layout.rightMargin: 6
                            text: (fed.modelData.invert ? "ø  " : "") + Number(fed.modelData.gain).toFixed(1) + " " + qsTr("dB")
                            font: Theme.fontMono
                            color: Theme.textMuted
                        }
                    }
                    background: Rectangle {
                        radius: Theme.radiusSmall
                        color: fed.hovered ? Theme.controlHover : Theme.control
                        border.width: fed.visualFocus ? 2 : 0
                        border.color: Theme.focusRing
                    }
                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("%1, %2 dB").arg(target.label).arg(Number(modelData.gain).toFixed(1))
                    Accessible.onPressAction: fed.clicked()
                }
            }
            AppButton {
                objectName: "Inspector.editRouting"
                Layout.topMargin: 4
                text: qsTr("Edit Routing…")
                onClicked: Shell.trigger("tools.matrix")
            }
        }
    }
    Section {
        sectionId: "headphones"
        openAtFirst: false
        visible: info.known && !inspector.isOutput
        title: qsTr("Headphone Profile")
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            Note { text: qsTr("A measured correction for your headphones, written to this input's bands.") }
            AppButton {
                objectName: "Inspector.browseProfiles"
                text: qsTr("Browse Profiles…")
                enabled: Shell.allows("autoeq.browse")
                onClicked: Shell.trigger("autoeq.browse")
            }
        }
    }

    // ---- An output ----
    Section {
        sectionId: "crossover"
        visible: inspector.crossover
        title: qsTr("Crossover")
        summary: {
            const parts = []
            if (highPass.on) parts.push(qsTr("HP %1").arg(highPass.frequencyText))
            if (lowPass.on) parts.push(qsTr("LP %1").arg(lowPass.frequencyText))
            return parts.length ? parts.join(", ") : qsTr("Off")
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            Text { text: qsTr("High pass"); font: Theme.fontBody; color: Theme.text }
            EqualiserCrossoverSide {
                id: highPass
                objectName: "Inspector.highPass"
                Layout.fillWidth: true
                channel: inspector.crossover ? inspector.channel : -1
                highPass: true
            }
            Text { text: qsTr("Low pass"); font: Theme.fontBody; color: Theme.text }
            EqualiserCrossoverSide {
                id: lowPass
                objectName: "Inspector.lowPass"
                Layout.fillWidth: true
                channel: inspector.crossover ? inspector.channel : -1
                highPass: false
            }
            Note {
                objectName: "Inspector.tweeter"
                // The user's layout, not the one suggested from the filters
                readonly property bool stored: ["confirmed", "unconfirmed", "imported"].indexOf(SpeakersController.status) >= 0
                visible: inspector.isOutput && stored && SpeakersController.rows.length >= 0
                         && SpeakersController.roleOf(inspector.output) === "high" && (!highPass.on || highPass.bypassed)
                color: Theme.warning
                text: qsTr("The speaker layout has a tweeter here. A tweeter without its high pass can be damaged.")
            }
        }
    }
    Section {
        id: limiterSection
        sectionId: "limiter"
        openAtFirst: false
        visible: inspector.isOutput && ChannelController.limiterSupported
        title: qsTr("Limiter")
        summary: inspector.out.limiter === true ? qsTr("%1 dBFS").arg(Number(inspector.out.limiterThreshold).toFixed(1)) : qsTr("Off")
        LimiterSettingsContent {
            showHeading: false
            output: Math.max(0, inspector.output)
            active: limiterSection.shown
        }
    }
    Section {
        id: linkwitzSection
        sectionId: "linkwitz"
        openAtFirst: false
        visible: inspector.linkwitzSupported
        title: qsTr("Linkwitz Transform")
        summary: inspector.linkwitzBand >= 0 ? qsTr("Band %1").arg(inspector.linkwitzBand + 1) : qsTr("Off")
        // Made anew for another channel or band: a draft is its own band's
        readonly property string target: inspector.linkwitzBand >= 0 ? inspector.channel + "/" + inspector.linkwitzBand : ""
        Repeater {
            model: linkwitzSection.target.length ? [linkwitzSection.target] : []
            LinkwitzContent {
                required property string modelData
                showHeading: false
                channel: Number(modelData.split("/")[0])
                band: Number(modelData.split("/")[1])
                active: linkwitzSection.shown
                // Shown from the start: its values are the band's
                Component.onCompleted: if (active) revert()
            }
        }
        ColumnLayout {
            visible: inspector.linkwitzBand < 0
            Layout.fillWidth: true
            spacing: 6
            Note { text: qsTr("Re-aligns a sealed woofer's rolloff. It takes one of this output's bands, and changes nothing until its values are applied.") }
            AppButton {
                objectName: "Inspector.addLinkwitz"
                readonly property int free: {
                    if (ChannelController.revision < 0 || inspector.channel < 0) return -1
                    for (let b = 0; b < 10; ++b) if (ChannelController.band(inspector.channel, b, false).type === 0) return b
                    return -1
                }
                enabled: free >= 0 && ChannelController.writable
                text: free >= 0 ? qsTr("Use Band %1").arg(free + 1) : qsTr("All bands are in use")
                onClicked: ChannelController.setBandType(inspector.channel, free, false, ChannelController.linkwitzType)
            }
        }
    }
    AppButton {
        objectName: "Inspector.editOutputRouting"
        visible: inspector.isOutput
        text: qsTr("Edit Routing…")
        onClicked: Shell.trigger("tools.matrix")
    }
}
