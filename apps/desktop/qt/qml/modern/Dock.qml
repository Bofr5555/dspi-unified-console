import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The dock along the bottom, on every page: the source, the
// User or Master volume, Mute, Bypass EQ, a meter per channel and the cores'
// loads. The same controllers as Classic's sidebar (GlobalControls).
// -- WP UA5 -- User and Master say what they are on hover; the two cores'
// loads are one "DSP 3 %" readout (the busier core), both cores in its tooltip.
// -- WP UB34 -- (UXP B4) With two or more devices open, an "All devices" chip
// links them (DeviceLinkController, off by default): the User volume moves them
// all keeping their differences, Mute mutes them all, a preset is recalled on
// all after one question (asked here).
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
            AiTouched { path: "in." }   // -- WP AIVIS --
            readonly property string shownName: displayText
            visible: SidebarController.sourceSupported
            implicitWidth: 116
            fill: Theme.barControl   // -- WP REVDFIX -- D1
            fillHover: Theme.barControlHover
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
            id: mode
            objectName: "Dock.mode"
            track: Theme.barControl   // -- WP REVDFIX -- D1
            label: qsTr("Volume mode")
            model: [qsTr("User"), qsTr("Master")]
            current: volume.master ? 1 : 0
            onChosen: index => SidebarController.volumeMode = index === 1 ? "master" : "user"
            // -- WP UA5 -- what the two volumes are
            readonly property string help: qsTr("User: the everyday volume, −60…0 dB") + "\n"
                                           + qsTr("Master: the installer's ceiling, −128…0 dB")
            Accessible.description: help
            HoverHandler { id: modeHover }
            HelpTip {
                objectName: "Dock.mode.tip"
                text: mode.help
                visible: modeHover.hovered
            }
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
            AiTouched { objectName: "Dock.volume.touched"; path: "vol." }   // -- WP AIVIS --
            readonly property bool master: SidebarController.volumeMode === "master"
            readonly property real deviceDb: master ? SidebarController.masterVolumeDb : SidebarController.userVolumeDb
            function toPosition(db) { return master ? SidebarController.masterDbToPosition(db) : SidebarController.userDbToPosition(db) }
            function toDb(position) { return master ? SidebarController.masterPositionToDb(position) : SidebarController.userPositionToDb(position) }
            function send(db) {
                if (master) SidebarController.setMasterVolume(db)
                else if (DeviceLinkController.linked) DeviceLinkController.setUserVolume(db)   // -- WP UB34 --
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

        // -- WP UB34 -- (UXP B4) the link: a chip, chosen while on
        AbstractButton {
            id: link
            objectName: "Dock.link"
            visible: DeviceLinkController.available
            checkable: true
            checked: DeviceLinkController.linked
            enabled: dock.writable
            implicitHeight: 28
            implicitWidth: dock.narrow ? 30 : linkFace.implicitWidth + 16
            hoverEnabled: true
            focusPolicy: Qt.TabFocus
            text: qsTr("All devices")
            onToggled: DeviceLinkController.linked = checked
            readonly property color ink: !enabled ? Theme.textDisabled : checked ? Theme.accentText : Theme.text
            contentItem: Item {
                Row {
                    id: linkFace
                    anchors.centerIn: parent
                    spacing: 6
                    Icon {
                        anchors.verticalCenter: parent.verticalCenter
                        name: "link"
                        size: 14
                        color: link.ink
                    }
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: !dock.narrow
                        textFormat: Text.PlainText
                        text: link.text
                        font: Theme.fontBody
                        color: link.ink
                    }
                }
            }
            background: Rectangle {
                radius: Theme.radiusSmall
                color: link.checked && link.enabled ? Theme.selection
                     : link.down ? Theme.controlPressed : link.hovered ? Theme.barControlHover : Theme.barControl
                border.width: link.visualFocus ? 2 : 0
                border.color: Theme.focusRing
            }
            HoverHandler { id: linkHover }
            HelpTip {
                objectName: "Dock.link.tip"
                text: DeviceLinkController.help
                visible: linkHover.hovered && text.length > 0
            }
            Accessible.role: Accessible.CheckBox
            Accessible.name: text
            Accessible.description: DeviceLinkController.help
            Accessible.checkable: true
            Accessible.checked: checked
        }

        DockToggle {
            objectName: "Dock.mute"
            iconName: "volume-x"
            text: qsTr("Mute")
            on: ShortcutController.muted
            onClicked: ShortcutController.toggleMute()   // -- WP UB34 -- every device while linked
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
                    required property bool hidden
                    required property string label
                    required channel
                    objectName: "Dock.meter." + channel
                    // -- WP INTE -- (owner 2026-11-24) an output nothing is routed
                    // to has no meter: the dock only shows. -- WP FIN015 -- (REVF R1)
                    // one the generator plays on keeps it: it is loud
                    visible: kind === "input" || (outputEnabled && !hidden)
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
        // -- WP UA5 -- one readout (was "C0 0 %  C1 0 %")
        DspLoad {}
    }

    // -- WP UB34 -- a preset on every linked device: the one question
    QuestionDialog {
        id: recallQuestion
        objectName: "Dock.recallQuestion"
        parent: Overlay.overlay
        acceptText: qsTr("Recall on All")
        onAnswered: answer => DeviceLinkController.answerRecall(answer)
    }
    Connections {
        target: DeviceLinkController
        function onRecallQuestion(title, text) {
            recallQuestion.title = title
            recallQuestion.text = text
            recallQuestion.open()
        }
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
                 : toggle.down ? Theme.controlPressed : toggle.hovered ? Theme.barControlHover : Theme.barControl   // -- WP REVDFIX -- D1
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

    // -- WP UA5 -- The DSP's load: the busier of its two cores, both on hover
    component DspLoad: Text {
        id: load
        objectName: "Dock.dsp"
        textFormat: Text.PlainText
        readonly property bool live: Poller.running
        readonly property int busiest: Math.max(Poller.cpu0, Poller.cpu1)
        readonly property string shownName: live ? busiest + " %" : "–"
        visible: !dock.narrow
        Layout.preferredWidth: 60
        horizontalAlignment: Text.AlignRight
        //: The dock's DSP load readout; %1 is "3 %" or "–" while it is not read
        text: qsTr("DSP %1").arg(shownName)
        font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontCaption.pixelSize })
        color: live && busiest > 90 ? Theme.dangerText : Theme.textMuted
        //: %1 and %2 are the two cores' loads in per cent
        readonly property string help: live ? qsTr("The DSP's load: the busier of its two cores (%1 % and %2 %)").arg(Poller.cpu0).arg(Poller.cpu1)
                                            : qsTr("The DSP's load (not read now)")
        HoverHandler { id: loadHover }
        HelpTip {
            objectName: "Dock.dsp.tip"
            text: load.help
            visible: loadHover.hovered
        }
        Accessible.role: Accessible.ProgressBar
        Accessible.name: qsTr("DSP load")
        Accessible.description: load.help
    }
}
