import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP9 -- The spectrum as bars at the top of the overview and of a
// channel page (the Mac's SpectrumBarStrip): one cell per channel of the
// page's selection (the device's while following another console), in up to
// four columns; a gear for the layout and the analyser window; the bottom
// edge drags the cells' height. It shows on the page's "RTA Bars" switch.
Rectangle {
    id: strip
    // "dashboard" or "page": names the view and its objects
    property string place: "dashboard"
    readonly property var win: Window.window
    readonly property bool shown: RtaController.supported && RtaController.ownChannels.length > 0 && RtaController.showBars
    readonly property bool onScreen: shown && parent !== null && parent.visible && win !== null && win.visible
                                     && (place === "dashboard") === RtaController.onDashboard
    readonly property var cells: RtaController.drawnChannels
    readonly property int count: cells.length
    readonly property int columns: Math.min(Math.max(count, 1), Math.max(1, Math.min(4, RtaController.prefs.barColumns)))
    readonly property int rows: Math.ceil(Math.max(count, 1) / columns)
    readonly property real cellHeight: {
        const h = Math.max(64, Math.min(240, RtaController.prefs.barHeight))
        return count <= 1 ? h : Math.round(0.75 * h)
    }

    objectName: "SpectrumStrip." + place
    visible: shown
    implicitHeight: shown ? grid.implicitHeight + 20 : 0
    radius: 10
    color: Theme.control
    border.width: 1
    border.color: count === 1 && cellsRepeater.count === 1 ? Qt.alpha(cellsRepeater.itemAt(0).tint, 0.3) : Theme.separator

    onOnScreenChanged: RtaController.setView("strip." + place, RtaController.Bars, onScreen)
    Component.onCompleted: RtaController.setView("strip." + place, RtaController.Bars, onScreen)
    Component.onDestruction: RtaController.setView("strip." + place, RtaController.Bars, false)

    Accessible.role: Accessible.Graphic
    Accessible.name: qsTr("Spectrum")

    HoverHandler { id: hover }

    GridLayout {
        id: grid
        x: 10
        y: 10
        width: strip.width - 20
        columns: strip.columns
        columnSpacing: 12
        rowSpacing: 8

        Repeater {
            id: cellsRepeater
            model: strip.cells
            ColumnLayout {
                id: cell
                required property int modelData
                required property int index
                readonly property var info: { DeviceModel.revision; return RtaController.channelInfo(RtaController.drawnTap, modelData) }
                readonly property color tint: ChannelPalette.forRow(info.kind, info.number, info.pdm)
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.alignment: Qt.AlignTop
                spacing: 2
                RowLayout {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 14
                    spacing: 5
                    Rectangle {
                        width: 5
                        height: 5
                        radius: 2.5
                        color: cell.tint
                    }
                    Text {
                        text: cell.info.name
                        font: Qt.font({ pixelSize: 9, weight: Font.DemiBold })
                        color: Theme.textMuted
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                    // Keeps the names clear of the gear
                    Item { implicitWidth: 16; implicitHeight: 14 }
                }
                RtaBars {
                    objectName: strip.objectName + ".bars." + cell.index
                    Layout.fillWidth: true
                    Layout.preferredHeight: strip.cellHeight
                    active: strip.onScreen
                    tap: RtaController.drawnTap
                    channel: cell.modelData
                    color: cell.tint
                    frequencyLabels: true
                    dbLabels: false
                    gridColor: Theme.text
                    labelColor: Theme.textMuted
                    labelFont: Qt.font({ family: Theme.fontMono.family, pixelSize: 8 })
                }
            }
        }
        // A short last row keeps the cells' widths
        Repeater {
            model: strip.count > 0 ? (strip.columns - strip.count % strip.columns) % strip.columns : 0
            Item { Layout.fillWidth: true; Layout.preferredWidth: 1 }
        }
    }

    // The gear: on hover, or while its popover is open
    IconButton {
        id: gear
        objectName: strip.objectName + ".options"
        anchors { right: parent.right; top: parent.top; margins: 6 }
        iconName: "settings"
        iconSize: 12
        iconColor: popover.opened ? Theme.text : Theme.textMuted
        toolTip: qsTr("Spectrum strip options")
        opacity: hover.hovered || popover.opened || activeFocus ? 1 : 0
        Behavior on opacity { OpacityAnimator { duration: 150 } }
        onClicked: popover.opened ? popover.close() : popover.open()

        PopoverPanel {
            id: popover
            objectName: strip.objectName + ".popover"
            title: qsTr("Spectrum strip options")
            x: gear.width - width
            y: gear.height + 4
            width: 200
            padding: 0
            topPadding: 6
            bottomPadding: 6

            contentItem: ColumnLayout {
                spacing: 0
                Text {
                    visible: strip.count > 1
                    text: qsTr("LAYOUT")
                    font: Theme.fontHeading
                    color: Theme.textMuted
                    Layout.leftMargin: 12
                    Layout.topMargin: 6
                    Layout.bottomMargin: 8
                }
                RowLayout {
                    visible: strip.count > 1
                    Layout.leftMargin: 12
                    Layout.rightMargin: 12
                    spacing: 6
                    Repeater {
                        model: 4
                        AbstractButton {
                            id: tile
                            required property int index
                            readonly property int n: index + 1
                            readonly property bool chosen: Math.max(1, Math.min(4, RtaController.prefs.barColumns)) === n
                            objectName: strip.objectName + ".columns." + n
                            Layout.fillWidth: true
                            implicitHeight: 38
                            hoverEnabled: true
                            focusPolicy: Qt.TabFocus
                            background: Rectangle {
                                radius: 6
                                color: tile.chosen ? Qt.alpha(Theme.accent, 0.16) : tile.hovered ? Theme.controlHover : Theme.control
                                border.width: tile.visualFocus ? 2 : 1
                                border.color: tile.visualFocus ? Theme.focusRing : tile.chosen ? Qt.alpha(Theme.accent, 0.7) : Theme.separator
                            }
                            contentItem: Column {
                                spacing: 2
                                topPadding: 5
                                // Two rows of n cells
                                Grid {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    columns: tile.n
                                    spacing: 1
                                    Repeater {
                                        model: tile.n * 2
                                        Rectangle {
                                            width: (22 - (tile.n - 1)) / tile.n
                                            height: 6
                                            radius: 1
                                            color: tile.chosen ? Theme.accentText : Theme.textMuted
                                            opacity: 0.7
                                        }
                                    }
                                }
                                Text {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    text: String(tile.n)
                                    font: Qt.font({ pixelSize: 9, weight: tile.chosen ? Font.DemiBold : Font.Normal })
                                    color: tile.chosen ? Theme.accentText : Theme.textMuted
                                }
                            }
                            HelpTip {
                                text: tile.n === 1 ? qsTr("Stack the channels in one column")
                                                   : qsTr("Lay the channels out in up to %1 columns").arg(tile.n)
                                visible: tile.hovered
                            }
                            Accessible.role: Accessible.RadioButton
                            Accessible.name: String(tile.n)
                            Accessible.checked: chosen
                            onClicked: RtaController.prefs.barColumns = n
                        }
                    }
                }
                Rectangle {
                    visible: strip.count > 1
                    color: Theme.separator
                    implicitHeight: 1
                    Layout.fillWidth: true
                    Layout.topMargin: 8
                    Layout.bottomMargin: 6
                }
                AppButton {
                    objectName: strip.objectName + ".openWindow"
                    Layout.fillWidth: true
                    Layout.leftMargin: 6
                    Layout.rightMargin: 6
                    text: qsTr("Open in Window")
                    iconName: "app-window"
                    onClicked: {
                        popover.close()
                        Shell.trigger("tools.spectrum")
                    }
                }
            }
        }
    }

    // The bottom edge: drag down to grow the cells, up to shrink them
    MouseArea {
        id: drag
        objectName: strip.objectName + ".resize"
        anchors { left: parent.left; right: parent.right; bottom: parent.bottom }
        height: 10
        cursorShape: Qt.SizeVerCursor
        property real startY: 0
        property real startHeight: 0
        onPressed: mouse => {
            startY = mapToItem(strip, mouse.x, mouse.y).y
            startHeight = RtaController.prefs.barHeight
        }
        onPositionChanged: mouse => {
            if (!pressed) return
            const dy = mapToItem(strip, mouse.x, mouse.y).y - startY
            const perUnit = strip.rows * (strip.count <= 1 ? 1 : 0.75)
            RtaController.prefs.barHeight = Math.round(Math.max(64, Math.min(240, startHeight + dy / perUnit)))
        }
    }
}
