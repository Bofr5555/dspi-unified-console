import QtQuick
import QtQuick.Controls.Basic
import UConsole

// One headphone in the AutoEQ browser (HeadphoneRow): its kind, its name, the
// measurement source's tag and the form factor, and the heart that shows on
// hover or while it is a favourite.
ItemDelegate {
    id: row
    required property int index
    required property string profileId
    required property string displayName
    required property string source
    required property string sourceName
    required property string formFactor
    required property bool favorite
    property bool selected: false
    signal chosen()

    objectName: "AutoEqWindow.row." + index
    width: ListView.view ? ListView.view.width : implicitWidth
    implicitHeight: 48
    hoverEnabled: true
    focusPolicy: Qt.NoFocus
    padding: 0
    onClicked: chosen()

    Accessible.role: Accessible.ListItem
    Accessible.name: displayName
    Accessible.description: sourceName + ", " + formFactor
    Accessible.selected: selected

    background: Rectangle {
        color: row.selected ? Theme.selection : row.hovered ? Theme.controlHover : "transparent"
    }

    contentItem: Item {
        Icon {
            id: kind
            x: 12
            anchors.verticalCenter: parent.verticalCenter
            width: 30
            name: row.formFactor === "in-ear" || row.formFactor === "earbud" ? "ear" : "headphones"
            size: 22
            color: Theme.accentText
        }
        Column {
            anchors.left: kind.right
            anchors.leftMargin: 12
            anchors.right: heart.left
            anchors.rightMargin: Theme.spacingSmall
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2
            Text {
                width: parent.width
                text: row.displayName
                font: Theme.fontBody
                color: Theme.text
                elide: Text.ElideRight
            }
            Row {
                spacing: Theme.spacingSmall
                // The source's tag: a faint capsule of its colour
                Rectangle {
                    width: tag.implicitWidth + 12
                    height: tag.implicitHeight + 4
                    radius: height / 2
                    color: Qt.alpha(Theme.autoEqSourceTint(row.source), row.selected ? 0.35 : 0.22)
                    Text {
                        id: tag
                        anchors.centerIn: parent
                        text: row.sourceName
                        font: Theme.fontSmall
                        color: Theme.text
                    }
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.formFactor
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
            }
        }
        IconButton {
            id: heart
            objectName: "AutoEqWindow.favorite." + row.index
            anchors.right: parent.right
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            visible: row.hovered || row.favorite
            iconName: "heart"
            iconSize: 14
            iconColor: row.favorite ? Theme.dangerText : Theme.textMuted
            toolTip: row.favorite ? qsTr("Remove from favorites") : qsTr("Add to favorites")
            onClicked: AutoEqController.toggleFavorite(row.profileId)
        }
    }
}
