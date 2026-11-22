import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- An output limiter's settings (the Mac's OutputLimiterSettings):
// the switch; threshold and release; the link group and who shares it; copy to
// all outputs, and the All outputs menu (link stereo pairs, unlink, protect all
// outputs, switch every limiter off). The settings stay in place, greyed out,
// while the limiter is off, so the content never changes size.
//
// Every edit is part of the output configuration (ToolsController marks the
// save bar first).
//
// -- WP LAY2 -- The content alone: Classic shows it in a popover over the
// output card's limiter cell (LimiterSettings), Modern in its inspector
// (InspectorSection). The host sets `active` while it shows, which polls the
// limiter's gain reduction.
ColumnLayout {
    id: content
    property int output: 0
    property bool active: false

    readonly property var settings: ToolsController.revision < 0 ? ({}) : ToolsController.limiter(output)
    readonly property bool on: settings.enabled === true
    readonly property bool writable: ToolsController.writable && ToolsController.limiterSupported
    readonly property string outputName: { ToolsController.revision; return ToolsController.outputName(output) }

    onActiveChanged: ChannelController.watchLimiter(active)
    Component.onDestruction: if (active) ChannelController.watchLimiter(false)

    Layout.fillWidth: true
    spacing: 14

    RowLayout {
        Layout.fillWidth: true
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            Text {
                text: qsTr("Output Limiter")
                font.family: Theme.fontBody.family
                font.pixelSize: Theme.fontBody.pixelSize
                font.weight: Font.DemiBold
                color: Theme.text
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            Text {
                objectName: "LimiterSettings.output"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: content.outputName
                font: Theme.fontCaption
                color: Theme.textMuted
                elide: Text.ElideRight
            }
        }
        AppSwitch {
            id: toggle
            objectName: "LimiterSettings.enabled"
            text: ""
            enabled: content.writable
            checked: content.on
            onToggled: ToolsController.setLimiterEnabled(content.output, checked)
            Accessible.name: qsTr("Output limiter")
            HoverHandler { id: toggleHover }
            HelpTip {
                visible: toggleHover.hovered
                text: qsTr("A test signal from the Signal Generator is limited like any other signal. Switch this output's limiter off for an unaltered full-scale measurement.")
            }
        }
    }

    // Greyed out while off, so an output without a limiter never looks limited
    ColumnLayout {
        objectName: "LimiterSettings.settings"
        Layout.fillWidth: true
        spacing: 14
        enabled: content.on
        opacity: content.on ? 1 : 0.4
        Behavior on opacity { NumberAnimation { duration: 150 } }

        ToolParameterRow {
            objectName: "LimiterSettings.threshold"
            Layout.fillWidth: true
            enabled: content.writable
            title: qsTr("Threshold")
            subtitle: qsTr("The ceiling, in dBFS. No sample leaves this output above it.")
            unit: qsTr("dB")
            value: content.settings.threshold !== undefined ? content.settings.threshold : -1
            from: -30
            to: 0
            scrollStep: 0.5
            resetValue: -1
            ends: [qsTr("-30 dBFS"), qsTr("0 dBFS")]
            help: qsTr("The limiter runs after every gain stage, so this is the absolute level leaving the device. The -1 dBFS default leaves room for the small overshoot a DAC can produce between samples.")
            onLive: v => ToolsController.setLimiterThreshold(content.output, v)
            onCommit: v => ToolsController.setLimiterThreshold(content.output, v)
        }
        ToolParameterRow {
            objectName: "LimiterSettings.release"
            Layout.fillWidth: true
            enabled: content.writable
            title: qsTr("Release")
            subtitle: qsTr("How fast the gain recovers after a peak")
            unit: qsTr("ms")
            value: content.settings.release !== undefined ? content.settings.release : 100
            from: 10
            to: 1000
            decimals: 0
            scrollStep: 10
            resetValue: 100
            ends: [qsTr("10 ms"), qsTr("1000 ms")]
            help: qsTr("The gain recovers 8.7 dB per release time. Short releases keep the level up but can be heard pumping on dense material; long ones are smoother but hold the level down for longer after a peak. Attack is fixed at 16 samples and always completes before the peak arrives.")
            onLive: v => ToolsController.setLimiterRelease(content.output, v)
            onCommit: v => ToolsController.setLimiterRelease(content.output, v)
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            Text {
                text: qsTr("Link group")
                font: Theme.fontBody
                color: content.writable ? Theme.text : Theme.textDisabled
            }
            ToolSegments {
                objectName: "LimiterSettings.group"
                Layout.fillWidth: true
                enabled: content.writable
                label: qsTr("Link group")
                model: [qsTr("Off"), "1", "2", "3", "4"]
                current: content.settings.group !== undefined ? content.settings.group : 0
                onChosen: index => ToolsController.setLimiterGroup(content.output, index)
            }
            Text {
                objectName: "LimiterSettings.groupSummary"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: ToolsController.revision < 0 ? "" : ToolsController.limiterLinkSummary(content.output)
                font: Theme.fontCaption
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            HoverHandler { id: groupHover }
            HelpTip {
                visible: groupHover.hovered
                text: qsTr("Outputs in the same group act as one limiter: they share on/off, threshold and release, so changing one changes them all, and each applies the deepest gain reduction any of them needs. A stereo image cannot shift and a pair of woofers stays matched. An output joining a group takes on the group's settings; leaving keeps them.")
            }
        }
    }

    EffectRule {}

    // The actions that reach across outputs work whether this one is on or not
    RowLayout {
        Layout.fillWidth: true
        enabled: content.writable
        AppButton {
            objectName: "LimiterSettings.copyToAll"
            text: qsTr("Copy to all outputs")
            toolTip: qsTr("Give every output this output's threshold, release and on/off state. Link groups are left as they are.")
            onClicked: ToolsController.copyLimiterToAll(content.output)
        }
        Item { Layout.fillWidth: true }
        EffectMenuButton {
            objectName: "LimiterSettings.allOutputs"
            text: qsTr("All outputs")
            AppMenuItem {
                objectName: "LimiterSettings.linkPairs"
                text: qsTr("Link all stereo pairs")
                onTriggered: ToolsController.linkLimiterPairs()
            }
            AppMenuItem {
                objectName: "LimiterSettings.unlinkAll"
                text: qsTr("Unlink all outputs")
                onTriggered: ToolsController.unlinkLimiters()
            }
            MenuSeparator {}
            AppMenuItem {
                objectName: "LimiterSettings.protectAll"
                //: Switches every output's limiter on at -1 dBFS and links the stereo pairs
                text: qsTr("Protect all outputs")
                onTriggered: ToolsController.protectAllOutputs()
            }
            AppMenuItem {
                objectName: "LimiterSettings.allOff"
                text: qsTr("Switch every limiter off")
                onTriggered: ToolsController.switchLimitersOff()
            }
        }
    }
    Text {
        Layout.fillWidth: true
        text: qsTr("Protect all outputs switches every limiter on at -1 dBFS and links the stereo pairs.")
        font: Theme.fontCaption
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
    // -- WP P1QT2 -- PRO1 S2's warning: an output above -10 dB of gain with no limiter
    RowLayout {
        objectName: "LimiterSettings.unprotected"
        Layout.fillWidth: true
        visible: ToolsController.unprotectedText.length > 0
        spacing: Theme.spacingSmall
        Icon {
            name: "triangle-alert"
            size: 14
            color: Theme.warning
        }
        Text {
            Layout.fillWidth: true
            text: ToolsController.unprotectedText
            font: Theme.fontCaption
            color: Theme.text
            wrapMode: Text.Wrap
        }
    }
}
