import QtQuick
import UConsole

// -- WP GAP2 -- While an output is soloed: which one plays, that it is not
// saved, and the way out (SoloController)
Banner {
    objectName: "SoloBanner"
    kind: "info"
    iconName: "headphones"
    visible: SoloController.output >= 0
    // The output as the pages name it ("Left tweeter · 2")
    readonly property ChannelInfo info: ChannelInfo {
        channel: SoloController.output < 0 || DeviceModel.revision < 0
                 ? -1 : ChannelController.outputChannel(SoloController.output)
    }
    text: qsTr("Solo: only %1 plays. The other outputs are muted until you end it; nothing is saved.")
              .arg(info.title || qsTr("Out %1").arg(SoloController.output + 1))

    AppButton {
        objectName: "SoloBanner.end"
        text: qsTr("End Solo")
        onClicked: SoloController.end()
    }
}
