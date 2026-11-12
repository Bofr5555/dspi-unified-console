import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
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

    leftPadding: 12
    rightPadding: 12
    contentItem: RowLayout {
        spacing: 12
        Icon {
            Layout.preferredWidth: 30
            Layout.alignment: Qt.AlignVCenter
            name: row.formFactor === "in-ear" || row.formFactor === "earbud" ? "ear" : "headphones"
            size: 22
            color: Theme.accentText
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignVCenter
            spacing: 2
            Text {
                Layout.fillWidth: true
                text: row.displayName
                font: Theme.fontBody
                color: Theme.text
                elide: Text.ElideRight
            }
            RowLayout {
                spacing: Theme.spacingSmall
                // The source's tag: a faint capsule of its colour
                Label {
                    leftPadding: 6
                    rightPadding: 6
                    topPadding: 2
                    bottomPadding: 2
                    text: row.sourceName
                    font: Theme.fontSmall
                    color: Theme.text
                    background: Rectangle {
                        radius: parent.height / 2
                        color: Qt.alpha(Theme.autoEqSourceTint(row.source), row.selected ? 0.35 : 0.22)
                    }
                }
                Label {
                    text: row.formFactor
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
            }
        }
        // The heart: on hover, and always on a favourite
        IconButton {
            objectName: "AutoEqWindow.favorite." + row.index
            Layout.alignment: Qt.AlignVCenter
            opacity: row.hovered || row.favorite ? 1 : 0
            enabled: opacity > 0
            iconName: "heart"
            iconSize: 14
            iconColor: row.favorite ? Theme.dangerText : Theme.textMuted
            toolTip: row.favorite ? qsTr("Remove from favorites") : qsTr("Add to favorites")
            onClicked: AutoEqController.flipFavourite(row.profileId)
        }
    }
}
