import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP3 -- The graph's gear and its popover (the Mac's GraphOptionsButton
// and GraphOptionsPanel): the analyser's part (GraphController.RtaOptions,
// QP9's), Graph Setup, and Pop Out Graph; the setup page holds the scale, the
// grid and labels, and the curves, the same preferences as the Graphing
// settings page. The popover is the gear's child, so a click on the gear while
// it is open closes it once and does not reopen it (behaviour.md §26).
IconButton {
    id: gear
    property bool popOut: false
    readonly property bool open: panel.opened

    objectName: (popOut ? "GraphWindow" : "GraphHost") + ".options"
    iconName: "settings"
    iconColor: open ? Theme.text : Theme.textMuted
    toolTip: GraphController.selectedChannel < 0 ? qsTr("Graph options for the dashboard")
                                                 : qsTr("Graph options for this channel page")
    onClicked: panel.opened ? panel.close() : panel.open()

    PopoverPanel {
        id: panel
        objectName: gear.objectName + ".popover"
        title: qsTr("Graph options")
        // Reopening starts on the main page, as the Mac builds a fresh panel
        property bool setup: false
        onAboutToShow: setup = false
        x: gear.width - width
        y: gear.height + 4
        width: 280
        padding: 0
        topPadding: 6
        bottomPadding: 6

        contentItem: Item {
            implicitHeight: panel.setup ? setupPage.implicitHeight : mainPage.implicitHeight
            clip: true
            Behavior on implicitHeight { NumberAnimation { duration: 200; easing.type: Easing.InOutQuad } }

            // ---- Main page ----
            ColumnLayout {
                id: mainPage
                width: parent.width
                spacing: 0
                x: panel.setup ? -width : 0
                opacity: panel.setup ? 0 : 1
                visible: opacity > 0
                Behavior on x { NumberAnimation { duration: 200; easing.type: Easing.InOutQuad } }
                Behavior on opacity { NumberAnimation { duration: 200 } }

                Loader {
                    objectName: gear.objectName + ".rtaSlot"
                    readonly property string type: GraphController.slotType(GraphController.RtaOptions)
                    Layout.fillWidth: true
                    active: type !== ""
                    sourceComponent: active ? Qt.createComponent("UConsole", type) : null
                    onLoaded: if ("popOut" in item) item.popOut = gear.popOut
                }
                Rule { visible: GraphController.slotType(GraphController.RtaOptions) !== "" }

                ActionRow {
                    objectName: gear.objectName + ".setup"
                    iconName: "sliders-horizontal"
                    text: qsTr("Graph Setup")
                    trailingIcon: "chevron-right"
                    onActivated: panel.setup = true
                }
                ActionRow {
                    objectName: gear.objectName + ".popOut"
                    visible: !gear.popOut
                    iconName: "maximize-2"
                    text: qsTr("Pop Out Graph")
                    onActivated: {
                        panel.close()
                        Shell.trigger("window.popOutGraph")
                    }
                }
            }

            // ---- Setup page ----
            GraphSetupPage {
                id: setupPage
                width: parent.width
                inPopOutWindow: gear.popOut
                prefix: gear.objectName
                x: panel.setup ? 0 : width
                opacity: panel.setup ? 1 : 0
                visible: opacity > 0
                Behavior on x { NumberAnimation { duration: 200; easing.type: Easing.InOutQuad } }
                Behavior on opacity { NumberAnimation { duration: 200 } }
                onBack: panel.setup = false
            }
        }
    }

    // A separator line across the panel
    component Rule: Rectangle {
        Layout.fillWidth: true
        Layout.topMargin: 6
        Layout.bottomMargin: 6
        implicitHeight: 1
        color: Theme.separator
    }

    // A menu-like row: highlighted under the pointer, a chevron when it opens a page
    component ActionRow: Rectangle {
        id: row
        property string iconName
        property string text
        property string trailingIcon
        signal activated()

        Layout.fillWidth: true
        Layout.leftMargin: 4
        Layout.rightMargin: 4
        implicitHeight: 26
        radius: Theme.radiusSmall
        color: rowHover.hovered || activeFocus ? Theme.accent : "transparent"
        activeFocusOnTab: true
        readonly property color ink: rowHover.hovered || activeFocus ? Theme.textOnAccent : Theme.text

        Accessible.role: Accessible.MenuItem
        Accessible.name: text
        Accessible.onPressAction: activated()
        Keys.onReturnPressed: activated()
        Keys.onSpacePressed: activated()

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            spacing: 8
            Icon {
                name: row.iconName
                size: 14
                color: row.ink
                Layout.preferredWidth: 18
            }
            Text {
                text: row.text
                font: Theme.fontBody
                color: row.ink
                Layout.fillWidth: true
            }
            Icon {
                visible: row.trailingIcon !== ""
                name: row.trailingIcon
                size: 12
                color: row.ink
                opacity: rowHover.hovered ? 1 : 0.5
            }
        }
        HoverHandler { id: rowHover }
        TapHandler { onTapped: row.activated() }
    }
}
