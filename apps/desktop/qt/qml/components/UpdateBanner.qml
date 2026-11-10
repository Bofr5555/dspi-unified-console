import QtQuick 2.15

// WP QD16: registered through Registrar in AppUpdater.cpp. Shown when an automatic
// check found a newer release that was not skipped; the notes are on the Updates
// settings page.
Item {
    id: banner
    objectName: "updateBanner"
    property bool shown: appUpdater.announce
    implicitHeight: 44

    Rectangle {
        anchors.fill: parent
        radius: 8
        color: Qt.rgba(0.04, 0.49, 1, 0.18)
        border.color: Qt.rgba(0.04, 0.49, 1, 0.5)
        Text {
            objectName: "updateBannerText"
            anchors { left: parent.left; leftMargin: 12; right: buttons.left; rightMargin: 12; verticalCenter: parent.verticalCenter }
            text: qsTr("DSPi uConsole %1 is available. You have %2.").arg(appUpdater.availableVersion).arg(appUpdater.currentVersion)
            textFormat: Text.PlainText
            color: "white"
            font.pixelSize: 12
            wrapMode: Text.WordWrap
            maximumLineCount: 2
            elide: Text.ElideRight
        }
        Row {
            id: buttons
            spacing: 6
            anchors { right: parent.right; rightMargin: 8; verticalCenter: parent.verticalCenter }
            AppButton {
                objectName: "updateBannerDetails"
                text: qsTr("Details")
                onClicked: { root.openSettingsPage("updates"); appUpdater.dismiss() }
            }
            AppButton {
                objectName: "updateBannerLater"
                text: qsTr("Later")
                onClicked: appUpdater.dismiss()
            }
        }
    }
}
