import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P1QT1 -- Device > Snapshots…: whole-device states kept on this
// computer for the open device (P1SN), with the A/B compare (P1AB). Capture,
// name and note them, recall one (with rollback when asked, or always over
// DSPi Link to a hub with the guard), export and import them as files, and
// compare two: the toggle (B) recalls the other side; the rows say
// what differs from A to B.
// -- WP P1DCFIX -- In Simple mode the window recalls and compares only: no
// capture, rename, note, delete, import, export, rollback choice or timeout.
// -- WP P2QT -- Over DSPi Link to a hub that keeps snapshots, an "On the Hub"
// section lists the hub's (the hub's copy wins on an id both hold): recall one
// from the hub, copy it here, delete it (an Admin of the hub); "Save to the
// Hub" files the selected one of this console there.
// -- WP LAY1b -- A panel (Panel.qml): PanelTable's row for device.snapshots gives
// its window's title and sizes.
Panel {
    id: panel
    objectName: "SnapshotsPanel"

    readonly property bool hasDevice: DeviceModel.connected
    readonly property bool writable: hasDevice && !DeviceModel.readOnly
    readonly property bool simple: ModeController.mode === "simple"   // -- WP P1DCFIX --
    property string selectedId: ""
    readonly property var selected: {
        const rows = SnapshotsController.snapshots
        for (let i = 0; i < rows.length; ++i) if (rows[i].id === selectedId) return rows[i]
        return null
    }

    onShownChanged: if (shown) SnapshotsController.reload()

    // A side's button: "A: Bench" for a snapshot, "A" alone for the state the
    // compare captured
    function sideName(side, name) {
        //: %1 is A or B, %2 the snapshot on that side
        return name.length && name !== side ? qsTr("%1: %2").arg(side).arg(name) : side
    }

    // A/B's choices: A may be the device as it is now
    // -- WP P2QT -- the hub's snapshot chosen in "On the Hub"
    property string hubSelectedId: ""
    readonly property var hubSelected: {
        const rows = SnapshotsController.hubSnapshots
        for (let i = 0; i < rows.length; ++i) if (rows[i].id === hubSelectedId) return rows[i]
        return null
    }

    readonly property var aChoices: [{ id: "", name: qsTr("Current settings") }].concat(SnapshotsController.snapshots)
    readonly property var bChoices: SnapshotsController.snapshots

    QuestionDialog {
        id: hubDeleteQuestion   // -- WP P2QT --
        objectName: "SnapshotsWindow.hubDeleteQuestion"
        property string snapshotId
        title: qsTr("Delete Snapshot from the Hub")
        acceptText: qsTr("Delete")
        rejectText: qsTr("Cancel")
        destructive: true
        onAnswered: answer => {
            if (answer === "accept") SnapshotsController.removeFromHub(snapshotId)
        }
    }

    QuestionDialog {
        id: deleteQuestion
        objectName: "SnapshotsWindow.deleteQuestion"
        property string snapshotId
        title: qsTr("Delete Snapshot")
        acceptText: qsTr("Delete")
        rejectText: qsTr("Cancel")
        destructive: true
        onAnswered: answer => {
            if (answer === "accept") SnapshotsController.remove(snapshotId)
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacingSmall

        // ---- Capture ----
        RowLayout {
            objectName: "SnapshotsWindow.captureRow"
            Layout.fillWidth: true
            visible: !panel.simple
            spacing: Theme.spacingSmall
            BorderedField {
                id: captureName
                objectName: "SnapshotsWindow.captureName"
                Layout.fillWidth: true
                Layout.preferredWidth: 200
                label: qsTr("New snapshot's name")
                placeholderText: qsTr("Name")
                enabled: panel.hasDevice
                onAccepted: captureButton.clicked()
            }
            BorderedField {
                id: captureNote
                objectName: "SnapshotsWindow.captureNote"
                Layout.fillWidth: true
                Layout.preferredWidth: 300
                label: qsTr("New snapshot's note")
                placeholderText: qsTr("Note (optional)")
                enabled: panel.hasDevice
                onAccepted: captureButton.clicked()
            }
            AppButton {
                id: captureButton
                objectName: "SnapshotsWindow.capture"
                text: qsTr("Capture")
                iconName: "plus"
                primary: true
                enabled: panel.hasDevice && !SnapshotsController.busy
                toolTip: qsTr("Keep the device's settings as they are now")
                onClicked: {
                    SnapshotsController.capture(captureName.text, captureNote.text)
                    captureName.text = ""
                    captureNote.text = ""
                }
            }
        }

        // ---- The list ----
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 120
            radius: Theme.radius
            color: Theme.panel
            border.color: Theme.separator

            ListView {
                id: list
                objectName: "SnapshotsWindow.list"
                anchors.fill: parent
                anchors.margins: 1
                clip: true
                model: SnapshotsController.snapshots
                activeFocusOnTab: true
                keyNavigationEnabled: true
                ScrollBar.vertical: AppScrollBar {}
                Accessible.role: Accessible.List
                Accessible.name: qsTr("Snapshots")
                onCurrentIndexChanged: if (currentIndex >= 0 && currentIndex < count) panel.selectedId = model[currentIndex].id

                delegate: Rectangle {
                    id: row
                    required property var modelData
                    required property int index
                    objectName: "SnapshotsWindow.row." + index
                    width: ListView.view.width
                    height: Theme.rowHeight + 14
                    color: panel.selectedId === row.modelData.id ? Theme.selection
                         : index % 2 ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03) : "transparent"
                    TapHandler {
                        onTapped: {
                            list.currentIndex = row.index
                            panel.selectedId = row.modelData.id
                        }
                        onDoubleTapped: if (panel.writable) SnapshotsController.recall(row.modelData.id)
                    }
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.spacingSmall
                        anchors.rightMargin: Theme.spacingSmall
                        spacing: Theme.spacingSmall
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
                            Text {
                                Layout.fillWidth: true
                                visible: text.length > 0
                                textFormat: Text.PlainText
                                text: row.modelData.note
                                font: Theme.fontSmall
                                color: Theme.textMuted
                                elide: Text.ElideRight
                            }
                        }
                        // -- WP P2QT -- where it is kept too
                        Pill {
                            objectName: "SnapshotsWindow.onHub." + row.index
                            visible: row.modelData.where === "both"
                            text: qsTr("On the hub")
                            tint: Theme.accent
                        }
                        Pill {
                            objectName: "SnapshotsWindow.localCopy." + row.index
                            visible: row.modelData.localCopy === true
                            text: qsTr("Local copy")
                            tint: Theme.warning
                            ToolTip.visible: hovered
                            ToolTip.text: qsTr("The hub holds a different snapshot under this one's id. This console keeps its own copy.")
                        }
                        Text {
                            textFormat: Text.PlainText
                            text: row.modelData.saved
                            font: Theme.fontMono
                            color: Theme.textMuted
                        }
                    }
                    Accessible.role: Accessible.ListItem
                    Accessible.name: [row.modelData.name, row.modelData.note, row.modelData.saved,
                                      row.modelData.where === "both" ? qsTr("On the hub") : "",
                                      row.modelData.localCopy === true ? qsTr("Local copy") : ""].filter(s => s && s.length).join(", ")
                }

                Text {
                    anchors.centerIn: parent
                    width: parent.width - 2 * Theme.spacingLarge
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    visible: list.count === 0
                    textFormat: Text.PlainText
                    text: !panel.hasDevice ? qsTr("No Device Connected")
                          : panel.simple ? qsTr("No snapshots of this device yet. They are captured in Advanced mode.")
                                          : qsTr("No snapshots of this device yet. Capture keeps its settings as they are now.")
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
            }
        }

        // ---- The selected snapshot ----
        RowLayout {
            objectName: "SnapshotsWindow.editRow"
            Layout.fillWidth: true
            visible: !panel.simple
            spacing: Theme.spacingSmall
            enabled: panel.selected !== null
            BorderedField {
                objectName: "SnapshotsWindow.name"
                Layout.fillWidth: true
                Layout.preferredWidth: 200
                label: qsTr("Snapshot name")
                placeholderText: qsTr("Name")
                text: panel.selected ? panel.selected.name : ""
                onCommitted: text => {
                    if (panel.selected && text.trim().length && text !== panel.selected.name)
                        SnapshotsController.rename(panel.selectedId, text)
                }
            }
            BorderedField {
                objectName: "SnapshotsWindow.note"
                Layout.fillWidth: true
                Layout.preferredWidth: 300
                label: qsTr("Snapshot note")
                placeholderText: qsTr("Note")
                text: panel.selected ? panel.selected.note : ""
                onCommitted: text => {
                    if (panel.selected && text !== panel.selected.note) SnapshotsController.setNote(panel.selectedId, text)
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "SnapshotsWindow.recall"
                text: qsTr("Recall")
                enabled: panel.selected !== null && panel.writable && !SnapshotsController.guardWaiting
                toolTip: qsTr("Put the device back to this snapshot")
                onClicked: SnapshotsController.recall(panel.selectedId)
            }
            AppButton {
                objectName: "SnapshotsWindow.compare"
                text: qsTr("Compare with Current")
                enabled: panel.selected !== null && panel.writable && !SnapshotsController.abActive
                         && !SnapshotsController.guardWaiting
                toolTip: qsTr("A is the device as it is now, B this snapshot")
                onClicked: SnapshotsController.abStart("", panel.selectedId)
            }
            AppButton {
                objectName: "SnapshotsWindow.export"
                text: qsTr("Export…")
                iconName: "file-down"
                visible: !panel.simple
                enabled: panel.selected !== null
                onClicked: SnapshotsController.exportSnapshot(panel.selectedId)
            }
            AppButton {
                objectName: "SnapshotsWindow.import"
                text: qsTr("Import…")
                iconName: "file-up"
                visible: !panel.simple
                enabled: panel.hasDevice
                toolTip: qsTr("Keep a configuration file as a snapshot of this device")
                onClicked: SnapshotsController.importSnapshot()
            }
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "SnapshotsWindow.delete"
                text: qsTr("Delete…")
                iconName: "trash"
                visible: !panel.simple
                enabled: panel.selected !== null
                onClicked: {
                    deleteQuestion.snapshotId = panel.selectedId
                    //: %1 is the snapshot's name
                    deleteQuestion.text = qsTr("Delete the snapshot “%1”? It cannot be brought back.").arg(panel.selected.name)
                    deleteQuestion.open()
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: !panel.simple   // -- WP P1DCFIX --
            spacing: Theme.spacingSmall
            AppCheckBox {
                objectName: "SnapshotsWindow.rollback"
                text: SnapshotsController.guardDefault
                      ? qsTr("Recall with rollback: always over DSPi Link (Keep or Revert)")
                      : qsTr("Recall with rollback (Keep or Revert)")
                checked: SnapshotsController.guardDefault || SnapshotsController.rollback
                enabled: !SnapshotsController.guardDefault
                onToggled: SnapshotsController.rollback = checked
                Accessible.name: text
            }
            Item { Layout.fillWidth: true }
            // -- WP P1DCFIX -- how long a guarded recall or import waits for Keep
            Text {
                text: qsTr("Rollback after")
                font: Theme.fontBody
                color: Theme.text
            }
            AppSpinBox {
                objectName: "SnapshotsWindow.timeout"
                label: qsTr("Rollback after, seconds")
                realFrom: 5
                realTo: 300
                realStep: 5
                unit: qsTr("s")
                realValue: SnapshotsController.guardTimeoutS
                onCommitted: value => SnapshotsController.guardTimeoutS = Math.round(value)
                Accessible.name: label
            }
        }

        // ---- -- WP P2QT -- On the hub ----
        SectionHeading {
            Layout.fillWidth: true
            Layout.topMargin: Theme.spacingSmall
            visible: SnapshotsController.hubAvailable || SnapshotsController.hubProblem.length > 0
            text: qsTr("On the Hub")
            rule: true
        }
        Text {
            objectName: "SnapshotsWindow.hubProblem"
            Layout.fillWidth: true
            visible: text.length > 0
            textFormat: Text.PlainText
            text: SnapshotsController.hubProblem
            font: Theme.fontSmall
            color: Theme.warning
            wrapMode: Text.Wrap
        }
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 120
            visible: SnapshotsController.hubAvailable
            radius: Theme.radius
            color: Theme.panel
            border.color: Theme.separator
            ListView {
                id: hubList
                objectName: "SnapshotsWindow.hubList"
                anchors.fill: parent
                anchors.margins: 1
                clip: true
                model: SnapshotsController.hubSnapshots
                activeFocusOnTab: true
                keyNavigationEnabled: true
                ScrollBar.vertical: AppScrollBar {}
                Accessible.role: Accessible.List
                Accessible.name: qsTr("Snapshots on the hub")
                onCurrentIndexChanged: if (currentIndex >= 0 && currentIndex < count) panel.hubSelectedId = model[currentIndex].id
                delegate: Rectangle {
                    id: hubRow
                    required property var modelData
                    required property int index
                    objectName: "SnapshotsWindow.hubRow." + index
                    width: ListView.view.width
                    height: Theme.rowHeight + 6
                    color: panel.hubSelectedId === hubRow.modelData.id ? Theme.selection
                         : index % 2 ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03) : "transparent"
                    TapHandler {
                        onTapped: {
                            hubList.currentIndex = hubRow.index
                            panel.hubSelectedId = hubRow.modelData.id
                        }
                        onDoubleTapped: if (panel.writable) SnapshotsController.recallFromHub(hubRow.modelData.id)
                    }
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.spacingSmall
                        anchors.rightMargin: Theme.spacingSmall
                        spacing: Theme.spacingSmall
                        Text {
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: hubRow.modelData.note.length
                                  //: A snapshot's name and its note
                                  ? qsTr("%1 · %2").arg(hubRow.modelData.name).arg(hubRow.modelData.note)
                                  : hubRow.modelData.name
                            font: Theme.fontBody
                            color: Theme.text
                            elide: Text.ElideRight
                        }
                        Pill {
                            visible: hubRow.modelData.where === "both"
                            text: qsTr("Here too")
                            tint: Theme.accent
                        }
                        Text {
                            textFormat: Text.PlainText
                            text: hubRow.modelData.saved
                            font: Theme.fontMono
                            color: Theme.textMuted
                        }
                    }
                    Accessible.role: Accessible.ListItem
                    Accessible.name: [hubRow.modelData.name, hubRow.modelData.note, hubRow.modelData.saved,
                                      hubRow.modelData.where === "both" ? qsTr("Here too") : ""].filter(s => s && s.length).join(", ")
                }
                Text {
                    anchors.centerIn: parent
                    width: parent.width - 2 * Theme.spacingLarge
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    visible: hubList.count === 0
                    text: qsTr("The hub keeps no snapshots of this device yet.")
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: SnapshotsController.hubAvailable
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "SnapshotsWindow.saveToHub"
                text: qsTr("Save Selected to the Hub")
                iconName: "network"
                visible: !panel.simple
                enabled: panel.selected !== null && panel.selected.where !== "both" && !SnapshotsController.busy
                         && !DeviceModel.readOnly
                toolTip: qsTr("Keep the snapshot selected above on the hub too, for every console")
                onClicked: SnapshotsController.saveToHub(panel.selectedId)
            }
            AppButton {
                objectName: "SnapshotsWindow.hubRecall"
                text: qsTr("Recall")
                Accessible.name: qsTr("Recall from the hub")
                enabled: panel.hubSelected !== null && panel.writable && !SnapshotsController.guardWaiting
                toolTip: qsTr("Put the device back to the hub's snapshot")
                onClicked: SnapshotsController.recallFromHub(panel.hubSelectedId)
            }
            AppButton {
                objectName: "SnapshotsWindow.hubCopy"
                text: qsTr("Copy Here")
                iconName: "download"
                visible: !panel.simple
                enabled: panel.hubSelected !== null && panel.hubSelected.where === "hub" && !SnapshotsController.busy
                toolTip: qsTr("Keep a copy of the hub's snapshot on this console")
                onClicked: SnapshotsController.copyHere(panel.hubSelectedId)
            }
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "SnapshotsWindow.hubDelete"
                text: qsTr("Delete…")
                Accessible.name: qsTr("Delete from the hub")
                iconName: "trash"
                visible: !panel.simple
                enabled: panel.hubSelected !== null && !DeviceModel.readOnly
                onClicked: {
                    hubDeleteQuestion.snapshotId = panel.hubSelectedId
                    //: %1 is the snapshot's name
                    hubDeleteQuestion.text = qsTr("Delete the snapshot “%1” from the hub? Every console loses it; a copy kept on a console stays there.").arg(panel.hubSelected.name)
                    hubDeleteQuestion.open()
                }
            }
        }

        // ---- A/B ----
        SectionHeading {
            Layout.fillWidth: true
            Layout.topMargin: Theme.spacingSmall
            text: qsTr("A/B Compare")
            rule: true
        }
        RowLayout {
            Layout.fillWidth: true
            visible: !SnapshotsController.abActive
            spacing: Theme.spacingSmall
            Text {
                text: qsTr("A")
                font: Theme.fontBody
                color: Theme.text
            }
            AppComboBox {
                id: aPicker
                objectName: "SnapshotsWindow.abA"
                Layout.fillWidth: true
                label: qsTr("A side")
                model: panel.aChoices.map(r => r.name)
                currentIndex: 0
            }
            Text {
                text: qsTr("B")
                font: Theme.fontBody
                color: Theme.text
            }
            AppComboBox {
                id: bPicker
                objectName: "SnapshotsWindow.abB"
                Layout.fillWidth: true
                label: qsTr("B side")
                model: panel.bChoices.map(r => r.name)
                currentIndex: panel.bChoices.length ? 0 : -1
            }
            AppButton {
                objectName: "SnapshotsWindow.abStart"
                text: qsTr("Start")
                enabled: panel.writable && bPicker.currentIndex >= 0 && !SnapshotsController.guardWaiting
                         && (aPicker.currentIndex <= 0
                             || panel.aChoices[aPicker.currentIndex].id !== panel.bChoices[bPicker.currentIndex].id)
                onClicked: SnapshotsController.abStart(panel.aChoices[Math.max(0, aPicker.currentIndex)].id,
                                                       panel.bChoices[bPicker.currentIndex].id)
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: SnapshotsController.abActive
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "SnapshotsWindow.abSideA"
                text: panel.sideName("A", SnapshotsController.abAName)
                primary: SnapshotsController.abSide === "a"
                enabled: SnapshotsController.canToggle
                onClicked: if (SnapshotsController.abSide !== "a") SnapshotsController.abToggle()
                Accessible.description: SnapshotsController.abSide === "a" ? qsTr("On the device now") : ""
            }
            AppButton {
                objectName: "SnapshotsWindow.abSideB"
                text: panel.sideName("B", SnapshotsController.abBName)
                primary: SnapshotsController.abSide === "b"
                enabled: SnapshotsController.canToggle
                onClicked: if (SnapshotsController.abSide !== "b") SnapshotsController.abToggle()
                Accessible.description: SnapshotsController.abSide === "b" ? qsTr("On the device now") : ""
            }
            AppButton {
                objectName: "SnapshotsWindow.abToggle"
                text: qsTr("Toggle A/B")
                toolTip: SnapshotsController.guardWaiting ? qsTr("Finish the safe apply first: Keep or Revert.")
                                                          : qsTr("Recall the other side (B)")
                enabled: SnapshotsController.canToggle
                onClicked: SnapshotsController.abToggle()
            }
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "SnapshotsWindow.abKeepA"
                text: qsTr("Keep A")
                enabled: !SnapshotsController.busy && !SnapshotsController.guardWaiting
                onClicked: SnapshotsController.abKeep("a")
            }
            AppButton {
                objectName: "SnapshotsWindow.abKeepB"
                text: qsTr("Keep B")
                enabled: !SnapshotsController.busy && !SnapshotsController.guardWaiting
                onClicked: SnapshotsController.abKeep("b")
            }
        }
        Text {
            Layout.fillWidth: true
            visible: SnapshotsController.abActive && SnapshotsController.guardWaiting
            text: qsTr("Finish the safe apply first: Keep or Revert.")
            font: Theme.fontSmall
            color: Theme.warning
            wrapMode: Text.Wrap
        }
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 150
            visible: SnapshotsController.abActive
            radius: Theme.radius
            color: Theme.panel
            border.color: Theme.separator
            ListView {
                id: diff
                objectName: "SnapshotsWindow.abDiff"
                anchors.fill: parent
                anchors.margins: 1
                clip: true
                model: SnapshotsController.abRows
                ScrollBar.vertical: AppScrollBar {}
                Accessible.role: Accessible.List
                Accessible.name: qsTr("What differs from A to B")
                delegate: RowLayout {
                    id: change
                    required property var modelData
                    width: ListView.view.width
                    height: Theme.rowHeight
                    spacing: Theme.spacingSmall
                    Text {
                        Layout.leftMargin: Theme.spacingSmall
                        Layout.preferredWidth: 180
                        textFormat: Text.PlainText
                        text: change.modelData.section + " · " + change.modelData.item
                        font: Theme.fontSmall
                        color: Theme.text
                        elide: Text.ElideRight
                    }
                    Text {
                        Layout.fillWidth: true
                        //: A value in A, then in B
                        textFormat: Text.PlainText
                        text: qsTr("%1 → %2").arg(change.modelData.before).arg(change.modelData.after)
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        elide: Text.ElideRight
                    }
                    Accessible.role: Accessible.ListItem
                    Accessible.name: change.modelData.section + ", " + change.modelData.item + ": "
                                     + change.modelData.before + ", " + change.modelData.after
                }
                Text {
                    anchors.centerIn: parent
                    visible: diff.count === 0
                    text: qsTr("A and B hold the same settings.")
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
            }
        }
    }
}
