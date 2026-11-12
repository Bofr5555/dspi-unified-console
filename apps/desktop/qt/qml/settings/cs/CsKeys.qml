import QtQuick
import UConsole

// -- WP QP12 -- The cards one list shows (CsController.keys), replaced only
// when the list itself changes: a Repeater over it keeps its cards (and their
// open state and focus) while their contents change.
QtObject {
    id: keys
    property string page
    property var list: []
    function sync() {
        const next = CsController.keys(page)
        if (next.join("|") !== list.join("|")) list = next
    }
    property Connections watch: Connections {
        target: CsController
        function onChanged() { keys.sync() }
    }
    onPageChanged: sync()
    Component.onCompleted: sync()
}
