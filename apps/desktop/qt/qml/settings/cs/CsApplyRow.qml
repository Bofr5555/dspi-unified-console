import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- A card's Apply and Revert (the Mac's applyRow): the reason a
// failed apply gives, then Revert and Apply. `leading` holds a button of the
// card's own (the receiver's Add Remote Button).
RowLayout {
    id: applyRow
    property bool dirty
    property bool applying
    property string message
    property bool isError
    // Apply is possible at all (the draft is one the device takes)
    property bool ready: true
    // Words in place of a message while the draft is not ready
    property string hint
    default property alias leading: lead.data
    signal apply()
    signal revert()

    Layout.fillWidth: true
    spacing: Theme.spacingSmall

    Item {
        id: lead
        implicitWidth: childrenRect.width
        implicitHeight: childrenRect.height
        visible: children.length > 0
    }
    Icon {
        visible: msg.visible && msg.failed
        name: "triangle-alert"
        size: 12
        color: Theme.warning
    }
    Text {
        id: msg
        objectName: applyRow.objectName.length ? applyRow.objectName + ".text" : ""
        Layout.fillWidth: true
        visible: text.length > 0
        // A refusal shows while its draft is kept, so it can be fixed
        readonly property bool failed: applyRow.isError && applyRow.message.length > 0
        textFormat: Text.PlainText
        text: failed || (applyRow.message.length > 0 && !applyRow.dirty) ? applyRow.message
            : applyRow.dirty && !applyRow.ready ? applyRow.hint : ""
        font: Theme.fontSmall
        color: failed ? Theme.warning : Theme.textMuted
        wrapMode: Text.Wrap
        Accessible.role: applyRow.isError ? Accessible.AlertMessage : Accessible.StaticText
        Accessible.name: text
    }
    Item { Layout.fillWidth: !msg.visible }
    Icon {
        visible: applyRow.applying
        name: "loader-circle"
        size: 14
        color: Theme.textMuted
    }
    AppButton {
        objectName: applyRow.objectName.length ? applyRow.objectName + ".revert" : ""
        text: qsTr("Revert")
        enabled: applyRow.dirty && !applyRow.applying
        onClicked: applyRow.revert()
    }
    AppButton {
        objectName: applyRow.objectName.length ? applyRow.objectName + ".apply" : ""
        text: qsTr("Apply")
        primary: true
        enabled: applyRow.dirty && applyRow.ready && !applyRow.applying && !CsController.busy && CsController.writable
        onClicked: applyRow.apply()
    }
}
