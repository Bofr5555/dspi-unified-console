import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP QP9 -- One signal type in the generator's grid (the Mac's tile): a
// small drawing of the waveform over its name, in the accent while chosen.
AbstractButton {
    id: tile
    property int type: 0
    property bool chosen: false
    property string toolTip
    readonly property color ink: chosen ? Theme.accentText : (enabled ? Theme.textMuted : Theme.textDisabled)

    implicitWidth: 60
    implicitHeight: 52
    hoverEnabled: true
    focusPolicy: Qt.TabFocus

    background: Rectangle {
        color: tile.chosen ? Qt.alpha(Theme.accent, 0.16) : (tile.hovered && tile.enabled ? Theme.controlHover : Theme.control)
        border.color: tile.visualFocus ? Theme.focusRing : (tile.chosen ? Qt.alpha(Theme.accent, 0.7) : Theme.separator)
        border.width: tile.visualFocus ? 2 : 1
        opacity: tile.enabled ? 1 : 0.6
        radius: 7
    }

    contentItem: Item {
        Canvas {
            id: drawing
            x: 6
            y: 6
            width: tile.width - 12
            height: 22
            readonly property color pen: tile.ink
            onPenChanged: requestPaint()
            onWidthChanged: requestPaint()
            onPaint: {
                const g = getContext("2d")
                g.clearRect(0, 0, width, height)
                Object.assign(g, { strokeStyle: pen, lineWidth: 1.4, lineCap: "round", lineJoin: "round" })
                const mid = height / 2, reach = height * 0.42
                SiggenGlyphs.paths(tile.type).forEach(points => {
                    g.beginPath()
                    points.forEach((pt, n) => {
                        const at = [pt[0] * width, mid - pt[1] * reach]
                        n === 0 ? g.moveTo(at[0], at[1]) : g.lineTo(at[0], at[1])
                    })
                    g.stroke()
                })
            }
        }
        Text {
            y: drawing.y + drawing.height + 2
            width: tile.width
            text: tile.text
            color: tile.ink
            font.pixelSize: Theme.fontCaption.pixelSize
            font.bold: tile.chosen
            elide: Text.ElideRight
            horizontalAlignment: Text.AlignHCenter
        }
    }

    HelpTip {
        visible: tile.hovered && tile.toolTip.length > 0
        text: tile.toolTip
    }

    Accessible.role: Accessible.RadioButton
    Accessible.name: text
    Accessible.description: toolTip
    Accessible.checkable: true
    Accessible.checked: chosen
}
