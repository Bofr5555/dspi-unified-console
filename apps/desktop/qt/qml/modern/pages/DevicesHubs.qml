import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5b -- Devices > Hubs: the hub's devices, the hubs found and the
// address field (DevicesLink), with what a hub asks answered here: its PIN, with
// its key to compare, and pairing again after its key changed. The dialogs for
// these stay closed while this page shows (PageHost.pairsInline), and come up
// when the person leaves it. Beside it, the hub this console is on.
Item {
    id: page
    objectName: "DevicesHubs"
    property bool shown: false
    property Item host: null

    readonly property bool wide: width >= 900
    readonly property bool pairing: LinkController.phase === "needs_pairing"

    Flickable {
        id: flick
        anchors.fill: parent
        contentHeight: grid.implicitHeight + 2 * Theme.spacingLarge
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: AppScrollBar {}

        GridLayout {
            id: grid
            x: (flick.width - width) / 2
            y: Theme.spacingLarge
            width: Math.min(page.wide ? 1000 : 640, flick.width - 2 * Theme.spacingLarge)
            columns: page.wide ? 2 : 1
            columnSpacing: Theme.spacingLarge
            rowSpacing: Theme.spacing

            ColumnLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignTop
                spacing: Theme.spacing

                // ---- The hub asks for its PIN
                Card {
                    id: pairCard
                    objectName: "DevicesHubs.pair"
                    visible: page.pairing
                    // It came up, or the person came to it: an empty field (a
                    // pairing link's PIN), ready to type in
                    readonly property bool asks: page.pairing && page.shown
                    onAsksChanged: {
                        if (!asks) return
                        pairContent.start()
                        Qt.callLater(pairCard.focusPin)
                    }
                    // Once the call that came to it is through: the field is
                    // off until then, and takes no focus
                    readonly property bool ready: asks && !LinkController.busy
                    onReadyChanged: if (ready) Qt.callLater(pairCard.focusPin)
                    function focusPin() { if (asks) pairContent.takeFocus() }
                    Layout.fillWidth: true
                    padding: Theme.spacingLarge
                    title: qsTr("Pair with %1").arg(LinkController.hubName.length ? LinkController.hubName : qsTr("the hub"))
                    PairingContent {
                        id: pairContent
                        names: "DevicesHubs.pair"
                        Layout.fillWidth: true
                    }
                    RowLayout {
                        Layout.alignment: Qt.AlignRight
                        spacing: Theme.spacingSmall
                        AppButton {
                            objectName: "DevicesHubs.pair.cancel"
                            text: qsTr("Cancel")
                            onClicked: LinkController.closeSession()
                        }
                        AppButton {
                            objectName: "DevicesHubs.pair.accept"
                            text: qsTr("Pair")
                            primary: true
                            enabled: pairContent.complete && !LinkController.busy
                            onClicked: pairContent.submit()
                        }
                    }
                }
                // ---- A failure only pairing again can answer (RePairDialog)
                Card {
                    objectName: "DevicesHubs.rePair"
                    visible: LinkController.rePairOffered
                    Layout.fillWidth: true
                    padding: Theme.spacingLarge
                    title: words.title
                    RePairWords { id: words }
                    Text {
                        objectName: "DevicesHubs.rePair.text"
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        text: words.text
                        font: Theme.fontBody
                        color: Theme.text
                        wrapMode: Text.Wrap
                        Accessible.role: Accessible.AlertMessage
                        Accessible.name: words.title
                        Accessible.description: text
                    }
                    RowLayout {
                        Layout.alignment: Qt.AlignRight
                        spacing: Theme.spacingSmall
                        AppButton {
                            objectName: "DevicesHubs.rePair.cancel"
                            text: qsTr("Cancel")
                            onClicked: LinkController.dismissRePair()
                        }
                        AppButton {
                            objectName: "DevicesHubs.rePair.accept"
                            text: words.acceptText
                            primary: true
                            onClicked: LinkController.rePair()
                        }
                    }
                }
                DevicesLink {
                    id: link
                    Layout.fillWidth: true
                }
            }

            // ---- The hub this console is on
            ColumnLayout {
                objectName: "DevicesHubs.hub"
                Layout.alignment: Qt.AlignTop
                Layout.preferredWidth: page.wide ? 320 : -1
                Layout.maximumWidth: page.wide ? 320 : Number.POSITIVE_INFINITY
                Layout.fillWidth: !page.wide
                spacing: Theme.spacing

                Card {
                    visible: link.inSession
                    Layout.fillWidth: true
                    title: qsTr("This hub")
                    Fact { name: "name"; label: qsTr("Name"); value: LinkController.hubName }
                    Fact {
                        name: "role"
                        label: qsTr("Your role")
                        value: LinkController.role === "admin" ? qsTr("Admin") : LinkController.role === "control" ? qsTr("Control")
                             : LinkController.role === "viewer" ? qsTr("Viewer") : LinkController.role
                    }
                    Fact {
                        name: "security"
                        label: qsTr("Security")
                        value: LinkController.encrypted ? qsTr("Encrypted") : LinkController.unsecured ? qsTr("Not encrypted")
                             : LinkController.local ? qsTr("On this computer") : ""
                        ink: LinkController.unsecured ? Theme.dangerText : Theme.text
                    }
                    Text {
                        visible: LinkController.encrypted && LinkController.hubKey.length > 0
                        text: qsTr("Hub key")
                        font: Theme.fontSmall
                        color: Theme.textMuted
                    }
                    TextEdit {
                        objectName: "DevicesHubs.hub.key"
                        Layout.fillWidth: true
                        visible: LinkController.encrypted && LinkController.hubKey.length > 0
                        textFormat: TextEdit.PlainText
                        text: LinkController.hubKey
                        readOnly: true
                        selectByMouse: true
                        font: Theme.fontMono
                        color: Theme.text
                        selectionColor: Theme.textSelection
                        selectedTextColor: Theme.text
                        wrapMode: TextEdit.WrapAnywhere
                        Accessible.role: Accessible.StaticText
                        Accessible.name: qsTr("Hub key")
                        Accessible.description: text
                    }
                }
                // What is said about it (the drawer's notices too, until OK)
                Said {
                    objectName: "DevicesHubs.keyNotice"
                    text: LinkController.keyNotice
                    onOk: LinkController.dismissKeyNotice()
                }
                Said {
                    objectName: "DevicesHubs.tokenNotice"
                    text: LinkController.tokenWarning
                    onOk: LinkController.dismissTokenWarning()
                }
                Said {
                    objectName: "DevicesHubs.lockNotice"
                    text: LinkController.lockNotice
                    warn: true
                    dismissable: false
                }
                Card {
                    Layout.fillWidth: true
                    title: qsTr("This computer's DSPi")
                    Text {
                        Layout.fillWidth: true
                        text: qsTr("Share it with other consoles on this network, and choose who may do what.")
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                    AppButton {
                        objectName: "DevicesHubs.networking"
                        text: qsTr("Networking Settings…")
                        onClicked: SettingsController.showPage("networking")
                    }
                }
            }
        }
    }

    component Fact: RowLayout {
        id: fact
        property string name
        property string label
        property string value
        property color ink: Theme.text
        Layout.fillWidth: true
        visible: value.length > 0
        spacing: Theme.spacingSmall
        Text {
            text: fact.label
            font: Theme.fontBody
            color: Theme.textMuted
        }
        Text {
            objectName: "DevicesHubs.hub." + fact.name
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignRight
            textFormat: Text.PlainText
            text: fact.value
            font: Theme.fontBody
            color: fact.ink
            elide: Text.ElideRight
        }
        Accessible.role: Accessible.StaticText
        Accessible.name: label + ": " + value
    }
    component Said: Card {
        id: said
        property string text
        property bool warn: false
        property bool dismissable: true
        signal ok()
        Layout.fillWidth: true
        visible: text.length > 0
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: said.text
            font: Theme.fontSmall
            color: said.warn ? Theme.warning : Theme.text
            wrapMode: Text.Wrap
        }
        AppButton {
            objectName: said.objectName + ".ok"
            visible: said.dismissable
            Layout.alignment: Qt.AlignRight
            text: qsTr("OK")
            onClicked: said.ok()
        }
    }
}
