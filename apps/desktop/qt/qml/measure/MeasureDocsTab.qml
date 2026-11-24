import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P5QUI -- The documents (the web's P5WAT docs.tsx): measurements,
// microphone calibrations and targets, this console's and the hub's merged
// (the hub's copy wins); open, rename, note, delete (the hub's by its rules:
// Admin deletes). A measurement opens on Results, a calibration and a target
// are used from then on.
ColumnLayout {
    id: tab
    signal showResults()
    spacing: Theme.spacingSmall

    readonly property var mc: MeasureController
    property string kind: ""
    // -- INTC (DL2 fold) -- Modern's page shows the filter, Reload and Import
    // Calibration… in its tab row (MeasurePanel's pageTools), not here
    property bool toolsAbove: false
    readonly property var kinds: [
        { id: "", text: qsTr("Every kind") }, { id: "measurement", text: qsTr("Measurements") },
        { id: "mic", text: qsTr("Microphones") }, { id: "target", text: qsTr("Targets") }
    ]
    readonly property var rows: mc.documents.filter(d => tab.kind === "" || d.kind === tab.kind)
    function kindWords(k) {
        return k === "measurement" ? qsTr("Measurement") : k === "mic" ? qsTr("Microphone") : k === "target" ? qsTr("Target") : k
    }
    function whereWords(w) {
        return w === "hub" ? qsTr("On the hub") : w === "both" ? qsTr("Here and on the hub") : qsTr("This console")
    }

    Component.onCompleted: mc.reloadDocuments()

    // -- WP DL2 -- Modern: no documents at all is an empty state (the filter
    // row only while there is something to filter); rows are list rows
    readonly property bool modern: LayoutController.modern
    readonly property bool none: mc.documents.length === 0 && tab.kind === ""

    RowLayout {
        visible: !(tab.modern && tab.none) && !tab.toolsAbove   // -- INTC --
        spacing: Theme.spacingSmall
        AppComboBox {
            label: qsTr("Kind")
            Layout.preferredWidth: 180
            model: tab.kinds
            textRole: "text"
            onActivated: index => tab.kind = tab.kinds[index].id
        }
        AppButton {
            objectName: "MeasureDocs.reload"
            text: qsTr("Reload")
            onClicked: mc.reloadDocuments()
        }
        Item { Layout.fillWidth: true }
        AppButton {
            text: qsTr("Import Calibration…")
            onClicked: mc.chooseMicFile()
        }
    }
    Text {
        visible: text.length > 0
        Layout.fillWidth: true
        textFormat: Text.PlainText
        text: mc.documentsNote
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.WordWrap
    }
    Text {
        visible: tab.rows.length === 0 && !(tab.modern && tab.none)
        Layout.fillWidth: true
        text: tab.modern ? qsTr("No documents of this kind.") : qsTr("No documents yet: save a measurement from Results.")
        font: Theme.fontBody
        color: Theme.textMuted
    }
    EmptyState {
        objectName: "MeasureDocs.empty"
        visible: tab.modern && tab.none
        Layout.fillWidth: true
        Layout.fillHeight: true
        iconName: "folder-open"
        title: qsTr("No documents yet")
        text: qsTr("Measurements you save from Results, microphone calibrations and targets are kept here.")
        actionText: qsTr("Import Calibration…")
        onAction: mc.chooseMicFile()
    }
    ListView {
        id: list
        objectName: "MeasureDocs.list"
        visible: !(tab.modern && tab.none)
        Layout.fillWidth: true
        Layout.fillHeight: true
        clip: true
        spacing: tab.modern ? 0 : 4
        model: tab.rows
        ScrollBar.vertical: AppScrollBar {}
        delegate: Rectangle {
            id: row
            required property var modelData
            required property int index
            width: list.width
            implicitHeight: cols.implicitHeight + 12
            radius: Theme.radiusSmall
            // -- WP DL2 -- Modern: the stripe on every second row, no box
            color: tab.modern ? (row.index % 2 === 1 ? Theme.stripe : "transparent") : Theme.panel
            border.width: tab.modern ? 0 : 1
            border.color: Theme.separator
            ColumnLayout {
                id: cols
                anchors.fill: parent
                anchors.margins: 6
                spacing: 4
                RowLayout {
                    spacing: Theme.spacingSmall
                    BorderedField {
                        objectName: "MeasureDocs.name." + row.index
                        label: qsTr("Name")
                        Layout.fillWidth: true
                        text: row.modelData.name
                        onCommitted: t => { if (t !== row.modelData.name) mc.renameDocument(row.modelData.id, row.modelData.where === "hub" ? "hub" : "local", t) }
                    }
                    Text {
                        textFormat: Text.PlainText
                        text: tab.kindWords(row.modelData.kind) + " · " + tab.whereWords(row.modelData.where) +
                              (row.modelData.saved ? " · " + row.modelData.saved : "") + (row.modelData.serial ? " · " + row.modelData.serial : "")
                        font: Theme.fontSmall
                        color: Theme.textMuted
                    }
                    AppButton {
                        objectName: "MeasureDocs.open." + row.index
                        text: row.modelData.kind === "measurement" ? qsTr("Open") : qsTr("Use")
                        onClicked: {
                            mc.openDocument(row.modelData.id, row.modelData.where === "hub" ? "hub" : "local")
                            if (row.modelData.kind === "measurement") tab.showResults()
                        }
                    }
                    AppButton {
                        objectName: "MeasureDocs.delete." + row.index
                        text: qsTr("Delete…")
                        destructive: !tab.modern   // -- WP DL2 -- the question is the destructive one
                        onClicked: {
                            deleteQuestion.doc = row.modelData
                            deleteQuestion.open()
                        }
                    }
                }
                BorderedField {
                    label: qsTr("Note")
                    Layout.fillWidth: true
                    placeholderText: qsTr("A note")
                    text: row.modelData.note
                    onCommitted: t => { if (t !== row.modelData.note) mc.noteDocument(row.modelData.id, row.modelData.where === "hub" ? "hub" : "local", t) }
                }
            }
        }
    }

    QuestionDialog {
        id: deleteQuestion
        objectName: "MeasureDocs.deleteQuestion"
        ofDevice: true   // -- WP MDQT2c -- the hub's copy is that of its hub
        parent: Overlay.overlay
        property var doc: ({})
        title: qsTr("Delete \"%1\"?").arg(doc.name || "")
        text: doc.where === "hub" || doc.where === "both" ? qsTr("The hub's copy is deleted for every console (the hub lets its Admin do that). It cannot be undone.")
                                                           : qsTr("It is deleted from this console. It cannot be undone.")
        acceptText: qsTr("Delete")
        destructive: true
        onAnswered: answer => { if (answer === "accept") mc.deleteDocument(doc.id, doc.where === "hub" || doc.where === "both" ? "hub" : "local") }
    }
}
