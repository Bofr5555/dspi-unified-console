import QtQuick
import UConsole

// -- WP INSTQ -- An element's name, edited in place; follows the draft when it
// is not being typed in.
BorderedField {
    id: field
    property string elementId
    property string name
    property bool editable: false
    implicitWidth: 200
    enabled: editable
    maximumLength: 64
    selectAllOnFocus: true
    function sync() { if (!activeFocus) text = name }
    Component.onCompleted: sync()
    onNameChanged: sync()
    Connections {
        target: InstallationController
        function onDraftChanged() { field.sync() }
    }
    onCommitted: t => { if (!InstallationController.rename(elementId, t)) text = name }
}
