import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP P1QT2 -- One limit of the profile: a box that sets it (from
// `defaultValue`) or removes it, and its value. A limit left out limits
// nothing. `edited(value)` gives the new value, NaN to remove it; the change
// applies at once (LimitsController).
RowLayout {
    id: field
    property string title
    // The limit's value, or undefined when it is not set
    property var value
    property real from: 0
    property real to: 100
    property real step: 1
    property int decimals: 0
    property string unit
    property real defaultValue: 0
    signal edited(real value)

    readonly property bool isSet: value !== undefined && value !== null
    Layout.fillWidth: true
    spacing: Theme.spacingSmall

    AppCheckBox {
        objectName: field.objectName + ".set"
        Layout.fillWidth: true
        text: field.title
        checked: field.isSet
        Accessible.name: field.title
        onToggled: field.edited(checked ? field.defaultValue : NaN)
    }
    AppSpinBox {
        objectName: field.objectName + ".value"
        Layout.preferredWidth: 130
        enabled: field.isSet && field.enabled
        opacity: field.isSet ? 1 : 0.5
        label: field.title
        realFrom: field.from
        realTo: field.to
        realStep: field.step
        decimals: field.decimals
        unit: field.unit
        realValue: field.isSet ? field.value : field.defaultValue
        Accessible.name: field.title
        onCommitted: v => field.edited(v)
    }
}
