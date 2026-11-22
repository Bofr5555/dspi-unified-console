import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY2 -- One section of the Modern layout's inspector: a heading that
// folds it, over the same content Classic shows in a popover
// (InspectorSection { LimiterSettingsContent { active: section.shown } }).
// `shown` is true while the content can be seen, for content that works only
// then: the limiter's poll, the Linkwitz draft started afresh.
ColumnLayout {
    id: section
    property string title
    property bool expanded: true
    // `visible` is the effective visibility: false while an ancestor is hidden
    readonly property bool shown: visible && expanded
    default property alias content: body.data

    spacing: Theme.spacingSmall

    AbstractButton {
        id: heading
        objectName: section.objectName.length ? section.objectName + ".heading" : ""
        Layout.fillWidth: true
        implicitHeight: 26
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        onClicked: section.expanded = !section.expanded

        contentItem: RowLayout {
            spacing: 6
            Icon {
                name: section.expanded ? "chevron-down" : "chevron-right"
                size: 12
                color: Theme.textMuted
            }
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: section.title
                font: Theme.fontHeading
                color: heading.hovered ? Theme.text : Theme.textMuted
                elide: Text.ElideRight
            }
        }
        background: Rectangle {
            radius: Theme.radiusSmall
            color: "transparent"
            border.width: heading.visualFocus ? 2 : 0
            border.color: Theme.focusRing
        }

        Accessible.role: Accessible.Button
        Accessible.name: section.title
        Accessible.description: section.expanded ? qsTr("Expanded") : qsTr("Collapsed")
        Accessible.onPressAction: heading.clicked()
    }

    ColumnLayout {
        id: body
        Layout.fillWidth: true
        visible: section.expanded
        spacing: 0
    }
}
