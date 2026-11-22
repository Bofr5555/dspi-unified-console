import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP PGQT -- One raw request by command id on a paged device
// (DeviceInfoController.rawGet / rawSet, the core's dspi_raw_get_id and
// dspi_raw_set_id): the id as 0xPPCC (page and command), wValue, and the
// length of a GET or the payload of a SET as hex bytes. The core refuses what
// it gates (flash, restart and bulk commands, confirmation-token requests, the
// core voltage, a 1.1.6 device) and the dialog says it in words; anything that
// would go out cut (above 16 bits, a byte above 0xFF) is refused before.
Dialog {
    id: dialog
    objectName: "RawRequestDialog"
    modal: true
    closePolicy: Popup.CloseOnEscape
    width: Math.min(480, (parent ? parent.width : 480) - 2 * Theme.spacingLarge)
    x: parent ? (parent.width - width) / 2 : 0
    y: Theme.spacingLarge
    padding: Theme.spacingLarge
    title: qsTr("Raw Request by Command Id")
    property bool set: false

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.color: Theme.border
    }
    header: Text {
        leftPadding: Theme.spacingLarge
        topPadding: Theme.spacingLarge
        text: dialog.title
        font: Theme.fontTitle
        color: Theme.text
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }
    contentItem: ColumnLayout {
        spacing: Theme.spacingSmall
        Text {
            Layout.fillWidth: true
            text: DeviceInfoController.paged
                ? qsTr("One GET or SET of a paged command, sent as typed. Numbers are 0x hexadecimal or decimal.")
                : qsTr("This device speaks the 1.1.6 protocol, which has no command ids: the core will refuse a request here and send nothing.")
            font: Theme.fontSmall
            color: DeviceInfoController.paged ? Theme.textMuted : Theme.warning
            wrapMode: Text.Wrap
        }
        RowLayout {
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "RawRequestDialog.get"
                text: qsTr("GET")
                primary: !dialog.set
                onClicked: dialog.set = false
            }
            AppButton {
                objectName: "RawRequestDialog.set"
                text: qsTr("SET")
                primary: dialog.set
                onClicked: dialog.set = true
            }
            Item { Layout.fillWidth: true }
        }
        GridLayout {
            Layout.fillWidth: true
            columns: 2
            columnSpacing: Theme.spacingSmall
            rowSpacing: 4
            Text { text: qsTr("Command id"); font: Theme.fontBody; color: Theme.text }
            BorderedField {
                id: idField
                objectName: "RawRequestDialog.id"
                Layout.fillWidth: true
                placeholderText: "0x0550"
                font: Theme.fontMono
            }
            Text { text: qsTr("wValue"); font: Theme.fontBody; color: Theme.text }
            BorderedField {
                id: valueField
                objectName: "RawRequestDialog.value"
                Layout.fillWidth: true
                placeholderText: "0"
                font: Theme.fontMono
            }
            Text { text: dialog.set ? qsTr("Payload (hex)") : qsTr("Length"); font: Theme.fontBody; color: Theme.text }
            BorderedField {
                id: lastField
                objectName: "RawRequestDialog.last"
                Layout.fillWidth: true
                placeholderText: dialog.set ? "01 ff" : "4"
                font: Theme.fontMono
            }
        }
        TextEdit {
            objectName: "RawRequestDialog.result"
            Layout.fillWidth: true
            visible: text.length > 0
            text: DeviceInfoController.rawResult
            readOnly: true
            selectByMouse: true
            wrapMode: TextEdit.Wrap
            font: Theme.fontMono
            color: DeviceInfoController.rawFailed ? Theme.warning : Theme.text
            selectionColor: Theme.textSelection
            Accessible.role: Accessible.StaticText
            Accessible.name: qsTr("Result")
            Accessible.description: text
        }
    }
    footer: RowLayout {
        spacing: Theme.spacingSmall
        Item { Layout.fillWidth: true }
        AppButton {
            objectName: "RawRequestDialog.close"
            Layout.bottomMargin: Theme.spacing
            text: qsTr("Close")
            onClicked: dialog.close()
        }
        AppButton {
            objectName: "RawRequestDialog.send"
            Layout.bottomMargin: Theme.spacing
            Layout.rightMargin: Theme.spacingLarge
            primary: true
            text: qsTr("Send")
            enabled: DeviceModel.connected && !DeviceInfoController.rawBusy && idField.text.length > 0
            onClicked: dialog.set ? DeviceInfoController.rawSet(idField.text, valueField.text, lastField.text)
                                  : DeviceInfoController.rawGet(idField.text, valueField.text, lastField.text)
        }
    }
}
