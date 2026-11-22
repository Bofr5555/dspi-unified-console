import QtQuick
import UConsole

// -- WP LAY5a -- The button in the row over a page that shows or hides its
// inspector (LAY4's, taken out of EqualiserTools for the other pages).
AppButton {
    text: qsTr("Inspector")
    iconName: InspectorState.open ? "chevron-right" : "chevron-left"
    toolTip: InspectorState.open ? qsTr("Hide the Inspector") : qsTr("Show the Inspector")
    onClicked: InspectorState.toggle()
    Accessible.role: Accessible.CheckBox
    Accessible.checkable: true
    Accessible.checked: InspectorState.open
}
