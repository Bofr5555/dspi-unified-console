import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP2 -- A channel's band list in a card (the Mac's FilterListView): the
// column header pinned on top, the rows (BandRow) scrolling under it, and a
// footer with Enable All / Bypass All (firmware 1.1.4+), Clear All and, on an
// output page, the PEQ / XO tabs.
//
//     FilterList { channel: 3; crossover: false; includeLinkwitz: true; showClear: true }
//
// The on-graph editor (QP4) passes `graphSelection`, any object with:
//   revision                    a number it changes whenever any of below changes
//   isSelected(band) -> bool    the graph has this PEQ band selected
//   bandColor(band) -> color    the band's graph colour
//   live(band) -> {freq, gain, q} | null   values while the graph drags it
//   listClick(band, modifiers)  a click on a row's number
//   listHovered                 a writable band number, -1 when none
// and calls showBand(band) to bring a band's row into view. Crossover lists
// take no selection.
Rectangle {
    id: list
    property int channel: -1
    property bool crossover: false
    property bool includeLinkwitz: false
    property bool showClear: true
    // The PEQ / XO tabs in the footer (output pages, firmware with crossovers)
    property bool showTabs: false
    property bool crossoverTab: false
    property var graphSelection: null
    readonly property int bandCount: crossover ? 4 : 10
    readonly property bool bypassSupported: ChannelController.bypassSupported
    // Off bands have nothing to bypass: they count for neither half
    readonly property var actionable: {
        const out = []
        if (ChannelController.revision < 0) return out
        for (let b = 0; b < bandCount; ++b) {
            const p = ChannelController.band(channel, b, crossover)
            if (p.type !== 0) out.push(p)
        }
        return out
    }
    readonly property bool allBypassed: actionable.every(p => p.bypass)
    readonly property bool allEnabled: actionable.every(p => !p.bypass)
    signal tabChosen(bool crossover)
    // -- WP LAY4 -- Modern's table: denser rows, no outline; an input's Clear
    // clears its linked partner too
    property int rowHeight: 36
    property bool flat: false
    property bool clearsInput: false
    property string clearText: qsTr("Clear All")

    function showBand(band) {
        const row = rows.itemAt(band)
        if (!row) return
        const top = row.y, bottom = row.y + row.height
        if (top < flick.contentY) flick.contentY = top
        else if (bottom > flick.contentY + flick.height) flick.contentY = bottom - flick.height
    }

    objectName: crossover ? "FilterList.xo" : "FilterList"
    radius: Theme.radius
    color: Theme.panel
    border.width: list.flat ? 0 : 1
    border.color: Theme.separator
    clip: true
    implicitHeight: header.height + rowsColumn.implicitHeight + footer.height

    // ---- Column header ----
    Rectangle {
        id: header
        z: 1
        width: parent.width
        height: list.flat ? 26 : 32
        color: Theme.panel
        radius: Theme.radius
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Theme.spacingLarge
            anchors.rightMargin: Theme.spacingSmall
            spacing: Theme.spacing
            component Heading: Text {
                font: Theme.fontHeading
                color: Theme.textMuted
            }
            Item { visible: list.bypassSupported; Layout.preferredWidth: 18 }
            Heading { text: "#"; Layout.preferredWidth: 24 }
            Heading { text: list.crossover ? qsTr("FAMILY") : qsTr("TYPE"); Layout.preferredWidth: list.crossover ? 130 : 140 }
            Heading { visible: list.crossover; text: qsTr("TYPE"); Layout.preferredWidth: 100 }
            Heading { visible: list.crossover; text: qsTr("SLOPE"); Layout.preferredWidth: 100 }
            Heading { text: qsTr("FREQ"); Layout.preferredWidth: 104; horizontalAlignment: Text.AlignHCenter }
            Heading { visible: !list.crossover; text: qsTr("GAIN"); Layout.preferredWidth: 84; horizontalAlignment: Text.AlignHCenter }
            Heading { visible: !list.crossover; text: qsTr("WIDTH"); Layout.preferredWidth: 74; horizontalAlignment: Text.AlignHCenter }
            Item { Layout.fillWidth: true }
        }
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.separator }
    }

    // ---- Rows ----
    Flickable {
        id: flick
        objectName: list.objectName + ".rows"
        anchors.top: header.bottom
        anchors.bottom: footer.top
        width: parent.width
        contentWidth: width
        contentHeight: rowsColumn.implicitHeight
        boundsBehavior: Flickable.StopAtBounds
        clip: true
        ScrollBar.vertical: AppScrollBar {}
        Column {
            id: rowsColumn
            width: flick.width
            spacing: 1
            Repeater {
                id: rows
                model: list.bandCount
                BandRow {
                    required property int index
                    objectName: list.objectName + ".band." + index
                    width: rowsColumn.width
                    rowHeight: list.rowHeight
                    channel: list.channel
                    band: index
                    crossover: list.crossover
                    includeLinkwitz: list.includeLinkwitz
                    readonly property var sel: list.graphSelection && !list.crossover ? list.graphSelection : null
                    selected: sel ? (sel.revision < 0 ? false : sel.isSelected(index)) : false
                    bandColor: sel ? (sel.revision < 0 ? undefined : sel.bandColor(index)) : "transparent"
                    readonly property var liveValues: sel ? (sel.revision < 0 ? undefined : sel.live(index)) : null
                    liveFreq: liveValues ? liveValues.freq : NaN
                    liveGain: liveValues ? liveValues.gain : NaN
                    liveQ: liveValues ? liveValues.q : NaN
                    onNumberClicked: (band, modifiers) => { if (sel) sel.listClick(band, modifiers) }
                    onHoverChanged: (band, hovering) => {
                        if (!sel) return
                        if (hovering) sel.listHovered = band
                        else if (sel.listHovered === band) sel.listHovered = -1
                    }
                }
            }
        }
    }

    // ---- Footer ----
    Rectangle {
        id: footer
        anchors.bottom: parent.bottom
        width: parent.width
        height: footerRow.implicitHeight + 2 * Theme.spacingSmall
        color: Theme.panel
        radius: Theme.radius
        Rectangle { width: parent.width; height: 1; color: Theme.separator }
        RowLayout {
            id: footerRow
            anchors.fill: parent
            anchors.leftMargin: Theme.spacingLarge
            anchors.rightMargin: Theme.spacingSmall
            spacing: Theme.spacingSmall

            // Two one-shot actions; a half is dimmed when it would change nothing
            Row {
                visible: list.bypassSupported
                AppButton {
                    objectName: list.objectName + ".enableAll"
                    text: qsTr("Enable All")
                    enabled: !list.allEnabled && ChannelController.writable
                    toolTip: enabled ? text : ""
                    onClicked: ChannelController.setAllBypassed(list.channel, list.crossover, false)
                }
                AppButton {
                    objectName: list.objectName + ".bypassAll"
                    text: qsTr("Bypass All")
                    enabled: !list.allBypassed && ChannelController.writable
                    toolTip: enabled ? text : ""
                    onClicked: {
                        // A crossover protects the drivers behind it: ask first
                        if (list.crossover) bypassQuestion.open()
                        else ChannelController.setAllBypassed(list.channel, false, true)
                    }
                }
            }
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: list.objectName + ".clearAll"
                visible: list.showClear
                text: list.clearText
                toolTip: qsTr("Reset all bands")
                enabled: ChannelController.writable
                onClicked: clearQuestion.open()
            }
            AppTabBar {
                objectName: list.objectName + ".tabs"
                visible: list.showTabs
                label: qsTr("Band list")
                currentIndex: list.crossoverTab ? 1 : 0
                AppTabButton {
                    objectName: list.objectName + ".tab.peq"
                    width: 52
                    text: qsTr("PEQ")
                    onClicked: list.tabChosen(false)
                }
                AppTabButton {
                    objectName: list.objectName + ".tab.xo"
                    width: 52
                    text: qsTr("XO")
                    onClicked: list.tabChosen(true)
                }
            }
        }
    }

    QuestionDialog {
        id: bypassQuestion
        objectName: list.objectName + ".bypassQuestion"
        ofDevice: true   // -- WP MDQT2a --
        parent: Overlay.overlay
        title: qsTr("Bypass this output's crossovers?")
        text: qsTr("This sends full-range audio to this output with no crossover protection, which can damage unprotected drivers such as tweeters. Continue only if you are sure.")
        acceptText: qsTr("Bypass All")
        destructive: true
        onAnswered: answer => { if (answer === "accept") ChannelController.setAllBypassed(list.channel, true, true) }
    }
    QuestionDialog {
        id: clearQuestion
        objectName: list.objectName + ".clearQuestion"
        ofDevice: true   // -- WP MDQT2a --
        parent: Overlay.overlay
        title: qsTr("Clear All Bands?")
        // -- WP P1QT2 -- one undo step now (Edit > Undo puts every band back)
        // -- WP LAY7 -- (review RC, RC11) Modern has no Edit menu (its words are ContextMenus')
        text: list.clearsInput && ChannelController.revision >= 0 && ChannelController.linkedPartner(list.channel) >= 0 // -- WP LAY4 --
              ? (LayoutController.modern
                 ? qsTr("Every band on both channels of the linked pair will be reset to its default (flat) state. Undo puts them back.")
                 : qsTr("Every band on both channels of the linked pair will be reset to its default (flat) state. Edit > Undo puts them back."))
              : LayoutController.modern
                ? qsTr("Every band in this list will be reset to its default (flat) state. Undo puts them back.")
                : qsTr("Every band in this list will be reset to its default (flat) state. Edit > Undo puts them back.")
        acceptText: qsTr("Clear All")
        onAnswered: answer => {
            if (answer !== "accept") return
            if (list.clearsInput) ChannelController.clearInputPeq(list.channel) // -- WP LAY4 --
            else ChannelController.clearBands(list.channel, list.crossover)
        }
    }

    Accessible.role: Accessible.Table
    Accessible.name: crossover ? qsTr("Crossover bands") : qsTr("Filter bands")
}
