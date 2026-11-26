import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP MX1 -- A source row's header in the matrix (DESREV's mockup): the
// input's colour dot, its name and its trim (the preamp; muted at 0 dB, not
// in the dense grid, not on an upmix row). Every second row is striped. On a
// page a click selects the input, which the inspector shows; the trim keeps
// its own clicks.
Item {
    id: row
    required property int index
    property Item view: null
    readonly property var src: view ? view.source(index) : undefined
    readonly property bool dense: view ? view.dense : false
    property string shownName: src ? src.name : ""

    objectName: "Matrix.row." + index

    Rectangle {
        anchors.fill: parent
        visible: row.index % 2 === 1
        color: Theme.stripe
    }

    AbstractButton {
        id: select
        objectName: "Matrix.row." + row.index + ".select"
        readonly property int channel: row.src && row.src.input >= 0 ? row.src.input : -1
        readonly property bool current: channel >= 0 && row.view !== null && channel === row.view.selected
                                        && MatrixController.currentInput < 0
        anchors.fill: parent
        anchors.margins: 1
        visible: row.view !== null && row.view.inPage && channel >= 0
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        onClicked: row.view.selectChannel(channel)
        background: Rectangle {
            radius: Theme.radiusControl
            color: select.current ? Theme.selection : select.hovered ? Theme.controlHover : "transparent"
            border.width: select.visualFocus ? Theme.focusRingWidth : 0
            border.color: Theme.focusRing
        }
        Accessible.role: Accessible.Button
        //: %1 is a channel's name
        Accessible.name: qsTr("Select %1").arg(row.shownName)
        Accessible.checkable: true
        Accessible.checked: current
        Accessible.onPressAction: select.clicked()
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 6
        spacing: 8
        Rectangle {
            Layout.alignment: Qt.AlignVCenter
            implicitWidth: 8
            implicitHeight: 8
            radius: 4
            color: ChannelPalette.input(row.index)
        }
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: row.shownName
            font.pixelSize: Theme.fontTable.pixelSize
            font.weight: Font.DemiBold
            color: Theme.text
            elide: Text.ElideRight
        }
        MatrixField {
            objectName: "Matrix.row." + row.index + ".trim"
            visible: !row.dense && row.src !== undefined && row.src.input >= 0
            Layout.preferredWidth: 52
            Layout.preferredHeight: 22
            kind: "gain"
            showUnit: false
            horizontalAlignment: TextInput.AlignRight
            font.pixelSize: 11
            value: row.src ? row.src.trim : 0
            color: activeFocus ? Theme.accentText : !enabled ? Theme.textDisabled : value === 0 ? Theme.textMuted : Theme.text
            label: qsTr("Input trim (preamp) for %1").arg(row.src ? row.src.fullName : "")
            enabled: MatrixController.writable
            onValueCommitted: v => MatrixController.setTrim(row.index, v)
            HelpTip {
                text: qsTr("Input trim (preamp), dB")
                visible: trimHover.hovered
            }
            HoverHandler { id: trimHover }
        }
    }

    HoverHandler { id: rowHover }
    HelpTip {
        text: row.src ? row.src.fullName : ""
        visible: rowHover.hovered && !trimHover.hovered
    }
}
