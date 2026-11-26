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
// -- WP DL3a -- Modern (DESIGN-LANGUAGE §3, §9, §12): the search at the left
// over the list, the headphones as ListRows, and beside them (under them in a
// narrow window) the chosen one: its name, the pair picker with the one
// accent button, Apply, and the preview in a GraphFrame. No Cancel on a page;
// a window keeps one. Classic keeps its window as it was.
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
    onShownChanged: if (shown && body.item) body.item.focusSearch()

    // The selection follows the list: a profile the search hides is not applied
    Connections {
        target: AutoEqController.profiles
        function onModelReset() {
            if (panel.selectedId.length && AutoEqController.profiles.rowOf(panel.selectedId) < 0) panel.selectedId = ""
            if (body.item) body.item.listIndex = AutoEqController.profiles.rowOf(panel.selectedId)
        }
    }

    // -- INTC (DL3a fold) -- Modern's page: the search, its count and the ▾
    // menu in the tab row (PageHost.pageTools), with the same names; the body
    // keeps them in a window. The keys reach the list through the body.
    property Item toolSearch: null
    readonly property Component pageTools: Component {
        Row {
            objectName: "AutoEqTools"
            spacing: 6
            BorderedField {
                id: toolSearchField
                objectName: "AutoEqWindow.search"
                anchors.verticalCenter: parent.verticalCenter
                width: 240
                placeholderText: qsTr("Search headphones…")
                text: AutoEqController.profiles.searchText
                onTextEdited: AutoEqController.profiles.searchText = text
                Accessible.name: qsTr("Search headphones")
                Keys.onDownPressed: if (body.item) body.item.focusList()
                Keys.onReturnPressed: panel.applySelected()
                Keys.onEscapePressed: {
                    if (text.length) AutoEqController.profiles.searchText = ""
                    else panel.close()
                }
                Component.onCompleted: panel.toolSearch = toolSearchField
                Component.onDestruction: if (panel.toolSearch === toolSearchField) panel.toolSearch = null
            }
            IconButton {
                objectName: "AutoEqWindow.clearSearch"
                anchors.verticalCenter: parent.verticalCenter
                visible: toolSearchField.text.length > 0
                iconName: "circle-x"
                iconColor: Theme.textMuted
                toolTip: qsTr("Clear search")
                onClicked: {
                    AutoEqController.profiles.searchText = ""
                    toolSearchField.forceActiveFocus()
                }
            }
            Text {
                objectName: "AutoEqWindow.count"
                anchors.verticalCenter: parent.verticalCenter
                readonly property int count: body.item && body.item.count !== undefined ? body.item.count : 0
                visible: count > 0
                //: How many headphones the list shows; %1 is a number
                text: qsTr("%1 headphones").arg(count)
                font: Theme.fontSmall
                color: Theme.textMuted
            }
            // -- WP LAY3b -- what Classic's AutoEQ menu has besides
            IconButton {
                id: more
                objectName: "AutoEqWindow.more"
                anchors.verticalCenter: parent.verticalCenter
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

    // -- WP DL3a -- one body or the other, by layout
    Loader {
        id: body
        anchors.fill: parent
        sourceComponent: LayoutController.modern ? modernBody : classicBody
        onLoaded: if (panel.shown) item.focusSearch()
    }

    Component {
        id: classicBody
        ColumnLayout {
            function focusSearch() { search.forceActiveFocus() }
            property alias listIndex: list.currentIndex
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

    // -- WP DL3a --
    Component {
        id: modernRow
        ListRow {
            id: row
            required property int index
            required property string profileId
            required property string displayName
            required property string sourceName
            required property string formFactor
            required property bool favorite
            objectName: "AutoEqWindow.row." + index
            width: ListView.view.width
            rowIndex: index
            title: displayName
            //: A headphone's measurement source and its form factor
            subtitle: qsTr("%1 · %2").arg(sourceName).arg(formFactor)
            iconName: formFactor === "in-ear" || formFactor === "earbud" ? "ear" : "headphones"
            selected: profileId === panel.selectedId
            // The heart shows on hover, and always on a favourite
            actionsAlways: true
            onClicked: {
                ListView.view.currentIndex = index
                panel.selectedId = profileId
            }
            onActivated: {
                panel.selectedId = profileId
                panel.applySelected()
            }
            IconButton {
                objectName: "AutoEqWindow.favorite." + row.index
                opacity: row.hovered || row.selected || row.favorite || activeFocus ? 1 : 0
                iconName: "heart"
                iconSize: 14
                iconColor: row.favorite ? Theme.dangerText : Theme.textMuted
                toolTip: row.favorite ? qsTr("Remove from favorites") : qsTr("Add to favorites")
                onClicked: AutoEqController.flipFavourite(row.profileId)
            }
        }
    }

    Component {
        id: modernBody
        Item {
            id: modern
            // -- INTC -- on a page the search is in the tab row
            function focusSearch() { (panel.inPage && panel.toolSearch ? panel.toolSearch : search).forceActiveFocus() }
            function focusList() {
                list.forceActiveFocus()
                if (list.currentIndex < 0 && list.count > 0) list.currentIndex = 0
            }
            readonly property int count: list.count
            property alias listIndex: list.currentIndex
            // The chosen one beside the list when there is room for both
            readonly property bool wide: width >= Theme.contentMaxWidth + InspectorState.width + 3 * Theme.spacing

            GridLayout {
                anchors.fill: parent
                anchors.margins: Theme.spacing
                columns: modern.wide ? 2 : 1
                columnSpacing: Theme.spacingSection
                rowSpacing: Theme.spacing

                // ---- The search and the list ----
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.maximumWidth: Theme.contentMaxWidth
                    Layout.alignment: Qt.AlignLeft | Qt.AlignTop
                    spacing: Theme.spacingSmall

                    RowLayout {
                        Layout.fillWidth: true
                        visible: !panel.inPage   // -- INTC -- on a page: the tab row
                        spacing: Theme.spacingSmall
                        BorderedField {
                            id: search
                            objectName: "AutoEqWindow.search"
                            Layout.fillWidth: true
                            Layout.maximumWidth: 360
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
                        Item { Layout.fillWidth: true }
                        Text {
                            objectName: "AutoEqWindow.count"
                            visible: list.count > 0
                            //: How many headphones the list shows; %1 is a number
                            text: qsTr("%1 headphones").arg(list.count)
                            font: Theme.fontSmall
                            color: Theme.textMuted
                        }
                        // -- WP LAY3b -- AutoEQ's ▾ (on a page only: INTC, the tab row's)
                    }

                    Item {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        Layout.minimumHeight: 4 * Theme.rowHeight
                        ListView {
                            id: list
                            objectName: "AutoEqWindow.list"
                            anchors.fill: parent
                            clip: true
                            visible: count > 0
                            model: AutoEqController.profiles
                            currentIndex: -1
                            boundsBehavior: Flickable.StopAtBounds
                            activeFocusOnTab: true
                            keyNavigationEnabled: true
                            ScrollBar.vertical: AppScrollBar {}
                            Accessible.role: Accessible.List
                            Accessible.name: qsTr("Headphones")
                            onCurrentIndexChanged: if (currentIndex >= 0) panel.selectedId = AutoEqController.profiles.idAt(currentIndex)
                            Keys.onReturnPressed: panel.applySelected()
                            Keys.onEscapePressed: panel.close()
                            delegate: modernRow
                        }
                        EmptyState {
                            objectName: "AutoEqWindow.emptyState"
                            anchors.fill: parent
                            visible: list.count === 0
                            iconName: AutoEqController.loadProblem.length > 0 ? "triangle-alert"
                                    : search.text.length === 0 ? "loader-circle" : "search"
                            title: AutoEqController.loadProblem.length > 0 ? qsTr("The headphone database did not load")
                                 : search.text.length === 0 ? qsTr("Loading headphone database…")
                                 : qsTr("No headphones found")
                            text: AutoEqController.loadProblem.length > 0 ? AutoEqController.loadProblem
                                : search.text.length === 0 ? ""
                                //: %1 is the search text
                                : qsTr("Nothing matches \"%1\".").arg(search.text)
                            actionText: search.text.length > 0 && AutoEqController.loadProblem.length === 0 ? qsTr("Clear Search") : ""
                            onAction: AutoEqController.profiles.searchText = ""
                        }
                    }
                }

                // ---- The chosen one: where it goes, what it would do ----
                ColumnLayout {
                    objectName: "AutoEqWindow.previewArea"
                    Layout.fillWidth: true
                    Layout.fillHeight: modern.wide
                    Layout.maximumWidth: modern.wide ? -1 : Theme.contentMaxWidth
                    Layout.preferredWidth: modern.wide ? InspectorState.width * 1.5 : -1
                    Layout.alignment: Qt.AlignLeft | Qt.AlignTop
                    spacing: Theme.spacingSmall

                    SectionHeading {
                        Layout.fillWidth: true
                        text: qsTr("Headphone profile")
                    }
                    Text {
                        objectName: "AutoEqWindow.selectedName"
                        Layout.fillWidth: true
                        visible: panel.hasSelection
                        textFormat: Text.PlainText
                        text: panel.selected.name || ""
                        font: Theme.fontTitle
                        color: Theme.text
                        elide: Text.ElideRight
                    }
                    Text {
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        text: panel.hasSelection
                              //: A headphone's measurement source and its form factor
                              ? qsTr("%1 · %2").arg(panel.selected.sourceName || "").arg(panel.selected.formFactor || "")
                              : qsTr("Select a headphone to apply its EQ profile")
                        font: panel.hasSelection ? Theme.fontSmall : Theme.fontBody
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.topMargin: Theme.spacingSmall
                        spacing: Theme.spacingSmall
                        Text {
                            text: qsTr("Apply to")
                            font: Theme.fontBody
                            color: Theme.text
                        }
                        AppComboBox {
                            objectName: "AutoEqWindow.pair"
                            Layout.fillWidth: true
                            Layout.maximumWidth: 240
                            label: qsTr("Apply to")
                            model: AutoEqController.pairs
                            textRole: "title"
                            currentIndex: AutoEqController.pair
                            onActivated: index => AutoEqController.pair = index
                        }
                        AppButton {
                            objectName: "AutoEqWindow.apply"
                            text: qsTr("Apply")
                            primary: true
                            enabled: panel.hasSelection && AutoEqController.canApply
                            toolTip: qsTr("Put the profile on the chosen channels")
                            onClicked: panel.applySelected()
                        }
                        AppButton {
                            objectName: "AutoEqWindow.cancel"
                            visible: !panel.inPage
                            text: qsTr("Cancel")
                            onClicked: panel.close()
                        }
                        Item { Layout.fillWidth: true }
                    }
                    GraphFrame {
                        id: frame
                        objectName: "AutoEqWindow.preview"
                        Layout.fillWidth: true
                        Layout.topMargin: Theme.spacingSmall
                        Layout.preferredHeight: 180 + (hint.length ? 40 : 0)
                        readonly property var gridY: panel.previewShown ? panel.preview.gridY : []
                        readonly property var gridDb: panel.previewShown ? panel.preview.gridDb : []
                        xTicks: [100, 1000, 10000].map(hz => ({ at: EffectsController.freqX(hz), text: hz >= 1000 ? (hz / 1000) + "k" : String(hz) }))
                        yTicks: gridY.map((y, i) => ({ at: 1 - y, text: String(gridDb[i]) }))
                        empty: !panel.previewShown
                        emptyText: qsTr("Select a headphone to see what it would do")
                        hint: panel.previewShown
                              ? [panel.preview.levelText, panel.preview.notes].filter(t => t && t.length).join(" ")
                              : ""
                        Accessible.role: Accessible.Chart
                        Accessible.name: qsTr("The profile's response on the chosen channels")

                        // The curves (a Canvas: the software renderer draws a Shape outside its clip)
                        Canvas {
                            id: curves
                            anchors.fill: parent
                            renderStrategy: Canvas.Cooperative
                            readonly property var lines: panel.previewShown
                                ? panel.preview.curves.map(c => ({ points: c.points, color: c.index === 0 ? Theme.accentText : Theme.warning }))
                                : []
                            onLinesChanged: requestPaint()
                            onWidthChanged: requestPaint()
                            onHeightChanged: requestPaint()
                            onPaint: {
                                const ctx = getContext("2d")
                                ctx.reset()
                                ctx.lineWidth = 2
                                ctx.lineCap = "round"
                                ctx.lineJoin = "round"
                                for (const c of lines) {
                                    const pts = c.points || []
                                    if (pts.length < 4) continue
                                    ctx.strokeStyle = c.color
                                    ctx.beginPath()
                                    ctx.moveTo(pts[0] * width, height - pts[1] * height)
                                    for (let i = 2; i + 1 < pts.length; i += 2) ctx.lineTo(pts[i] * width, height - pts[i + 1] * height)
                                    ctx.stroke()
                                }
                            }
                        }
                    }
                    Item { Layout.fillHeight: true; visible: modern.wide }
                }
            }
        }
    }
}
