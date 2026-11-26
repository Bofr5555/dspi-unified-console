import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP UB2 -- (UXP B2, D4) Home: a remote for the daily listener, Simple mode's
// first page and the locked user's page (docs/UI-MODES.md §2). The device and its
// connection, one big User volume with Mute, the source as segments, the presets
// as a Recall list and the hub installation's health in one line; nothing else.
// Nothing here is refused: the installer's lock lets volume, mute, the source
// (-- FIX016 -- LOCKSRC, one core rule) and preset loads through, and the Master
// volume, Save and Copy are not on the page (the Installer's and Library's).
Item {
    id: page
    objectName: "HomeSection"
    property bool shown: false
    property Item host: null

    readonly property bool writable: DeviceModel.connected && !DeviceModel.readOnly
    readonly property bool locked: LimitsController.locked
    readonly property int sourceIndex: SidebarController.sources.findIndex(s => s.value === SidebarController.source)

    EmptyState {
        objectName: "HomeSection.empty"
        anchors.fill: parent
        visible: !DeviceModel.connected
        iconName: "plug"
        title: qsTr("No Device Connected")
        text: qsTr("Open a DSPi, a hub's device or the demo.")
        actionText: qsTr("Choose a Device…")
        actionPrimary: true
        onAction: if (page.host) page.host.go("devices", "devices")
    }

    Flickable {
        id: flick
        objectName: "HomeSection.scroll"
        anchors.fill: parent
        visible: DeviceModel.connected
        contentHeight: column.implicitHeight + 2 * Theme.spacingSection
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: AppScrollBar { visible: flick.contentHeight > flick.height }

        ColumnLayout {
            id: column
            x: Theme.spacingSection
            y: Theme.spacingSection
            width: Math.min(flick.width - 2 * Theme.spacingSection, Theme.contentMaxWidth)
            spacing: Theme.spacingLarge

            // ---- The device and its connection
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacing
                Text {
                    objectName: "HomeSection.name"
                    Layout.fillWidth: true
                    text: DeviceModel.deviceTitle
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    font: Qt.font({ family: Theme.fontTitle.family, pixelSize: 22, weight: Font.DemiBold })
                    color: Theme.text
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
                Rectangle {
                    width: 8
                    height: 8
                    radius: 4
                    color: DeviceModel.connected ? Theme.success : Theme.offline
                }
                Text {
                    objectName: "HomeSection.connection"
                    text: !DeviceModel.connected ? qsTr("Not connected")
                          : DeviceModel.demo ? qsTr("Connected · Demo")
                          : LinkController.hubName.length ? qsTr("Connected · %1").arg(LinkController.hubName)
                          : qsTr("Connected · USB")
                    textFormat: Text.PlainText
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
            }

            // ---- One big User volume, Mute, the source
            Rectangle {
                objectName: "HomeSection.volumeCard"
                Layout.fillWidth: true
                implicitHeight: volumeColumn.implicitHeight + 2 * Theme.spacingLarge
                radius: Theme.radius
                color: Theme.panel
                ColumnLayout {
                    id: volumeColumn
                    anchors.fill: parent
                    anchors.margins: Theme.spacingLarge
                    spacing: Theme.spacing
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.spacing
                        Icon {
                            name: ShortcutController.muted ? "volume-x" : "volume-2"
                            size: 20
                            color: Theme.textMuted
                        }
                        CustomSlider {
                            id: volume
                            objectName: "HomeSection.volume"
                            Layout.fillWidth: true
                            implicitHeight: 32
                            from: 0
                            to: 1
                            label: qsTr("Volume")
                            enabled: page.writable
                            resetValue: SidebarController.userDbToPosition(0)
                            Binding on value {
                                when: !volume.pressed
                                value: SidebarController.userDbToPosition(SidebarController.userVolumeDb)
                                restoreMode: Binding.RestoreNone
                            }
                            onValueMoved: position => SidebarController.setUserVolume(SidebarController.userPositionToDb(position))
                            onCommitted: position => SidebarController.setUserVolume(SidebarController.userPositionToDb(position))
                            Accessible.description: readout.text
                        }
                        Text {
                            id: readout
                            objectName: "HomeSection.readout"
                            property string shownName: text
                            Layout.preferredWidth: 96
                            horizontalAlignment: Text.AlignRight
                            text: SidebarController.formatUserDb(SidebarController.userPositionToDb(volume.value))
                            font: Qt.font({ family: Theme.fontMono.family, pixelSize: 20, weight: Font.DemiBold })
                            color: Theme.text
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.spacingLarge
                        OptionChip {
                            objectName: "HomeSection.mute"
                            implicitHeight: 32
                            iconName: "volume-x"
                            text: qsTr("Mute")
                            checked: ShortcutController.muted
                            enabled: page.writable
                            onClicked: ShortcutController.toggleMute()
                        }
                        ToolSegments {
                            id: sources
                            objectName: "HomeSection.source"
                            // -- FIX016 -- (LOCKSRC) a working control under the lock too
                            visible: SidebarController.sourceSupported && SidebarController.sources.length > 1
                            enabled: page.writable
                            label: qsTr("Source")
                            model: SidebarController.sources.map(s => s.title)
                            current: page.sourceIndex
                            onChosen: index => SidebarController.setSource(SidebarController.sources[index].value)
                        }
                        // One source only: what plays, in words
                        Text {
                            objectName: "HomeSection.sourceText"
                            property string shownName: text
                            visible: SidebarController.sourceSupported && !sources.visible
                            text: qsTr("Source: %1").arg(SidebarController.sourceTitle(SidebarController.source))
                            textFormat: Text.PlainText
                            font: Theme.fontBody
                            color: Theme.textMuted
                        }
                        Item { Layout.fillWidth: true }
                    }
                }
            }

            // ---- The presets as a Recall list
            Rectangle {
                objectName: "HomeSection.presets"
                Layout.fillWidth: true
                implicitHeight: presetColumn.implicitHeight + 2 * Theme.spacingLarge
                radius: Theme.radius
                color: Theme.panel
                ColumnLayout {
                    id: presetColumn
                    anchors.fill: parent
                    anchors.margins: Theme.spacingLarge
                    spacing: Theme.spacing
                    Text {
                        text: qsTr("PRESETS")
                        font: Theme.fontHeading
                        color: Theme.textMuted
                    }
                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.spacingSmall
                        Repeater {
                            model: DeviceModel.presets
                            delegate: OptionChip {
                                required property int slot
                                required property string name
                                required property bool occupied
                                required property bool active
                                objectName: "HomeSection.preset." + slot
                                property string shownName: text
                                // Stored slots and the one that plays; an empty one has nothing to recall
                                visible: occupied || active
                                implicitHeight: 32
                                text: name.length ? name : occupied ? qsTr("Preset %1").arg(slot + 1) : qsTr("Empty")
                                toolTip: active ? qsTr("Preset %1: %2, playing now").arg(slot + 1).arg(text)
                                                : qsTr("Recall preset %1: %2").arg(slot + 1).arg(text)
                                exclusive: true
                                checked: active
                                enabled: page.writable && !SidebarController.presetBusy
                                onClicked: if (!active) SidebarController.requestPresetSwitch(slot)
                            }
                        }
                    }
                }
            }

            // ---- One health line: the hub installation's, where there is one
            RowLayout {
                objectName: "HomeSection.health"
                Layout.fillWidth: true
                visible: SystemController.available && SystemController.installed
                spacing: Theme.spacingSmall
                Rectangle {
                    width: 8
                    height: 8
                    radius: 4
                    color: SystemController.health === "ok" ? Theme.success
                         : SystemController.health === "warning" ? Theme.warning
                         : SystemController.health === "problem" ? Theme.dangerText : Theme.offline
                }
                Text {
                    Layout.fillWidth: true
                    text: SystemController.summary
                    textFormat: Text.PlainText
                    wrapMode: Text.Wrap
                    font: Theme.fontBody
                    color: Theme.text
                }
            }
        }
    }

    Accessible.role: Accessible.Pane
    Accessible.name: qsTr("Home")
}
