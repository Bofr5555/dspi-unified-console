import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP3 -- The graph options (the Mac's GraphOptionsPanel): the analyser's
// part (GraphController.RtaOptions, QP9's), Graph Setup, and Pop Out Graph; the
// setup page holds the scale, the grid and labels, and the curves, the same
// preferences as the Graphing settings page.
//
// -- WP LAY2 -- The content alone, both pages and the slide between them:
// Classic shows it in the gear's popover (GraphOptions), Modern wherever its
// graph puts its options (InspectorSection). `done` asks the host to put it
// away (Pop Out Graph was chosen).
Item {
    id: content
    // Object names start with this (the gear's name)
    property string prefix: "GraphHost.options"
    // In the pop-out window: no Pop Out row, and the follow switch
    property bool popOut: false
    // The setup page shows; the host starts on the main page each time it
    // shows, as the Mac builds a fresh panel each time
    property bool setup: false
    // 0 on the main page, 1 on the setup page, sliding between them
    property real slide: setup ? 1 : 0
    Behavior on slide { NumberAnimation { duration: 200; easing.type: Easing.InOutQuad } }
    signal done()

    Layout.fillWidth: true
    clip: true
    implicitHeight: mainPage.implicitHeight + (setupPage.implicitHeight - mainPage.implicitHeight) * content.slide

    // ---- Main page ----
    ColumnLayout {
        id: mainPage
        spacing: 0
        width: parent.width
        x: -width * content.slide
        opacity: 1 - content.slide
        visible: content.slide < 1

        Loader {
            objectName: content.prefix + ".rtaSlot"
            readonly property string type: GraphController.slotType(GraphController.RtaOptions)
            Layout.fillWidth: true
            active: type !== ""
            sourceComponent: active ? Qt.createComponent("UConsole", type) : null
            onLoaded: if ("popOut" in item) item.popOut = content.popOut
        }
        Rectangle {
            visible: GraphController.slotType(GraphController.RtaOptions) !== ""
            color: Theme.separator
            implicitHeight: 1
            Layout.fillWidth: true
            Layout.topMargin: 6
            Layout.bottomMargin: 6
        }
        MenuRow {
            objectName: content.prefix + ".setup"
            text: qsTr("Graph Setup")
            iconName: "sliders-horizontal"
            trailingIcon: "chevron-right"
            onActivated: content.setup = true
        }
        MenuRow {
            objectName: content.prefix + ".popOut"
            visible: !content.popOut
            text: qsTr("Pop Out Graph")
            iconName: "maximize-2"
            onActivated: {
                content.done()
                Shell.trigger("window.popOutGraph")
            }
        }
    }

    // ---- Setup page ----
    GraphSetupPage {
        id: setupPage
        prefix: content.prefix
        inPopOutWindow: content.popOut
        width: parent.width
        x: width * (1 - content.slide)
        opacity: content.slide
        visible: content.slide > 0
        onBack: content.setup = false
    }

    // A menu-like row: the accent under the pointer or the keyboard focus,
    // a chevron when it opens a page
    component MenuRow: Rectangle {
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
                textFormat: Text.PlainText
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
