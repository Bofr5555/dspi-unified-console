import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP9 -- The spectrum as bars at the top of the overview and of a
// channel page (the Mac's SpectrumBarStrip): a cell per channel of the page's
// selection (the device's while following another console), in up to four
// columns; a gear for the layout and the analyser window; dragging the bottom
// edge sets the cells' height. It shows on the page's "RTA Bars" switch.
Rectangle {
    id: strip
    // "dashboard" or "page": names the view and its objects
    property string place: "dashboard"
    readonly property var win: Window.window
    readonly property bool shown: RtaController.available && RtaController.ownChannels.length > 0 && RtaController.showBars
    readonly property bool onScreen: shown && parent !== null && parent.visible && win !== null && win.visible
                                     && (place === "dashboard") === RtaController.onDashboard
    readonly property var cells: RtaController.drawnChannels
    readonly property int count: cells.length
    // The layout from the controller (spectrum/barColumns, spectrum/barHeight)
    readonly property int columns: { RtaController.prefs.barColumns; return RtaController.barColumnsFor(count) }
    readonly property int cellHeight: { RtaController.prefs.barHeight; return RtaController.barCellHeight(count) }
    readonly property color edge: count === 1 ? Qt.alpha(tintOf(cells[0]), 0.3) : Theme.separator

    function infoOf(ch) { DeviceModel.revision; return RtaController.channelInfo(RtaController.drawnTap, ch) }
    function tintOf(ch) { const i = infoOf(ch); return ChannelPalette.forRow(i.kind, i.number, i.pdm) }

    objectName: "SpectrumStrip." + place
    visible: shown
    implicitHeight: shown ? cellGrid.implicitHeight + 2 * Theme.spacingSmall + 4 : 0
    color: Theme.control
    radius: 10
    border { color: strip.edge; width: 1 }

    onOnScreenChanged: RtaController.setView("strip." + place, RtaController.Bars, onScreen)
    Component.onCompleted: RtaController.setView("strip." + place, RtaController.Bars, onScreen)
    Component.onDestruction: RtaController.setView("strip." + place, RtaController.Bars, false)

    Accessible.role: Accessible.Graphic
    Accessible.name: qsTr("Spectrum")

    HoverHandler { id: pointer }

    GridLayout {
        id: cellGrid
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: Theme.spacingSmall + 2 }
        columns: strip.columns
        columnSpacing: Theme.spacing
        rowSpacing: Theme.spacingSmall

        Repeater {
            model: strip.cells
            ColumnLayout {
                id: cell
                required property int modelData
                required property int index
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.alignment: Qt.AlignTop
                spacing: 2
                RowLayout {
                    spacing: 5
                    Layout.fillWidth: true
                    Layout.preferredHeight: 14
                    Rectangle {
                        implicitWidth: 5
                        implicitHeight: 5
                        radius: 2.5
                        color: strip.tintOf(cell.modelData)
                    }
                    Text {
                        Layout.fillWidth: true
                        text: strip.infoOf(cell.modelData).name
                        color: Theme.textMuted
                        font.pixelSize: 9
                        font.bold: true
                        elide: Text.ElideRight
                    }
                    // Room for the gear
                    Item { implicitWidth: 16; implicitHeight: 14 }
                }
                RtaBars {
                    objectName: strip.objectName + ".bars." + cell.index
                    Layout.fillWidth: true
                    Layout.preferredHeight: strip.cellHeight
                    active: strip.onScreen
                    tap: RtaController.drawnTap
                    channel: cell.modelData
                    tint: strip.tintOf(cell.modelData)
                    frequencyLabels: true
                    dbLabels: false
                    gridColor: Theme.text
                    labelColor: Theme.textMuted
                    labelFont.family: Theme.fontMono.family
                    labelFont.pixelSize: 8
                }
            }
        }
        // Empty cells keep a short last row's widths
        Repeater {
            model: strip.count > 0 ? (strip.columns - strip.count % strip.columns) % strip.columns : 0
            Item { Layout.fillWidth: true; Layout.preferredWidth: 1 }
        }
    }

    // The gear, while the pointer is over the strip or its popover is open
    IconButton {
        id: gear
        objectName: strip.objectName + ".options"
        anchors { top: parent.top; right: parent.right; margins: 6 }
        iconName: "settings"
        iconSize: 12
        iconColor: menu.opened ? Theme.text : Theme.textMuted
        toolTip: qsTr("Spectrum strip options")
        opacity: pointer.hovered || menu.opened || activeFocus ? 1 : 0
        Behavior on opacity { OpacityAnimator { duration: 150 } }
        onClicked: menu.opened ? menu.close() : menu.open()

        PopoverPanel {
            id: menu
            objectName: strip.objectName + ".popover"
            title: qsTr("Spectrum strip options")
            width: 200
            x: gear.width - width
            y: gear.height + 4
            padding: 0
            topPadding: 6
            bottomPadding: 6

            contentItem: ColumnLayout {
                spacing: 0
                Text {
                    visible: strip.count > 1
                    Layout.margins: Theme.spacing
                    Layout.bottomMargin: Theme.spacingSmall
                    text: qsTr("LAYOUT")
                    color: Theme.textMuted
                    font: Theme.fontHeading
                }
                RowLayout {
                    visible: strip.count > 1
                    spacing: 6
                    Layout.leftMargin: Theme.spacing
                    Layout.rightMargin: Theme.spacing
                    Repeater {
                        model: [1, 2, 3, 4]
                        StripColumnsTile {
                            required property int modelData
                            objectName: strip.objectName + ".columns." + modelData
                            Layout.fillWidth: true
                            columns: modelData
                            chosen: Math.max(1, Math.min(4, RtaController.prefs.barColumns)) === modelData
                            onClicked: RtaController.prefs.barColumns = modelData
                        }
                    }
                }
                Rectangle {
                    visible: strip.count > 1
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.spacingSmall
                    Layout.bottomMargin: 6
                    implicitHeight: 1
                    color: Theme.separator
                }
                AppButton {
                    objectName: strip.objectName + ".openWindow"
                    Layout.fillWidth: true
                    Layout.leftMargin: 6
                    Layout.rightMargin: 6
                    iconName: "app-window"
                    text: qsTr("Open in Window")
                    onClicked: {
                        menu.close()
                        Shell.trigger("tools.spectrum")
                    }
                }
            }
        }
    }

    // The bottom edge: down grows the cells, up shrinks them
    Item {
        id: resize
        objectName: strip.objectName + ".resize"
        anchors { left: parent.left; right: parent.right; bottom: parent.bottom }
        height: 10
        property real fromHeight: 0
        HoverHandler { cursorShape: Qt.SizeVerCursor }
        DragHandler {
            target: null
            xAxis.enabled: false
            onActiveChanged: if (active) resize.fromHeight = RtaController.prefs.barHeight
            onTranslationChanged: if (active)
                RtaController.prefs.barHeight = RtaController.draggedBarHeight(resize.fromHeight, translation.y, strip.count)
        }
    }
}
