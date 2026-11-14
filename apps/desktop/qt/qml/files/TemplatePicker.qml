import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P1QT1 -- File > New from Template…: the templates shipped with the app
// first, then those saved here (Save as Template…). The chosen one opens as a
// new file with no device (P1OF); Save File asks where it goes.
Dialog {
    id: picker
    objectName: "TemplatePicker"
    property var templates: []
    property int chosen: -1

    function ask(list) {
        templates = list
        chosen = list.length ? 0 : -1
        open()
    }

    title: qsTr("New from Template")
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 320
    width: Math.min(460, hostWidth - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: Math.max(Theme.spacingLarge, (hostHeight - implicitHeight) / 3)
    padding: Theme.spacingLarge
    topPadding: Theme.spacingSmall

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.color: Theme.border
    }
    header: Text {
        leftPadding: Theme.spacingLarge
        rightPadding: Theme.spacingLarge
        topPadding: Theme.spacingLarge
        text: picker.title
        font: Theme.fontTitle
        color: Theme.text
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }

    contentItem: ColumnLayout {
        spacing: Theme.spacingSmall
        Text {
            Layout.fillWidth: true
            text: qsTr("A template opens as a new file with no device. Edit it on every screen, then save it, or import it to a DSPi.")
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 200
            radius: Theme.radiusSmall
            color: Theme.window
            border.color: Theme.separator
            ListView {
                id: list
                objectName: "TemplatePicker.list"
                anchors.fill: parent
                anchors.margins: 1
                clip: true
                model: picker.templates
                currentIndex: picker.chosen
                activeFocusOnTab: true
                keyNavigationEnabled: true
                onCurrentIndexChanged: picker.chosen = currentIndex
                ScrollBar.vertical: AppScrollBar {}
                Accessible.role: Accessible.List
                Accessible.name: qsTr("Templates")
                delegate: Rectangle {
                    id: row
                    required property var modelData
                    required property int index
                    width: ListView.view.width
                    height: Theme.rowHeight + 12
                    color: picker.chosen === index ? Theme.selection : "transparent"
                    TapHandler {
                        onTapped: picker.chosen = row.index
                        onDoubleTapped: {
                            picker.chosen = row.index
                            openButton.clicked()
                        }
                    }
                    ColumnLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.spacingSmall
                        anchors.rightMargin: Theme.spacingSmall
                        spacing: 0
                        Text {
                            Layout.fillWidth: true
                            text: row.modelData.name
                            font: Theme.fontBody
                            color: Theme.text
                            elide: Text.ElideRight
                        }
                        Text {
                            Layout.fillWidth: true
                            //: %1 the platform, %2 and %3 the input and output counts, %4 "shipped" or "yours"
                            text: qsTr("%1, %2 in, %3 out · %4").arg(row.modelData.platform).arg(row.modelData.inputs)
                                      .arg(row.modelData.outputs)
                                      .arg(row.modelData.builtin ? qsTr("shipped with the app") : qsTr("saved here"))
                            font: Theme.fontSmall
                            color: Theme.textMuted
                            elide: Text.ElideRight
                        }
                    }
                    Accessible.role: Accessible.ListItem
                    Accessible.name: row.modelData.name
                }
                Text {
                    anchors.centerIn: parent
                    visible: list.count === 0
                    text: qsTr("No templates")
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
            }
        }
    }

    footer: Item {
        implicitHeight: buttons.implicitHeight + 2 * Theme.spacing
        Row {
            id: buttons
            x: parent.width - width - Theme.spacingLarge
            y: Theme.spacing
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "TemplatePicker.cancel"
                text: qsTr("Cancel")
                onClicked: picker.close()
            }
            AppButton {
                id: openButton
                objectName: "TemplatePicker.open"
                text: qsTr("Open")
                primary: true
                enabled: picker.chosen >= 0
                onClicked: {
                    const t = picker.templates[picker.chosen]
                    picker.close()
                    FilesController.openTemplate(t.id, t.name)
                }
            }
        }
    }
}
