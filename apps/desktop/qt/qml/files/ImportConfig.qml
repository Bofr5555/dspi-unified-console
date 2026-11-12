import QtQuick
import QtQuick.Controls.Basic
import UConsole

// Import Device Configuration's options (FileMenuActions.askImportOptions):
// audio processing is the point of the file and always comes in; volumes and
// the physical wiring are opt-in, since neither need belong to this machine.
Dialog {
    id: options
    objectName: "ImportConfig"
    property string prompt

    function ask(prompt) {
        options.prompt = prompt
        volumes.checked = false
        hardware.checked = false
        open()
    }

    property bool importing: false
    onAboutToShow: importing = false
    onOpened: importButton.forceActiveFocus()
    onClosed: {
        if (importing) FilesController.applyConfiguration(volumes.checked, hardware.checked)
        else FilesController.cancelImport()
    }

    title: qsTr("Import Device Configuration")
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 320
    width: Math.min(440, hostWidth - 2 * Theme.spacingLarge)
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

    contentItem: Item {
        implicitHeight: promptText.height + Theme.spacing + boxes.implicitHeight
        Accessible.role: Accessible.Dialog
        Accessible.name: options.title
        Accessible.description: options.prompt
        Text {
            id: promptText
            objectName: "ImportConfig.prompt"
            width: parent.width
            text: options.prompt
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        Column {
            id: boxes
            y: promptText.height + Theme.spacing
            width: parent.width
            spacing: 6
            AppCheckBox {
                id: volumes
                objectName: "ImportConfig.volumes"
                width: parent.width
                text: qsTr("Volume levels (master and listening volume)")
                Accessible.name: text
            }
            AppCheckBox {
                id: hardware
                objectName: "ImportConfig.hardware"
                width: parent.width
                text: qsTr("Hardware I/O (GPIO pins, clocks, ADAT, inputs, output limiters)")
                Accessible.name: text
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
                objectName: "ImportConfig.cancel"
                text: qsTr("Cancel")
                onClicked: options.close()
            }
            AppButton {
                id: importButton
                objectName: "ImportConfig.import"
                text: qsTr("Import")
                primary: true
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
