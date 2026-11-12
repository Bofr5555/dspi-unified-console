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
    readonly property bool open: panel.opened
    // In the pop-out window: no Pop Out row, and the follow switch
    property bool popOut: false

    objectName: (popOut ? "GraphWindow" : "GraphHost") + ".options"
    iconName: "settings"
    iconColor: open ? Theme.text : Theme.textMuted
    toolTip: GraphController.selectedChannel < 0 ? qsTr("Graph options for the dashboard")
                                                 : qsTr("Graph options for this channel page")
    onClicked: panel.opened ? panel.close() : panel.open()

    PopoverPanel {
        id: panel
        // The setup page shows; reopening starts on the main page, as the Mac
        // builds a fresh panel each time
        property bool setup: false
        // 0 on the main page, 1 on the setup page, sliding between them
        property real slide: setup ? 1 : 0
        Behavior on slide { NumberAnimation { duration: 200; easing.type: Easing.InOutQuad } }

        objectName: gear.objectName + ".popover"
        title: qsTr("Graph options")
        onAboutToShow: setup = false
        x: gear.width - width
        y: gear.height + 4
        width: 280
        padding: 0
        topPadding: 6
        bottomPadding: 6

        contentItem: Item {
            clip: true
            implicitHeight: mainPage.implicitHeight + (setupPage.implicitHeight - mainPage.implicitHeight) * panel.slide

            // ---- Main page ----
            ColumnLayout {
                id: mainPage
                spacing: 0
                width: parent.width
                x: -width * panel.slide
                opacity: 1 - panel.slide
                visible: panel.slide < 1

                Loader {
                    objectName: gear.objectName + ".rtaSlot"
                    readonly property string type: GraphController.slotType(GraphController.RtaOptions)
                    Layout.fillWidth: true
                    active: type !== ""
                    sourceComponent: active ? Qt.createComponent("UConsole", type) : null
                    onLoaded: if ("popOut" in item) item.popOut = gear.popOut
                }
                Rectangle {
                    visible: GraphController.slotType(GraphController.RtaOptions) !== ""
                    color: Theme.separator
                    implicitHeight: 1
                    Layout.fillWidth: true
                    Layout.topMargin: 6
                    Layout.bottomMargin: 6
                }
                ActionRow {
                    objectName: gear.objectName + ".setup"
                    text: qsTr("Graph Setup")
                    iconName: "sliders-horizontal"
                    trailingIcon: "chevron-right"
                    onActivated: panel.setup = true
                }
                ActionRow {
                    objectName: gear.objectName + ".popOut"
                    visible: !gear.popOut
                    text: qsTr("Pop Out Graph")
                    iconName: "maximize-2"
                    onActivated: {
                        panel.close()
                        Shell.trigger("window.popOutGraph")
                    }
                }
            }

            // ---- Setup page ----
            GraphSetupPage {
                id: setupPage
                prefix: gear.objectName
                inPopOutWindow: gear.popOut
                width: parent.width
                x: width * (1 - panel.slide)
                opacity: panel.slide
                visible: panel.slide > 0
                onBack: panel.setup = false
            }
        }
    }

    // A menu-like row: the accent under the pointer or the keyboard focus,
    // a chevron when it opens a page
    component ActionRow: Rectangle {
        id: row
        property string text
        property string iconName
        property string trailingIcon
        readonly property bool lit: rowHover.hovered || activeFocus
        signal activated()

        implicitHeight: 26
        Layout.fillWidth: true
        Layout.leftMargin: 4
        Layout.rightMargin: 4
        radius: Theme.radiusSmall
        color: lit ? Theme.accent : "transparent"
        activeFocusOnTab: true

        Accessible.role: Accessible.MenuItem
        Accessible.name: text
        Accessible.onPressAction: activated()
        Keys.onReturnPressed: activated()
        Keys.onSpacePressed: activated()
        HoverHandler { id: rowHover }
        TapHandler { onTapped: row.activated() }

        RowLayout {
            anchors { fill: parent; leftMargin: 8; rightMargin: 8 }
            spacing: 8
            Icon {
                name: row.iconName
                color: row.lit ? Theme.textOnAccent : Theme.text
                size: 14
                Layout.preferredWidth: 18
            }
            Text {
                text: row.text
                color: row.lit ? Theme.textOnAccent : Theme.text
                font: Theme.fontBody
                Layout.fillWidth: true
            }
            Icon {
                visible: row.trailingIcon !== ""
                name: row.trailingIcon
                color: row.lit ? Theme.textOnAccent : Theme.text
                opacity: rowHover.hovered ? 1 : 0.5
                size: 12
            }
        }
    }
}
