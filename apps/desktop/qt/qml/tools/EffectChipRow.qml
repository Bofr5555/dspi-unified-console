import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP DL1 -- The chips of an effect's mask (outputs, pairs, channels, tube
// types): EffectChips (or columns holding one) as its children, in a grid.
// Classic keeps the window's grid: `classicColumns` equal columns across the
// row (0: one row of them all). Modern sets them as a chip group
// (DESIGN-LANGUAGE §13): from the left, every chip as wide as the widest
// label (a chip reads `widest` from the grid it is in), as many to a line as
// the row has room for (at most `modernColumns`, 0: no limit), the rest
// wrapping under them; the room they leave stays empty at the right.
//
//     EffectChipRow {
//         Layout.fillWidth: true
//         Repeater { model: 9; EffectChip { ... } }
//     }
RowLayout {
    id: chipRow
    default property alias chips: grid.data
    property int classicColumns: 0
    property int modernColumns: 0
    readonly property alias widest: grid.widest

    spacing: 0

    GridLayout {
        id: grid
        // The widest chip, every column's width in Modern, and how many
        // there are (a Repeater is a child too, of no width)
        readonly property real widest: {
            let w = 0
            for (let i = 0; i < children.length; ++i) w = Math.max(w, children[i].implicitWidth)
            return w
        }
        readonly property int count: {
            let n = 0
            for (let i = 0; i < children.length; ++i) if (children[i].delegate === undefined) ++n
            return n
        }
        // How many fit the row's width (not the grid's: that is what they take)
        readonly property int fits: widest > 0 ? Math.max(1, Math.floor((chipRow.width + columnSpacing) / (widest + columnSpacing))) : 1
        readonly property int modernCount: Math.max(1, Math.min(count, fits, chipRow.modernColumns > 0 ? chipRow.modernColumns : count))

        // -- WP REVDFIX -- (REVD D4) Modern: the grid may be as narrow as one
        // chip and no wider than its chips, its lines following the width it
        // gets (`fits`). A grid that does not fill is fixed at its width, so
        // the chips of one line set its column's minimum (the Crossfeed window
        // at 780 px clipped its right column). Classic fills the row as before.
        Layout.fillWidth: true
        Layout.minimumWidth: LayoutController.modern ? widest : -1
        Layout.maximumWidth: LayoutController.modern ? implicitWidth : Number.POSITIVE_INFINITY
        columns: LayoutController.modern ? modernCount : Math.max(1, chipRow.classicColumns > 0 ? chipRow.classicColumns : count)
        columnSpacing: 6
        rowSpacing: 6
    }
    // Modern: the room the chips leave, so the row still takes its width
    Item {
        visible: LayoutController.modern
        Layout.fillWidth: true
        implicitHeight: 0
    }
}
