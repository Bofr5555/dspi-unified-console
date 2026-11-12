import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// AutoEQ > Browse Profiles... (AutoEQBrowser, 600×500): search the headphone
// database, pick one, Apply sends its profile to inputs 1 and 2. The heart on
// a row adds it to AutoEQ > Favorite Profiles.
ToolWindow {
    id: window
    objectName: "AutoEqWindow"
    windowId: "autoeq"
    title: qsTr("AutoEQ - Browse Profiles")
    defaultWidth: 600
    defaultHeight: 500
    minimumWidth: 450
    minimumHeight: 350

    property string selectedId
    readonly property var selected: selectedId.length ? AutoEqController.profile(selectedId) : ({})
    readonly property bool hasSelection: selected.name !== undefined

    function applySelected() {
        if (!hasSelection || !AutoEqController.canApply) return
        AutoEqController.apply(selectedId)
        close()
    }
    onVisibleChanged: if (visible) search.forceActiveFocus()

    // The selection follows the list: a profile the search hides is not applied
    Connections {
        target: AutoEqController.profiles
        function onModelReset() {
            if (window.selectedId.length && AutoEqController.profiles.rowOf(window.selectedId) < 0) window.selectedId = ""
            list.currentIndex = AutoEqController.profiles.rowOf(window.selectedId)
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Search
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: searchRow.implicitHeight + 20
            color: Theme.panel
            RowLayout {
                id: searchRow
                anchors.fill: parent
                anchors.margins: 10
                spacing: Theme.spacingSmall
                Icon {
                    name: "search"
                    color: Theme.textMuted
                }
                BorderedField {
                    id: search
                    objectName: "AutoEqWindow.search"
                    Layout.fillWidth: true
                    placeholderText: qsTr("Search headphones...")
                    text: AutoEqController.profiles.searchText
                    onTextEdited: AutoEqController.profiles.searchText = text
                    Accessible.name: qsTr("Search headphones")
                    Keys.onDownPressed: {
                        list.forceActiveFocus()
                        if (list.currentIndex < 0 && list.count > 0) list.currentIndex = 0
                    }
                    Keys.onReturnPressed: window.applySelected()
                    Keys.onEscapePressed: {
                        if (text.length) AutoEqController.profiles.searchText = ""
                        else window.close()
                    }
                }
                IconButton {
                    objectName: "AutoEqWindow.clearSearch"
                    visible: search.text.length > 0
                    iconName: "circle-x"
                    iconColor: Theme.textMuted
                    toolTip: qsTr("Clear search")
                    onClicked: {
                        AutoEqController.profiles.searchText = ""
                        search.forceActiveFocus()
                    }
                }
            }
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }

        // Results
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ListView {
                id: list
                objectName: "AutoEqWindow.list"
                anchors.fill: parent
                clip: true
                visible: count > 0
                model: AutoEqController.profiles
                currentIndex: -1
                boundsBehavior: Flickable.StopAtBounds
                keyNavigationEnabled: true
                ScrollBar.vertical: AppScrollBar {}
                Accessible.role: Accessible.List
                Accessible.name: qsTr("Headphones")
                onCurrentIndexChanged: if (currentIndex >= 0) window.selectedId = AutoEqController.profiles.idAt(currentIndex)
                Keys.onReturnPressed: window.applySelected()
                Keys.onEscapePressed: window.close()
                delegate: AutoEqRow {
                    selected: profileId === window.selectedId
                    onChosen: {
                        list.currentIndex = index
                        window.selectedId = profileId
                    }
                }
            }

            // Nothing to list: why
            Column {
                anchors.centerIn: parent
                width: parent.width - 2 * Theme.spacingLarge
                visible: list.count === 0
                spacing: Theme.spacingSmall
                Icon {
                    anchors.horizontalCenter: parent.horizontalCenter
                    visible: AutoEqController.loadProblem.length > 0
                    name: "triangle-alert"
                    size: 32
                    color: Theme.warning
                }
                Text {
                    objectName: "AutoEqWindow.empty"
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    font: Theme.fontBody
                    color: Theme.textMuted
                    text: AutoEqController.loadProblem.length > 0 ? AutoEqController.loadProblem
                        : search.text.length === 0 ? qsTr("Loading headphone database...")
                        //: %1 is the search text
                        : qsTr("No headphones found matching \"%1\"").arg(search.text)
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }

        // The selection and the answers
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: bottom.implicitHeight + 2 * Theme.spacingLarge
            color: Theme.window
            RowLayout {
                id: bottom
                anchors.fill: parent
                anchors.margins: Theme.spacingLarge
                spacing: Theme.spacingSmall

                Column {
                    Layout.fillWidth: true
                    spacing: 2
                    visible: window.hasSelection
                    Text {
                        objectName: "AutoEqWindow.selectedName"
                        width: parent.width
                        text: window.selected.name || ""
                        font: Theme.fontTitle
                        color: Theme.text
                        elide: Text.ElideRight
                    }
                    Row {
                        spacing: Theme.spacingSmall
                        Icon {
                            name: "audio-waveform"
                            size: 12
                            color: Theme.textMuted
                        }
                        Text {
                            text: window.selected.sourceName || ""
                            font: Theme.fontSmall
                            color: Theme.textMuted
                        }
                        Icon {
                            name: window.selected.formFactorIcon || "headphones"
                            size: 12
                            color: Theme.textMuted
                        }
                        Text {
                            text: window.selected.formFactor || ""
                            font: Theme.fontSmall
                            color: Theme.textMuted
                        }
                    }
                }
                Text {
                    Layout.fillWidth: true
                    visible: !window.hasSelection
                    text: qsTr("Select a headphone to apply its EQ profile")
                    font: Theme.fontBody
                    color: Theme.textMuted
                    elide: Text.ElideRight
                }
                AppButton {
                    objectName: "AutoEqWindow.cancel"
                    text: qsTr("Cancel")
                    onClicked: window.close()
                }
                AppButton {
                    objectName: "AutoEqWindow.apply"
                    text: qsTr("Apply")
                    primary: true
                    enabled: window.hasSelection && AutoEqController.canApply
                    onClicked: window.applySelected()
                }
            }
        }
    }
}
