import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The dock along the bottom, on every page: the source, the
// User or Master volume, Mute, Bypass EQ, a meter per channel and the cores'
// loads. The same controllers as Classic's sidebar (GlobalControls).
Rectangle {
    id: dock
    objectName: "Dock"
    // The source's rename menu is the shell's (PresetDialogs)
    property Item dialogs: null
    readonly property bool writable: DeviceModel.connected && !DeviceModel.readOnly
    readonly property bool narrow: width < 1180

    implicitHeight: 44
    color: Theme.sidebar
    Accessible.role: Accessible.ToolBar
    Accessible.name: qsTr("Volume and Meters")

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.spacing
        anchors.rightMargin: Theme.spacing
        spacing: Theme.spacingSmall

        AppComboBox {
            id: sourceBox
            objectName: "Dock.source"
            readonly property string shownName: displayText
            visible: SidebarController.sourceSupported
            implicitWidth: 116
            label: qsTr("Source")
            model: SidebarController.sources
            textRole: "title"
            valueRole: "value"
            enabled: dock.writable
            // The device's, not the click's; again when the list changes
            function follow() { currentIndex = Qt.binding(() => indexOfValue(SidebarController.source)) }
            Component.onCompleted: follow()
            onModelChanged: Qt.callLater(follow)
            onActivated: index => {
                SidebarController.setSource(valueAt(index))
                follow()
            }
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: point => {
                    if (dock.writable && dock.dialogs && SidebarController.sourceNamesSupported)
                        dock.dialogs.popupSourceMenu(sourceBox, point.position.x, point.position.y)
                }
            }
        }
        ToolSegments {
            objectName: "Dock.mode"
            label: qsTr("Volume mode")
            model: [qsTr("User"), qsTr("Master")]
            current: volume.master ? 1 : 0
            onChosen: index => SidebarController.volumeMode = index === 1 ? "master" : "user"
        }
        Icon {
            name: ShortcutController.muted ? "volume-x" : "volume-2"
            size: 14
            color: Theme.textMuted
        }

        // User (-60..0 dB, square-root taper) or Master (-128..0 dB)
        CustomSlider {
            id: volume
            objectName: "Dock.volume"
            readonly property bool master: SidebarController.volumeMode === "master"
            readonly property real deviceDb: master ? SidebarController.masterVolumeDb : SidebarController.userVolumeDb
            function toPosition(db) { return master ? SidebarController.masterDbToPosition(db) : SidebarController.userDbToPosition(db) }
            function toDb(position) { return master ? SidebarController.masterPositionToDb(position) : SidebarController.userPositionToDb(position) }
            function send(db) {
                if (master) SidebarController.setMasterVolume(db)
                else SidebarController.setUserVolume(db)
            }
            Layout.fillWidth: true
            Layout.minimumWidth: 100
            from: 0
            to: 1
            label: master ? qsTr("Master Volume") : qsTr("User Volume")
            enabled: dock.writable
            // Right-click: back to 0 dB
            resetValue: toPosition(0)
            // Follows the device except while the person holds it
            Binding on value {
                when: !volume.pressed
                value: volume.toPosition(volume.deviceDb)
                restoreMode: Binding.RestoreNone
            }
            onValueMoved: position => volume.send(volume.toDb(position))
            onCommitted: position => volume.send(volume.toDb(position))
            Accessible.description: readout.text
        }
        Text {
            id: readout
            objectName: "Dock.readout"
            Layout.preferredWidth: 68
            horizontalAlignment: Text.AlignRight
            text: !DeviceModel.connected ? "–"
                  : volume.master ? SidebarController.formatMasterDb(volume.toDb(volume.value))
                                  : SidebarController.formatUserDb(volume.toDb(volume.value))
            font: Theme.fontMono
            color: Theme.text
        }

        DockToggle {
            objectName: "Dock.mute"
            iconName: "volume-x"
            text: qsTr("Mute")
            on: ShortcutController.muted
            onClicked: ShortcutController.toggleMute()
        }
        DockToggle {
            objectName: "Dock.bypass"
            iconName: "circle-x"
            text: qsTr("Bypass EQ")
            on: ShortcutController.bypassed
            onClicked: ShortcutController.toggleBypass()
        }

        // A meter per channel
        RowLayout {
            objectName: "Dock.meters"
            Layout.leftMargin: Theme.spacingSmall
            spacing: 3
            Repeater {
                model: DeviceModel.connected ? DeviceModel.channels : null
                delegate: LevelMeter {
                    id: channelMeter
                    required property string kind
                    required property bool outputEnabled
                    required property string label
                    required channel
                    objectName: "Dock.meter." + channel
                    visible: kind === "input" || outputEnabled
                    orientation: Qt.Vertical
                    segmented: false
                    implicitWidth: 6
                    implicitHeight: 28
                    accessibleName: label
                    HoverHandler { id: meterHover }
                    HelpTip {
                        text: channelMeter.label
                        visible: meterHover.hovered
                    }
                }
            }
        }
        CpuLoad { core: 0; load: Poller.cpu0 }
        CpuLoad { core: 1; load: Poller.cpu1 }
    }

    component DockToggle: AbstractButton {
        id: toggle
        property string iconName
        property bool on: false
        enabled: dock.writable
        implicitHeight: 28
        implicitWidth: dock.narrow ? 30 : face.implicitWidth + 16
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        readonly property color ink: !enabled ? Theme.textDisabled : on ? Theme.textOnAccent : Theme.text
        contentItem: Item {
            Row {
                id: face
                anchors.centerIn: parent
                spacing: 6
                Icon {
                    anchors.verticalCenter: parent.verticalCenter
                    name: toggle.iconName
                    size: 14
                    color: toggle.ink
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !dock.narrow
                    textFormat: Text.PlainText
                    text: toggle.text
                    font: Theme.fontBody
                    color: toggle.ink
                }
            }
        }
        background: Rectangle {
            radius: Theme.radiusSmall
            color: toggle.on && toggle.enabled ? Theme.danger
                 : toggle.down ? Theme.controlPressed : toggle.hovered ? Theme.controlHover : Theme.control
            border.width: toggle.visualFocus ? 2 : 0
            border.color: Theme.focusRing
        }
        HelpTip {
            text: toggle.text
            visible: dock.narrow && toggle.hovered
        }
        Accessible.role: Accessible.Button
        Accessible.name: text
        Accessible.checkable: true
        Accessible.checked: on
    }

    component CpuLoad: Text {
        textFormat: Text.PlainText
        required property int core
        required property int load
        readonly property bool live: Poller.running
        objectName: "Dock.cpu" + core
        readonly property string shownName: live ? load + "%" : "–"
        visible: !dock.narrow
        Layout.preferredWidth: 52
        horizontalAlignment: Text.AlignRight
        text: "C" + core + " " + shownName
        font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontCaption.pixelSize })
        color: live && load > 90 ? Theme.dangerText : Theme.textMuted
        Accessible.role: Accessible.ProgressBar
        //: %1 is the core's number, 0 or 1
        Accessible.name: qsTr("Core %1 load").arg(core)
        Accessible.description: shownName
    }
}
