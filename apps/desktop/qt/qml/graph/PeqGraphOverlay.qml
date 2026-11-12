import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP QP4 -- The on-graph PEQ editor in the graph's PeqOverlay slot
// (GraphController.PeqOverlay, GraphPlot.qml): PeqOverlay draws the page's
// channel and takes the pointer and the keys; on it sit the band chip, the
// Ctrl-click shape card, the frequency and level readouts, and the band and
// graph menus. It edits only while its graph follows the page, on the PEQ
// tab, with a device that takes writes; otherwise it keeps only the dB zoom
// strip at the left edge.
Item {
    id: root
    // The GraphPlot it sits in (set by the slot)
    property var plot: null
    readonly property alias editor: overlay

    objectName: plot && plot.popOut ? "GraphWindow.peq" : "PeqEditor"

    PeqOverlay {
        id: overlay
        objectName: root.objectName + ".overlay"
        anchors.fill: parent
        graph: GraphController
        pageChannel: root.plot && root.plot.following && !root.plot.onDashboard ? root.plot.activeChannel : -1
        curveColor: root.plot && pageChannel >= 0 && root.plot.channelColors[pageChannel] !== undefined
                    ? root.plot.channelColors[pageChannel] : Theme.text
        bandColors: PeqSelection.colors
        backdrop: Theme.panel
        ink: Theme.text

        Accessible.role: Accessible.Graphic
        Accessible.name: qsTr("Band editor")
        Accessible.description: qsTr("Double-click to add a band; drag a dot to move it")

        // A handler, not the item's own hover, so the graph's gear still shows
        HoverHandler {
            onPointChanged: if (hovered) overlay.hoverAt(point.position)
            onHoveredChanged: if (!hovered) overlay.hoverLeft()
        }

        onMenuRequested: (x, y) => {
            const m = menuModel()
            if (m.kind === "band") bandMenu.open(m, x, y)
            else graphMenu.open(m, x, y)
        }

        // ---- Readouts on empty graph: frequency at the bottom, level at the left ----
        Rectangle {
            objectName: root.objectName + ".frequencyReadout"
            readonly property string shownName: overlay.axisText
            visible: overlay.axisText.length > 0
            width: axisLabel.implicitWidth + 12
            height: 16
            radius: Theme.radiusSmall
            color: Theme.tooltip
            x: Math.min(Math.max(overlay.axisX - width / 2, 4), Math.max(overlay.width - width - 4, 4))
            y: overlay.height - height - 3
            Text {
                id: axisLabel
                anchors.centerIn: parent
                text: overlay.axisText
                font: Theme.fontCaption
                color: Theme.tooltipText
            }
        }
        Rectangle {
            objectName: root.objectName + ".levelReadout"
            readonly property string shownName: overlay.levelText
            visible: overlay.levelText.length > 0
            width: levelLabel.implicitWidth + 12
            height: 16
            radius: Theme.radiusSmall
            color: Theme.tooltip
            x: 4
            y: Math.min(Math.max(overlay.levelY - height / 2, 4), Math.max(overlay.height - height - 4, 4))
            Text {
                id: levelLabel
                anchors.centerIn: parent
                text: overlay.levelText
                font: Theme.fontCaption
                color: Theme.tooltipText
            }
        }

        BandChip {
            id: chip
            editor: overlay
        }

        // ---- The Ctrl-click card: a shape, then its slope, for a band not made yet ----
        Rectangle {
            id: card
            objectName: "PeqCard"
            visible: overlay.cardOpen
            x: overlay.cardX
            y: overlay.cardY
            // The size of a band's chip with three rows (the Mac's card)
            width: 110
            height: 78
            radius: Theme.radius
            color: Theme.menu
            border { width: 1; color: Theme.border }
            Accessible.role: Accessible.Grouping
            Accessible.name: qsTr("Add Band")
            onVisibleChanged: if (visible) chooser.reset()
            // Its clicks are its own, not the graph's
            MouseArea {
                acceptedButtons: Qt.AllButtons
                anchors.fill: parent
            }
            ShapeChooser {
                id: chooser
                anchors.fill: parent
                editor: overlay
                namePrefix: "PeqCard"
                backHelp: qsTr("Cancel")
                onPicked: (shape, order) => overlay.cardPick(shape, order)
                onBack: overlay.closeCard()
            }
        }
    }

    // ---- Band menu: slope, bypass, invert gain, delete ----
    AppMenu {
        id: bandMenu
        objectName: "PeqEditor.bandMenu"
        property var model: ({})
        function open(m, x, y) {
            model = m
            if ((m.orders || []).length > 0) {
                if (!hasSlope) insertMenu(1, slopeMenu)
                hasSlope = true
            } else if (hasSlope) {
                takeMenu(1)
                hasSlope = false
            }
            popup(overlay, x, y)
        }
        property bool hasSlope: false

        AppMenuItem {
            objectName: "PeqEditor.bandMenu.title"
            text: bandMenu.model.title || ""
            enabled: false
        }
        MenuSeparator {}
        AppMenuItem {
            objectName: "PeqEditor.bandMenu.bypass"
            visible: (bandMenu.model.bypass || "").length > 0
            height: visible ? implicitHeight : 0
            text: bandMenu.model.bypass || ""
            onTriggered: overlay.menuAction("bypass")
        }
        AppMenuItem {
            objectName: "PeqEditor.bandMenu.invert"
            visible: bandMenu.model.invert === true
            height: visible ? implicitHeight : 0
            text: qsTr("Invert Gain")
            onTriggered: overlay.menuAction("invert")
        }
        MenuSeparator {}
        AppMenuItem {
            objectName: "PeqEditor.bandMenu.delete"
            text: bandMenu.model.delete || ""
            onTriggered: overlay.menuAction("delete")
        }
    }
    AppMenu {
        id: slopeMenu
        objectName: "PeqEditor.slopeMenu"
        title: bandMenu.model.slopeTitle || ""
        Repeater {
            model: bandMenu.model.orders || []
            delegate: AppMenuItem {
                required property var modelData
                objectName: "PeqEditor.slopeMenu." + modelData.order
                text: modelData.title
                checkable: true
                checked: modelData.checked
                onTriggered: overlay.menuAction("order", modelData.order)
            }
        }
    }

    // ---- Graph menu: select all, deselect, delete the selection ----
    AppMenu {
        id: graphMenu
        objectName: "PeqEditor.graphMenu"
        property var model: ({})
        function open(m, x, y) {
            model = m
            popup(overlay, x, y)
        }
        AppMenuItem {
            objectName: "PeqEditor.graphMenu.selectAll"
            text: qsTr("Select All Bands")
            enabled: graphMenu.model.selectAll === true
            onTriggered: overlay.menuAction("selectAll")
        }
        AppMenuItem {
            objectName: "PeqEditor.graphMenu.deselect"
            text: qsTr("Deselect All")
            enabled: graphMenu.model.deselect === true
            onTriggered: overlay.menuAction("deselect")
        }
        MenuSeparator {
            visible: (graphMenu.model.delete || "").length > 0
            height: visible ? implicitHeight : 0
        }
        AppMenuItem {
            objectName: "PeqEditor.graphMenu.delete"
            visible: (graphMenu.model.delete || "").length > 0
            height: visible ? implicitHeight : 0
            text: graphMenu.model.delete || ""
            onTriggered: overlay.menuAction("deleteSelected")
        }
    }
}
