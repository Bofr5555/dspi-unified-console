import QtQuick 2.15
import "../../components"

// An on/off option row.
CsRow {
    id: row
    property bool checked: false
    signal toggled(bool on)
    ToggleSwitch {
        Accessible.name: row.title
        checked: row.checked
        // -- WP QD1 -- A click breaks the binding: back to row.checked, so a refusal shows
        onToggled: { row.toggled(checked); checked = Qt.binding(function () { return row.checked }) }
    }
}
