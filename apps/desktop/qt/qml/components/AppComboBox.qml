import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A popup button choosing one value from a list (the Mac's Picker with the
// menu style: presets, sources, settings choices). `label` names it for
// screen readers; `activated(index)` fires on a person's choice only.
ComboBox {
    id: control
    property string label
    // -- WP LAY3b -- A right-click on a row of the list asks for its menu
    property bool rowMenus: false
    signal rowMenu(int index, Item row, real x, real y)
    // -- WP USR1 -- The index the app says is current (the active preset, the
    // source): set `follows` and bind `followIndex` instead of `currentIndex`.
    // A ComboBox given a new list puts its index back to 0 and runs its
    // currentIndex binding again only when what that reads changes, so the
    // preset box showed the first preset while another was active (each new
    // "*" on a title is a new list). The box takes followIndex again after
    // each new list, and after a person's choice: the app then says what is
    // current, not the click.
    property bool follows: false
    property int followIndex: -1
    // -- WP REVDFIX -- (REVD D1) its tone, and on hover (Theme.barControl on a bar)
    property color fill: Theme.control
    property color fillHover: Theme.controlHover
    function _follow() {
        if (follows) currentIndex = Qt.binding(() => followIndex)
    }
    onModelChanged: {
        if (!follows) return
        _follow()
        Qt.callLater(_follow)   // the list's own reset may come after this signal
    }
    onActivated: _follow()
    Component.onCompleted: _follow()

    implicitHeight: Theme.controlHeight
    implicitWidth: 160
    font: Theme.fontBody
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    leftPadding: 8
    rightPadding: 24

    delegate: ItemDelegate {
        id: row
        required property int index
        required property var model
        width: ListView.view ? ListView.view.width : control.width
        height: Theme.rowHeight
        highlighted: control.highlightedIndex === index
        contentItem: Text {
            textFormat: Text.PlainText
            text: control.textRole ? row.model[control.textRole] : row.model.modelData
            font: control.font
            color: Theme.text
            elide: Text.ElideRight
            verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            color: row.highlighted ? Theme.selection : "transparent"
            radius: Theme.radiusSmall
        }
        TapHandler {   // -- WP LAY3b --
            acceptedButtons: Qt.RightButton
            enabled: control.rowMenus
            onTapped: point => control.rowMenu(row.index, row, point.position.x, point.position.y)
        }
        Accessible.role: Accessible.ListItem
        Accessible.name: contentItem.text
    }
    indicator: Icon {
        x: control.width - width - 6
        y: (control.height - height) / 2
        name: "chevrons-up-down"
        size: 12
        color: control.enabled ? Theme.textMuted : Theme.textDisabled
    }
    contentItem: Text {
        textFormat: Text.PlainText
        text: control.displayText
        font: control.font
        color: control.enabled ? Theme.text : Theme.textDisabled
        elide: Text.ElideRight
        verticalAlignment: Text.AlignVCenter
    }
    background: Rectangle {
        radius: Theme.radiusControl   // -- WP DL0 -- Modern: no outline but the focus ring
        color: control.down ? Theme.controlPressed : control.hovered ? control.fillHover : control.fill
        border.width: control.visualFocus ? Theme.focusRingWidth : Theme.controlLineWidth
        border.color: control.visualFocus ? Theme.focusRing : Theme.controlLine   // -- WP UB13 --
    }
    popup: Popup {
        y: control.height + 2
        width: Math.max(control.width, 160)
        // The whole list up to a screenful, then it scrolls
        implicitHeight: Math.min(contentItem.implicitHeight + topPadding + bottomPadding, Theme.popupMaxHeight)
        padding: 4
        contentItem: ListView {
            clip: true
            implicitHeight: contentHeight
            model: control.popup.visible ? control.delegateModel : null
            currentIndex: control.highlightedIndex
            ScrollBar.vertical: AppScrollBar {}
        }
        // -- WP REVDFIX -- (REVD D5) the floating surface, as AppMenu's
        background: FloatingBackground {
            color: Theme.menu
            radius: LayoutController.modern ? Theme.radius : Theme.radiusSmall
        }
    }

    Accessible.role: Accessible.ComboBox
    Accessible.name: label
    Accessible.description: displayText
}
