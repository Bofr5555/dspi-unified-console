import QtQuick
import QtQuick.Controls.Basic
import UConsole

// Import Filters' channel picker (FileMenuActions.runChannelPicker): what the
// file holds, and a check box per channel it can go to. Import sends the
// ticked ones; Cancel, Escape or no tick sends nothing.
Dialog {
    id: picker
    objectName: "ImportFilters"
    property string prompt
    // [{wire, label, checked}], as FilesController.filtersPicker gives them
    property var channels: []
    property var ticked: ({})

    function ask(prompt, channels) {
        picker.prompt = prompt
        const t = {}
        for (const c of channels) t[c.wire] = c.checked
        ticked = t
        picker.channels = channels
        open()
    }
    // The ticked wire channels, in the list's order
    function chosen() {
        return channels.filter(c => ticked[c.wire]).map(c => c.wire)
    }
    function setTicked(wire, on) {
        const t = Object.assign({}, ticked)
        t[wire] = on
        ticked = t
    }

    // Set when Import answered, so the close that follows is not a cancel
    property bool importing: false
    onAboutToShow: importing = false
    onClosed: {
        if (importing) FilesController.applyFilters(chosen())
        else FilesController.cancelImport()
    }

    title: qsTr("Import Filters")
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 320
    width: Math.min(420, hostWidth - 2 * Theme.spacingLarge)
    // -- WP QTPOP -- as tall as the channels, within the window less
    // Theme.fitMargin each side; the list scrolls beyond that
    height: Math.min(implicitHeight, hostHeight - 2 * Theme.fitMargin)
    x: (hostWidth - width) / 2
    y: Math.max(Theme.spacingLarge, (hostHeight - height) / 3)
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
            textFormat: Text.PlainText
            text: picker.title
            font: Theme.fontTitle
            color: Theme.text
            wrapMode: Text.Wrap
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
    }

    contentItem: Item {
        implicitHeight: promptText.height + Theme.spacing + boxes.implicitHeight
        Accessible.role: Accessible.Dialog
        Accessible.name: picker.title
        Accessible.description: picker.prompt
        Text {
            id: promptText
            objectName: "ImportFilters.prompt"
            width: parent.width
            textFormat: Text.PlainText
            text: picker.prompt
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        Flickable {
            id: list
            y: promptText.height + Theme.spacing
            width: parent.width
            height: Math.max(0, parent.height - y)
            contentHeight: boxes.implicitHeight
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}
            Column {
                id: boxes
                width: list.width
                spacing: 4
                Repeater {
                    model: picker.channels
                    AppCheckBox {
                        required property var modelData
                        objectName: "ImportFilters.channel." + modelData.wire
                        text: modelData.label
                        checked: picker.ticked[modelData.wire] === true
                        onToggled: picker.setTicked(modelData.wire, checked)
                        Accessible.name: modelData.label
                    }
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
                objectName: "ImportFilters.cancel"
                text: qsTr("Cancel")
                onClicked: picker.close()
            }
            AppButton {
                id: importButton
                objectName: "ImportFilters.import"
                text: qsTr("Import")
                primary: true
                onClicked: {
                    picker.importing = true
                    picker.close()
                }
            }
        }
    }
}
