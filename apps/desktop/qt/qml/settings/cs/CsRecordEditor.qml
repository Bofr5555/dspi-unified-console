import QtQuick 2.15
import "../../components"

// The rows that set what a record does: a control's binding, a learned
// remote button, or a macro step. Which rows there are, what they offer and
// what an edit makes of the record are the core's (cs_describe, cs_edit), so
// nothing here can draft a record the device refuses. Edits come back through
// edited(record); nothing here talks to the device.
Column {
    id: ed
    property var cs                  // CsHelper
    property string mode: "binding"  // "binding", "ir" or "step"
    property var rec
    property int slot: 0
    property int indent: 0
    signal edited(var rec)

    width: parent ? parent.width : 400

    readonly property var d: cs.describe(mode, slot, rec)
    function change(id, value) { edited(cs.edit(mode, slot, rec, id, value)) }

    // The rows' ids, replaced only when the set changes: a row that stays is not
    // rebuilt, so an edit keeps the field's focus
    property var ids: []
    function syncIds() {
        var now = []
        // The type belongs to the card's badge, not to these rows
        for (var i = 0; i < d.rows.length; i++) if (d.rows[i].id !== "type") now.push(d.rows[i].id)
        if (JSON.stringify(now) !== JSON.stringify(ids)) ids = now
    }
    onDChanged: syncIds()
    Component.onCompleted: syncIds()

    Repeater {
        model: ed.ids
        Loader {
            id: rowLoader
            width: ed.width
            readonly property string rowId: modelData
            readonly property var r: ed.cs.rowOf(ed.d, rowId)
            sourceComponent: !r ? null
                : r.kind === "pick" ? (rowId === "noun" ? familyRow : pickRow)
                : r.kind === "number" ? numberRow
                : r.kind === "toggle" ? toggleRow : noteRow

            Component {
                id: familyRow
                CsPickerRow {
                    indent: ed.indent
                    title: rowLoader.r ? rowLoader.r.title : ""
                    detail: rowLoader.r ? rowLoader.r.detail : ""
                    categories: ed.cs.pickCategories(rowLoader.r)
                    value: rowLoader.r ? rowLoader.r.value : 0
                    text: rowLoader.r ? rowLoader.r.text : ""
                    onChosen: ed.change(rowLoader.rowId, value)
                }
            }
            Component {
                id: pickRow
                CsPickerRow {
                    indent: ed.indent
                    title: rowLoader.r ? rowLoader.r.title : ""
                    detail: rowLoader.r ? rowLoader.r.detail : ""
                    options: ed.cs.pickOptions(rowLoader.r)
                    value: rowLoader.r ? rowLoader.r.value : 0
                    onChosen: ed.change(rowLoader.rowId, value)
                }
            }
            Component {
                id: numberRow
                CsRow {
                    indent: ed.indent
                    title: rowLoader.r ? rowLoader.r.title : ""
                    detail: rowLoader.r ? rowLoader.r.detail : ""
                    ValueField {
                        value: rowLoader.r && rowLoader.r.value !== undefined ? rowLoader.r.value : 0
                        suffix: rowLoader.r ? rowLoader.r.unit : ""
                        decimals: rowLoader.r ? rowLoader.r.decimals : 0
                        minValue: rowLoader.r ? rowLoader.r.min : 0
                        maxValue: rowLoader.r ? rowLoader.r.max : 0
                        wheelStep: rowLoader.r ? rowLoader.r.step : 1
                        fieldWidth: 64
                        onValueEdited: ed.change(rowLoader.rowId, newValue)
                    }
                }
            }
            Component {
                id: toggleRow
                CsSwitchRow {
                    indent: ed.indent
                    title: rowLoader.r ? rowLoader.r.title : ""
                    detail: rowLoader.r ? rowLoader.r.detail : ""
                    enabled: !!rowLoader.r && rowLoader.r.enabled
                    checked: !!rowLoader.r && rowLoader.r.on === true
                    onToggled: ed.change(rowLoader.rowId, on)
                }
            }
            Component {
                id: noteRow
                CsNote {
                    indent: ed.indent
                    warning: !!rowLoader.r && rowLoader.r.warning === true
                    text: rowLoader.r ? rowLoader.r.detail : ""
                }
            }
        }
    }
}
