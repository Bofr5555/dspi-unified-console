import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP ROLEQ -- Settings › Speakers (MPLAN 2.2): which output drives what,
// the person's speaker layout for the open DSPi. A SettingsPage, so Classic
// and Modern show the same page. Confirm is the only way a layout becomes
// confirmed; the Measure tables write their edits back without confirming.
SettingsPage {
    id: page
    objectName: "SpeakersPage"
    readonly property var sc: SpeakersController
    readonly property bool confirmed: sc.status === "confirmed" && !sc.edited
    readonly property alias detectDialog: detectQuestion   // -- WP ADETQ -- (the smoke driver)

    Component.onCompleted: sc.reload()

    SettingsGroup {
        objectName: "Speakers.layout"
        title: qsTr("Speaker layout")
        iconName: "volume-2"
        footer: qsTr("An output with an active high-pass at 1 kHz or above is measured as a tweeter, whatever its role here.")

        Text {
            Layout.fillWidth: true
            text: qsTr("Which output drives what? The measurement and its safety checks take each output's role from here.")
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        Banner {
            objectName: "SpeakersPage.banner"
            Layout.fillWidth: true
            readonly property string words: page.sc.status, page.sc.loaded, page.sc.stampText, page.sc.bannerText(true)
            kind: page.sc.status === "imported" ? "warning" : "info"
            text: words
            visible: words.length > 0
        }
        // -- WP ADETQ -- when the kept layout was confirmed, detected or kept
        Text {
            objectName: "SpeakersPage.stamp"
            visible: text.length > 0
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: page.sc.stampText
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        Text {
            objectName: "SpeakersPage.readOnly"
            visible: DeviceModel.connected && !page.sc.editable
            Layout.fillWidth: true
            text: qsTr("This connection may not change the DSPi: the speaker layout is shown, not changed.")
            font: Theme.fontBody
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        SpeakersTable {
            Layout.fillWidth: true
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "SpeakersPage.confirm"
                text: qsTr("Confirm")
                primary: true
                enabled: page.sc.editable && page.sc.loaded && !page.confirmed
                onClicked: page.sc.confirm()
            }
            AppButton {
                objectName: "SpeakersPage.suggestion"
                text: qsTr("Use the Suggestion")
                enabled: page.sc.editable && page.sc.loaded
                onClicked: page.sc.useSuggestion()
            }
            // -- WP ADETQ -- (AUTODES 4.4) the app's own detection, after its question
            AppButton {
                objectName: "SpeakersPage.detect"
                text: qsTr("Detect…")
                enabled: page.sc.loaded && page.sc.detectBlocker.length === 0
                toolTip: page.sc.detectBlocker.length ? page.sc.detectBlocker
                         : qsTr("Find out which outputs reach a driver and which drive a tweeter, with quiet test sweeps and the microphone")
                onClicked: detectQuestion.open()
            }
            Text {
                objectName: "SpeakersPage.state"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: page.confirmed ? qsTr("Confirmed.")
                      : page.sc.edited && page.sc.status !== "none" ? qsTr("Changed: not kept until you confirm.") : ""
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
        }
        // -- WP ADETQ -- while the detection runs: where it is, and Stop
        RowLayout {
            objectName: "SpeakersPage.detecting"
            visible: page.sc.detecting
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            Text {
                objectName: "SpeakersPage.detectText"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: page.sc.detectText
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.Wrap
            }
            AppButton {
                objectName: "SpeakersPage.stopDetect"
                text: qsTr("Stop")
                onClicked: page.sc.stopDetect()
            }
        }
        // -- WP ADETQ -- the detection's summary, and its evidence on request
        ColumnLayout {
            id: found
            objectName: "SpeakersPage.detection"
            readonly property var d: page.sc.detection
            property bool open: false
            visible: d.summary !== undefined
            Layout.fillWidth: true
            spacing: 4
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacingSmall
                Text {
                    objectName: "SpeakersPage.summary"
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    //: %1 the speaker detection's summary ("2 tweeters, 2 woofers; nothing heard on Out 4.")
                    text: found.d.summary === undefined ? "" : qsTr("Detection: %1").arg(found.d.summary)
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
                AppButton {
                    objectName: "SpeakersPage.details"
                    text: found.open ? qsTr("Hide Details") : qsTr("Show Details")
                    onClicked: found.open = !found.open
                }
            }
            GridLayout {
                objectName: "SpeakersPage.detailsTable"
                visible: found.open
                Layout.fillWidth: true
                columns: 6
                columnSpacing: Theme.spacing
                rowSpacing: 2
                Repeater {
                    model: [qsTr("Output"), qsTr("Found"), qsTr("Heard at"), qsTr("Top falls"), qsTr("−6 dB at"), qsTr("Lowest played")]
                    delegate: Text {
                        required property string modelData
                        textFormat: Text.PlainText
                        text: modelData
                        font: Theme.fontCaption
                        color: Theme.textMuted
                    }
                }
                Repeater {
                    model: {
                        const cells = []
                        for (const r of (found.d.rows || []))
                            cells.push(r.label, r.klass, r.heard, r.topFall, r.fLow, r.lowest)
                        return cells
                    }
                    delegate: Text {
                        required property string modelData
                        required property int index
                        objectName: "SpeakersPage.detail." + Math.floor(index / 6) + "." + (index % 6)
                        textFormat: Text.PlainText
                        text: modelData
                        font: Theme.fontSmall
                        color: Theme.text
                    }
                }
            }
        }
        Text {
            objectName: "SpeakersPage.said"
            visible: text.length > 0
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: page.sc.said
            font: Theme.fontSmall
            color: page.sc.saidError ? Theme.dangerText : Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    // -- WP ADETQ -- the one question before the app's own detection (AUTODES 4.4)
    QuestionDialog {
        id: detectQuestion
        objectName: "SpeakersPage.detectQuestion"
        parent: page.Window.window ? page.Window.window.contentItem : null
        ofDevice: true
        title: page.sc.detectHeading
        text: page.sc.detectQuestion
        acceptText: qsTr("Detect")
        onAnswered: answer => { if (answer === "accept") page.sc.detect() }
    }
}
