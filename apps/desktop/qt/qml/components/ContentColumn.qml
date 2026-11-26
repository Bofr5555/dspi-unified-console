import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP DL0 -- The column a Modern form or list lives in (DESIGN-LANGUAGE §3):
// left-aligned, never centred, no wider than Theme.contentMaxWidth (760);
// sections Theme.spacingSection (24) apart. The room on its right is for an
// aside (a preview graph, a chain) or stays empty. In a layout it takes the
// row's width up to the maximum by itself; outside one, bind `width` to
// Math.min(room, maximumWidth).
//
//     ContentColumn {                    // in a ColumnLayout or RowLayout
//         SectionHeading { text: qsTr("Detector") }
//         FormRow { title: qsTr("Attack"); NumericField { ... } }
//     }
ColumnLayout {
    id: column
    property int maximumWidth: Theme.contentMaxWidth

    spacing: Theme.spacingSection
    Layout.fillWidth: true
    Layout.maximumWidth: maximumWidth
    Layout.alignment: Qt.AlignLeft | Qt.AlignTop
}
