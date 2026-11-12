import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP6 -- The matrix mixer (the Mac's MatrixMixerView), 960×660: one
// table with a column per output and a row per source.
//
//   header    the output's label, its OUTn tag; right-click (or the chevron)
//             opens Identify, Rename, Copy Parameters, Paste Parameters
//   ROUTING   a crosspoint per input and output (MatrixCell); in the 8-channel
//             layout also Direct 1:1 and Clear, and each input's trim
//   OUTPUT    ENABLE (with the PDM / Core 1 question), GAIN, DELAY, MUTE
//
// Everything that writes is disabled on read-only firmware. Opened by
// Tools > Matrix Mixer... and the quick strip.
ToolWindow {
    id: window
    objectName: "MatrixWindow"
    windowId: "matrix"
    defaultWidth: 960
    defaultHeight: 660
    minimumWidth: 480
    minimumHeight: 320
    title: qsTr("Matrix Mixer")

    readonly property int columnWidth: 72
    readonly property int labelWidth: 80
    readonly property int rowHeight: 78
    readonly property int controlRowHeight: 30
    readonly property int headerHeight: 58
    readonly property int cols: MatrixController.columns
    readonly property int rows: MatrixController.rows
    readonly property bool wide: MatrixController.wide
    // The output being renamed in its header, -1 for none
    property int renaming: -1
    // The output whose header menu is open
    property int menuOutput: -1

    function column(o) { return MatrixController.revision < 0 ? undefined : MatrixController.column(o) }
    function source(r) { return MatrixController.revision < 0 ? undefined : MatrixController.row(r) }
    // Stereo keeps one L/R separator; 8 channels are grouped in pairs
    function dividerAfter(r) { return wide ? (r % 2 === 1 && r < rows - 1) : r === 0 && rows > 1 }
    function openMenu(o, item) {
        menuOutput = o
        columnMenu.popup(item, 0, item.height)
    }

    Connections {
        target: MatrixController
        function onEnableQuestion(output, title, text, acceptText) {
            enableQuestion.output = output
            enableQuestion.title = title
            enableQuestion.text = text
            enableQuestion.acceptText = acceptText
            enableQuestion.open()
        }
    }

    component SectionLabel: Rectangle {
        property alias text: caption.text
        x: 1
        width: table.width - 2
        height: 22
        color: Theme.panel
        Text {
            id: caption
            anchors.verticalCenter: parent.verticalCenter
            x: 12
            font: Theme.fontHeading
            color: Theme.textMuted
        }
    }
    component Rule: Rectangle {
        property int indent: 0
        x: Math.max(1, indent)
        width: table.width - x - 1
        height: 1
        color: Theme.separator
    }
    component ControlRow: Row {
        id: controlRow
        property string label
        default property alias cells: cellRow.data
        height: window.controlRowHeight
        Text {
            width: window.labelWidth - 8
            height: parent.height
            text: controlRow.label
            font: Theme.fontHeading
            color: Theme.textMuted
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
        }
        Item { width: 8; height: 1 }
        Row { id: cellRow; height: parent.height }
    }

    ScrollView {
        id: scroller
        objectName: "Matrix.scroll"
        anchors.fill: parent
        contentWidth: table.width + 2 * Theme.spacingLarge
        contentHeight: table.height + 2 * Theme.spacingLarge
        ScrollBar.vertical: AppScrollBar {}
        ScrollBar.horizontal: AppScrollBar { orientation: Qt.Horizontal }

        Rectangle {
            id: table
            objectName: "Matrix.table"
            x: Theme.spacingLarge
            y: Theme.spacingLarge
            width: window.labelWidth + window.cols * window.columnWidth
            height: body.implicitHeight + 4
            radius: 10
            color: Theme.panel
            border.width: 1
            border.color: Theme.separator

            Column {
                id: body
                width: parent.width

                // ---- Column headers --------------------------------------
                Row {
                    Item { width: window.labelWidth; height: window.headerHeight }
                    Repeater {
                        model: window.cols
                        Item {
                            id: header
                            required property int index
                            readonly property var col: window.column(index)
                            objectName: "Matrix.header." + index
                            property string shownName: col ? col.label : ""
                            width: window.columnWidth
                            height: window.headerHeight

                            Column {
                                anchors.centerIn: parent
                                width: parent.width - 4
                                spacing: 3
                                Text {
                                    visible: window.renaming !== header.index
                                    width: parent.width
                                    height: 2 * Math.ceil(fontMetrics.height)
                                    text: header.shownName
                                    font.pixelSize: Theme.fontSmall.pixelSize
                                    font.weight: Font.Medium
                                    color: Theme.textMuted
                                    // The derived labels ("Out 1 · L · Low") take two lines
                                    wrapMode: Text.Wrap
                                    maximumLineCount: 2
                                    elide: Text.ElideRight
                                    horizontalAlignment: Text.AlignHCenter
                                    verticalAlignment: Text.AlignBottom
                                    FontMetrics {
                                        id: fontMetrics
                                        font.pixelSize: Theme.fontSmall.pixelSize
                                    }
                                }
                                BorderedField {
                                    id: renameField
                                    objectName: "Matrix.header." + header.index + ".rename"
                                    visible: window.renaming === header.index
                                    width: parent.width
                                    font: Theme.fontSmall
                                    horizontalAlignment: TextInput.AlignHCenter
                                    label: qsTr("Name of %1").arg(header.shownName)
                                    onVisibleChanged: if (visible) {
                                        text = header.col ? header.col.name : ""
                                        forceActiveFocus()
                                        selectAll()
                                    }
                                    onCommitted: t => {
                                        if (window.renaming !== header.index) return
                                        window.renaming = -1
                                        if (header.col && t.trim() !== header.col.name)
                                            MatrixController.renameOutput(header.index, t)
                                    }
                                    Keys.onEscapePressed: window.renaming = -1
                                    onActiveFocusChanged: if (!activeFocus && window.renaming === header.index)
                                        Qt.callLater(() => { if (window.renaming === header.index) window.renaming = -1 })
                                }
                                Row {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    spacing: 2
                                    Pill {
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: header.col ? header.col.descriptor : ""
                                        tint: ChannelPalette.output(header.index, header.col && header.col.pdm)
                                    }
                                    IconButton {
                                        id: menuButton
                                        objectName: "Matrix.header." + header.index + ".menu"
                                        anchors.verticalCenter: parent.verticalCenter
                                        iconName: "chevron-down"
                                        iconSize: 12
                                        toolTip: qsTr("%1 options").arg(header.shownName)
                                        onClicked: window.openMenu(header.index, menuButton)
                                    }
                                }
                            }
                            TapHandler {
                                acceptedButtons: Qt.RightButton
                                onTapped: window.openMenu(header.index, header)
                            }
                            HelpTip {
                                text: header.shownName
                                visible: headerHover.hovered && window.renaming !== header.index
                            }
                            HoverHandler { id: headerHover }
                        }
                    }
                }

                Rule {}

                // ---- Routing ---------------------------------------------
                SectionLabel {
                    objectName: "Matrix.routing"
                    text: qsTr("ROUTING")
                    RowLayout {
                        visible: window.wide
                        anchors.right: parent.right
                        anchors.rightMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 8
                        AppButton {
                            objectName: "Matrix.direct"
                            implicitHeight: 20
                            text: qsTr("Direct 1:1")
                            toolTip: qsTr("Route each input to the matching output (FL→OUT1, FR→OUT2, …) and disable the PDM sub")
                            enabled: MatrixController.writable
                            onClicked: MatrixController.directRouting()
                        }
                        AppButton {
                            objectName: "Matrix.clear"
                            implicitHeight: 20
                            text: qsTr("Clear")
                            toolTip: qsTr("Disconnect every crosspoint")
                            enabled: MatrixController.writable
                            onClicked: MatrixController.clearRoutes()
                        }
                    }
                }

                Item {
                    width: table.width
                    height: window.rows * window.rowHeight

                    // Source labels
                    Column {
                        Repeater {
                            model: window.rows
                            Item {
                                id: srcRow
                                required property int index
                                readonly property var src: window.source(index)
                                objectName: "Matrix.row." + index
                                property string shownName: src ? src.name : ""
                                width: window.labelWidth
                                height: window.rowHeight
                                Column {
                                    anchors.centerIn: parent
                                    spacing: 4
                                    Row {
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        spacing: 5
                                        Rectangle {
                                            anchors.verticalCenter: parent.verticalCenter
                                            width: 8
                                            height: 8
                                            radius: 4
                                            color: ChannelPalette.input(srcRow.index)
                                        }
                                        Text {
                                            width: Math.min(implicitWidth, window.labelWidth - 20)
                                            text: srcRow.shownName
                                            font.pixelSize: Theme.fontSmall.pixelSize
                                            font.weight: window.wide ? Font.DemiBold : Font.Medium
                                            color: Theme.text
                                            elide: Text.ElideRight
                                        }
                                    }
                                    MatrixField {
                                        objectName: "Matrix.row." + srcRow.index + ".trim"
                                        visible: window.wide && srcRow.src && srcRow.src.input >= 0
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        kind: "gain"
                                        value: srcRow.src ? srcRow.src.trim : 0
                                        label: qsTr("Input trim (preamp) for %1").arg(srcRow.src ? srcRow.src.fullName : "")
                                        enabled: MatrixController.writable
                                        onValueCommitted: v => MatrixController.setTrim(srcRow.index, v)
                                    }
                                }
                                HoverHandler { id: rowHover }
                                HelpTip {
                                    text: srcRow.src ? srcRow.src.fullName : ""
                                    visible: rowHover.hovered
                                }
                            }
                        }
                    }

                    // Crosspoints
                    Grid {
                        x: window.labelWidth
                        columns: Math.max(1, window.cols)
                        Repeater {
                            model: MatrixController.model
                            MatrixCell {
                                width: window.columnWidth
                                height: window.rowHeight
                                inputName: { const s = window.source(input); return s ? s.name : "" }
                                outputName: { const c = window.column(output); return c ? c.label : "" }
                            }
                        }
                    }

                    // Row separators
                    Repeater {
                        model: window.rows
                        Rule {
                            required property int index
                            visible: window.dividerAfter(index)
                            indent: window.labelWidth
                            y: (index + 1) * window.rowHeight
                            opacity: 0.6
                        }
                    }
                }

                Rule {}

                // ---- Output controls ---------------------------------------
                SectionLabel { text: qsTr("OUTPUT") }

                ControlRow {
                    objectName: "Matrix.enableRow"
                    label: qsTr("ENABLE")
                    Repeater {
                        model: window.cols
                        Item {
                            id: enableCell
                            required property int index
                            readonly property var col: window.column(index)
                            width: window.columnWidth
                            height: window.controlRowHeight
                            IconButton {
                                objectName: "Matrix.enable." + enableCell.index
                                anchors.centerIn: parent
                                iconName: "power"
                                checkable: false
                                enabled: MatrixController.writable
                                iconColor: !enableCell.col ? Theme.textDisabled
                                         : enableCell.col.enabled ? Theme.accentText
                                         : enableCell.col.conflict ? Theme.warning : Theme.textMuted
                                toolTip: enableCell.col ? enableCell.col.enableTip : ""
                                property bool shownOn: enableCell.col ? enableCell.col.enabled : false
                                Accessible.role: Accessible.CheckBox
                                Accessible.name: qsTr("Enable %1").arg(enableCell.col ? enableCell.col.label : "")
                                Accessible.checkable: true
                                Accessible.checked: shownOn
                                onClicked: MatrixController.toggleEnable(enableCell.index)
                            }
                        }
                    }
                }
                Rule { indent: window.labelWidth; opacity: 0.6 }

                ControlRow {
                    label: qsTr("GAIN")
                    Repeater {
                        model: window.cols
                        Item {
                            id: gainCell
                            required property int index
                            readonly property var col: window.column(index)
                            width: window.columnWidth
                            height: window.controlRowHeight
                            opacity: col && col.enabled ? 1 : 0.3
                            MatrixField {
                                objectName: "Matrix.gain." + gainCell.index
                                anchors.centerIn: parent
                                kind: "gain"
                                value: gainCell.col ? gainCell.col.gain : 0
                                label: qsTr("%1 gain").arg(gainCell.col ? gainCell.col.label : "")
                                enabled: MatrixController.writable
                                onValueCommitted: v => MatrixController.setOutputGain(gainCell.index, v)
                            }
                        }
                    }
                }
                Rule { indent: window.labelWidth; opacity: 0.6 }

                ControlRow {
                    label: qsTr("DELAY")
                    Repeater {
                        model: window.cols
                        Item {
                            id: delayCell
                            required property int index
                            readonly property var col: window.column(index)
                            width: window.columnWidth
                            height: window.controlRowHeight
                            opacity: col && col.enabled ? 1 : 0.3
                            MatrixField {
                                objectName: "Matrix.delay." + delayCell.index
                                anchors.centerIn: parent
                                kind: "delay"
                                maxValue: MatrixController.maxDelayMs
                                value: delayCell.col ? delayCell.col.delay : 0
                                label: qsTr("%1 delay").arg(delayCell.col ? delayCell.col.label : "")
                                enabled: MatrixController.writable
                                onValueCommitted: v => MatrixController.setOutputDelay(delayCell.index, v)
                            }
                        }
                    }
                }
                Rule { indent: window.labelWidth; opacity: 0.6 }

                ControlRow {
                    label: qsTr("MUTE")
                    Repeater {
                        model: window.cols
                        Item {
                            id: muteCell
                            required property int index
                            readonly property var col: window.column(index)
                            readonly property bool muted: col ? col.muted === true : false
                            width: window.columnWidth
                            height: window.controlRowHeight
                            opacity: col && col.enabled ? 1 : 0.3
                            IconButton {
                                objectName: "Matrix.mute." + muteCell.index
                                anchors.centerIn: parent
                                iconName: muteCell.muted ? "volume-x" : "volume-2"
                                iconSize: 14
                                iconColor: muteCell.muted ? Theme.dangerText : Theme.textMuted
                                enabled: MatrixController.writable
                                toolTip: muteCell.muted ? qsTr("Unmute") : qsTr("Mute")
                                Accessible.role: Accessible.CheckBox
                                Accessible.name: qsTr("Mute %1").arg(muteCell.col ? muteCell.col.label : "")
                                Accessible.checkable: true
                                Accessible.checked: muteCell.muted
                                onClicked: MatrixController.toggleMute(muteCell.index)
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- The header menu ---------------------------------------------------
    AppMenu {
        id: columnMenu
        objectName: "Matrix.columnMenu"
        readonly property var col: window.menuOutput >= 0 ? window.column(window.menuOutput) : undefined
        readonly property bool canIdentify: DeviceModel.connected && SidebarController.identifySupported

        AppMenuItem {
            objectName: "Matrix.menu.identify"
            text: qsTr("Identify")
            visible: columnMenu.canIdentify
            height: visible ? implicitHeight : 0
            // A switched-off output would play nothing
            enabled: MatrixController.writable && columnMenu.col !== undefined && columnMenu.col.enabled === true
            onTriggered: SidebarController.identifyOutput(window.menuOutput)
        }
        MenuSeparator {
            visible: columnMenu.canIdentify
            height: visible ? implicitHeight : 0
        }
        AppMenuItem {
            objectName: "Matrix.menu.rename"
            text: qsTr("Rename")
            enabled: MatrixController.writable
            onTriggered: window.renaming = window.menuOutput
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

    QuestionDialog {
        id: enableQuestion
        objectName: "Matrix.enableQuestion"
        property int output: -1
        parent: Overlay.overlay
        onAnswered: answer => { if (answer === "accept") MatrixController.confirmEnable(output) }
    }
}
