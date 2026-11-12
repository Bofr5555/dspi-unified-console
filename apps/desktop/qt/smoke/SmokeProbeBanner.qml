import QtQuick
import UConsole

// -- WP QP0 -- The smoke driver's banner (SmokeProbe.h)
Banner {
    objectName: "SmokeProbeBanner"
    kind: "warning"
    text: SmokeProbe.message
    visible: SmokeProbe.bannerShown
    dismissable: true
    onDismissed: SmokeProbe.bannerShown = false
}
