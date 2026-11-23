import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP QP6 -- One crosspoint of the matrix (the Mac's MatrixPoint): the gain
// field over the connect dot over INV, the field and INV only while
// connected. The dot carries the input's colour when connected; otherwise a
// ring, outlined in the warning colour on the PDM column while enabling PDM
// would collide on Core 1. A cell whose output is off is dimmed.
//
// A delegate of MatrixController.model (roles input, output, connected, gain,
// invert, outputEnabled, conflict).
Item {
    id: cell
    required property int index
    required property int input
    required property int output
    required property bool connected
    required property real gain
    required property bool invert
    required property bool outputEnabled
    required property bool conflict
    property string inputName
    property string outputName

    objectName: "Matrix.cell." + input + "." + output
    implicitWidth: 72
    implicitHeight: 78
    opacity: outputEnabled ? 1 : Theme.dim(0.3)
    enabled: MatrixController.writable

    Column {
        anchors.centerIn: parent
        spacing: 4

        Item {
            width: 62
            height: 22
            anchors.horizontalCenter: parent.horizontalCenter
            MatrixField {
                objectName: "Matrix.cell." + cell.input + "." + cell.output + ".gain"
                anchors.fill: parent
                visible: cell.connected
                kind: "gain"
                value: cell.gain
                label: qsTr("%1 to %2 gain").arg(cell.inputName).arg(cell.outputName)
                onValueCommitted: v => MatrixController.setGain(cell.input, cell.output, v)
            }
        }

        AbstractButton {
            id: dot
            objectName: "Matrix.cell." + cell.input + "." + cell.output + ".connect"
            width: 28
            height: 28
            anchors.horizontalCenter: parent.horizontalCenter
            hoverEnabled: true
            focusPolicy: Qt.TabFocus
            onClicked: MatrixController.setConnected(cell.input, cell.output, !cell.connected)
            contentItem: Item {
                Rectangle {
                    anchors.centerIn: parent
                    readonly property real size: cell.connected ? 16 : 18
                    width: size
                    height: size
                    radius: size / 2
                    color: cell.connected ? (cell.outputEnabled ? ChannelPalette.input(cell.input) : Theme.textDisabled)
                                          : Theme.panel
                    border.width: cell.connected ? 0 : (dot.hovered ? 2 : 1.5)
                    border.color: cell.conflict ? Theme.warning : (dot.hovered ? Theme.textMuted : Theme.border)
                }
            }
            background: Rectangle {
                radius: width / 2
                color: Theme.panel
                border.width: dot.visualFocus ? 2 : 0
                border.color: Theme.focusRing
            }
            HelpTip {
                text: cell.connected ? qsTr("Disconnect") : qsTr("Connect")
                visible: dot.hovered
            }
            Accessible.role: Accessible.CheckBox
            Accessible.name: qsTr("%1 to %2").arg(cell.inputName).arg(cell.outputName)
            Accessible.checkable: true
            Accessible.checked: cell.connected
            Accessible.onPressAction: dot.clicked()
        }

        Item {
            width: 62
            height: 16
            anchors.horizontalCenter: parent.horizontalCenter
            AbstractButton {
                id: inv
                objectName: "Matrix.cell." + cell.input + "." + cell.output + ".invert"
                anchors.centerIn: parent
                visible: cell.connected
                hoverEnabled: true
                focusPolicy: Qt.TabFocus
                onClicked: MatrixController.setInvert(cell.input, cell.output, !cell.invert)
                contentItem: Text {
                    text: qsTr("INV")
                    font.pixelSize: Theme.fontCaption.pixelSize
                    font.weight: cell.invert ? Font.Bold : Font.Medium
                    color: cell.invert ? Theme.warning : (inv.hovered ? Theme.text : Theme.textMuted)
                }
                background: Rectangle {
                    color: Theme.panel
                    radius: Theme.radiusSmall
                    border.width: inv.visualFocus ? 2 : 0
                    border.color: Theme.focusRing
                }
                HelpTip {
                    text: cell.invert ? qsTr("Phase inverted") : qsTr("Normal phase")
                    visible: inv.hovered
                }
                Accessible.role: Accessible.CheckBox
                Accessible.name: qsTr("%1 to %2 phase invert").arg(cell.inputName).arg(cell.outputName)
                Accessible.checkable: true
                Accessible.checked: cell.invert
                Accessible.onPressAction: inv.clicked()
            }
        }
    }

    // -- WP AIVIS -- an agent changed it just now
    AiTouched { objectName: cell.objectName + ".touched"; path: "mix"; indices: [cell.input, cell.output] }
}
