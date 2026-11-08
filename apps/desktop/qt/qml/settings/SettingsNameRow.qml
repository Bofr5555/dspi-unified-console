import QtQuick 2.15
import QtQuick.Controls 2.15
import "../components"

// A name the device stores: an editable field that holds a draft until Save, with
// a dot while it differs from what is stored and a button to empty it.
SettingsRow {
    id: row
    property string value: ""
    property bool pending: false
    property string fieldName: ""      // objectName of the field
    property string clearTip: ""
    signal edited(string text)

    Row {
        spacing: 10
        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 6; height: 6; radius: 3
            color: "#ff9f0a"
            opacity: row.pending ? 1 : 0
        }
        Rectangle {
            width: 190
            height: 26
            radius: 6
            color: Qt.rgba(1, 1, 1, field.activeFocus ? 0.10 : 0.05)
            border.width: field.activeFocus ? 1 : 0
            border.color: isMacOS ? MacColors.accent : "#0a7cff"

            TextInput {
                id: field
                objectName: row.fieldName
                x: 8
                width: parent.width - 8 - 24
                height: parent.height
                verticalAlignment: Text.AlignVCenter
                text: row.value
                maximumLength: 31
                selectByMouse: true
                clip: true
                activeFocusOnTab: true   // -- WP QD22 --
                Accessible.name: row.title
                font.pixelSize: 13
                color: isMacOS ? MacColors.label : "white"
                selectionColor: isMacOS ? MacColors.selectedControl : "#0a7cff"
                selectedTextColor: "white"
                onTextEdited: {
                    var clean = deviceNames.clean(text)
                    if (clean !== text) {
                        var at = Math.min(cursorPosition, clean.length)
                        text = clean
                        cursorPosition = at
                    }
                    row.edited(clean)
                }
                onAccepted: focus = false
                Connections {
                    target: row
                    function onValueChanged() { if (field.text !== row.value) field.text = row.value }
                }
                Text {
                    visible: field.text === ""
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Add a name"
                    font: field.font
                    color: isMacOS ? MacColors.tertiaryLabel : Qt.rgba(1, 1, 1, 0.35)
                }
            }
            Item {
                anchors.right: parent.right
                width: 24
                height: parent.height
                visible: field.text !== ""
                Icon {
                    anchors.centerIn: parent
                    name: "xmark"
                    size: 11
                    color: isMacOS ? MacColors.tertiaryLabel : Qt.rgba(1, 1, 1, 0.45)
                }
                ClickArea {
                    id: clearMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    Accessible.name: row.clearTip !== "" ? row.clearTip : "Clear " + row.title
                    onActivated: { field.text = ""; row.edited("") }
                }
                ToolTip.visible: clearMouse.containsMouse && row.clearTip !== ""
                ToolTip.delay: 600
                ToolTip.text: row.clearTip
            }
        }
    }
}
