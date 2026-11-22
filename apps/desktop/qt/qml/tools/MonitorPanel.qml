import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP9 -- The Interrupt Monitor (the Mac's InterruptMonitorView, 900×480,
// at least 720×320): Pause, Clear and Copy, the state and the count, then one
// monospaced row per notification, newest at the bottom. The rows are the
// core's text with the local time in front (MonitorController).
// -- WP LAY0 -- A panel (Panel.qml): PanelTable's row for
// tools.interruptMonitor gives its window's title and sizes.
Panel {
    id: panel
    objectName: "MonitorPanel"

    readonly property var monitor: MonitorController

    onShownChanged: MonitorController.windowShown = shown
    Component.onCompleted: if (shown) MonitorController.windowShown = true   // -- WP LAY3 -- another host may show it
    Component.onDestruction: if (shown) MonitorController.windowShown = false   // -- WP LAY0 -- a page host unloaded

    Shortcut {
        sequence: "Ctrl+P"
        enabled: panel.shown && !panel.locked   // -- WP QTSENDER -- -- WP LAY0 -- only while it shows
        onActivated: MonitorController.togglePause()
    }
    // -- WP LAY6 -- as a terminal's; Ctrl+K is the command palette's
    Shortcut {
        sequence: "Ctrl+L"
        enabled: panel.shown && !panel.locked   // -- WP QTSENDER -- -- WP LAY0 -- only while it shows
        onActivated: MonitorController.clear()
    }

    Connections {
        target: MonitorController
        function onAppended() {
            if (!MonitorController.paused) log.positionViewAtEnd()
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.margins: 10
            spacing: Theme.spacing

            AppButton {
                objectName: "MonitorWindow.pause"
                text: MonitorController.paused ? qsTr("Resume") : qsTr("Pause")
                iconName: MonitorController.paused ? "play" : "pause"
                toolTip: qsTr("Ctrl+P")
                onClicked: MonitorController.togglePause()
            }
            AppButton {
                objectName: "MonitorWindow.clear"
                text: qsTr("Clear")
                iconName: "trash"
                toolTip: qsTr("Ctrl+L")
                onClicked: MonitorController.clear()
            }
            AppButton {
                objectName: "MonitorWindow.copy"
                text: qsTr("Copy")
                iconName: "copy"
                toolTip: qsTr("Copies the lines shown, oldest first")
                enabled: MonitorController.lines.count > 0
                onClicked: MonitorController.copyAll()
            }
            Item { Layout.fillWidth: true }
            Text {
                objectName: "MonitorWindow.status"
                readonly property string state: MonitorController.status
                property string shownName: text
                text: state === "inactive" ? qsTr("Inactive") : state === "paused" ? qsTr("Paused") : qsTr("Listening")
                color: state === "inactive" ? Theme.textMuted : state === "paused" ? Theme.warning : Theme.success
                font: Theme.fontSmall
            }
            Text {
                objectName: "MonitorWindow.count"
                readonly property int count: MonitorController.lines.count
                property string shownName: text
                //: The Interrupt Monitor's row count
                text: count === 1 ? qsTr("1 event") : qsTr("%1 events").arg(count)
                color: Theme.textMuted
                font: Theme.fontSmall
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }

        ListView {
            id: log
            objectName: "MonitorWindow.log"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 1
            topMargin: 8
            bottomMargin: 8
            leftMargin: 8
            rightMargin: 8
            boundsBehavior: Flickable.StopAtBounds
            model: MonitorController.lines
            ScrollBar.vertical: AppScrollBar {}
            Accessible.role: Accessible.List
            Accessible.name: qsTr("Notifications")

            delegate: TextEdit {
                required property string line
                width: ListView.view.width - 16
                text: line
                readOnly: true
                selectByMouse: true
                wrapMode: TextEdit.NoWrap
                font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontSmall.pixelSize })
                color: Theme.text
                selectionColor: Theme.textSelection
                selectedTextColor: Theme.text
            }
        }
    }
}
