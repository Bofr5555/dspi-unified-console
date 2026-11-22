import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- One parameter of the subharmonic, tube, upmixer and limiter
// windows (the Mac's ParameterRow with its options): the title, an optional
// subtitle, the typed value; the slider; the end labels and a caption. The
// Mac's help text is the title's tooltip.
//
// A drag sends `live` (at most 30 a second) and `commit` on release; a typed
// value commits. `liveValue` is the held slider's value (NaN otherwise), for a
// window's graph. A value at or beyond `offAt` (a floor that switches the
// stage off, or a ceiling at full scale) reads `offText` instead of a number.
ColumnLayout {
    id: row
    property string title
    property string subtitle
    property string unit
    property real value: 0
    property real from: 0
    property real to: 1
    property int decimals: 1
    property real scrollStep: 0.1
    property real resetValue: NaN
    property string caption
    property string help
    property var ends: []
    // The end of the range that means "off": -1 the bottom, +1 the top, 0 none
    property int offAt: 0
    property string offText: qsTr("Off")
    readonly property real liveValue: slider.pressed ? slider.value : NaN
    readonly property bool off: {
        const v = isNaN(liveValue) ? value : liveValue
        return offAt < 0 ? v <= from : offAt > 0 ? v >= to : false
    }
    signal live(real value)
    signal commit(real value)

    spacing: 4

    RowLayout {
        Layout.fillWidth: true
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: row.title
                font: Theme.fontBody
                color: row.enabled ? Theme.text : Theme.textDisabled
                elide: Text.ElideRight
                HoverHandler { id: titleHover }
                HelpTip {
                    text: row.help
                    visible: titleHover.hovered && row.help.length > 0
                }
            }
            Text {
                visible: row.subtitle.length > 0
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: row.subtitle
                font: Theme.fontCaption
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
        }
        Item {
            Layout.alignment: Qt.AlignTop
            implicitWidth: field.implicitWidth
            implicitHeight: field.implicitHeight
            ChannelValueField {
                id: field
                objectName: row.objectName.length ? row.objectName + ".value" : ""
                anchors.fill: parent
                label: row.title
                unit: row.unit
                value: row.value
                liveValue: row.liveValue
                maxDecimals: row.decimals
                scrollStep: row.scrollStep
                minValue: row.from
                maxValue: row.to
                resetValue: row.resetValue
                onCommitted: v => row.commit(Math.min(row.to, Math.max(row.from, v)))
            }
            // "Off" over the number while the stage is off and nobody types;
            // it takes no clicks, so a click reaches the field underneath
            Rectangle {
                objectName: row.objectName.length ? row.objectName + ".off" : ""
                visible: row.off && !field.editing
                width: field.fieldWidth
                height: field.height
                radius: Theme.radiusSmall
                color: Theme.control
                border.width: 1
                border.color: Theme.border
                Text {
                    anchors.fill: parent
                    anchors.rightMargin: 6
                    textFormat: Text.PlainText
                    text: row.offText
                    font: Theme.fontMono
                    color: row.enabled ? Theme.textMuted : Theme.textDisabled
                    horizontalAlignment: Text.AlignRight
                    verticalAlignment: Text.AlignVCenter
                }
            }
        }
    }
    CustomSlider {
        id: slider
        objectName: row.objectName.length ? row.objectName + ".slider" : ""
        Layout.fillWidth: true
        label: row.title
        from: row.from
        to: row.to
        stepSize: Math.pow(10, -row.decimals)
        resetValue: row.resetValue
        Binding on value {
            when: !slider.pressed
            value: row.value
            restoreMode: Binding.RestoreNone
        }
        onValueMoved: v => row.live(v)
        onCommitted: v => row.commit(v)
    }
    RowLayout {
        visible: row.ends.length === 2
        Layout.fillWidth: true
        Text {
            textFormat: Text.PlainText
            text: row.ends.length === 2 ? row.ends[0] : ""
            font: Theme.fontCaption
            color: Theme.textMuted
        }
        Item { Layout.fillWidth: true }
        Text {
            textFormat: Text.PlainText
            text: row.ends.length === 2 ? row.ends[1] : ""
            font: Theme.fontCaption
            color: Theme.textMuted
        }
    }
    Text {
        visible: row.caption.length > 0
        Layout.fillWidth: true
        textFormat: Text.PlainText
        text: row.caption
        font: Theme.fontCaption
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
}
