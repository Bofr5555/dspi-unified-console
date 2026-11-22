import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY4 -- An input pair's link on the channel strip (InputDetail's LINK,
// in Modern's dress): a pair that differs is asked about before it is linked.
AbstractButton {
    id: button
    property int input: -1
    readonly property int partner: input ^ 1
    readonly property bool available: ChannelController.revision >= 0 && input >= 0 && ChannelController.pairAvailable(input)
    readonly property bool linked: ChannelController.revision >= 0 && input >= 0 && ChannelController.linkedPartner(input) >= 0
    // "1/2", "3/4"
    readonly property string pair: (Math.min(input, partner) + 1) + "/" + (Math.max(input, partner) + 1)

    leftPadding: 10
    rightPadding: 10
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    onClicked: {
        if (!available) return
        if (ChannelController.isLinked(input)) {
            ChannelController.unlinkPair(input)
            return
        }
        // Linking mirrors only later edits: a pair that already differs is
        // reconciled now, or "linked" would quietly stay mismatched
        const m = ChannelController.pairMismatch(input)
        if (!m.bands && !m.preamp) {
            ChannelController.linkPair(input, -1)
            return
        }
        const differences = m.bands && m.preamp ? qsTr("different filters and input trims")
                          : m.bands ? qsTr("different filters") : qsTr("different input trims")
        question.asked = input
        question.title = qsTr("Inputs %1 and %2 don't match").arg(input + 1).arg(partner + 1)
        question.text = qsTr("These inputs have %1. Linking mirrors future edits across both channels, but it cannot merge settings that already differ.\n\nChoose which channel's filters and trim to copy onto the other. This overwrites the other channel and cannot be undone.").arg(differences)
        question.acceptText = qsTr("Keep IN%1").arg(input + 1)
        question.alternativeText = qsTr("Keep IN%1").arg(partner + 1)
        question.open()
    }
    contentItem: Row {
        spacing: 6
        Icon {
            anchors.verticalCenter: parent.verticalCenter
            name: button.linked ? "link" : "unlink"
            size: 14
            color: button.linked ? Theme.accentText : Theme.textMuted
        }
        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: button.linked ? qsTr("Linked %1").arg(button.pair) : qsTr("Link %1").arg(button.pair)
            font: Theme.fontSmall
            color: button.linked ? Theme.accentText : Theme.textMuted
        }
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        color: button.hovered ? Theme.controlHover : Theme.control
        border.width: button.visualFocus ? 2 : 0
        border.color: Theme.focusRing
    }
    HelpTip {
        text: button.linked ? qsTr("Unlink %1 (preamp & PEQ stay independent)").arg(button.pair)
                            : qsTr("Link %1 (preamp & PEQ edits mirrored)").arg(button.pair)
        visible: button.hovered
    }
    QuestionDialog {
        id: question
        objectName: "ChannelStrip.linkQuestion"
        // The input it was asked for, whatever is selected when it is answered
        property int asked: -1
        ofDevice: true
        parent: Overlay.overlay
        rejectText: qsTr("Cancel")
        onAnswered: answer => {
            if (answer === "accept") ChannelController.linkPair(asked, asked)
            else if (answer === "alternative") ChannelController.linkPair(asked, asked ^ 1)
        }
    }
    Accessible.role: Accessible.CheckBox
    Accessible.name: qsTr("Link %1").arg(pair)
    Accessible.checkable: true
    Accessible.checked: linked
}
