import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP1 -- One channel in the sidebar (Components.swift's ChannelRow and
// OutputRow): the selection mark, the name (or its rename field), the level
// meter and the descriptor pill that shows or hides the channel's curve.
// A click selects, Alt+click renames, a right-click opens the row's menu.
ItemDelegate {
    id: row
    required property string kind
    required property int number
    required property int channel
    required property string name
    required property string label
    required property string descriptor
    required property bool outputEnabled
    required property bool pdm

    // From the sidebar
    property int rowHeight: 28
    property int nameWidth: 80
    property bool renaming: false
    signal renameRequested()
    signal renameFinished()

    readonly property bool isOutput: kind === "output"
    readonly property color tint: ChannelPalette.forRow(kind, number, pdm)
    // The graph owns which curves show (it solos a channel page and hands back)
    readonly property bool curveVisible: GraphController.visibleChannels.indexOf(channel) >= 0
    readonly property bool selectedRow: (SidebarController.highlightedMask >> channel & 1) === 1
    readonly property bool inactive: isOutput && (SidebarController.inactiveOutputs >> number & 1) === 1
    readonly property string slotName: isOutput && number < SidebarController.slotNames.length ? SidebarController.slotNames[number] : ""
    readonly property real level: channel < Poller.peaks.length ? Math.max(0, Math.min(1, Poller.peaks[channel])) : 0
    // -- WP QP18 -- the core's ballistics (P0M): the clip hold and the held peak
    readonly property bool clipping: (Poller.clipHold >> channel & 1) === 1
    readonly property real held: channel < Poller.peakHold.length ? Math.max(0, Math.min(1, Poller.peakHold[channel])) : 0

    objectName: "Sidebar.channel." + channel
    // What the row shows, for the smoke driver
    readonly property string shownName: label
    width: ListView.view ? ListView.view.width : implicitWidth
    visible: kind === "input" || outputEnabled
    height: visible ? rowHeight : 0
    padding: 0
    hoverEnabled: true
    highlighted: selectedRow

    function startRename() {
        if (DeviceModel.readOnly) return
        row.renameRequested()
    }

    onClicked: SidebarController.rowClicked(channel)

    // -- WP LAY3b -- Modern's channel menu has more, and the menu key opens it
    function openMenu(x, y) {
        if (LayoutController.modern) ContextMenus.channel(row.channel, row, x, y, row.startRename)
        else rowMenu.popup(row, x, y)
    }
    Keys.onPressed: event => {
        event.accepted = LayoutController.modern
            && (event.key === Qt.Key_Menu || (event.key === Qt.Key_F10 && event.modifiers === Qt.ShiftModifier))
        if (event.accepted) row.openMenu(Theme.spacing, row.height)
    }

    // Alt+click renames (the desktop gesture for the Mac's Option-click) and a
    // right-click opens the row's menu. A plain click passes through to the
    // row (selection) and to the pill under it.
    MouseArea {
        anchors.fill: parent
        z: 1
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        onPressed: mouse => {
            mouse.accepted = mouse.button === Qt.RightButton || (mouse.modifiers & Qt.AltModifier) !== 0
        }
        onClicked: mouse => {
            if (mouse.button === Qt.RightButton) row.openMenu(mouse.x, mouse.y)
            else row.startRename()
        }
    }

    contentItem: RowLayout {
        spacing: 0
        // The selection mark
        Rectangle {
            Layout.preferredWidth: 3
            Layout.fillHeight: true
            Layout.topMargin: 4
            Layout.bottomMargin: 4
            color: row.selectedRow ? Theme.accent : "transparent"
        }
        Text {
            visible: !row.renaming
            Layout.preferredWidth: row.nameWidth
            Layout.leftMargin: 8
            textFormat: Text.PlainText
            text: row.label
            font: Theme.fontBody
            color: Theme.text
            elide: Text.ElideRight
            HelpTip {
                text: row.slotName
                visible: row.isOutput && row.hovered && row.slotName.length > 0
            }
        }
        BorderedField {
            id: renameField
            objectName: "Sidebar.rename." + row.channel
            visible: row.renaming
            Layout.preferredWidth: row.nameWidth
            Layout.leftMargin: 4
            label: qsTr("Rename %1").arg(row.label)
            onCommitted: value => {
                SidebarController.renameChannel(row.channel, value)
                row.renameFinished()
            }
            onActiveFocusChanged: if (!activeFocus && row.renaming) row.renameFinished()
        }
        // The level meter: the channel's peak, a red mark at its end while it clips
        Item {
            objectName: "Sidebar.meter." + row.channel
            readonly property real shownLevel: row.level
            Layout.fillWidth: true
            Layout.leftMargin: 4
            Layout.preferredHeight: 6
            opacity: row.inactive ? 0.4 : 1
            Rectangle {
                anchors.fill: parent
                radius: 2
                color: Theme.separator
            }
            Rectangle {
                width: Math.max(0, parent.width - 3) * row.level
                height: parent.height
                radius: 2
                // -- WP QP18 -- level in the one meter colour; the pill is the channel's tag
                color: Theme.meterLevel
                Behavior on width { NumberAnimation { duration: 60 } }
            }
            // -- WP QP18 -- the hold line
            Rectangle {
                objectName: "Sidebar.hold." + row.channel
                visible: row.held > row.level && row.held > 0
                x: Math.max(0, parent.width - 5) * row.held
                width: 2
                height: parent.height
                color: Theme.meterHold
            }
            Rectangle {
                visible: row.clipping
                x: parent.width - 3
                width: 3
                height: parent.height
                radius: 1
                color: Theme.danger
            }
        }
        Pill {
            id: pill
            objectName: "Sidebar.pill." + row.channel
            Layout.leftMargin: 10
            Layout.rightMargin: 8
            Layout.minimumWidth: 40
            text: row.descriptor
            checkable: true
            checked: row.curveVisible
            tint: row.curveVisible ? row.tint : Theme.textDisabled
            accessibleName: row.isOutput
                ? [row.descriptor, row.label.split(" · ").join(", "),
                   row.slotName.length ? qsTr("slot %1").arg(row.slotName) : ""].filter(s => s.length).join(", ")
                : row.descriptor
            onToggled: GraphController.setCurveVisible(row.channel, checked)
            HelpTip {
                text: row.curveVisible ? qsTr("Hide %1 on the graph").arg(row.descriptor)
                                       : qsTr("Show %1 on the graph").arg(row.descriptor)
                visible: pill.hovered
            }
        }
    }

    onRenamingChanged: {
        if (renaming) {
            renameField.text = row.name.length ? row.name : row.label
            renameField.forceActiveFocus()
            renameField.selectAll()
        }
    }

    background: Rectangle {
        color: row.selectedRow ? Theme.selection : row.hovered ? Theme.controlHover : "transparent"
    }

    AppMenu {
        id: rowMenu
        objectName: "Sidebar.rowMenu." + row.channel
        AppMenuItem {
            objectName: "Sidebar.menu.identify." + row.channel
            text: qsTr("Identify")
            visible: row.isOutput && DeviceModel.connected && SidebarController.identifySupported
            height: visible ? implicitHeight : 0
            enabled: !DeviceModel.readOnly
            onTriggered: SidebarController.identifyOutput(row.number)
        }
        MenuSeparator {
            visible: row.isOutput && DeviceModel.connected && SidebarController.identifySupported
            height: visible ? implicitHeight : 0
            contentItem: Rectangle { implicitHeight: 1; color: Theme.separator }
        }
        AppMenuItem {
            objectName: "Sidebar.menu.rename." + row.channel
            text: qsTr("Rename")
            enabled: !DeviceModel.readOnly
            onTriggered: row.startRename()
        }
        MenuSeparator {
            contentItem: Rectangle { implicitHeight: 1; color: Theme.separator }
        }
        AppMenuItem {
            objectName: "Sidebar.menu.copy." + row.channel
            text: qsTr("Copy Parameters")
            onTriggered: SidebarController.copyChannel(row.channel)
        }
        AppMenuItem {
            objectName: "Sidebar.menu.paste." + row.channel
            text: qsTr("Paste Parameters")
            enabled: SidebarController.hasClipboard && !DeviceModel.readOnly
            onTriggered: SidebarController.pasteChannel(row.channel)
        }
    }

    Accessible.role: Accessible.ListItem
    Accessible.name: label + ", " + descriptor
    Accessible.selected: selectedRow
    Accessible.description: qsTr("Alt+click or the context menu renames it")

    // -- WP AIVIS -- an agent changed it just now
    AiTouched { objectName: row.objectName + ".touched"; path: "eq."; indices: [row.channel] }
    AiTouched { path: "ch."; indices: [row.channel] }
    AiTouched { path: "out."; indices: [row.number]; visible: lit && row.isOutput }
}
