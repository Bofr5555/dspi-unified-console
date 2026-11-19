import QtQuick
import UConsole

// -- WP QTSENDER -- The alink sender is open (DeviceInfoController.sender):
// the DSP menus, shortcuts and tool windows are greyed, and this says why.
Banner {
    objectName: "SenderBanner"
    kind: "info"
    iconName: "radio"
    text: qsTr("This is an audio link sender. It has no DSP: only its device info, the audio link and firmware updates are available.")
    visible: DeviceInfoController.sender
}
