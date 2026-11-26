import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP6 -- The matrix mixer (the Mac's MatrixMixerView): a column per
// output and a row per source.
// -- WP MX1 -- Redrawn after DESREV's matrix-mockup.html (owner-approved
// 2026-11-24), the same panel on the Routing page and in its window (Classic,
// and "Open in Window"):
//
//   corner    FROM INPUTS ↓ · TO OUTPUTS →: rows feed columns
//   headers   an output's name, OUTn with its menu (Identify, Rename, Copy /
//             Paste Parameters), enable and mute (MatrixColumnHeader); an
//             input's colour, name and trim (MatrixRowHeader)
//   cells     fixed tiles with the gain and ø inside, dots where nothing is
//             connected (MatrixCell); a hairline after each input and output
//             pair; every second row striped; the row and column of the cell
//             under the pointer or the keyboard lit
//   tails     "→ Out 1, 2, 3" at the end of each row
//   footers   each output's gain and delay, and what feeds it
//   tools     Link L/R pairs, Direct 1:1, Clear, Dense (MatrixTools): over
//             the grid in a window, in the row over the page on a page
//
// The headers stay in place while the grid scrolls under them. Normal cells
// are 56 × 40; the dense grid (chosen, or past 12 outputs) has 44 × 32 cells
// and no trims and footers.
//
// Mouse: see MatrixCell; while a gain is typed, a click on another connected
// tile types that one's gain next.
// Keyboard (on the grid): arrows, Home / End (first / last column), PgUp /
// PgDn (first / last row) move; Space connects or disconnects; Enter, a
// digit, -, +, . or , types the gain (connecting first; -- WP REVDFIX -- REVD
// G1: "-6" is −6 dB, as on the web); Alt with + or - steps it by 0.5 dB,
// Ctrl by 3 dB; I inverts the phase; M mutes and E enables the column's
// output; Delete disconnects; Shift+F10 or the menu key opens the cell's menu; Escape
// leaves the cell. The crosspoint the grid is on is MatrixController's
// current one, which the Routing inspector shows.
//
// Everything that writes is disabled on read-only firmware. Opened by
// Tools > Matrix Mixer... and the quick strip.
// -- WP LAY1a -- A panel (Panel.qml): PanelTable's row for tools.matrix gives
// its window's title and sizes.
Panel {
    id: panel
    objectName: "MatrixPanel"

    readonly property int cols: MatrixController.columns
    readonly property int rows: MatrixController.rows
    readonly property bool dense: MatrixController.dense || cols > 12
    readonly property int cellWidth: dense ? 44 : 56
    readonly property int cellHeight: dense ? 32 : 40
    readonly property int tileWidth: dense ? 36 : 48
    readonly property int tileHeight: dense ? 22 : 28
    readonly property int rowHeaderWidth: dense ? 120 : 160
    readonly property int markWidth: 12
    readonly property int tailWidth: dense ? 110 : 150
    readonly property int headerHeight: dense ? 52 : 78
    readonly property int footerRowHeight: 26
    readonly property int footerHeight: dense ? 0 : 8 + 2 * footerRowHeight + 24
    readonly property int toolsHeight: inPage ? 0 : 28 + Theme.spacing
    readonly property int margin: Theme.spacing
    readonly property int leftWidth: rowHeaderWidth + markWidth
    readonly property int bodyWidth: cols * cellWidth + tailWidth
    readonly property int bodyHeight: rows * cellHeight
    readonly property int naturalWidth: leftWidth + bodyWidth
    readonly property int naturalHeight: toolsHeight + headerHeight + bodyHeight + footerHeight

    // The output being renamed in its header, -1 for none
    property int renaming: -1
    // -- WP LAY5a -- On a page, an output's header and an input's label select
    // their channel, which the inspector beside the matrix shows
    readonly property int selected: inPage ? SidebarController.selectedChannel : -1
    // The output whose header menu is open
    property int menuOutput: -1
    // The crosspoint whose gain is typed in its tile, and the key that began it
    property int editInput: -1
    property int editOutput: -1
    property string editSeed: ""
    // The cell under the pointer
    property int hoverInput: -1
    property int hoverOutput: -1
    readonly property bool gridFocused: gridFocus.activeFocus
    // The row and column lit: the pointer's, else the keyboard's
    readonly property int litInput: hoverInput >= 0 ? hoverInput : gridFocused || inPage ? MatrixController.currentInput : -1
    readonly property int litOutput: hoverInput >= 0 ? hoverOutput : gridFocused || inPage ? MatrixController.currentOutput : -1

    function column(o) { return MatrixController.revision < 0 ? undefined : MatrixController.column(o) }
    function source(r) { return MatrixController.revision < 0 ? undefined : MatrixController.row(r) }
    function cellOf(i, o) { return MatrixController.model.version < 0 ? undefined : MatrixController.model.get(i, o, "connected") }

    // -- WP QPI5 -- Its window is fitted to the grid on every show, as the
    // Mac's is, within the screen (beyond it the grid scrolls)
    showSize: cols > 0 ? Qt.size(naturalWidth + 2 * margin, naturalHeight + 2 * margin) : Qt.size(0, 0)

    // ---- Selection and editing ------------------------------------------------
    function selectChannel(channel) {
        MatrixController.setCurrent(-1, -1)
        SidebarController.select(channel)
    }
    // A gain being typed ends first: its field lets go of the focus and sends
    // what was typed (BorderedField), else the scope would hand it the focus again
    function focusGrid() {
        editInput = -1
        editOutput = -1
        gridFocus.forceActiveFocus()
    }
    // A click: an empty cell connects; a connected one is selected, and
    // disconnected once it is the selected one
    function cellClicked(i, o) {
        const wasCurrent = MatrixController.currentInput === i && MatrixController.currentOutput === o
        const on = MatrixController.model.get(i, o, "connected") === true
        // While a gain is typed, a click on another tile types its gain next
        if (editInput >= 0 && on) {
            beginEdit(i, o, "")
            return
        }
        MatrixController.setCurrent(i, o)
        focusGrid()
        if (!on) MatrixController.setConnected(i, o, true)
        else if (wasCurrent) MatrixController.setConnected(i, o, false)
    }
    function beginEdit(i, o, seed) {
        if (!MatrixController.writable) return
        MatrixController.setCurrent(i, o)
        if (MatrixController.model.get(i, o, "connected") !== true) MatrixController.setConnected(i, o, true)
        reveal(i, o)
        // The one typed before lets go first (and sends what was typed in it)
        editInput = -1
        editOutput = -1
        editSeed = seed
        editInput = i
        editOutput = o
    }
    // Enter: back to the grid
    function endEdit() { focusGrid() }
    // The field lost the focus (Escape, a click elsewhere)
    function editLeft() {
        editInput = -1
        editOutput = -1
    }
    function hovering(i, o, on) {
        if (on) {
            hoverInput = i
            hoverOutput = o
        } else if (hoverInput === i && hoverOutput === o) {
            hoverInput = -1
            hoverOutput = -1
        }
    }
    function move(di, dout) {
        let i = MatrixController.currentInput, o = MatrixController.currentOutput
        if (i < 0) { i = 0; o = 0 }
        else {
            i = Math.max(0, Math.min(rows - 1, i + di))
            o = Math.max(0, Math.min(cols - 1, o + dout))
        }
        MatrixController.setCurrent(i, o)
        reveal(i, o)
    }
    function moveTo(i, o) {
        MatrixController.setCurrent(i, o)
        reveal(i, o)
    }
    // The cell in sight in the grid
    function reveal(i, o) {
        const x = o * cellWidth, y = i * cellHeight
        if (x < flick.contentX) flick.contentX = x
        else if (x + cellWidth > flick.contentX + flick.width) flick.contentX = Math.min(Math.max(0, flick.contentWidth - flick.width), x + cellWidth - flick.width)
        if (y < flick.contentY) flick.contentY = y
        else if (y + cellHeight > flick.contentY + flick.height) flick.contentY = Math.min(Math.max(0, flick.contentHeight - flick.height), y + cellHeight - flick.height)
    }
    function stepGain(i, o, db) {
        if (MatrixController.model.get(i, o, "connected") !== true) return
        const g = MatrixController.model.get(i, o, "gain")
        MatrixController.setGain(i, o, Math.round((g + db) * 2) / 2)
    }
    function keyPressed(event) {
        const i = MatrixController.currentInput, o = MatrixController.currentOutput
        const ctrl = (event.modifiers & Qt.ControlModifier) !== 0
        const alt = (event.modifiers & Qt.AltModifier) !== 0   // -- WP REVDFIX --
        switch (event.key) {
        case Qt.Key_Left: move(0, -1); break
        case Qt.Key_Right: move(0, 1); break
        case Qt.Key_Up: move(-1, 0); break
        case Qt.Key_Down: move(1, 0); break
        case Qt.Key_Home: moveTo(Math.max(0, i), 0); break
        case Qt.Key_End: moveTo(Math.max(0, i), cols - 1); break
        case Qt.Key_PageUp: moveTo(0, Math.max(0, o)); break
        case Qt.Key_PageDown: moveTo(rows - 1, Math.max(0, o)); break
        case Qt.Key_Escape:
            if (i < 0) return false
            MatrixController.setCurrent(-1, -1)
            break
        default:
            if (i < 0) return false
            switch (event.key) {
            case Qt.Key_Space: MatrixController.setConnected(i, o, MatrixController.model.get(i, o, "connected") !== true); break
            case Qt.Key_Return:
            case Qt.Key_Enter: beginEdit(i, o, ""); break
            // -- WP REVDFIX -- (REVD G1) a step takes a modifier; a bare - or +
            // starts a typed gain as a digit does ("-6" stepped, then typed +6 dB)
            case Qt.Key_Plus:
            case Qt.Key_Equal:
            case Qt.Key_Minus:
                if (ctrl || alt) stepGain(i, o, (event.key === Qt.Key_Minus ? -1 : 1) * (ctrl ? 3 : 0.5))
                else if (event.key !== Qt.Key_Equal) beginEdit(i, o, event.key === Qt.Key_Minus ? "-" : "+")
                else return false
                break
            case Qt.Key_I:
                if (MatrixController.model.get(i, o, "connected") === true)
                    MatrixController.setInvert(i, o, MatrixController.model.get(i, o, "invert") !== true)
                break
            case Qt.Key_M: MatrixController.toggleMute(o); break
            case Qt.Key_E: MatrixController.toggleEnable(o); break
            case Qt.Key_Delete:
            case Qt.Key_Backspace: MatrixController.setConnected(i, o, false); break
            case Qt.Key_Menu: openCellMenu(i, o, null); break
            case Qt.Key_F10:
                if (!(event.modifiers & Qt.ShiftModifier)) return false
                openCellMenu(i, o, null)
                break
            default:
                // -- WP REVDFIX -- and so do "." and "," (a decimal)
                if (event.text.length === 1 && (event.text >= "0" && event.text <= "9" || event.text === "." || event.text === ",") && !ctrl)
                    beginEdit(i, o, event.text)
                else return false
            }
        }
        return true
    }

    // ---- Menus -------------------------------------------------------------------
    function openColumnMenu(o, item) {
        menuOutput = o
        columnMenu.popup(item, 0, item.height)
    }
    function openCellMenu(i, o, item) {
        MatrixController.setCurrent(i, o)
        if (!item) {
            const cell = cells.itemAt(i * cols + o)
            item = cell
        }
        cellMenu.input = i
        cellMenu.output = o
        if (item) cellMenu.popup(item, item.width / 2, item.height / 2)
        else cellMenu.popup()
    }

    Connections {
        target: MatrixController
        function onEnableQuestion(output, title, text, acceptText) {
            if (!panel.shown) return
            enableQuestion.output = output
            enableQuestion.title = title
            enableQuestion.text = text
            enableQuestion.acceptText = acceptText
            enableQuestion.open()
        }
    }
    // A row or a column chosen elsewhere: the grid leaves its crosspoint
    Connections {
        target: SidebarController
        enabled: panel.inPage
        function onSelectionChanged() {
            if (SidebarController.selectedChannel >= 0) MatrixController.setCurrent(-1, -1)
        }
    }

    // ---- No device (the window; a page has the page host's) ------------------------
    EmptyState {
        objectName: "Matrix.empty"
        anchors.centerIn: parent
        width: Math.min(parent.width - 2 * panel.margin, 420)
        visible: panel.cols === 0 && !panel.inPage
        iconName: "plug"
        title: qsTr("No device")
        text: qsTr("Connect a DSPi over USB, or open the demo, to see its routing.")
    }

    // A pair of hairlines' rule: after every second row and column, not after the last
    component PairRule: Rectangle {
        color: Theme.separator
    }
    component Heading: Text {
        textFormat: Text.PlainText
        font: Theme.fontHeading
        color: Theme.textMuted
    }

    Item {
        id: table
        objectName: "Matrix.table"
        x: panel.margin
        y: panel.margin
        visible: panel.cols > 0
        // The table's own size (its window fits it, -- WP LAY1a --); what of it
        // shows is the room the panel has, the grid scrolling under its headers
        width: panel.naturalWidth
        height: panel.naturalHeight
        readonly property real viewWidth: Math.min(panel.naturalWidth, Math.max(0, panel.width - 2 * panel.margin))
        readonly property real viewHeight: Math.min(panel.naturalHeight, Math.max(0, panel.height - 2 * panel.margin))

        MatrixTools {
            id: tools
            objectName: "Matrix.tools"
            visible: !panel.inPage
            prefix: "Matrix"
            height: 28
        }

        Item {
            id: area
            y: panel.toolsHeight
            width: table.viewWidth
            height: table.viewHeight - y

            // ---- The corner: which way the signal goes ------------------------
            Column {
                objectName: "Matrix.corner"
                x: 8
                width: panel.leftWidth - 8
                height: panel.headerHeight
                spacing: 2
                bottomPadding: 6
                Item { width: 1; height: Math.max(0, panel.headerHeight - 2 * cornerFrom.height - 8) }
                Heading {
                    id: cornerFrom
                    text: qsTr("From inputs ↓")
                }
                Heading {
                    text: qsTr("To outputs →")
                }
                Accessible.role: Accessible.StaticText
                Accessible.name: qsTr("Inputs, in rows, feed outputs, in columns")
            }

            // ---- Column headers, held at the top --------------------------------
            Item {
                x: panel.leftWidth
                width: Math.max(0, parent.width - x)
                height: panel.headerHeight
                clip: true
                Item {
                    x: -flick.contentX
                    width: panel.bodyWidth
                    height: parent.height
                    Rectangle {
                        visible: panel.litOutput >= 0
                        x: panel.litOutput * panel.cellWidth
                        width: panel.cellWidth
                        height: parent.height
                        radius: Theme.radiusControl
                        color: Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.08)
                    }
                    Row {
                        Repeater {
                            model: panel.cols
                            MatrixColumnHeader {
                                view: panel
                                width: panel.cellWidth
                                height: panel.headerHeight
                            }
                        }
                    }
                }
            }

            // ---- Row headers and the footer's labels, held at the left ------------
            Item {
                y: panel.headerHeight
                width: panel.leftWidth
                height: Math.max(0, parent.height - y)
                clip: true
                Item {
                    y: -flick.contentY
                    width: parent.width
                    height: panel.bodyHeight + panel.footerHeight
                    Rectangle {
                        visible: panel.litInput >= 0
                        y: panel.litInput * panel.cellHeight
                        width: panel.rowHeaderWidth
                        height: panel.cellHeight
                        color: Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.08)
                    }
                    Column {
                        Repeater {
                            model: panel.rows
                            MatrixRowHeader {
                                view: panel
                                width: panel.rowHeaderWidth
                                height: panel.cellHeight
                            }
                        }
                    }
                    // Link L/R pairs: a glyph joins the rows of each linked pair
                    Repeater {
                        model: panel.rows
                        Icon {
                            required property int index
                            objectName: "Matrix.pairMark." + index
                            visible: MatrixController.linkPairs && index % 2 === 0 && MatrixController.revision >= 0
                                     && MatrixController.pairRow(index) === index + 1
                            x: panel.rowHeaderWidth + (panel.markWidth - size) / 2
                            y: (index + 1) * panel.cellHeight - size / 2
                            name: "link"
                            size: 10
                            color: Theme.textMuted
                        }
                    }
                    Repeater {
                        model: panel.rows
                        PairRule {
                            required property int index
                            visible: index % 2 === 1 && index < panel.rows - 1
                            x: 4
                            y: (index + 1) * panel.cellHeight
                            width: panel.rowHeaderWidth - 4
                            height: 1
                        }
                    }
                    Column {
                        visible: !panel.dense
                        y: panel.bodyHeight + 8
                        width: panel.rowHeaderWidth
                        Repeater {
                            //: Section labels under the grid, in capitals but for the units
                            model: [qsTr("GAIN dB"), qsTr("DELAY ms"), qsTr("FED BY")]
                            Item {
                                required property string modelData
                                required property int index
                                width: parent.width
                                height: index < 2 ? panel.footerRowHeight : 24
                                Heading {
                                    anchors.right: parent.right
                                    anchors.rightMargin: 8
                                    anchors.verticalCenter: parent.verticalCenter
                                    font.pixelSize: Theme.fontCaption.pixelSize
                                    font.capitalization: Font.MixedCase
                                    text: parent.modelData
                                }
                            }
                        }
                    }
                }
            }

            // ---- The grid --------------------------------------------------------
            ScrollView {
                id: scroller
                objectName: "Matrix.scroll"
                x: panel.leftWidth
                y: panel.headerHeight
                width: Math.max(0, parent.width - x)
                height: Math.max(0, parent.height - y)
                ScrollBar.vertical: AppScrollBar {}
                ScrollBar.horizontal: AppScrollBar { orientation: Qt.Horizontal }

                Flickable {
                    id: flick
                    contentWidth: panel.bodyWidth
                    contentHeight: panel.bodyHeight + panel.footerHeight
                    boundsBehavior: Flickable.StopAtBounds
                    clip: true

                    // Stripes and the lit row and column
                    Repeater {
                        model: panel.rows
                        Rectangle {
                            required property int index
                            visible: index % 2 === 1
                            y: index * panel.cellHeight
                            width: panel.bodyWidth
                            height: panel.cellHeight
                            color: Theme.stripe
                        }
                    }
                    Rectangle {
                        visible: panel.litInput >= 0
                        y: panel.litInput * panel.cellHeight
                        width: panel.cols * panel.cellWidth
                        height: panel.cellHeight
                        color: Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.08)
                    }
                    Rectangle {
                        visible: panel.litOutput >= 0
                        x: panel.litOutput * panel.cellWidth
                        width: panel.cellWidth
                        height: panel.bodyHeight
                        color: Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.08)
                    }
                    // A hairline after each input pair and each output pair
                    Repeater {
                        model: panel.rows
                        PairRule {
                            required property int index
                            visible: index % 2 === 1 && index < panel.rows - 1
                            y: (index + 1) * panel.cellHeight
                            width: panel.bodyWidth
                            height: 1
                        }
                    }
                    Repeater {
                        model: panel.cols
                        PairRule {
                            required property int index
                            visible: index % 2 === 1 && index < panel.cols - 1
                            x: (index + 1) * panel.cellWidth
                            width: 1
                            height: panel.bodyHeight
                        }
                    }

                    // The crosspoints: the keyboard's grid
                    FocusScope {
                        id: gridFocus
                        objectName: "Matrix.grid"
                        width: panel.cols * panel.cellWidth
                        height: panel.bodyHeight
                        activeFocusOnTab: true
                        // Tab comes back to the cell it left, else the first
                        onActiveFocusChanged: if (activeFocus && MatrixController.currentInput < 0) MatrixController.setCurrent(0, 0)
                        Keys.onPressed: event => { event.accepted = panel.keyPressed(event) }
                        Grid {
                            columns: Math.max(1, panel.cols)
                            Repeater {
                                id: cells
                                model: MatrixController.model
                                MatrixCell {
                                    view: panel
                                    width: panel.cellWidth
                                    height: panel.cellHeight
                                    tileWidth: panel.tileWidth
                                    tileHeight: panel.tileHeight
                                    inputName: { const s = panel.source(input); return s ? s.name : "" }
                                    outputName: { const c = panel.column(output); return c ? c.label : "" }
                                }
                            }
                        }
                        Accessible.role: Accessible.Table
                        Accessible.name: qsTr("Routing matrix")
                    }

                    // "→ Out 1, 2, 3": what each row feeds
                    Column {
                        x: panel.cols * panel.cellWidth
                        Repeater {
                            model: panel.rows
                            Text {
                                required property int index
                                readonly property var feeds: MatrixController.model.version < 0 ? [] : MatrixController.model.feeds(index)
                                objectName: "Matrix.tail." + index
                                width: panel.tailWidth
                                height: panel.cellHeight
                                leftPadding: 8
                                rightPadding: 8
                                textFormat: Text.PlainText
                                //: What an input feeds: %1 is a list of output numbers ("1, 2, 3")
                                text: feeds.length ? qsTr("→ Out %1").arg(feeds.map(o => o + 1).join(", ")) : qsTr("→ nothing")
                                font: Theme.fontSmall
                                color: Theme.textMuted
                                elide: Text.ElideRight
                                verticalAlignment: Text.AlignVCenter
                            }
                        }
                    }

                    // ---- Footers: each output's gain, delay and sources ------------
                    Column {
                        visible: !panel.dense
                        y: panel.bodyHeight + 8
                        Row {
                            Repeater {
                                model: panel.cols
                                Item {
                                    id: gainCell
                                    required property int index
                                    readonly property var col: panel.column(index)
                                    width: panel.cellWidth
                                    height: panel.footerRowHeight
                                    opacity: col && col.enabled ? 1 : Theme.dim(0.45)
                                    MatrixField {
                                        objectName: "Matrix.gain." + gainCell.index
                                        anchors.centerIn: parent
                                        width: panel.cellWidth - 8
                                        height: 22
                                        kind: "gain"
                                        maxValue: 20 // -- WP QPI2 -- the device's output range, as the card's
                                        showUnit: false
                                        horizontalAlignment: TextInput.AlignRight
                                        font.pixelSize: 11
                                        value: gainCell.col ? gainCell.col.gain : 0
                                        color: activeFocus ? Theme.accentText : !enabled ? Theme.textDisabled : value === 0 ? Theme.textMuted : Theme.text
                                        label: qsTr("%1 gain").arg(gainCell.col ? gainCell.col.label : "")
                                        enabled: MatrixController.writable
                                        onValueCommitted: v => MatrixController.setOutputGain(gainCell.index, v)
                                    }
                                }
                            }
                        }
                        Row {
                            Repeater {
                                model: panel.cols
                                Item {
                                    id: delayCell
                                    required property int index
                                    readonly property var col: panel.column(index)
                                    width: panel.cellWidth
                                    height: panel.footerRowHeight
                                    opacity: col && col.enabled ? 1 : Theme.dim(0.45)
                                    MatrixField {
                                        objectName: "Matrix.delay." + delayCell.index
                                        anchors.centerIn: parent
                                        width: panel.cellWidth - 8
                                        height: 22
                                        kind: "delay"
                                        maxValue: MatrixController.maxDelayMs
                                        showUnit: false
                                        horizontalAlignment: TextInput.AlignRight
                                        font.pixelSize: 11
                                        value: delayCell.col ? delayCell.col.delay : 0
                                        color: activeFocus ? Theme.accentText : !enabled ? Theme.textDisabled : value === 0 ? Theme.textMuted : Theme.text
                                        label: qsTr("%1 delay").arg(delayCell.col ? delayCell.col.label : "")
                                        enabled: MatrixController.writable
                                        onValueCommitted: v => MatrixController.setOutputDelay(delayCell.index, v)
                                    }
                                }
                            }
                        }
                        Row {
                            Repeater {
                                model: panel.cols
                                Item {
                                    id: fedBy
                                    required property int index
                                    readonly property var sources: MatrixController.model.version < 0 ? [] : MatrixController.model.sources(index)
                                    readonly property string names: sources.map(i => { const s = panel.source(i); return s ? s.name : "" }).join(", ")
                                    objectName: "Matrix.fedBy." + index
                                    width: panel.cellWidth
                                    height: 24
                                    Row {
                                        anchors.centerIn: parent
                                        spacing: 2
                                        Repeater {
                                            model: fedBy.sources.slice(0, 3)
                                            Rectangle {
                                                required property int modelData
                                                anchors.verticalCenter: parent.verticalCenter
                                                width: 6
                                                height: 6
                                                radius: 3
                                                color: ChannelPalette.input(modelData)
                                            }
                                        }
                                        Text {
                                            anchors.verticalCenter: parent.verticalCenter
                                            visible: text.length > 0
                                            width: Math.min(implicitWidth, panel.cellWidth - 12)
                                            textFormat: Text.PlainText
                                            text: fedBy.sources.length === 0 ? "–"
                                                : fedBy.sources.length === 1 ? fedBy.names
                                                : fedBy.sources.length > 3 ? String(fedBy.sources.length) : ""
                                            font: Theme.fontCaption
                                            color: Theme.textMuted
                                            elide: Text.ElideRight
                                        }
                                    }
                                    HoverHandler { id: fedHover }
                                    HelpTip {
                                        text: fedBy.sources.length ? qsTr("Fed by %1").arg(fedBy.names) : qsTr("Fed by nothing")
                                        visible: fedHover.hovered
                                    }
                                    Accessible.role: Accessible.StaticText
                                    Accessible.name: fedBy.sources.length ? qsTr("Fed by %1").arg(fedBy.names) : qsTr("Fed by nothing")
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- The header menu -----------------------------------------------------------
    AppMenu {
        id: columnMenu
        objectName: "Matrix.columnMenu"
        readonly property var col: panel.menuOutput >= 0 ? panel.column(panel.menuOutput) : undefined
        readonly property bool canIdentify: DeviceModel.connected && SidebarController.identifySupported

        AppMenuItem {
            objectName: "Matrix.menu.identify"
            text: qsTr("Identify")
            visible: columnMenu.canIdentify
            height: visible ? implicitHeight : 0
            // A switched-off output would play nothing
            enabled: MatrixController.writable && columnMenu.col !== undefined && columnMenu.col.enabled === true
            onTriggered: SidebarController.identifyOutput(panel.menuOutput)
        }
        MenuSeparator {
            visible: columnMenu.canIdentify
            height: visible ? implicitHeight : 0
        }
        AppMenuItem {
            objectName: "Matrix.menu.rename"
            text: qsTr("Rename")
            enabled: MatrixController.writable
            onTriggered: panel.renaming = panel.menuOutput
        }
        MenuSeparator {}
        AppMenuItem {
            objectName: "Matrix.menu.copy"
            text: qsTr("Copy Parameters")
            enabled: columnMenu.col !== undefined
            onTriggered: SidebarController.copyChannel(columnMenu.col.channel)
        }
        AppMenuItem {
            objectName: "Matrix.menu.paste"
            text: qsTr("Paste Parameters")
            enabled: MatrixController.writable && SidebarController.hasClipboard && columnMenu.col !== undefined
            onTriggered: SidebarController.pasteChannel(columnMenu.col.channel)
        }
    }

    // ---- A cell's menu (right-click, Shift+F10) ---------------------------------------
    AppMenu {
        id: cellMenu
        objectName: "Matrix.cellMenu"
        property int input: -1
        property int output: -1
        readonly property bool on: input >= 0 && MatrixController.model.version >= 0 && MatrixController.model.get(input, output, "connected") === true
        readonly property bool inverted: on && MatrixController.model.get(input, output, "invert") === true
        readonly property var col: output >= 0 ? panel.column(output) : undefined
        onClosed: if (panel.editInput < 0) panel.focusGrid()

        AppMenuItem {
            objectName: "Matrix.cellMenu.connect"
            text: cellMenu.on ? qsTr("Disconnect") : qsTr("Connect")
            enabled: MatrixController.writable
            onTriggered: MatrixController.setConnected(cellMenu.input, cellMenu.output, !cellMenu.on)
        }
        AppMenuItem {
            objectName: "Matrix.cellMenu.gain"
            text: qsTr("Gain…")
            enabled: MatrixController.writable
            onTriggered: {
                const i = cellMenu.input, o = cellMenu.output
                Qt.callLater(() => panel.beginEdit(i, o, ""))
            }
        }
        AppMenuItem {
            objectName: "Matrix.cellMenu.reset"
            text: qsTr("Gain to 0 dB")
            enabled: MatrixController.writable && cellMenu.on
            onTriggered: MatrixController.setGain(cellMenu.input, cellMenu.output, 0)
        }
        AppMenuItem {
            objectName: "Matrix.cellMenu.invert"
            text: qsTr("Invert Phase")
            checkable: true
            checked: cellMenu.inverted
            enabled: MatrixController.writable && cellMenu.on
            onTriggered: MatrixController.setInvert(cellMenu.input, cellMenu.output, !cellMenu.inverted)
        }
        MenuSeparator {
            visible: columnMenu.canIdentify
            height: visible ? implicitHeight : 0
        }
        AppMenuItem {
            objectName: "Matrix.cellMenu.identify"
            text: qsTr("Identify Output")
            visible: columnMenu.canIdentify
            height: visible ? implicitHeight : 0
            enabled: MatrixController.writable && cellMenu.col !== undefined && cellMenu.col.enabled === true
            onTriggered: SidebarController.identifyOutput(cellMenu.output)
        }
    }

    QuestionDialog {
        id: enableQuestion
        objectName: "Matrix.enableQuestion"
        ofDevice: true   // -- WP MDQT2a --
        property int output: -1
        parent: Overlay.overlay
        onAnswered: answer => { if (answer === "accept") MatrixController.confirmEnable(output) }
    }
}
