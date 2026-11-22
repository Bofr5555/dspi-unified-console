import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5b -- Library > Files: the filter and configuration files, the files
// with no device (P1OF) and the templates. The buttons are the menu table's
// rows; their questions and pickers are FilesHost's, as everywhere.
Item {
    id: page
    objectName: "LibraryFiles"
    property bool shown: false
    property Item host: null

    property var templates: []
    onShownChanged: if (shown) FilesController.readTemplates()
    Connections {
        target: FilesController
        function onTemplatesRead(list) { page.templates = list }
        // A template saved, a file opened or left
        function onOfflineChanged() { if (page.shown) FilesController.readTemplates() }
    }

    component Action: AppButton {
        required property string actionId
        objectName: "LibraryFiles." + actionId
        text: MenuTable.textOf(actionId, false)
        toolTip: MenuTable.shortcutOf(actionId)
        enabled: MenuTable.can(actionId)
        onClicked: Shell.trigger(actionId)
    }
    component Note: Text {
        Layout.fillWidth: true
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }

    Flickable {
        id: flick
        objectName: "LibraryFiles.body"
        anchors.fill: parent
        contentHeight: column.implicitHeight + 2 * Theme.spacingLarge
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: AppScrollBar {}

        ColumnLayout {
            id: column
            x: (flick.width - width) / 2
            y: Theme.spacingLarge
            width: Math.min(760, flick.width - 2 * Theme.spacingLarge)
            spacing: Theme.spacing

            Card {
                Layout.fillWidth: true
                title: qsTr("Filters")
                Note { text: qsTr("The bands of the channels you choose, as a text file that other programs read too.") }
                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                    Action { actionId: "file.importFilters" }
                    Action { actionId: "file.exportFilters" }
                }
            }
            Card {
                Layout.fillWidth: true
                title: qsTr("Device configuration")
                Note { text: qsTr("Every setting of the device. An import shows what it changes first.") }
                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                    Action { actionId: "file.importConfiguration" }
                    Action { actionId: "file.exportConfiguration" }
                }
            }
            Card {
                Layout.fillWidth: true
                title: qsTr("Files without a device")
                Note {
                    objectName: "LibraryFiles.open"
                    text: FilesController.offline
                          //: %1 is a file's name
                          ? qsTr("Open now: %1").arg(FilesController.offlineFile.length ? FilesController.offlineFile : qsTr("not saved yet"))
                          : qsTr("Edit a configuration on every page with no device, then save it or import it to a DSPi.")
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                    Action { actionId: "file.openFile" }
                    Action { actionId: "file.saveFile" }
                    Action { actionId: "file.saveAsTemplate" }
                }
            }
            Card {
                Layout.fillWidth: true
                title: qsTr("Templates")
                Note { text: qsTr("A template opens as a new file with no device.") }
                Note {
                    objectName: "LibraryFiles.noTemplates"
                    visible: page.templates.length === 0
                    text: qsTr("No templates")
                }
                Repeater {
                    model: page.templates
                    delegate: RowLayout {
                        id: row
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true
                        spacing: Theme.spacing
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Text {
                                Layout.fillWidth: true
                                textFormat: Text.PlainText
                                text: row.modelData.name
                                font: Theme.fontBody
                                color: Theme.text
                                elide: Text.ElideRight
                            }
                            Note {
                                //: %1 the platform, %2 and %3 the input and output counts, %4 "shipped" or "yours"
                                text: qsTr("%1, %2 in, %3 out · %4").arg(row.modelData.platform).arg(row.modelData.inputs)
                                          .arg(row.modelData.outputs)
                                          .arg(row.modelData.builtin ? qsTr("shipped with the app") : qsTr("saved here"))
                            }
                            // The template's own status line (the core's words)
                            Note {
                                visible: text.length > 0
                                text: row.modelData.status || ""
                                color: Theme.warning
                            }
                        }
                        AppButton {
                            objectName: "LibraryFiles.template." + row.index
                            text: qsTr("Open")
                            //: %1 is a template's name
                            Accessible.name: qsTr("Open template %1").arg(row.modelData.name)
                            onClicked: FilesController.openTemplate(row.modelData.id, row.modelData.name)
                        }
                    }
                }
            }
        }
    }
}
