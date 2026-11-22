import QtQuick
import QtQuick.Controls.Basic
import UConsole

// The app's tooltip: short help on hover, as the Mac's .help().
ToolTip {
    id: tip
    delay: 600
    timeout: 8000
    font: Theme.fontSmall
    contentItem: Text {
        textFormat: Text.PlainText
        text: tip.text
        font: tip.font
        color: Theme.tooltipText
        wrapMode: Text.Wrap
    }
    background: Rectangle {
        color: Theme.tooltip
        radius: Theme.radiusSmall
    }
}
