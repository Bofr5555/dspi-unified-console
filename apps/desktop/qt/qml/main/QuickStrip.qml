import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP1 -- The quick strip under the channel lists (ContentView.swift's
// "Quick Access Icons"): the matrix, the four effects, stats, settings and the
// master bypass. A click switches an effect or opens/closes a window; a
// right-click on an effect opens its tool window (PARITY M60). On read-only
// firmware the switches send nothing; their windows still open.
RowLayout {
    id: strip
    objectName: "QuickStrip"
    spacing: 0

    // One effect's switch, with its tool window on the right-click
    component EffectButton: IconButton {
        required property string effect
        required property string toolAction
        property bool on: false
        Layout.fillWidth: true
        checked: on
        onClicked: if (!DeviceModel.readOnly) SidebarController.setEffect(effect, !on)
        onSecondaryClicked: if (toolAction.length) Shell.trigger(toolAction)
        Accessible.checkable: true
        Accessible.checked: on
    }
    // A window's button, active while the window shows
    component WindowButton: IconButton {
        required property string windowAction
        Layout.fillWidth: true
        checked: SidebarController.openWindows.indexOf(windowAction) >= 0
        onClicked: SidebarController.toggleWindow(windowAction)
    }

    WindowButton {
        objectName: "QuickStrip.matrix"
        windowAction: "tools.matrix"
        iconName: "sliders-horizontal"
        toolTip: qsTr("Matrix Mixer")
    }
    EffectButton {
        objectName: "QuickStrip.crossfeed"
        effect: "crossfeed"
        toolAction: "tools.crossfeed"
        on: SidebarController.crossfeedOn
        iconName: "headphones"
        toolTip: qsTr("Headphone Crossfeed")
    }
    EffectButton {
        objectName: "QuickStrip.loudness"
        effect: "loudness"
        toolAction: "tools.loudness"
        on: SidebarController.loudnessOn
        iconName: "volume-2"
        toolTip: qsTr("Loudness Compensation")
    }
    EffectButton {
        objectName: "QuickStrip.leveller"
        effect: "leveller"
        toolAction: "tools.leveller"
        on: SidebarController.levellerOn
        iconName: "activity"
        toolTip: qsTr("Volume Leveller")
    }
    EffectButton {
        objectName: "QuickStrip.psybass"
        // Gone, not greyed, on firmware without it (behaviour.md 14.1)
        visible: SidebarController.psybassSupported
        effect: "psybass"
        toolAction: "tools.psybass"
        on: SidebarController.psybassOn
        iconName: "audio-lines"
        toolTip: qsTr("Psychoacoustic Bass")
    }
    WindowButton {
        objectName: "QuickStrip.stats"
        windowAction: "tools.stats"
        iconName: "info"
        toolTip: qsTr("Stats for Nerbs")
    }
    WindowButton {
        objectName: "QuickStrip.settings"
        windowAction: "edit.settings"
        iconName: "settings"
        toolTip: qsTr("Settings")
    }
    EffectButton {
        objectName: "QuickStrip.bypass"
        effect: "bypass"
        toolAction: ""
        on: SidebarController.bypassOn
        iconName: "x"
        toolTip: qsTr("Bypass Master EQ")
    }
}
