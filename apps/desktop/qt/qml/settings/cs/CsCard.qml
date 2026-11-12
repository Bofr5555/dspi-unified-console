import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import QtQml
import UConsole

// -- WP QP12 -- One control-surface card (the Mac's slotSection, groupCard
// and macroCard): the header a collapsed card shows (disclosure, badge,
// editable name over the summary, state, remove), the editor while open, and
// the Apply row while open or while something is pending.
Rectangle {
    id: card
    // The record (CsController.record(key))
    property var rec: ({})
    property bool expanded: false
    // The badge: a component type, or a plain glyph
    property int badgeType: -1
    property string iconName
    property color iconColor: Theme.accentText
    // The badge's menu: [{type, name, icon, disabled}] to change the type
    property var typeChoices: []
    property bool showPill: false
    property bool ready: true
    property string hint
    property bool removable: true
    default property alias body: bodyColumn.data
    // Beside the remove button (a macro's run button)
    property alias trailing: trailingRow.data
    // At the start of the Apply row (the receiver's Add Remote Button)
    property alias applyLeading: applyRow.leading
    readonly property string base: objectName

    Layout.fillWidth: true
    implicitHeight: column.implicitHeight + 2 * Theme.spacing
    radius: Theme.radius
    color: Theme.panel
    border.width: 1
    border.color: Theme.separator
    Accessible.role: Accessible.Grouping
    Accessible.name: (rec.name || "").length > 0 ? rec.name : (rec.placeholder || "")

    ColumnLayout {
        id: column
        x: Theme.spacing
        y: Theme.spacing
        width: parent.width - 2 * Theme.spacing
        spacing: Theme.spacing

        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            IconButton {
                objectName: card.base + ".disclose"
                iconName: card.expanded ? "chevron-down" : "chevron-right"
                iconSize: 12
                toolTip: card.expanded ? qsTr("Collapse") : qsTr("Expand")
                onClicked: card.expanded = !card.expanded
            }
            Item {
                implicitWidth: card.badgeType >= 0 ? 32 : 20
                implicitHeight: card.badgeType >= 0 ? 32 : 20
                CsBadge {
                    visible: card.badgeType >= 0
                    type: card.badgeType
                    MouseArea {
                        anchors.fill: parent
                        enabled: card.typeChoices.length > 1 && CsController.writable
                        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: typeMenu.popup()
                    }
                    ToolTip.visible: badgeHover.hovered && card.typeChoices.length > 1
                    ToolTip.text: qsTr("Change the component type")
                    HoverHandler { id: badgeHover }
                }
                Icon {
                    visible: card.badgeType < 0
                    anchors.centerIn: parent
                    name: card.iconName
                    size: 14
                    color: card.iconColor
                }
                AppMenu {
                    id: typeMenu
                    objectName: card.base + ".typeMenu"
                    Instantiator {
                        model: card.typeChoices
                        delegate: AppMenuItem {
                            required property var modelData
                            text: modelData.name
                            enabled: !modelData.disabled
                            onTriggered: CsController.edit(card.rec.key, "type", modelData.type)
                        }
                        onObjectAdded: (index, object) => typeMenu.insertItem(index, object)
                        onObjectRemoved: (index, object) => typeMenu.removeItem(object)
                    }
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                BorderedField {
                    objectName: card.base + ".name"
                    Layout.fillWidth: true
                    Layout.maximumWidth: 280
                    label: qsTr("Name")
                    text: card.rec.name || ""
                    placeholderText: card.rec.placeholder || ""
                    enabled: CsController.writable
                    onTextEdited: CsController.setName(card.rec.key, text)
                    ToolTip.visible: hovered && !activeFocus
                    ToolTip.text: qsTr("Click to rename")
                }
                Text {
                    objectName: card.base + ".summary"
                    Layout.fillWidth: true
                    text: card.rec.summary || ""
                    font: Theme.fontSmall
                    color: card.rec.running === true ? Theme.success : Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
            Icon {
                visible: (card.rec.warning || "").length > 0
                name: "triangle-alert"
                size: 12
                color: Theme.warning
                ToolTip.visible: warnHover.hovered
                ToolTip.text: card.rec.warning || ""
                HoverHandler { id: warnHover }
            }
            CsPill {
                objectName: card.base + ".pill"
                visible: card.showPill
                text: card.rec.pill || ""
                good: card.rec.pillGood === true
            }
            RowLayout {
                id: trailingRow
                spacing: 2
            }
            IconButton {
                objectName: card.base + ".remove"
                visible: card.removable
                iconName: "trash"
                iconSize: 13
                toolTip: qsTr("Remove")
                enabled: !card.rec.applying && (card.rec.liveConfigured !== true || CsController.writable)
                onClicked: CsController.remove(card.rec.key)
            }
        }

        ColumnLayout {
            id: bodyColumn
            visible: card.expanded
            Layout.fillWidth: true
            Layout.leftMargin: 26
            spacing: Theme.spacing
        }

        CsApplyRow {
            id: applyRow
            objectName: card.base + ".applyRow"
            visible: card.expanded || card.rec.dirty === true || card.rec.applying === true || (card.rec.message || "").length > 0
            dirty: card.rec.dirty === true
            applying: card.rec.applying === true
            message: card.rec.message || ""
            isError: card.rec.isError === true
            ready: card.ready
            hint: card.hint
            onApply: CsController.apply(card.rec.key)
            onRevert: CsController.revertRecord(card.rec.key)
        }
    }
}
