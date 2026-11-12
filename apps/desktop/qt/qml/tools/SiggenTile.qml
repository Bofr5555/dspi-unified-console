import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP9 -- One signal type in the generator's grid (the Mac's tile): a
// small drawing of the waveform above its label, the accent while chosen.
AbstractButton {
    id: tile
    property int type: 0
    property bool selected: false
    property string toolTip

    implicitHeight: glyph.height + label.implicitHeight + 16
    hoverEnabled: true
    focusPolicy: Qt.TabFocus

    readonly property color ink: selected ? Theme.accentText : enabled ? Theme.textMuted : Theme.textDisabled

    background: Rectangle {
        radius: 7
        color: tile.selected ? Qt.alpha(Theme.accent, 0.16) : tile.hovered && tile.enabled ? Theme.controlHover : Theme.control
        border.width: tile.visualFocus ? 2 : 1
        border.color: tile.visualFocus ? Theme.focusRing : tile.selected ? Qt.alpha(Theme.accent, 0.7) : Theme.separator
        opacity: tile.enabled ? 1 : 0.6
    }

    contentItem: Item {
        Canvas {
            id: glyph
            x: 6
            y: 6
            width: parent.width - 12
            height: 22
            property color ink: tile.ink
            onInkChanged: requestPaint()
            onWidthChanged: requestPaint()
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                ctx.strokeStyle = ink
                ctx.lineWidth = 1.4
                ctx.lineCap = "round"
                ctx.lineJoin = "round"
                const lines = SiggenGlyphs.paths(tile.type)
                for (const line of lines) {
                    ctx.beginPath()
                    for (let i = 0; i < line.length; ++i) {
                        const px = line[i][0] * width
                        const py = height / 2 - line[i][1] * height * 0.42
                        if (i === 0) ctx.moveTo(px, py)
                        else ctx.lineTo(px, py)
                    }
                    ctx.stroke()
                }
            }
        }
        Text {
            id: label
            anchors { top: glyph.bottom; topMargin: 2; horizontalCenter: parent.horizontalCenter }
            width: parent.width
            text: tile.text
            font: Qt.font({ pixelSize: Theme.fontCaption.pixelSize, weight: tile.selected ? Font.DemiBold : Font.Normal })
            color: tile.ink
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
        }
    }

    HelpTip {
        text: tile.toolTip
        visible: tile.hovered && tile.toolTip.length > 0
    }

    Accessible.role: Accessible.RadioButton
    Accessible.name: text
    Accessible.description: toolTip
    Accessible.checkable: true
    Accessible.checked: selected
}
