import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- One parameter of an effect window (the Mac's ParameterRow): its
// title and typed value on one line, the slider under them, an optional pair
// of end labels and a caption. A drag sends `live` values (at most 30 a
// second) and `commit` once on release; a typed value commits. While the
// slider is held, `liveValue` is its value (NaN otherwise), which the
// window's curve follows; the device's echo does not move the thumb then.
ColumnLayout {
    id: row
    property string title
    property string unit
    property real value: 0
    property real from: 0
    property real to: 1
    property int decimals: 1
    property real scrollStep: 0.1
    property real resetValue: NaN
    property string caption
    property var ends: []
    readonly property real liveValue: slider.pressed ? slider.value : NaN
    signal live(real value)
    signal commit(real value)

    spacing: 4

    RowLayout {
        Layout.fillWidth: true
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: row.title
            font: Theme.fontBody
            color: row.enabled ? Theme.text : Theme.textDisabled
            elide: Text.ElideRight
        }
        ChannelValueField {
            objectName: row.objectName.length ? row.objectName + ".value" : ""
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
            text: row.ends.length === 2 ? row.ends[0] : ""
            font: Theme.fontCaption
            color: Theme.textMuted
        }
        Item { Layout.fillWidth: true }
        Text {
            text: row.ends.length === 2 ? row.ends[1] : ""
            font: Theme.fontCaption
            color: Theme.textMuted
        }
    }
    Text {
        visible: row.caption.length > 0
        Layout.fillWidth: true
        text: row.caption
        font: Theme.fontCaption
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
}
