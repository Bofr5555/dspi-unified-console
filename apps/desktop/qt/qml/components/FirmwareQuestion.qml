import QtQuick 2.15
import QtQuick.Controls 2.15

// The firmware guard's question (WP QD11): a risk, and the words it takes. The
// answer has to be typed into a real field, and it goes to the core's guard, which
// alone compares it. Used by Firmware Update and by Getting Started.
Rectangle {
    id: card
    property string message: firmware.questionMessage
    property string phrase: firmware.questionPhrase
    property bool wrong: firmware.answerWrong

    radius: 8
    color: isMacOS ? MacColors.opacity(MacColors.controlBackground, 0.6) : Qt.rgba(1, 1, 1, 0.045)
    border.color: isMacOS ? MacColors.opacity(MacColors.gray, 0.2) : Qt.rgba(1, 1, 1, 0.07)

    // A different risk starts from an empty field
    onPhraseChanged: field.text = ""
    onVisibleChanged: if (visible) { field.text = ""; field.forceActiveFocus() }

    function submit() { firmware.answer(field.text) }

    Column {
        x: 14
        y: 12
        width: parent.width - 28
        spacing: 8
        Row {
            spacing: 6
            Icon { name: "warning"; size: 14; color: isMacOS ? MacColors.orange : "#ff9f0a"; anchors.verticalCenter: parent.verticalCenter }
            Text { text: qsTr("Core voltage risk"); font.pixelSize: 13; font.weight: Font.DemiBold; color: isMacOS ? MacColors.label : "white" }
        }
        Text {
            width: parent.width
            wrapMode: Text.WordWrap
            textFormat: Text.PlainText
            text: card.message
            font.pixelSize: 11
            color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.75)
        }
        Text {
            width: parent.width
            wrapMode: Text.WordWrap
            textFormat: Text.PlainText
            text: qsTr("To go ahead anyway, type this exactly: %1").arg(card.phrase)
            font.pixelSize: 11
            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.55)
        }
        TextField {
            Accessible.name: qsTr("Confirmation phrase")
            id: field
            objectName: "firmwareAnswer"
            width: parent.width
            height: 30
            topPadding: 0
            bottomPadding: 0
            leftPadding: 10
            rightPadding: 10
            verticalAlignment: Text.AlignVCenter
            font.pixelSize: 13
            color: isMacOS ? MacColors.label : "white"
            selectionColor: isMacOS ? MacColors.selectedControl : "#0a7cff"
            selectedTextColor: "white"
            selectByMouse: true
            inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
            onAccepted: card.submit()
            background: Rectangle {
                radius: 7
                color: isMacOS ? MacColors.controlBackground : Qt.rgba(1, 1, 1, 0.06)
                border.color: field.activeFocus ? (isMacOS ? MacColors.accent : "#0a7cff") : Qt.rgba(1, 1, 1, 0.14)
            }
        }
        Text {
            visible: card.wrong
            width: parent.width
            wrapMode: Text.WordWrap
            text: qsTr("That is not it. Case and spaces count.")
            font.pixelSize: 11
            color: isMacOS ? MacColors.orange : "#ff9f0a"
        }
        Row {
            spacing: 8
            AppButton {
                objectName: "firmwareAnswerGo"
                text: qsTr("Go Ahead")
                enabled: field.text.length > 0
                onClicked: card.submit()
            }
            AppButton {
                objectName: "firmwareAnswerCancel"
                text: qsTr("Don't Install")
                onClicked: firmware.reset()
            }
        }
    }
}
