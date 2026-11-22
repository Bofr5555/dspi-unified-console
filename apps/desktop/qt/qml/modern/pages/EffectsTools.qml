import QtQuick
import UConsole

// -- WP LAY5a -- The buttons in the row over the effects (MODLAY 2): every
// output's limiter (the effects can raise the level), and the signal chain.
Row {
    objectName: "EffectsTools"
    spacing: 6

    AppButton {
        objectName: "EffectsTools.limiters"
        // The signal chain's Limiter goes there too, where the tabs need the room
        visible: ChannelController.limiterSupported && Window.width >= InspectorState.foldWidth
        text: qsTr("Output Limiters…")
        toolTip: qsTr("Every output's limiter, on Equaliser › Delay & Gain")
        onClicked: if (Shell.pageHost) Shell.pageHost.go("equaliser", "delay")
    }
    InspectorButton { objectName: "EffectsTools.inspector" }
}
