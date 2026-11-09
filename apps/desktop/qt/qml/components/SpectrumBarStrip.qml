import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Window 2.15
import DSPi 1.0

// Third-octave bars of the page's spectrum channels, in a grid of 1-4
// columns. The gear (on hover) picks the columns or opens the analyser
// window; the handle along the bottom sets the height.
Item {
    id: strip
    property var app                      // main window: selection and settings
    signal openAnalyser()

    // -- WP CG30 -- what is drawn; the bars ask for the page's own selection
    readonly property var sel: app.rtaDrawn
    readonly property int count: sel.channels.length
    readonly property bool shown: app.rtaShowBars && rta.supported && bridge.connected && count > 0
    readonly property int columns: Math.max(1, Math.min(count, app.rtaBarColumns))
    readonly property int rows: Math.ceil(count / columns)
    readonly property real cellHeight: count === 1 ? app.rtaBarHeight : Math.round(app.rtaBarHeight * 0.75)

    visible: shown
    height: shown ? card.height : 0

    Rectangle {
        id: card
        width: parent.width
        height: grid.height + 20
        radius: 10
        color: isMacOS ? MacColors.opacity(MacColors.controlBackground, 0.6) : nativeAltBaseColor
        border.color: {
            if (strip.count !== 1) return isMacOS ? MacColors.opacity(MacColors.secondaryLabel, 0.2) : Qt.rgba(1, 1, 1, 0.1)
            var c = Qt.lighter(bridge.channelColor(rta.appChannel(strip.sel.tap, strip.sel.channels[0])), 1.0)
            return Qt.rgba(c.r, c.g, c.b, 0.3)
        }

        PointerTracker { id: cardHover; anchors.fill: parent }

        Grid {
            id: grid
            x: 10
            y: 10
            width: parent.width - 20
            columns: strip.columns
            columnSpacing: 12
            rowSpacing: 8
            Repeater {
                model: strip.sel.channels
                Column {
                    readonly property int app: rta.appChannel(strip.sel.tap, modelData)
                    width: (grid.width - (strip.columns - 1) * 12) / strip.columns
                    spacing: 2
                    Row {
                        height: 14
                        spacing: 5
                        Rectangle { width: 5; height: 5; radius: 2.5; color: bridge.channelColor(parent.parent.app); anchors.verticalCenter: parent.verticalCenter }
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: (bridge.labelsRevision, bridge.channelLabel(parent.parent.app))   // -- BUG1 -- follows the labels
                            font.pixelSize: 11
                            font.weight: Font.DemiBold
                            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.6)
                            elide: Text.ElideRight
                            width: Math.min(implicitWidth, parent.parent.width - 30)
                        }
                    }
                    SpectrumBarsItem {
                        width: parent.width
                        height: strip.cellHeight
                        controller: rta
                        active: strip.shown && Window.visibility !== Window.Minimized
                        tap: strip.sel.tap
                        channel: modelData
                        request: strip.app.rtaSelection
                        color: bridge.channelColor(parent.app)
                        floorDb: strip.app.rtaFloorDb
                        ceilingDb: strip.app.rtaCeilingDb
                        showPeak: strip.app.rtaShowPeakHold
                        smoothing: strip.app.rtaSmoothing
                    }
                }
            }
        }

        // Gear: columns and the window, on hover
        Rectangle {
            id: gear
            anchors.top: parent.top
            anchors.right: parent.right
            anchors.margins: 6
            width: 20
            height: 18
            radius: 5
            // -- WP QD22 -- Also while it has the keyboard focus: Tab reaches it while hidden
            opacity: cardHover.containsPointer || options.visible || gearMouse.activeFocus ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: 150; easing.type: Easing.InOutQuad } }
            color: isMacOS ? "transparent" : gearMouse.containsMouse || options.visible ? Qt.rgba(1, 1, 1, 0.08) : "transparent"
            Icon { anchors.centerIn: parent; name: "gear"; size: 12; color: isMacOS ? (options.visible ? MacColors.label : MacColors.secondaryLabel) : Qt.rgba(1, 1, 1, 0.7) }
            ClickArea {
                id: gearMouse
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                focusRadius: 7
                Accessible.role: Accessible.ButtonMenu
                Accessible.name: qsTr("Bar strip options")
                onActivated: options.toggleBelow(gear)
            }
        }

        // Height handle along the bottom edge
        MouseArea {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 10
            cursorShape: Qt.SizeVerCursor
            Accessible.ignored: true   // -- WP QD22 -- a drag for the strip's height, not a control
            property real startY: 0
            property real startHeight: 0
            onPressed: { startY = mapToItem(null, 0, mouse.y).y; startHeight = strip.app.rtaBarHeight }
            onPositionChanged: {
                if (!pressed) return
                var dy = mapToItem(null, 0, mouse.y).y - startY
                var scale = strip.rows * (strip.count === 1 ? 1 : 0.75)
                strip.app.rtaBarHeight = Math.max(64, Math.min(240, Math.round(startHeight + dy / scale)))
            }
        }
    }

    // Columns and the window: a popover of its own, centred under the gear
    PopoverWindow {
        id: options
        transientParent: strip.Window.window
        radius: 11
        contentWidth: 200
        contentHeight: optionsColumn.implicitHeight + 20

        Column {
            id: optionsColumn
            x: 10
            y: 10
            width: 180
            spacing: 8
            Text {
                visible: strip.count > 1
                text: qsTr("LAYOUT")
                font.pixelSize: 11
                font.weight: Font.Bold
                font.letterSpacing: 0.4
                color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
            }
            Row {
                visible: strip.count > 1
                spacing: 4
                Repeater {
                    model: 4
                    Rectangle {
                        id: tile
                        readonly property int cols: index + 1
                        readonly property bool isCurrent: strip.app.rtaBarColumns === cols
                        width: (180 - 12) / 4
                        height: 38
                        radius: 6
                        color: isMacOS ? (isCurrent ? MacColors.opacity(MacColors.accent, 0.16) : MacColors.opacity(MacColors.label, 0.05)) : isCurrent ? Qt.rgba(0.04, 0.49, 1, 0.16) : tileMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.08) : Qt.rgba(1, 1, 1, 0.04)
                        border.color: isMacOS ? (isCurrent ? MacColors.opacity(MacColors.accent, 0.7) : "transparent") : isCurrent ? Qt.rgba(0.04, 0.49, 1, 0.7) : Qt.rgba(1, 1, 1, 0.08)
                        Column {
                            anchors.centerIn: parent
                            spacing: 3
                            // A tiny grid of the layout
                            Row {
                                anchors.horizontalCenter: parent.horizontalCenter
                                spacing: 1.5
                                Repeater {
                                    model: tile.cols
                                    Rectangle {
                                        width: (22 - 1.5 * (tile.cols - 1)) / tile.cols
                                        height: 13
                                        radius: 1.5
                                        color: isMacOS ? (tile.isCurrent ? MacColors.accent : MacColors.secondaryLabel) : Qt.rgba(1, 1, 1, 0.45)
                                    }
                                }
                            }
                            Text {
                                anchors.horizontalCenter: parent.horizontalCenter
                                text: tile.cols
                                font.pixelSize: 10
                                color: isMacOS ? (tile.isCurrent ? MacColors.accent : MacColors.secondaryLabel) : Qt.rgba(1, 1, 1, 0.75)
                            }
                        }
                        ClickArea {
                            id: tileMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            Accessible.role: Accessible.RadioButton
                            Accessible.name: tile.cols === 1 ? qsTr("%1 column").arg(tile.cols) : qsTr("%1 columns").arg(tile.cols)
                            Accessible.checkable: true
                            Accessible.checked: tile.isCurrent
                            onActivated: strip.app.rtaBarColumns = tile.cols
                        }
                    }
                }
            }
            Rectangle { visible: strip.count > 1; width: 180; height: 1; color: isMacOS ? MacColors.separator : MenuStyle.separator }
            Rectangle {
                width: 180
                height: 28
                radius: 6
                color: isMacOS ? (openMouse.containsMouse ? MacColors.accent : "transparent") : openMouse.containsMouse ? MenuStyle.highlight : "transparent"
                Row {
                    anchors.left: parent.left
                    anchors.leftMargin: 6
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8
                    Icon { name: "spectrum"; size: 15; color: isMacOS ? (openMouse.containsMouse ? "white" : MacColors.label) : openMouse.containsMouse ? "white" : Qt.rgba(1, 1, 1, 0.65); anchors.verticalCenter: parent.verticalCenter }
                    Text { text: qsTr("Open in Window"); font.pixelSize: 13; color: isMacOS ? (openMouse.containsMouse ? "white" : MacColors.label) : openMouse.containsMouse ? "white" : Qt.rgba(1, 1, 1, 0.9); anchors.verticalCenter: parent.verticalCenter }
                }
                ClickArea {
                    id: openMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    focusRadius: 8
                    Accessible.name: qsTr("Open in Window")
                    onActivated: { options.close(); strip.openAnalyser() }
                }
            }
        }
    }
}
