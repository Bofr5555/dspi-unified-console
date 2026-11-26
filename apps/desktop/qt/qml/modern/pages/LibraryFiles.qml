import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5b -- Library > Files: the filter and configuration files, the files
// with no device (P1OF) and the templates. The buttons are the menu table's
// rows; their questions and pickers are FilesHost's, as everywhere.
// -- WP DL3a -- Sections in the 760 px column at the page's left, no cards
// (DESIGN-LANGUAGE §3, §8): a section label, one muted sentence, the buttons.
// The templates are rows (§9) with their description wrapped under the name
// in muted text, and Open on hover; a double click opens one too.
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
        font: Theme.fontBody
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
    // A section: its label, a sentence, then its content
    component Section: ColumnLayout {
        property alias title: heading.text
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        SectionHeading { id: heading; Layout.fillWidth: true }
    }

    Flickable {
        id: flick
        objectName: "LibraryFiles.body"
        anchors.fill: parent
        contentHeight: column.implicitHeight + 2 * Theme.spacing
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: AppScrollBar {}

        ContentColumn {
            id: column
            x: Theme.spacing
            y: Theme.spacing
            width: Math.min(maximumWidth, flick.width - 2 * Theme.spacing)

            Section {
                title: qsTr("Filters")
                Note { text: qsTr("The bands of the channels you choose, as a text file that other programs read too.") }
                Flow {
                    Layout.fillWidth: true
                    Layout.topMargin: 4
                    spacing: Theme.spacingSmall
                    Action { actionId: "file.importFilters" }
                    Action { actionId: "file.exportFilters" }
                }
            }
            Section {
                title: qsTr("Device configuration")
                Note { text: qsTr("Every setting of the device. An import shows what it changes first.") }
                Flow {
                    Layout.fillWidth: true
                    Layout.topMargin: 4
                    spacing: Theme.spacingSmall
                    Action { actionId: "file.importConfiguration" }
                    Action { actionId: "file.exportConfiguration" }
                }
            }
            Section {
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
                    Layout.topMargin: 4
                    spacing: Theme.spacingSmall
                    Action { actionId: "file.openFile" }
                    Action { actionId: "file.saveFile" }
                    Action { actionId: "file.saveAsTemplate" }
                }
            }
            Section {
                title: qsTr("Templates")
                Note { text: qsTr("A template opens as a new file with no device.") }
                Note {
                    objectName: "LibraryFiles.noTemplates"
                    visible: page.templates.length === 0
                    text: qsTr("No templates")
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.topMargin: 4
                    spacing: 0
                    Repeater {
                        model: page.templates
                        delegate: Item {
                            id: row
                            required property var modelData
                            required property int index
                            readonly property bool hot: hover.hovered || open.hovered || open.activeFocus
                            Layout.fillWidth: true
                            implicitHeight: lines.implicitHeight + 2 * 6
                            Accessible.role: Accessible.ListItem
                            Accessible.name: row.modelData.status ? row.modelData.name + ", " + row.modelData.status : row.modelData.name

                            // ListRow's tones: stripe, hover
                            Rectangle {
                                anchors.fill: parent
                                radius: Theme.radiusSmall
                                color: row.hot ? Theme.controlHover : row.index % 2 === 1 ? Theme.stripe : "transparent"
                            }
                            HoverHandler { id: hover }
                            TapHandler {
                                onDoubleTapped: open.clicked()
                            }
                            ColumnLayout {
                                id: lines
                                anchors.left: parent.left
                                anchors.right: open.left
                                anchors.verticalCenter: parent.verticalCenter
                                anchors.leftMargin: Theme.spacingSmall
                                anchors.rightMargin: Theme.spacingSmall
                                spacing: 1
                                Text {
                                    Layout.fillWidth: true
                                    textFormat: Text.PlainText
                                    text: row.modelData.name
                                    font: Theme.fontTable
                                    color: Theme.text
                                    elide: Text.ElideRight
                                }
                                Text {
                                    Layout.fillWidth: true
                                    textFormat: Text.PlainText
                                    //: %1 the platform, %2 and %3 the input and output counts, %4 "shipped" or "yours"
                                    text: qsTr("%1, %2 in, %3 out · %4").arg(row.modelData.platform).arg(row.modelData.inputs)
                                              .arg(row.modelData.outputs)
                                              .arg(row.modelData.builtin ? qsTr("shipped with the app") : qsTr("saved here"))
                                    font: Theme.fontSmall
                                    color: Theme.textMuted
                                    elide: Text.ElideRight
                                }
                                // The template's own description (the core's words)
                                Text {
                                    objectName: "LibraryFiles.status." + row.index
                                    Layout.fillWidth: true
                                    visible: text.length > 0
                                    textFormat: Text.PlainText
                                    text: row.modelData.status || ""
                                    font: Theme.fontSmall
                                    color: Theme.textMuted
                                    wrapMode: Text.Wrap
                                }
                            }
                            AppButton {
                                id: open
                                objectName: "LibraryFiles.template." + row.index
                                anchors.right: parent.right
                                anchors.rightMargin: Theme.spacingSmall / 2
                                anchors.verticalCenter: parent.verticalCenter
                                // On hover, as a ListRow's actions; still there for the keyboard
                                opacity: row.hot ? 1 : 0
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
}
