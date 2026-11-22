import QtQuick
import UConsole

// -- WP INSTQ -- A choice by id among `choices` ({ id, name }): shows `value`,
// the draft's, whatever was picked before the draft took it or refused it.
AppComboBox {
    id: choice
    property var choices: []
    property string value
    signal chosen(string id)

    implicitWidth: 180
    model: choices.map(c => c.name)
    function sync() {
        currentIndex = choices.findIndex(c => c.id === value)
    }
    onModelChanged: sync()
    onValueChanged: sync()
    Component.onCompleted: sync()
    Connections {
        target: InstallationController
        function onDraftChanged() { choice.sync() }
    }
    onActivated: i => {
        if (i >= 0 && i < choices.length && choices[i].id !== value) chosen(choices[i].id)
    }
}
