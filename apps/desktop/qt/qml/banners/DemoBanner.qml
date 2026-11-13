import QtQuick
import UConsole

// -- WP QP18 -- The demo device is open (behaviour.md §1.4), or nothing is
// connected and the demo is on offer (§1.3)
Banner {
    objectName: "DemoBanner"
    kind: "info"
    iconName: DemoController.demoOpen ? "play" : "unplug"
    text: DemoController.demoOpen
          ? qsTr("This is the demo DSPi, a simulated device: nothing you change here reaches a real DSPi.")
          : qsTr("No DSPi is connected. Connect one over USB, or try the demo device.")
    visible: DemoController.demoOpen || DemoController.nothingConnected

    AppButton {
        objectName: "DemoBanner.open"
        visible: !DemoController.demoOpen
        text: qsTr("Open Demo")
        onClicked: DemoController.openDemo()
    }
}
