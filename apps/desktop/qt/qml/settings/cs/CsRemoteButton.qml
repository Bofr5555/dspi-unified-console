import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- One learned remote button (the Mac's irCommandCard): the
// learned code, what it does, Learn (or Re-learn) and the editor while open.
Rectangle {
    id: card
    property string key
    property bool receiverLive
    property bool expanded: false
    readonly property var rec: CsController.revision < 0 ? ({}) : CsController.record(key)
    readonly property bool learning: rec.learning === true
    objectName: "Cs." + key.replace("/", ".")

    Layout.fillWidth: true
    implicitHeight: column.implicitHeight + 2 * Theme.spacingSmall
    radius: Theme.radiusSmall
    color: Theme.control
    Accessible.role: Accessible.Grouping
    Accessible.name: rec.codeText || ""

    Connections {
        target: CsController
        function onAdded(key) { if (key === card.key) card.expanded = true }
    }

    ColumnLayout {
        id: column
        x: Theme.spacingSmall
        y: Theme.spacingSmall
        width: parent.width - 2 * Theme.spacingSmall
        spacing: Theme.spacingSmall

        RowLayout {
            Layout.fillWidth: true
            spacing: 10
            IconButton {
                objectName: card.objectName + ".disclose"
                iconName: card.expanded ? "chevron-down" : "chevron-right"
                iconSize: 11
                toolTip: card.expanded ? qsTr("Collapse") : qsTr("Expand")
                onClicked: card.expanded = !card.expanded
            }
            Icon {
                name: card.rec.learned === true ? "circle-check" : "circle-dashed"
                size: 12
                color: card.rec.learned === true ? Theme.success : Theme.textMuted
            }
            Text {
                objectName: card.objectName + ".code"
                text: card.rec.codeText || ""
                font: card.rec.learned === true ? Theme.fontMono : Theme.fontSmall
                color: card.rec.learned === true ? Theme.text : Theme.textMuted
            }
            Text {
                Layout.fillWidth: true
                visible: !card.expanded && card.rec.learned === true
                text: card.rec.summary || ""
                font: Theme.fontSmall
                color: Theme.textMuted
                elide: Text.ElideRight
            }
            Item { Layout.fillWidth: card.expanded || card.rec.learned !== true }
            Icon {
                visible: card.learning
                name: "loader-circle"
                size: 14
                color: Theme.textMuted
            }
            AppButton {
                objectName: card.objectName + ".learn"
                text: card.learning ? qsTr("Cancel") : card.rec.learned === true ? qsTr("Re-learn") : qsTr("Learn Button")
                enabled: card.learning || (card.receiverLive && CsController.writable && CsController.learningSub < 0
                                           && !CsController.busy)
                onClicked: card.learning ? CsController.cancelLearn() : CsController.startLearn(card.rec.slot)
            }
            IconButton {
                objectName: card.objectName + ".remove"
                iconName: "trash"
                iconSize: 12
                toolTip: qsTr("Remove")
                enabled: !card.learning && card.rec.applying !== true
                onClicked: CsController.remove(card.key)
            }
        }
        Text {
            objectName: card.objectName + ".learnMessage"
            visible: card.learning && CsController.learnMessage.length > 0
            Layout.fillWidth: true
            text: CsController.learnMessage
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        CsRows {
            visible: card.expanded
            objectName: card.objectName + ".rows"
            rows: card.rec.rows || []
            enabled: CsController.writable
            onEdited: (id, value) => CsController.edit(card.key, id, value)
        }
        RowLayout {
            visible: !card.learning && (card.rec.message || "").length > 0
            Layout.fillWidth: true
            spacing: 6
            Icon {
                name: card.rec.isError === true ? "triangle-alert" : "circle-check"
                size: 12
                color: card.rec.isError === true ? Theme.warning : Theme.success
            }
            Text {
                objectName: card.objectName + ".message"
                Layout.fillWidth: true
                text: card.rec.message || ""
                font: Theme.fontSmall
                color: card.rec.isError === true ? Theme.warning : Theme.textMuted
                wrapMode: Text.Wrap
            }
        }
    }
}
