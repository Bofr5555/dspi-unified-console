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
// -- WP DL3a -- Modern (DESIGN-LANGUAGE §3, §8, §9, §15, §16): A/B compare as
// the first section, then the snapshots as ListRows with Capture… (a small
// popover: name and note) and Import… at the right of their label, an
// EmptyState while there are none, the recall options as form rows and the
// hub's snapshots as a section of their own; all in the 760 px column at the
// left. The selected snapshot's name, note and actions are in a sheet at the
// page's right, as the inspector. Classic keeps its window as it was.
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

    // -- WP DL3a -- one body or the other, by layout
    Loader {
        anchors.fill: parent
        sourceComponent: LayoutController.modern ? modernBody : classicBody
    }

    Component {
        id: classicBody
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

    // -- WP DL3a -- Modern
    Component {
        id: modernBody
        Item {
            id: modern
            // The sheet at the right while a snapshot is selected
            readonly property bool inspecting: panel.selected !== null && !panel.simple
            component Section: ColumnLayout {
                property alias title: heading.text
                default property alias content: rest.data
                // Buttons at the right of the section label
                property alias tools: toolRow.data
                Layout.fillWidth: true
                spacing: Theme.spacingSmall
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                    SectionHeading {
                        id: heading
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                    }
                    RowLayout {
                        id: toolRow
                        spacing: Theme.spacingSmall
                    }
                }
                ColumnLayout {
                    id: rest
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                }
            }

            Flickable {
                id: flick
                objectName: "SnapshotsWindow.body"
                anchors.left: parent.left
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                anchors.right: sheet.visible ? sheet.left : parent.right
                contentHeight: column.implicitHeight + 2 * Theme.spacing
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: AppScrollBar {}

                ContentColumn {
                    id: column
                    x: Theme.spacing
                    y: Theme.spacing
                    width: Math.min(maximumWidth, flick.width - 2 * Theme.spacing)

                    // ---- A/B ----
                    Section {
                        title: qsTr("A/B Compare")
                        RowLayout {
                            Layout.fillWidth: true
                            visible: !SnapshotsController.abActive
                            spacing: Theme.spacingSmall
                            Text {
                                text: qsTr("A")
                                font: Theme.fontBody
                                color: Theme.textMuted
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
                                color: Theme.textMuted
                            }
                            AppComboBox {
                                id: bPicker
                                objectName: "SnapshotsWindow.abB"
                                Layout.fillWidth: true
                                label: qsTr("B side")
                                enabled: panel.bChoices.length > 0
                                model: panel.bChoices.map(r => r.name)
                                currentIndex: panel.bChoices.length ? 0 : -1
                                displayText: currentIndex < 0 ? qsTr("No snapshot yet") : currentText
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
                        Text {
                            Layout.fillWidth: true
                            visible: !SnapshotsController.abActive
                            textFormat: Text.PlainText
                            text: qsTr("Recall two states in turn and hear the difference; Keep ends the compare on one of them.")
                            font: Theme.fontSmall
                            color: Theme.textMuted
                            wrapMode: Text.Wrap
                        }
                        RowLayout {
                            Layout.fillWidth: true
                            visible: SnapshotsController.abActive
                            spacing: Theme.spacingSmall
                            // The side on the device: chosen chips, not accent buttons
                            OptionChip {
                                objectName: "SnapshotsWindow.abSideA"
                                text: panel.sideName("A", SnapshotsController.abAName)
                                exclusive: true
                                checked: SnapshotsController.abSide === "a"
                                enabled: SnapshotsController.canToggle
                                onClicked: if (SnapshotsController.abSide !== "a") SnapshotsController.abToggle()
                                Accessible.description: checked ? qsTr("On the device now") : ""
                            }
                            OptionChip {
                                objectName: "SnapshotsWindow.abSideB"
                                text: panel.sideName("B", SnapshotsController.abBName)
                                exclusive: true
                                checked: SnapshotsController.abSide === "b"
                                enabled: SnapshotsController.canToggle
                                onClicked: if (SnapshotsController.abSide !== "b") SnapshotsController.abToggle()
                                Accessible.description: checked ? qsTr("On the device now") : ""
                            }
                            AppButton {
                                objectName: "SnapshotsWindow.abToggle"
                                text: qsTr("Toggle A/B")
                                toolTip: qsTr("Recall the other side")
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
                        NoticeRow {
                            Layout.fillWidth: true
                            visible: SnapshotsController.abActive && SnapshotsController.guardWaiting
                            kind: "warning"
                            text: qsTr("Finish the safe apply first: Keep or Revert.")
                        }
                        ListView {
                            id: diff
                            objectName: "SnapshotsWindow.abDiff"
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.min(contentHeight, 8 * Theme.rowHeight)
                            visible: SnapshotsController.abActive && count > 0
                            clip: true
                            interactive: contentHeight > height
                            boundsBehavior: Flickable.StopAtBounds
                            model: SnapshotsController.abRows
                            ScrollBar.vertical: AppScrollBar {}
                            Accessible.role: Accessible.List
                            Accessible.name: qsTr("What differs from A to B")
                            delegate: ListRow {
                                required property var modelData
                                required property int index
                                width: ListView.view.width
                                rowIndex: index
                                title: modelData.section + " · " + modelData.item
                                //: A value in A, then in B
                                value: qsTr("%1 → %2").arg(modelData.before).arg(modelData.after)
                                Accessible.name: modelData.section + ", " + modelData.item + ": " + modelData.before + ", " + modelData.after
                            }
                        }
                        Text {
                            visible: SnapshotsController.abActive && diff.count === 0
                            text: qsTr("A and B hold the same settings.")
                            font: Theme.fontBody
                            color: Theme.textMuted
                        }
                    }

                    // ---- The snapshots ----
                    Section {
                        title: qsTr("Snapshots")
                        tools: [
                            AppButton {
                                objectName: "SnapshotsWindow.import"
                                visible: !panel.simple
                                text: qsTr("Import…")
                                iconName: "file-up"
                                enabled: panel.hasDevice
                                toolTip: qsTr("Keep a configuration file as a snapshot of this device")
                                onClicked: SnapshotsController.importSnapshot()
                            },
                            AppButton {
                                id: captureButton
                                objectName: "SnapshotsWindow.capture"
                                visible: !panel.simple && list.count > 0
                                text: qsTr("Capture…")
                                iconName: "plus"
                                primary: true
                                enabled: panel.hasDevice && !SnapshotsController.busy
                                toolTip: qsTr("Keep the device's settings as they are now")
                                onClicked: capture.open()
                            }
                        ]
                        ListView {
                            id: list
                            objectName: "SnapshotsWindow.list"
                            Layout.fillWidth: true
                            Layout.preferredHeight: count > 0 ? Math.min(contentHeight, 12 * (Theme.rowHeight + 14)) : 0
                            visible: count > 0
                            clip: true
                            interactive: contentHeight > height
                            boundsBehavior: Flickable.StopAtBounds
                            model: SnapshotsController.snapshots
                            currentIndex: -1
                            activeFocusOnTab: true
                            keyNavigationEnabled: true
                            ScrollBar.vertical: AppScrollBar {}
                            Accessible.role: Accessible.List
                            Accessible.name: qsTr("Snapshots")
                            onCurrentIndexChanged: if (currentIndex >= 0 && currentIndex < count) panel.selectedId = model[currentIndex].id
                            delegate: ListRow {
                                id: row
                                required property var modelData
                                required property int index
                                objectName: "SnapshotsWindow.row." + index
                                width: ListView.view.width
                                rowIndex: index
                                title: modelData.name
                                subtitle: modelData.note
                                value: modelData.saved
                                selected: panel.selectedId === modelData.id
                                onClicked: {
                                    list.currentIndex = index
                                    panel.selectedId = modelData.id
                                }
                                onActivated: if (panel.writable) SnapshotsController.recall(modelData.id)
                                // Recall shows on hover; enabled all along, as on Presets
                                actionsAlways: true
                                // -- WP P2QT -- where it is kept too
                                // A Row, not a list: ListRow's leading layout widens when all it holds is hidden
                                leading: Row {
                                    spacing: 6
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
                                }
                                Accessible.name: [modelData.name, modelData.note, modelData.saved,
                                                  modelData.where === "both" ? qsTr("On the hub") : "",
                                                  modelData.localCopy === true ? qsTr("Local copy") : ""].filter(s => s && s.length).join(", ")
                                AppButton {
                                    objectName: "SnapshotsWindow.recall." + row.index
                                    opacity: row.hovered || row.selected || row.current || activeFocus ? 1 : 0
                                    text: qsTr("Recall")
                                    enabled: panel.writable && !SnapshotsController.guardWaiting
                                    toolTip: qsTr("Put the device back to this snapshot")
                                    onClicked: SnapshotsController.recall(row.modelData.id)
                                }
                            }
                        }
                        EmptyState {
                            objectName: "SnapshotsWindow.empty"
                            Layout.fillWidth: true
                            Layout.preferredHeight: implicitHeight + 2 * Theme.spacingLarge
                            visible: list.count === 0
                            iconName: panel.hasDevice ? "save" : "unplug"
                            title: !panel.hasDevice ? qsTr("No Device Connected") : qsTr("No snapshots yet")
                            text: !panel.hasDevice ? qsTr("Snapshots are kept on this computer for each device. Open one to see its snapshots.")
                                  : panel.simple ? qsTr("Snapshots are captured in Advanced mode.")
                                                 : qsTr("A snapshot keeps every setting of the device as it is now, to recall or compare later.")
                            actionText: panel.hasDevice && !panel.simple ? qsTr("Capture…") : ""
                            actionPrimary: true
                            actionEnabled: !SnapshotsController.busy
                            onAction: capture.open()
                        }
                    }

                    // ---- How a recall goes ----
                    Section {
                        title: qsTr("Recall")
                        visible: !panel.simple   // -- WP P1DCFIX --
                        FormRow {
                            Layout.fillWidth: true
                            title: qsTr("Recall with rollback")
                            caption: SnapshotsController.guardDefault ? qsTr("Always over DSPi Link: Keep or Revert after each recall")
                                                                      : qsTr("Keep or Revert after each recall")
                            AppSwitch {
                                objectName: "SnapshotsWindow.rollback"
                                checked: SnapshotsController.guardDefault || SnapshotsController.rollback
                                enabled: !SnapshotsController.guardDefault
                                onToggled: SnapshotsController.rollback = checked
                                Accessible.name: qsTr("Recall with rollback")
                            }
                        }
                        // -- WP P1DCFIX -- how long a guarded recall or import waits for Keep
                        FormRow {
                            Layout.fillWidth: true
                            title: qsTr("Rollback after")
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
                    }

                    // ---- -- WP P2QT -- On the hub ----
                    Section {
                        title: qsTr("On the Hub")
                        visible: SnapshotsController.hubAvailable || SnapshotsController.hubProblem.length > 0
                        NoticeRow {
                            objectName: "SnapshotsWindow.hubProblem"
                            Layout.fillWidth: true
                            visible: SnapshotsController.hubProblem.length > 0
                            kind: "warning"
                            text: SnapshotsController.hubProblem
                        }
                        ListView {
                            id: hubList
                            objectName: "SnapshotsWindow.hubList"
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.min(contentHeight, 8 * (Theme.rowHeight + 14))
                            visible: SnapshotsController.hubAvailable && count > 0
                            clip: true
                            interactive: contentHeight > height
                            boundsBehavior: Flickable.StopAtBounds
                            model: SnapshotsController.hubSnapshots
                            currentIndex: -1
                            activeFocusOnTab: true
                            keyNavigationEnabled: true
                            ScrollBar.vertical: AppScrollBar {}
                            Accessible.role: Accessible.List
                            Accessible.name: qsTr("Snapshots on the hub")
                            onCurrentIndexChanged: if (currentIndex >= 0 && currentIndex < count) panel.hubSelectedId = model[currentIndex].id
                            delegate: ListRow {
                                id: hubRow
                                required property var modelData
                                required property int index
                                objectName: "SnapshotsWindow.hubRow." + index
                                width: ListView.view.width
                                rowIndex: index
                                title: modelData.name
                                subtitle: modelData.note
                                value: modelData.saved
                                selected: panel.hubSelectedId === modelData.id
                                onClicked: {
                                    hubList.currentIndex = index
                                    panel.hubSelectedId = modelData.id
                                }
                                onActivated: if (panel.writable) SnapshotsController.recallFromHub(modelData.id)
                                actionsAlways: true
                                readonly property real actionOpacity: hovered || selected || current ? 1 : 0
                                leading: Row {
                                    Pill {
                                        visible: hubRow.modelData.where === "both"
                                        text: qsTr("Here too")
                                        tint: Theme.accent
                                    }
                                }
                                Accessible.name: [modelData.name, modelData.note, modelData.saved,
                                                  modelData.where === "both" ? qsTr("Here too") : ""].filter(s => s && s.length).join(", ")
                                AppButton {
                                    objectName: "SnapshotsWindow.hubRecall." + hubRow.index
                                    opacity: hubRow.actionOpacity || activeFocus ? 1 : 0
                                    text: qsTr("Recall")
                                    Accessible.name: qsTr("Recall from the hub")
                                    enabled: panel.writable && !SnapshotsController.guardWaiting
                                    toolTip: qsTr("Put the device back to the hub's snapshot")
                                    onClicked: SnapshotsController.recallFromHub(hubRow.modelData.id)
                                }
                                IconButton {
                                    objectName: "SnapshotsWindow.hubCopy." + hubRow.index
                                    opacity: hubRow.actionOpacity || activeFocus ? 1 : 0
                                    visible: !panel.simple && hubRow.modelData.where === "hub"
                                    enabled: !SnapshotsController.busy
                                    iconName: "download"
                                    toolTip: qsTr("Copy Here")
                                    onClicked: SnapshotsController.copyHere(hubRow.modelData.id)
                                }
                                IconButton {
                                    objectName: "SnapshotsWindow.hubDelete." + hubRow.index
                                    opacity: hubRow.actionOpacity || activeFocus ? 1 : 0
                                    visible: !panel.simple
                                    enabled: !DeviceModel.readOnly
                                    iconName: "trash"
                                    iconColor: Theme.dangerText
                                    toolTip: qsTr("Delete from the hub…")
                                    onClicked: {
                                        hubDeleteQuestion.snapshotId = hubRow.modelData.id
                                        //: %1 is the snapshot's name
                                        hubDeleteQuestion.text = qsTr("Delete the snapshot “%1” from the hub? Every console loses it; a copy kept on a console stays there.").arg(hubRow.modelData.name)
                                        hubDeleteQuestion.open()
                                    }
                                }
                            }
                        }
                        Text {
                            visible: SnapshotsController.hubAvailable && hubList.count === 0
                            text: qsTr("The hub keeps no snapshots of this device yet.")
                            font: Theme.fontBody
                            color: Theme.textMuted
                        }
                    }
                }
            }

            // ---- The selected snapshot, as the inspector ----
            Rectangle {
                id: sheet
                objectName: "SnapshotsWindow.inspector"
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                anchors.right: parent.right
                width: InspectorState.width
                visible: modern.inspecting
                color: Theme.sidebar
                Rectangle {
                    width: 1
                    height: parent.height
                    color: Theme.separator
                }
                IconButton {
                    objectName: "SnapshotsWindow.inspector.close"
                    z: 1
                    anchors { top: parent.top; right: parent.right; margins: 8 }
                    iconName: "x"
                    iconSize: 12
                    iconColor: Theme.textMuted
                    toolTip: qsTr("Done")
                    onClicked: {
                        list.currentIndex = -1
                        panel.selectedId = ""
                    }
                }
                ColumnLayout {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.margins: Theme.spacing
                    anchors.topMargin: Theme.spacing + 4
                    spacing: Theme.spacingSmall

                    SectionHeading {
                        Layout.fillWidth: true
                        Layout.rightMargin: 24
                        text: qsTr("Snapshot")
                    }
                    Text {
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        //: When a snapshot was taken; %1 a date and time
                        text: panel.selected ? qsTr("Saved %1").arg(panel.selected.saved) : ""
                        font: Theme.fontSmall
                        color: Theme.textMuted
                    }
                    BorderedField {
                        objectName: "SnapshotsWindow.name"
                        Layout.fillWidth: true
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
                        label: qsTr("Snapshot note")
                        placeholderText: qsTr("Note")
                        text: panel.selected ? panel.selected.note : ""
                        onCommitted: text => {
                            if (panel.selected && text !== panel.selected.note) SnapshotsController.setNote(panel.selectedId, text)
                        }
                    }
                    Flow {
                        Layout.fillWidth: true
                        Layout.topMargin: Theme.spacingSmall
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
                            enabled: panel.selected !== null
                            onClicked: SnapshotsController.exportSnapshot(panel.selectedId)
                        }
                        // -- WP P2QT --
                        AppButton {
                            objectName: "SnapshotsWindow.saveToHub"
                            visible: SnapshotsController.hubAvailable
                            text: qsTr("Save to the Hub")
                            iconName: "network"
                            enabled: panel.selected !== null && panel.selected.where !== "both" && !SnapshotsController.busy
                                     && !DeviceModel.readOnly
                            toolTip: qsTr("Keep this snapshot on the hub too, for every console")
                            onClicked: SnapshotsController.saveToHub(panel.selectedId)
                        }
                    }
                    AppButton {
                        objectName: "SnapshotsWindow.delete"
                        Layout.topMargin: Theme.spacingSmall
                        text: qsTr("Delete…")
                        iconName: "trash"
                        enabled: panel.selected !== null
                        contentItem: Row {
                            spacing: 6
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                name: "trash"
                                size: 14
                                color: Theme.dangerText
                            }
                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                textFormat: Text.PlainText
                                text: qsTr("Delete…")
                                font: Theme.fontBody
                                color: Theme.dangerText
                            }
                        }
                        onClicked: {
                            deleteQuestion.snapshotId = panel.selectedId
                            //: %1 is the snapshot's name
                            deleteQuestion.text = qsTr("Delete the snapshot “%1”? It cannot be brought back.").arg(panel.selected.name)
                            deleteQuestion.open()
                        }
                    }
                }
            }

            // ---- Capture: name and note, in a popover under its button ----
            PopoverPanel {
                id: capture
                objectName: "SnapshotsWindow.captureRow"
                title: qsTr("Capture a Snapshot")
                parent: captureButton.visible ? captureButton : modern
                x: captureButton.visible ? captureButton.width - width : (modern.width - width) / 2
                y: captureButton.visible ? captureButton.height + 4 : modern.height / 3
                width: 320
                onOpened: captureName.forceActiveFocus()
                // Not left open over another page
                Connections {
                    target: panel
                    function onShownChanged() { if (!panel.shown) capture.close() }
                }
                function take() {
                    SnapshotsController.capture(captureName.text, captureNote.text)
                    captureName.text = ""
                    captureNote.text = ""
                    close()
                }
                contentItem: ColumnLayout {
                    spacing: Theme.spacingSmall
                    BorderedField {
                        id: captureName
                        objectName: "SnapshotsWindow.captureName"
                        Layout.fillWidth: true
                        label: qsTr("New snapshot's name")
                        placeholderText: qsTr("Name")
                        onAccepted: capture.take()
                        // The field would restore its text; here Escape leaves the popover
                        Keys.onEscapePressed: capture.close()
                    }
                    BorderedField {
                        id: captureNote
                        objectName: "SnapshotsWindow.captureNote"
                        Layout.fillWidth: true
                        label: qsTr("New snapshot's note")
                        placeholderText: qsTr("Note (optional)")
                        onAccepted: capture.take()
                        Keys.onEscapePressed: capture.close()
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.topMargin: 4
                        spacing: Theme.spacingSmall
                        Item { Layout.fillWidth: true }
                        AppButton {
                            objectName: "SnapshotsWindow.captureCancel"
                            text: qsTr("Cancel")
                            onClicked: capture.close()
                        }
                        AppButton {
                            objectName: "SnapshotsWindow.captureTake"
                            text: qsTr("Capture")
                            primary: true
                            enabled: panel.hasDevice && !SnapshotsController.busy
                            onClicked: capture.take()
                        }
                    }
                }
            }
        }
    }
}
