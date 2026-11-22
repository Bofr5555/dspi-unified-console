import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// AutoEQ > Browse Profiles... (AutoEQBrowser, 600×500): search the headphone
// database, pick one, Apply sends its profile to inputs 1 and 2. The heart on
// a row adds it to AutoEQ > Favorite Profiles.
// -- WP P1QT2 -- Before Apply: the channel pair it goes to (any two inputs or
// outputs, P1AQ) and the core's preview of what it would do, drawn on a graph
// (nothing is sent until Apply; the outputs' gain is left alone).
// -- WP LAY1b -- A panel (Panel.qml): PanelTable's row for autoeq.browse gives
// its window's title and sizes.
Panel {
    id: panel
    objectName: "AutoEqPanel"

    property string selectedId
    readonly property var selected: selectedId.length ? AutoEqController.profile(selectedId) : ({})
    readonly property bool hasSelection: selected.name !== undefined
    // -- WP P1QT2 -- the core's preview of the selection on the chosen pair
    readonly property var preview: AutoEqController.preview
    readonly property bool previewShown: hasSelection && preview.profileId === selectedId
    onSelectedIdChanged: AutoEqController.requestPreview(selectedId)

    function applySelected() {
        if (!hasSelection || !AutoEqController.canApply) return
        AutoEqController.apply(selectedId)
        close()
    }
    onShownChanged: if (shown) search.forceActiveFocus()

    // The selection follows the list: a profile the search hides is not applied
    Connections {
        target: AutoEqController.profiles
        function onModelReset() {
            if (panel.selectedId.length && AutoEqController.profiles.rowOf(panel.selectedId) < 0) panel.selectedId = ""
            list.currentIndex = AutoEqController.profiles.rowOf(panel.selectedId)
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
                    placeholderText: qsTr("Search headphones…")
                    text: AutoEqController.profiles.searchText
                    onTextEdited: AutoEqController.profiles.searchText = text
                    Accessible.name: qsTr("Search headphones")
                    Keys.onDownPressed: {
                        list.forceActiveFocus()
                        if (list.currentIndex < 0 && list.count > 0) list.currentIndex = 0
                    }
                    Keys.onReturnPressed: panel.applySelected()
                    Keys.onEscapePressed: {
                        if (text.length) AutoEqController.profiles.searchText = ""
                        else panel.close()
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
                // -- WP LAY3b -- what Classic's AutoEQ menu has besides
                IconButton {
                    id: more
                    objectName: "AutoEqWindow.more"
                    visible: panel.inPage
                    iconName: "chevron-down"
                    toolTip: qsTr("More")
                    onClicked: moreMenu.dropFrom(more)
                    ActionMenu {
                        id: moreMenu
                        objectName: "AutoEqWindow.menu"
                        place: "autoeq"
                        handlers: ({ "autoeq.clearFavorites": { enabled: () => AutoEqController.favouriteList.length > 0 } })
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
                onCurrentIndexChanged: if (currentIndex >= 0) panel.selectedId = AutoEqController.profiles.idAt(currentIndex)
                Keys.onReturnPressed: panel.applySelected()
                Keys.onEscapePressed: panel.close()
                delegate: AutoEqRow {
                    selected: profileId === panel.selectedId
                    onChosen: {
                        list.currentIndex = index
                        panel.selectedId = profileId
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
                        : search.text.length === 0 ? qsTr("Loading headphone database…")
                        //: %1 is the search text
                        : qsTr("No headphones found matching \"%1\"").arg(search.text)
                }
            }
        }

        // -- WP P1QT2 -- Where it goes, and what it would do there
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }
        ColumnLayout {
            objectName: "AutoEqWindow.previewArea"
            Layout.fillWidth: true
            Layout.margins: Theme.spacingLarge
            Layout.bottomMargin: Theme.spacing
            spacing: Theme.spacingSmall
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacingSmall
                Text {
                    text: qsTr("Apply to")
                    font: Theme.fontBody
                    color: Theme.text
                }
                AppComboBox {
                    objectName: "AutoEqWindow.pair"
                    Layout.preferredWidth: 260
                    label: qsTr("Apply to")
                    model: AutoEqController.pairs
                    textRole: "title"
                    currentIndex: AutoEqController.pair
                    onActivated: index => AutoEqController.pair = index
                }
                Item { Layout.fillWidth: true }
            }
            EffectGraph {
                objectName: "AutoEqWindow.preview"
                Layout.fillWidth: true
                implicitHeight: 130
                title: qsTr("The profile's response on the chosen channels")
                curves: panel.previewShown
                        ? panel.preview.curves.map(c => ({ points: c.points, color: c.index === 0 ? Theme.accentText : Theme.warning }))
                        : []
                gridDb: panel.previewShown ? panel.preview.gridDb : []
                gridY: panel.previewShown ? panel.preview.gridY : []
                badge: panel.previewShown ? qsTr("Preview") : ""
                Text {
                    anchors.centerIn: parent
                    visible: !panel.previewShown
                    text: qsTr("Select a headphone to see what it would do")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
            }
            Text {
                objectName: "AutoEqWindow.previewText"
                Layout.fillWidth: true
                visible: text.length > 0
                text: panel.previewShown
                      ? [panel.preview.levelText, panel.preview.notes].filter(t => t && t.length).join("\n")
                      : ""
                font: Theme.fontCaption
                color: Theme.textMuted
                wrapMode: Text.Wrap
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
                    visible: panel.hasSelection
                    Text {
                        objectName: "AutoEqWindow.selectedName"
                        width: parent.width
                        text: panel.selected.name || ""
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
                            text: panel.selected.sourceName || ""
                            font: Theme.fontSmall
                            color: Theme.textMuted
                        }
                        Icon {
                            name: panel.selected.formFactorIcon || "headphones"
                            size: 12
                            color: Theme.textMuted
                        }
                        Text {
                            text: panel.selected.formFactor || ""
                            font: Theme.fontSmall
                            color: Theme.textMuted
                        }
                    }
                }
                Text {
                    Layout.fillWidth: true
                    visible: !panel.hasSelection
                    text: qsTr("Select a headphone to apply its EQ profile")
                    font: Theme.fontBody
                    color: Theme.textMuted
                    elide: Text.ElideRight
                }
                AppButton {
                    objectName: "AutoEqWindow.cancel"
                    text: qsTr("Cancel")
                    onClicked: panel.close()
                }
                AppButton {
                    objectName: "AutoEqWindow.apply"
                    text: qsTr("Apply")
                    primary: true
                    enabled: panel.hasSelection && AutoEqController.canApply
                    onClicked: panel.applySelected()
                }
            }
        }
    }
}
