import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP INSTQ -- A field of the installation's setup: its title, the control,
// and the issue on it (InstallationController.issueOf) below.
ColumnLayout {
    id: field
    property string title
    property string issue
    property string issueName        // the issue line's objectName
    default property alias content: holder.data
    spacing: 2

    Text {
        visible: field.title.length > 0
        textFormat: Text.PlainText
        text: field.title
        font: Theme.fontSmall
        color: Theme.textMuted
    }
    RowLayout {
        id: holder
        spacing: Theme.spacingSmall
    }
    Text {
        objectName: field.issueName
        visible: text.length > 0
        Layout.preferredWidth: 220
        Layout.fillWidth: true
        text: field.issue
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.dangerText
        wrapMode: Text.Wrap
        Accessible.role: Accessible.AlertMessage
        Accessible.name: text
    }
}
