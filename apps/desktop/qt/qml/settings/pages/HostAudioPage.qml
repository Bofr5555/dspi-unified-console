import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP FEATUI -- Settings > This Computer (FEAT3; both layouts): for each
// open DSPi, whether this computer plays to it, whether it is the default
// output, its format, what plays to it, the OS volume and whether that moves
// the DSPi's user volume; how to make it the default. The same reading an AI
// agent gets (dspi_get_host_audio). It reads; it changes nothing.
SettingsPage {
    id: page
    objectName: "HostAudioPage"
    readonly property var r: HostAudioController.reading
    readonly property bool known: r.status === "known"
    readonly property bool watching: visible && Window.window !== null && Window.window.visible
    onWatchingChanged: if (watching) HostAudioController.refresh()

    function yesNo(v) { return v === true ? qsTr("Yes") : v === false ? qsTr("No") : qsTr("Cannot tell") }
    function format(d) {
        const parts = []
        //: %n audio channels
        if (d.channels) parts.push(qsTr("%n channel(s)", "", d.channels))
        //: %1 a sample rate in kHz
        if (d.sample_rate_hz) parts.push(qsTr("%1 kHz").arg((d.sample_rate_hz / 1000).toLocaleString(Qt.locale(), "f", 1)))
        return parts.length > 0 ? parts.join(", ") : qsTr("Cannot tell")
    }
    function streams(list) {
        if (list === null || list === undefined) return qsTr("This system does not say which apps play where.")
        if (list.length === 0) return qsTr("Nothing")
        return list.map(s => s.app_name + (s.media_name ? " (" + s.media_name + ")" : "")).join(", ")
    }

    SettingsGroup {
        objectName: "HostAudio.status"
        title: qsTr("Sound on This Computer")
        iconName: "monitor"
        footer: qsTr("A DSPi has no player: it plays what reaches its input. Over USB that is this computer's sound, when the DSPi is its output.")
        SettingsValueRow {
            objectName: "HostAudio.system"
            title: qsTr("Sound system")
            value: page.r.status === undefined ? (HostAudioController.busy ? qsTr("Reading…") : "")
                   : page.known ? (page.r.audio_system || qsTr("Known"))
                   //: %1 why the computer's audio could not be read (in English, from the core)
                   : qsTr("Cannot be read: %1").arg(page.r.why_unknown || "")
        }
        SettingsValueRow {
            objectName: "HostAudio.default"
            visible: page.known
            title: qsTr("Default output")
            value: page.r.default_output ? page.r.default_output.output_name + (page.r.default_output.is_dspi ? "  " + qsTr("(a DSPi)") : "")
                                         : qsTr("None")
        }
        RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "HostAudio.refresh"
                text: qsTr("Read Again")
                enabled: !HostAudioController.busy
                onClicked: HostAudioController.refresh()
            }
        }
    }

    Repeater {
        model: page.r.devices || []
        delegate: SettingsGroup {
            id: dev
            required property var modelData
            required property int index
            objectName: "HostAudio.device." + index
            title: modelData.device_name || modelData.device
            iconName: "volume-2"
            footer: modelData.output_device === "yes" && modelData.is_default_output === false ? HostAudioController.howToDefault : ""
            SettingsValueRow {
                objectName: "HostAudio.output." + dev.index
                title: qsTr("Output of this computer")
                value: dev.modelData.output_device === "yes" ? qsTr("Yes, as %1").arg(dev.modelData.output_name)
                       //: %1 why not (in English, from the core)
                       : dev.modelData.output_device === "no" ? qsTr("No: %1").arg(dev.modelData.why)
                       //: %1 why it cannot tell (in English, from the core)
                       : qsTr("Cannot tell: %1").arg(dev.modelData.why)
            }
            SettingsValueRow {
                visible: dev.modelData.output_device === "yes"
                objectName: "HostAudio.isDefault." + dev.index
                title: qsTr("The default output")
                value: page.yesNo(dev.modelData.is_default_output)
            }
            SettingsValueRow {
                visible: dev.modelData.output_device === "yes"
                title: qsTr("Format")
                value: page.format(dev.modelData)
            }
            SettingsValueRow {
                visible: dev.modelData.output_device === "yes"
                objectName: "HostAudio.plays." + dev.index
                title: qsTr("Playing to it")
                value: page.streams(dev.modelData.streams)
            }
            SettingsValueRow {
                visible: dev.modelData.output_device === "yes"
                title: qsTr("System volume")
                value: (dev.modelData.os_volume_pct === null || dev.modelData.os_volume_pct === undefined ? qsTr("Cannot tell")
                        : qsTr("%1 %").arg(Math.round(dev.modelData.os_volume_pct)))
                       + (dev.modelData.os_muted === true ? "  " + qsTr("(muted)") : "")
            }
            SettingsValueRow {
                visible: dev.modelData.output_device === "yes"
                objectName: "HostAudio.drives." + dev.index
                title: qsTr("System volume moves the DSPi's volume")
                value: page.yesNo(dev.modelData.os_volume_drives_user_volume)
            }
        }
    }
    SettingsGroup {
        objectName: "HostAudio.none"
        visible: page.r.status !== undefined && (page.r.devices || []).length === 0
        footer: qsTr("No DSPi is open. Open one to see whether this computer plays to it.")
    }
}
