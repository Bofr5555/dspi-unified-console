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
ToolWindow {
    id: window
    objectName: "SnapshotsWindow"
    windowId: "snapshots"
    defaultWidth: 680
    defaultHeight: 620
    minimumWidth: 520
    minimumHeight: 460
    title: qsTr("Snapshots")

    readonly property bool hasDevice: DeviceModel.connected
    readonly property bool writable: hasDevice && !DeviceModel.readOnly
    readonly property bool simple: ModeController.mode === "simple"   // -- WP P1DCFIX --
    property string selectedId: ""
    readonly property var selected: {
        const rows = SnapshotsController.snapshots
        for (let i = 0; i < rows.length; ++i) if (rows[i].id === selectedId) return rows[i]
        return null
    }

    onVisibleChanged: if (visible) SnapshotsController.reload()

    // A side's button: "A: Bench" for a snapshot, "A" alone for the state the
    // compare captured
    function sideName(side, name) {
        //: %1 is A or B, %2 the snapshot on that side
        return name.length && name !== side ? qsTr("%1: %2").arg(side).arg(name) : side
    }

    // A/B's choices: A may be the device as it is now
    readonly property var aChoices: [{ id: "", name: qsTr("Current settings") }].concat(SnapshotsController.snapshots)
    readonly property var bChoices: SnapshotsController.snapshots

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
            visible: !window.simple
            spacing: Theme.spacingSmall
            BorderedField {
                id: captureName
                objectName: "SnapshotsWindow.captureName"
                Layout.fillWidth: true
                Layout.preferredWidth: 200
                label: qsTr("New snapshot's name")
                placeholderText: qsTr("Name")
                enabled: window.hasDevice
                onAccepted: captureButton.clicked()
            }
            BorderedField {
                id: captureNote
                objectName: "SnapshotsWindow.captureNote"
                Layout.fillWidth: true
                Layout.preferredWidth: 300
                label: qsTr("New snapshot's note")
                placeholderText: qsTr("Note (optional)")
                enabled: window.hasDevice
                onAccepted: captureButton.clicked()
            }
            AppButton {
                id: captureButton
                objectName: "SnapshotsWindow.capture"
                text: qsTr("Capture")
                iconName: "plus"
                primary: true
                enabled: window.hasDevice && !SnapshotsController.busy
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
                onCurrentIndexChanged: if (currentIndex >= 0 && currentIndex < count) window.selectedId = model[currentIndex].id

                delegate: Rectangle {
                    id: row
                    required property var modelData
                    required property int index
                    objectName: "SnapshotsWindow.row." + index
                    width: ListView.view.width
                    height: Theme.rowHeight + 14
                    color: window.selectedId === row.modelData.id ? Theme.selection
                         : index % 2 ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03) : "transparent"
                    TapHandler {
                        onTapped: {
                            list.currentIndex = row.index
                            window.selectedId = row.modelData.id
                        }
                        onDoubleTapped: if (window.writable) SnapshotsController.recall(row.modelData.id)
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
                                text: row.modelData.name
                                font: Theme.fontBody
                                color: Theme.text
                                elide: Text.ElideRight
                            }
                            Text {
                                Layout.fillWidth: true
                                visible: text.length > 0
                                text: row.modelData.note
                                font: Theme.fontSmall
                                color: Theme.textMuted
                                elide: Text.ElideRight
                            }
                        }
                        Text {
                            text: row.modelData.saved
                            font: Theme.fontMono
                            color: Theme.textMuted
                        }
                    }
                    Accessible.role: Accessible.ListItem
                    Accessible.name: [row.modelData.name, row.modelData.note, row.modelData.saved].filter(s => s && s.length).join(", ")
                }

                Text {
                    anchors.centerIn: parent
                    width: parent.width - 2 * Theme.spacingLarge
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    visible: list.count === 0
                    text: !window.hasDevice ? qsTr("No Device Connected")
                          : window.simple ? qsTr("No snapshots of this device yet. They are captured in Advanced mode.")
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
            visible: !window.simple
            spacing: Theme.spacingSmall
            enabled: window.selected !== null
            BorderedField {
                objectName: "SnapshotsWindow.name"
                Layout.fillWidth: true
                Layout.preferredWidth: 200
                label: qsTr("Snapshot name")
                placeholderText: qsTr("Name")
                text: window.selected ? window.selected.name : ""
                onCommitted: text => {
                    if (window.selected && text.trim().length && text !== window.selected.name)
                        SnapshotsController.rename(window.selectedId, text)
                }
            }
            BorderedField {
                objectName: "SnapshotsWindow.note"
                Layout.fillWidth: true
                Layout.preferredWidth: 300
                label: qsTr("Snapshot note")
                placeholderText: qsTr("Note")
                text: window.selected ? window.selected.note : ""
                onCommitted: text => {
                    if (window.selected && text !== window.selected.note) SnapshotsController.setNote(window.selectedId, text)
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "SnapshotsWindow.recall"
                text: qsTr("Recall")
                enabled: window.selected !== null && window.writable && !SnapshotsController.guardWaiting
                toolTip: qsTr("Put the device back to this snapshot")
                onClicked: SnapshotsController.recall(window.selectedId)
            }
            AppButton {
                objectName: "SnapshotsWindow.compare"
                text: qsTr("Compare with Current")
                enabled: window.selected !== null && window.writable && !SnapshotsController.abActive
                         && !SnapshotsController.guardWaiting
                toolTip: qsTr("A is the device as it is now, B this snapshot")
                onClicked: SnapshotsController.abStart("", window.selectedId)
            }
            AppButton {
                objectName: "SnapshotsWindow.export"
                text: qsTr("Export…")
                iconName: "file-down"
                visible: !window.simple
                enabled: window.selected !== null
                onClicked: SnapshotsController.exportSnapshot(window.selectedId)
            }
            AppButton {
                objectName: "SnapshotsWindow.import"
                text: qsTr("Import…")
                iconName: "file-up"
                visible: !window.simple
                enabled: window.hasDevice
                toolTip: qsTr("Keep a configuration file as a snapshot of this device")
                onClicked: SnapshotsController.importSnapshot()
            }
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "SnapshotsWindow.delete"
                text: qsTr("Delete…")
                iconName: "trash"
                visible: !window.simple
                enabled: window.selected !== null
                onClicked: {
                    deleteQuestion.snapshotId = window.selectedId
                    //: %1 is the snapshot's name
                    deleteQuestion.text = qsTr("Delete the snapshot “%1”? It cannot be brought back.").arg(window.selected.name)
                    deleteQuestion.open()
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: !window.simple   // -- WP P1DCFIX --
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
                model: window.aChoices.map(r => r.name)
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
                model: window.bChoices.map(r => r.name)
                currentIndex: window.bChoices.length ? 0 : -1
            }
            AppButton {
                objectName: "SnapshotsWindow.abStart"
                text: qsTr("Start")
                enabled: window.writable && bPicker.currentIndex >= 0 && !SnapshotsController.guardWaiting
                         && (aPicker.currentIndex <= 0
                             || window.aChoices[aPicker.currentIndex].id !== window.bChoices[bPicker.currentIndex].id)
                onClicked: SnapshotsController.abStart(window.aChoices[Math.max(0, aPicker.currentIndex)].id,
                                                       window.bChoices[bPicker.currentIndex].id)
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: SnapshotsController.abActive
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "SnapshotsWindow.abSideA"
                text: window.sideName("A", SnapshotsController.abAName)
                primary: SnapshotsController.abSide === "a"
                enabled: SnapshotsController.canToggle
                onClicked: if (SnapshotsController.abSide !== "a") SnapshotsController.abToggle()
                Accessible.description: SnapshotsController.abSide === "a" ? qsTr("On the device now") : ""
            }
            AppButton {
                objectName: "SnapshotsWindow.abSideB"
                text: window.sideName("B", SnapshotsController.abBName)
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
                        text: change.modelData.section + " · " + change.modelData.item
                        font: Theme.fontSmall
                        color: Theme.text
                        elide: Text.ElideRight
                    }
                    Text {
                        Layout.fillWidth: true
                        //: A value in A, then in B
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
