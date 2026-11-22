import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP2 -- An input's page under the graph (the Mac's input selection):
// the header card (pair link, preamp, Clear PEQ: InputChannelHeader) over its
// PEQ band list. Edits mirror onto the other half of a linked pair.
//
//     InputDetail { input: 0; graphSelection: ... }
ColumnLayout {
    id: page
    property int input: 0
    property var graphSelection: null

    readonly property int partner: input ^ 1
    readonly property bool pairAvailable: (ChannelController.revision < 0 ? undefined : ChannelController.pairAvailable(input))
    readonly property bool linked: (ChannelController.revision < 0 ? undefined : ChannelController.linkedPartner(input) >= 0)
    // "1/2", "3/4": the sidebar's IN numbering
    readonly property string pairLabel: (Math.min(input, partner) + 1) + "/" + (Math.max(input, partner) + 1)

    objectName: "InputDetail"
    spacing: Theme.spacingLarge

    function toggleLink() {
        if (!pairAvailable) return
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
        linkQuestion.title = qsTr("Inputs %1 and %2 don't match").arg(input + 1).arg(partner + 1)
        linkQuestion.text = qsTr("These inputs have %1. Linking mirrors future edits across both channels, but it cannot merge settings that already differ.\n\nChoose which channel's filters and trim to copy onto the other. This overwrites the other channel and cannot be undone.").arg(differences)
        linkQuestion.acceptText = qsTr("Keep IN%1").arg(input + 1)
        linkQuestion.alternativeText = qsTr("Keep IN%1").arg(partner + 1)
        linkQuestion.open()
    }

    // -- WP QP9 -- The spectrum as bars, at the top of the page
    SpectrumStrip {
        place: "page"
        Layout.fillWidth: true
    }

    // ---- Header card ----
    Rectangle {
        objectName: "InputDetail.header"
        Layout.fillWidth: true
        Layout.preferredHeight: 56
        radius: Theme.radius
        color: Theme.panel
        border.width: 1
        border.color: Theme.separator
        enabled: ChannelController.writable

        RowLayout {
            anchors.fill: parent
            spacing: 0

            // LINK (only while both halves of the pair are live)
            AbstractButton {
                id: linkButton
                objectName: "InputDetail.link"
                visible: page.pairAvailable
                Layout.leftMargin: Theme.spacing
                Layout.rightMargin: Theme.spacing
                hoverEnabled: true
                focusPolicy: Qt.StrongFocus
                padding: Theme.spacingSmall
                leftPadding: 10
                rightPadding: 10
                onClicked: page.toggleLink()
                contentItem: Row {
                    spacing: 6
                    Icon {
                        anchors.verticalCenter: parent.verticalCenter
                        name: page.linked ? "link" : "unlink"
                        size: 14
                        color: page.linked ? Theme.accentText : Theme.textMuted
                    }
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 52
                        textFormat: Text.PlainText
                        text: qsTr("Link %1").arg(page.pairLabel)
                        font: Theme.fontSmall
                        color: page.linked ? Theme.accentText : Theme.textMuted
                    }
                }
                background: Rectangle {
                    radius: Theme.radius
                    color: page.linked ? Theme.selection : (linkButton.hovered ? Theme.controlHover : "transparent")
                    border.width: linkButton.visualFocus ? 2 : 1
                    border.color: linkButton.visualFocus ? Theme.focusRing : page.linked ? Theme.accent : Theme.border
                }
                HelpTip {
                    text: page.linked ? qsTr("Unlink %1 (preamp & PEQ stay independent)").arg(page.pairLabel)
                                      : qsTr("Link %1 (preamp & PEQ edits mirrored)").arg(page.pairLabel)
                    visible: linkButton.hovered
                }
                Accessible.role: Accessible.CheckBox
                Accessible.name: qsTr("Link %1").arg(page.pairLabel)
                Accessible.checkable: true
                Accessible.checked: page.linked
            }
            Rectangle { visible: page.pairAvailable; Layout.fillHeight: true; Layout.preferredWidth: 1; color: Theme.separator }

            // PREAMP
            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: Theme.spacing
                Layout.rightMargin: Theme.spacing
                spacing: Theme.spacing
                Item { Layout.fillWidth: true }
                Text { text: qsTr("Preamp"); font: Theme.fontSmall; color: Theme.textMuted }
                CustomSlider {
                    id: preampSlider
                    objectName: "InputDetail.preamp"
                    Layout.preferredWidth: 240
                    label: qsTr("Preamp")
                    from: -60
                    to: ChannelController.preampMaxDb
                    stepSize: 0.1
                    resetValue: 0
                    // The device's value, except while a hand is on it
                    Binding on value {
                        when: !preampSlider.pressed
                        value: (ChannelController.revision < 0 ? undefined : ChannelController.preamp(page.input))
                        restoreMode: Binding.RestoreNone
                    }
                    onValueMoved: v => ChannelController.setPreamp(page.input, v)
                    onCommitted: v => ChannelController.setPreamp(page.input, v)
                }
                ChannelValueField {
                    objectName: "InputDetail.preampValue"
                    label: qsTr("Preamp")
                    unit: qsTr("dB")
                    fieldWidth: 48
                    resetValue: 0
                    value: preampSlider.pressed ? preampSlider.value : (ChannelController.revision < 0 ? undefined : ChannelController.preamp(page.input))
                    onCommitted: v => ChannelController.setPreamp(page.input, v)
                }
                Item { Layout.fillWidth: true }
            }
            Rectangle { Layout.fillHeight: true; Layout.preferredWidth: 1; color: Theme.separator }

            // CLEAR PEQ
            AppButton {
                objectName: "InputDetail.clear"
                Layout.leftMargin: Theme.spacing
                Layout.rightMargin: Theme.spacing
                text: page.linked ? qsTr("Clear %1 PEQ").arg(page.pairLabel) : qsTr("Clear PEQ")
                toolTip: page.linked ? qsTr("Reset all PEQ bands on both channels of the linked pair")
                                     : qsTr("Reset all PEQ bands on this input")
                onClicked: ChannelController.clearInputPeq(page.input)
            }
        }
    }

    FilterList {
        objectName: "InputDetail.bands"
        Layout.fillWidth: true
        Layout.fillHeight: true
        channel: page.input
        includeLinkwitz: false   // a driver tool: outputs only
        showClear: false          // the header's Clear PEQ does it
        graphSelection: page.graphSelection
    }

    QuestionDialog {
        id: linkQuestion
        objectName: "InputDetail.linkQuestion"
        ofDevice: true   // -- WP MDQT2a --
        parent: Overlay.overlay
        rejectText: qsTr("Cancel")
        onAnswered: answer => {
            if (answer === "accept") ChannelController.linkPair(page.input, page.input)
            else if (answer === "alternative") ChannelController.linkPair(page.input, page.partner)
        }
    }

    Accessible.role: Accessible.Pane
    Accessible.name: qsTr("Input %1").arg(input + 1)
}
