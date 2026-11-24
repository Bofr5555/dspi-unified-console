import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP MX1 -- An output's column header in the matrix (DESREV's mockup): its
// name (two lines at most), its OUTn pill and the chevron of its menu (Identify,
// Rename, Copy / Paste Parameters; a right-click opens it too), then the
// output's enable (power) and mute, the channel strip's icons. A switched-off
// output dims its name and pill. On a page a click selects the output, which
// the inspector shows; in a window there is nothing to select. The name turns
// into a field while it is renamed (MatrixPanel.renaming).
Item {
    id: header
    required property int index
    property Item view: null
    readonly property var col: view ? view.column(index) : undefined
    readonly property bool dense: view ? view.dense : false
    readonly property bool on: col ? col.enabled === true : true
    readonly property bool muted: col ? col.muted === true : false
    property string shownName: col ? col.label : ""

    objectName: "Matrix.header." + index

    // A header to click, under what it has in it (-- WP LAY5a --)
    AbstractButton {
        id: select
        objectName: "Matrix.header." + header.index + ".select"
        readonly property int channel: header.col && header.col.channel !== undefined ? header.col.channel : -1
        readonly property bool current: channel >= 0 && header.view !== null && channel === header.view.selected
                                        && MatrixController.currentInput < 0
        anchors.fill: parent
        anchors.margins: 1
        visible: header.view !== null && header.view.inPage && header.view.renaming !== header.index
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        onClicked: header.view.selectChannel(channel)
        background: Rectangle {
            radius: Theme.radiusControl
            color: select.current ? Theme.selection : select.hovered ? Theme.controlHover : "transparent"
            border.width: select.visualFocus ? Theme.focusRingWidth : 0
            border.color: Theme.focusRing
        }
        Accessible.role: Accessible.Button
        //: %1 is a channel's name
        Accessible.name: qsTr("Select %1").arg(header.shownName)
        Accessible.checkable: true
        Accessible.checked: current
        Accessible.onPressAction: select.clicked()
    }

    Column {
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 4
        anchors.horizontalCenter: parent.horizontalCenter
        width: parent.width - 4
        spacing: 2

        Text {
            visible: !header.dense && header.view !== null && header.view.renaming !== header.index
            width: parent.width
            height: 2 * Math.ceil(metrics.height)
            textFormat: Text.PlainText
            text: header.shownName
            font: Theme.fontSmall
            color: select.current || select.hovered ? Theme.text : Theme.textMuted
            opacity: header.on ? 1 : Theme.dim(0.45)
            // The derived labels ("Out 1 · L · Low") take two lines
            wrapMode: Text.Wrap
            maximumLineCount: 2
            elide: Text.ElideRight
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignBottom
            FontMetrics {
                id: metrics
                font: Theme.fontSmall
            }
        }
        BorderedField {
            id: renameField
            objectName: "Matrix.header." + header.index + ".rename"
            visible: header.view !== null && header.view.renaming === header.index
            width: parent.width
            font: Theme.fontSmall
            horizontalAlignment: TextInput.AlignHCenter
            label: qsTr("Name of %1").arg(header.shownName)
            // Opens on the stored name, all of it selected
            onVisibleChanged: {
                if (!visible) return
                text = header.col ? header.col.name : ""
                forceActiveFocus()
                selectAll()
            }
            onCommitted: t => {
                if (header.view.renaming !== header.index) return
                header.view.renaming = -1
                if (header.col && t.trim() !== header.col.name)
                    MatrixController.renameOutput(header.index, t)
            }
            Keys.onEscapePressed: header.view.renaming = -1
            onActiveFocusChanged: if (!activeFocus && header.view.renaming === header.index)
                Qt.callLater(() => { if (header.view.renaming === header.index) header.view.renaming = -1 })
        }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 0
            Pill {
                anchors.verticalCenter: parent.verticalCenter
                text: header.col ? header.col.descriptor : ""
                tint: ChannelPalette.output(header.index, header.col && header.col.pdm)
                opacity: header.on ? 1 : Theme.dim(0.45)
                onClicked: if (header.view && header.view.inPage) header.view.selectChannel(select.channel)
            }
            IconButton {
                id: menuButton
                objectName: "Matrix.header." + header.index + ".menu"
                anchors.verticalCenter: parent.verticalCenter
                visible: !header.dense   // dense: the right-click
                width: 16
                height: 18
                iconName: "chevron-down"
                iconSize: 10
                iconColor: Theme.textMuted
                toolTip: qsTr("%1 options").arg(header.shownName)
                onClicked: header.view.openColumnMenu(header.index, menuButton)
            }
        }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 2
            IconButton {
                objectName: "Matrix.enable." + header.index
                width: 20
                height: 20
                iconName: "power"
                iconSize: 13
                enabled: MatrixController.writable
                iconColor: !header.col ? Theme.textDisabled
                         : header.on ? Theme.accentText
                         : header.col.conflict ? Theme.warning : Theme.textMuted
                toolTip: header.col ? header.col.enableTip : ""
                property bool shownOn: header.on
                Accessible.role: Accessible.CheckBox
                Accessible.name: qsTr("Enable %1").arg(header.shownName)
                Accessible.checkable: true
                Accessible.checked: shownOn
                onClicked: MatrixController.toggleEnable(header.index)
            }
            IconButton {
                objectName: "Matrix.mute." + header.index
                width: 20
                height: 20
                iconName: header.muted ? "volume-x" : "volume-2"
                iconSize: 13
                iconColor: header.muted ? Theme.dangerText : Theme.textMuted
                enabled: MatrixController.writable
                toolTip: header.muted ? qsTr("Unmute") : qsTr("Mute")
                Accessible.role: Accessible.CheckBox
                Accessible.name: qsTr("Mute %1").arg(header.shownName)
                Accessible.checkable: true
                Accessible.checked: header.muted
                onClicked: MatrixController.toggleMute(header.index)
            }
        }
    }

    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: header.view.openColumnMenu(header.index, header)
    }
    HoverHandler { id: headerHover }
    HelpTip {
        text: header.shownName
        visible: headerHover.hovered && header.view !== null && header.view.renaming !== header.index
    }
}
