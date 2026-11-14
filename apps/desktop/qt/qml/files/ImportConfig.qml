import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// Import Device Configuration's options (FileMenuActions.askImportOptions):
// audio processing is the point of the file and comes in by default; volumes
// and the physical wiring are opt-in, since neither need belong to this machine.
// -- WP P1QT1 -- A box per block of the core's import (P1DF), each with the
// number of values it would change, and the list of those changes for the
// ticked blocks (dspi_config_preview) before anything is written. "Apply with
// rollback" captures the device first and waits for Keep (P1SA); over DSPi
// Link to a hub with the guard it is always on.
Dialog {
    id: options
    objectName: "ImportConfig"
    property string prompt
    property var preview: ({})
    property bool guardDefault: false

    // The blocks, in the core's order, with the old dialog's defaults: the
    // audio processing ticked, volumes, hardware I/O and limiters not
    readonly property var blockList: [
        { key: "eq", bit: 4, label: qsTr("EQ"), on: true },
        { key: "crossover", bit: 8, label: qsTr("Crossover"), on: true },
        { key: "routing", bit: 16, label: qsTr("Routing and matrix"), on: true },
        { key: "outputs", bit: 32, label: qsTr("Outputs (gains, delays, mutes)"), on: true },
        { key: "limiters", bit: 64, label: qsTr("Output limiters"), on: false },
        { key: "names", bit: 256, label: qsTr("Names"), on: true },
        { key: "effects", bit: 128, label: qsTr("DSP features (loudness, crossfeed, bass, leveller…)"), on: true },
        { key: "volumes", bit: 1, label: qsTr("Volume levels (master and listening volume)"), on: false },
        { key: "hardware", bit: 2, label: qsTr("Hardware I/O (GPIO pins, clocks, ADAT, inputs)"), on: false }
    ]
    property var ticked: ({})
    readonly property int blocks: {
        let b = 0
        for (const blk of blockList) if (ticked[blk.key]) b |= blk.bit
        return b
    }
    readonly property var shownRows: (preview.rows || []).filter(r => ticked[r.block])

    function ask(prompt, preview, guardDefault) {
        options.prompt = prompt
        options.preview = preview || {}
        options.guardDefault = guardDefault
        const t = {}
        for (const blk of blockList) t[blk.key] = blk.on
        ticked = t
        rollback.checked = guardDefault
        open()
    }
    function setTicked(key, on) {
        const t = Object.assign({}, ticked)
        t[key] = on
        ticked = t
    }

    property bool importing: false
    onAboutToShow: importing = false
    onOpened: importButton.forceActiveFocus()
    onClosed: {
        if (importing) FilesController.applyConfiguration(blocks, rollback.checked)
        else FilesController.cancelImport()
    }

    title: qsTr("Import Device Configuration")
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 320
    width: Math.min(560, hostWidth - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: Math.max(Theme.spacingLarge, (hostHeight - implicitHeight) / 4)
    padding: Theme.spacingLarge
    topPadding: Theme.spacingSmall

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.color: Theme.border
    }

    header: Item {
        implicitHeight: titleText.height
        Text {
            id: titleText
            width: parent.width
            leftPadding: Theme.spacingLarge
            rightPadding: Theme.spacingLarge
            topPadding: Theme.spacingLarge
            text: options.title
            font: Theme.fontTitle
            color: Theme.text
            wrapMode: Text.Wrap
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
    }

    contentItem: ColumnLayout {
        spacing: Theme.spacingSmall
        Accessible.role: Accessible.Dialog
        Accessible.name: options.title
        Accessible.description: options.prompt
        Text {
            id: promptText
            objectName: "ImportConfig.prompt"
            Layout.fillWidth: true
            text: options.prompt
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        Text {
            Layout.fillWidth: true
            text: qsTr("Bring in:")
            font: Theme.fontBody
            color: Theme.text
        }
        GridLayout {
            Layout.fillWidth: true
            columns: 2
            columnSpacing: Theme.spacing
            rowSpacing: 2
            Repeater {
                model: options.blockList
                AppCheckBox {
                    required property var modelData
                    readonly property int changes: options.preview.blocks ? (options.preview.blocks[modelData.key] || 0) : 0
                    objectName: "ImportConfig." + modelData.key
                    Layout.fillWidth: true
                    Layout.columnSpan: modelData.key === "volumes" || modelData.key === "hardware" || modelData.key === "effects" ? 2 : 1
                    //: %1 a block of settings, %2 how many of its values would change
                    text: changes > 0 ? qsTr("%1 (%2)").arg(modelData.label).arg(changes) : modelData.label
                    checked: options.ticked[modelData.key] === true
                    onToggled: options.setTicked(modelData.key, checked)
                    Accessible.name: text
                }
            }
        }
        Text {
            objectName: "ImportConfig.summary"
            Layout.fillWidth: true
            Layout.topMargin: Theme.spacingSmall
            text: options.shownRows.length === 0
                  ? qsTr("Nothing on the device would change.")
                  : options.shownRows.length === 1 ? qsTr("1 value on the device would change:")
                  //: %1 the number of values the import would change
                  : qsTr("%1 values on the device would change:").arg(options.shownRows.length)
            font: Theme.fontBody
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(200, Math.max(1, rows.contentHeight) + 2)
            visible: options.shownRows.length > 0
            radius: Theme.radiusSmall
            color: Theme.window
            border.color: Theme.separator
            ListView {
                id: rows
                objectName: "ImportConfig.rows"
                anchors.fill: parent
                anchors.margins: 1
                clip: true
                model: options.shownRows
                ScrollBar.vertical: AppScrollBar {}
                Accessible.role: Accessible.List
                Accessible.name: qsTr("What the import changes")
                delegate: RowLayout {
                    id: change
                    required property var modelData
                    width: ListView.view.width
                    height: Theme.rowHeight - 4
                    spacing: Theme.spacingSmall
                    Text {
                        Layout.leftMargin: Theme.spacingSmall
                        Layout.preferredWidth: 190
                        text: change.modelData.section + " · " + change.modelData.item
                        font: Theme.fontSmall
                        color: Theme.text
                        elide: Text.ElideRight
                    }
                    Text {
                        Layout.fillWidth: true
                        Layout.rightMargin: Theme.spacingSmall
                        //: A value on the device now, then after the import
                        text: qsTr("%1 → %2").arg(change.modelData.before).arg(change.modelData.after)
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        elide: Text.ElideRight
                    }
                    Accessible.role: Accessible.ListItem
                    Accessible.name: change.modelData.section + ", " + change.modelData.item + ": "
                                     + change.modelData.before + ", " + change.modelData.after
                }
            }
        }
        AppCheckBox {
            id: rollback
            objectName: "ImportConfig.rollback"
            Layout.fillWidth: true
            Layout.topMargin: Theme.spacingSmall
            text: options.guardDefault
                  ? qsTr("Apply with rollback: always over DSPi Link (Keep or Revert)")
                  : qsTr("Apply with rollback (Keep or Revert)")
            enabled: !options.guardDefault
            Accessible.name: text
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
                objectName: "ImportConfig.cancel"
                text: qsTr("Cancel")
                onClicked: options.close()
            }
            AppButton {
                id: importButton
                objectName: "ImportConfig.import"
                text: rollback.checked ? qsTr("Apply with Rollback") : qsTr("Import")
                primary: true
                enabled: options.blocks !== 0
                onClicked: {
                    options.importing = true
                    options.close()
                }
                Keys.onReturnPressed: clicked()
                Keys.onEnterPressed: clicked()
            }
        }
    }
}
