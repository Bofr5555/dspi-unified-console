import QtQuick 2.15
import QtQuick.Controls 2.15
import "components"

// WP QD9: the PIN a hub asks for (reference IOSPairingSheet in IOSLinkPairing.swift).
// It never blocks: it shows whenever linkSession's phase is needs_pairing, however
// the dial began, and goes when the phase moves on. Pair hands the PIN to
// linkSession, whose worker sends it; a wrong one keeps this open with the core's
// sentence for it. Cancel leaves the hub. What to type and where to find it is
// CG10's, by the hub's PIN source (fixed, or a pairing window).
Popup {
    id: dlg
    objectName: "pairingDialog"

    readonly property bool asked: linkSession.phase === "needs_pairing"
    readonly property string pin: pinField.text.trim()
    // -- WP QD13b -- (review c4q N1) Six digits, as IOSLinkPairing.isValid: the hub
    // locks a client out after five wrong tries, so a typo must not cost one
    readonly property bool valid: /^[0-9]{6}$/.test(pin)
    property bool sent: false     // Pair was pressed and its answer is out

    function submit() {
        if (!valid || linkSession.busy) return
        sent = true
        linkSession.pair(pin)
    }
    function cancel() {
        close()
        if (asked) linkSession.closeSession()
    }

    onAskedChanged: asked ? open() : close()
    Component.onCompleted: if (asked) open()
    onAboutToShow: { pinField.text = ""; sent = false }
    onOpened: pinField.forceActiveFocus()
    Connections {
        target: linkSession
        // A refused PIN: type it again
        function onErrorChanged() {
            if (dlg.sent && linkSession.errorCode !== "") {
                dlg.sent = false
                pinField.text = ""
                pinField.forceActiveFocus()
            }
        }
    }

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(360, parent ? parent.width - 48 : 360)
    padding: 20
    modal: true
    focus: true
    // Leaving needs a choice: Esc is Cancel, and a click outside does nothing
    closePolicy: Popup.NoAutoClose

    Overlay.modal: Rectangle { color: Qt.rgba(0, 0, 0, 0.45) }
    enter: Transition {
        ParallelAnimation {
            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 130; easing.type: Easing.OutCubic }
            NumberAnimation { property: "scale"; from: 0.95; to: 1; duration: 150; easing.type: Easing.OutCubic }
        }
    }
    exit: Transition { NumberAnimation { property: "opacity"; from: 1; to: 0; duration: 90 } }
    background: Rectangle {
        radius: 14
        color: MenuStyle.background
        border.color: MenuStyle.border
    }

    contentItem: Column {
        spacing: 14
        Keys.onEscapePressed: dlg.cancel()

        Row {
            width: parent.width
            spacing: 12
            Rectangle {
                width: 32; height: 32; radius: 9
                color: Qt.rgba(0.04, 0.49, 1, 0.18)
                Icon { anchors.centerIn: parent; name: "link"; size: 18; color: "#0a7cff" }
            }
            Text {
                width: parent.width - 44
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.WordWrap
                text: "Pair with " + (linkSession.hubName !== "" ? linkSession.hubName : "the hub")
                font.pixelSize: 15
                font.weight: Font.DemiBold
                color: isMacOS ? MacColors.label : "white"
            }
        }
        Text {
            objectName: "pairingText"
            width: parent.width
            wrapMode: Text.WordWrap
            text: linkSession.phaseText
            font.pixelSize: 13
            color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.65)
        }
        TextField {
            id: pinField
            objectName: "pairingPin"
            width: parent.width
            height: 38
            topPadding: 0
            bottomPadding: 0
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            font.pixelSize: 22
            font.letterSpacing: 4
            color: isMacOS ? MacColors.label : "white"
            selectionColor: isMacOS ? MacColors.selectedControl : "#0a7cff"
            selectedTextColor: "white"
            selectByMouse: true
            maximumLength: 6
            validator: RegularExpressionValidator { regularExpression: /[0-9]{0,6}/ }
            inputMethodHints: Qt.ImhDigitsOnly | Qt.ImhSensitiveData | Qt.ImhNoPredictiveText
            placeholderText: "000000"
            placeholderTextColor: Qt.rgba(1, 1, 1, 0.25)
            enabled: !linkSession.busy
            onAccepted: dlg.submit()
            background: Rectangle {
                radius: 8
                color: Qt.rgba(1, 1, 1, 0.06)
                border.width: pinField.activeFocus ? 1.5 : 1
                border.color: pinField.activeFocus ? "#0a7cff" : Qt.rgba(1, 1, 1, 0.12)
            }
        }
        Text {
            objectName: "pairingError"
            width: parent.width
            visible: text !== ""
            wrapMode: Text.WordWrap
            text: linkSession.busy ? "" : linkSession.errorText
            font.pixelSize: 12
            color: "#ff9f0a"
        }
        Row {
            width: parent.width
            spacing: 10
            AppButton {
                objectName: "pairingCancel"
                width: (parent.width - parent.spacing) / 2
                text: "Cancel"
                onClicked: dlg.cancel()
            }
            AppButton {
                objectName: "pairingPair"
                width: (parent.width - parent.spacing) / 2
                primary: true
                text: linkSession.busy && dlg.sent ? "Pairing…" : "Pair"
                enabled: dlg.valid && !linkSession.busy
                onClicked: dlg.submit()
            }
        }
    }
}
