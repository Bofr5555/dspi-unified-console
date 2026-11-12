import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A popup button choosing one value from a list (the Mac's Picker with the
// menu style: presets, sources, settings choices). `label` names it for
// screen readers; `activated(index)` fires on a person's choice only.
ComboBox {
    id: control
    property string label

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
        text: control.displayText
        font: control.font
        color: control.enabled ? Theme.text : Theme.textDisabled
        elide: Text.ElideRight
        verticalAlignment: Text.AlignVCenter
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        color: control.down ? Theme.controlPressed : control.hovered ? Theme.controlHover : Theme.control
        border.width: control.visualFocus ? 2 : 1
        border.color: control.visualFocus ? Theme.focusRing : Theme.border
    }
    popup: Popup {
        y: control.height + 2
        width: Math.max(control.width, 160)
        implicitHeight: Math.min(contentItem.implicitHeight + 8, 320)
        padding: 4
        contentItem: ListView {
            clip: true
            implicitHeight: contentHeight
            model: control.popup.visible ? control.delegateModel : null
            currentIndex: control.highlightedIndex
            ScrollBar.vertical: AppScrollBar {}
        }
        background: Rectangle {
            radius: Theme.radiusSmall
            color: Theme.menu
            border.width: 1
            border.color: Theme.border
        }
    }

    Accessible.role: Accessible.ComboBox
    Accessible.name: label
    Accessible.description: displayText
}
