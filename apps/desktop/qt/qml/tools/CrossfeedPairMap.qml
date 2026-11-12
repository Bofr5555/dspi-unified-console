import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- The crossfeed window's pair map (the Mac's
// CrossfeedPairMapControls): which two outputs each crossfeed pair processes,
// on firmware with the extension's pair map (ExtController). The Outputs menu
// chooses Default, the K340's one DAC per ear, or Custom, which opens the
// editor; a write sends the whole map and the view shows what the device
// reports. After the K340 choice a question offers to crossfeed both pairs,
// as the factory mask covers the woofers only: an explicit click, never
// automatic.
ColumnLayout {
    id: map
    objectName: "CrossfeedWindow.pairMap"
    readonly property bool writable: EffectsController.writable && ExtController.hasPairMap
    property bool editing: false
    property bool offerMask: false
    // The editor's pairs, [{left, right}]
    property var draft: []
    readonly property bool draftValid: ExtController.pairsValid(draft)

    spacing: Theme.spacingSmall

    // A pick in the Outputs menu
    function pick(key) {
        if (key === "custom") {
            draft = ExtController.pairs.map(p => ({ left: p.left, right: p.right }))
            editing = true
            return
        }
        offerMask = key === "k340"
        ExtController.applyChoice(key)
    }
    function setDraft(pair, side, output) {
        const next = draft.map(p => ({ left: p.left, right: p.right }))
        if (pair < 0 || pair >= next.length) return
        next[pair][side] = output
        draft = next
    }
    function applyDraft() {
        if (draftValid && ExtController.applyPairs(draft)) editing = false
    }
    function choiceTitle(key) {
        return key === "k340" ? qsTr("One DAC per ear (K340)") : key === "custom" ? qsTr("Custom") : qsTr("Default (1/2, 3/4, …)")
    }

    // Without the pair map, or once writes stop, the editor closes
    onWritableChanged: if (!writable) editing = false

    RowLayout {
        Layout.fillWidth: true
        Text {
            Layout.fillWidth: true
            text: qsTr("Outputs")
            font: Theme.fontBody
            color: Theme.text
        }
        EffectMenuButton {
            objectName: "CrossfeedWindow.pairMap.menu"
            enabled: map.writable
            text: map.choiceTitle(ExtController.choice)
            AppMenuItem {
                objectName: "CrossfeedWindow.pairMap.choice.default"
                text: qsTr("Default (1/2, 3/4, …)")
                onTriggered: map.pick("default")
            }
            AppMenuItem {
                objectName: "CrossfeedWindow.pairMap.choice.k340"
                text: qsTr("One DAC per ear (K340)")
                onTriggered: map.pick("k340")
            }
            AppMenuItem {
                objectName: "CrossfeedWindow.pairMap.choice.custom"
                text: qsTr("Custom…")
                onTriggered: map.pick("custom")
            }
        }
    }
    Text {
        Layout.fillWidth: true
        text: qsTr("Which two outputs each pair crossfeeds. An output can belong to one pair only.")
        font: Theme.fontCaption
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }

    // The K340's mask question
    RowLayout {
        objectName: "CrossfeedWindow.pairMap.offer"
        visible: map.offerMask && ExtController.choice === "k340"
                 && (EffectsController.crossfeedPairMask & ExtController.k340PairMask) !== ExtController.k340PairMask
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        Text {
            Layout.fillWidth: true
            text: qsTr("Crossfeed the woofers and the tweeters?")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        AppButton {
            objectName: "CrossfeedWindow.pairMap.offer.notNow"
            text: qsTr("Not Now")
            onClicked: map.offerMask = false
        }
        AppButton {
            objectName: "CrossfeedWindow.pairMap.offer.enable"
            text: qsTr("Enable Pairs 1 and 2")
            enabled: map.writable
            onClicked: {
                EffectsController.setCrossfeedPairMask(EffectsController.crossfeedPairMask | ExtController.k340PairMask)
                map.offerMask = false
            }
        }
    }

    // The custom editor
    Rectangle {
        objectName: "CrossfeedWindow.pairMap.editor"
        visible: map.editing && map.writable
        Layout.fillWidth: true
        implicitHeight: editor.implicitHeight + 16
        radius: 6
        color: Theme.control
        border.width: 1
        border.color: Theme.separator

        ColumnLayout {
            id: editor
            anchors.fill: parent
            anchors.margins: 8
            spacing: 6

            Repeater {
                model: map.draft.length
                RowLayout {
                    id: pairRow
                    required property int index
                    spacing: 6
                    readonly property var outputs: {
                        const n = 2 * ExtController.pairCount
                        const names = []
                        EffectsController.revision
                        for (let o = 0; o < n; ++o) names.push(qsTr("%1 — %2").arg(o + 1).arg(EffectsController.outputName(o)))
                        return names
                    }
                    Text {
                        Layout.preferredWidth: 46
                        text: qsTr("Pair %1").arg(pairRow.index + 1)
                        font: Theme.fontSmall
                        color: Theme.text
                    }
                    AppComboBox {
                        objectName: "CrossfeedWindow.pairMap.editor.left." + pairRow.index
                        implicitWidth: 92
                        label: qsTr("Pair %1, one side").arg(pairRow.index + 1)
                        model: pairRow.outputs
                        currentIndex: map.draft[pairRow.index] ? map.draft[pairRow.index].left : -1
                        displayText: qsTr("Out %1").arg(currentIndex + 1)
                        onActivated: index => map.setDraft(pairRow.index, "left", index)
                    }
                    Text {
                        text: "↔"
                        font: Theme.fontSmall
                        color: Theme.textMuted
                    }
                    AppComboBox {
                        objectName: "CrossfeedWindow.pairMap.editor.right." + pairRow.index
                        implicitWidth: 92
                        label: qsTr("Pair %1, other side").arg(pairRow.index + 1)
                        model: pairRow.outputs
                        currentIndex: map.draft[pairRow.index] ? map.draft[pairRow.index].right : -1
                        displayText: qsTr("Out %1").arg(currentIndex + 1)
                        onActivated: index => map.setDraft(pairRow.index, "right", index)
                    }
                    Item { Layout.fillWidth: true }
                }
            }
            Text {
                objectName: "CrossfeedWindow.pairMap.editor.warning"
                visible: !map.draftValid
                Layout.fillWidth: true
                text: qsTr("Each pair needs two different outputs, and no output can be in two pairs.")
                font: Theme.fontCaption
                color: Theme.warning
                wrapMode: Text.Wrap
            }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                AppButton {
                    objectName: "CrossfeedWindow.pairMap.editor.cancel"
                    text: qsTr("Cancel")
                    onClicked: map.editing = false
                }
                AppButton {
                    objectName: "CrossfeedWindow.pairMap.editor.apply"
                    text: qsTr("Apply")
                    primary: true
                    enabled: map.draftValid && map.writable
                    onClicked: map.applyDraft()
                }
            }
        }
    }
}
