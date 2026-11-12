import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- The display's settings and its dashboard pages (the Mac's
// displayStatusRow, displayConfigRows and displayPagesSection). Each change
// applies at once. While this shows, the panel's state is read every second.
ColumnLayout {
    id: section
    readonly property var disp: CsController.revision < 0 ? ({}) : CsController.display()

    Layout.fillWidth: true
    spacing: Theme.spacing
    Component.onCompleted: CsController.watchDisplay(true)
    Component.onDestruction: CsController.watchDisplay(false)

    RowLayout {
        visible: (section.disp.message || "").length > 0
        Layout.fillWidth: true
        spacing: 6
        Icon { name: "triangle-alert"; size: 12; color: Theme.warning }
        Text {
            objectName: "Cs.display.message"
            Layout.fillWidth: true
            text: section.disp.message || ""
            font: Theme.fontSmall
            color: Theme.warning
            wrapMode: Text.Wrap
        }
    }
    HwRow {
        title: qsTr("Panel State")
        caption: section.disp.panelDetail || ""
        Text {
            objectName: "Cs.display.state"
            text: section.disp.panelState || ""
            font: Theme.fontSmall
            color: section.disp.panelGood ? Theme.success : section.disp.panelError ? Theme.warning : Theme.textMuted
        }
    }

    SectionHeading { text: qsTr("Behavior"); Layout.fillWidth: true }
    CsRows {
        objectName: "Cs.display.rows"
        rows: section.disp.rows || []
        enabled: CsController.writable
        onEdited: (id, value) => CsController.editDisplay(id, value)
    }

    SectionHeading { text: qsTr("Dashboard Pages"); Layout.fillWidth: true }
    Repeater {
        model: section.disp.pages || []
        delegate: RowLayout {
            id: pageRow
            required property var modelData
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            Text {
                Layout.alignment: Qt.AlignTop
                Layout.preferredWidth: 18
                horizontalAlignment: Qt.AlignRight
                text: pageRow.modelData.index + 1
                font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontBody.pixelSize,
                                weight: pageRow.modelData.shown ? Font.Bold : Font.Normal })
                color: pageRow.modelData.shown ? Theme.accentText : Theme.textMuted
                Accessible.name: pageRow.modelData.shown ? qsTr("On the display now") : ""
            }
            CsRows {
                objectName: "Cs.page." + pageRow.modelData.index
                rows: pageRow.modelData.rows || []
                enabled: CsController.writable
                onEdited: (id, value) => CsController.editPage(pageRow.modelData.index, id, value)
            }
            IconButton {
                objectName: "Cs.page." + pageRow.modelData.index + ".remove"
                Layout.alignment: Qt.AlignTop
                iconName: "circle-minus"
                iconSize: 12
                toolTip: qsTr("Remove this page")
                enabled: CsController.writable
                onClicked: CsController.removePage(pageRow.modelData.index)
            }
        }
    }
    RowLayout {
        Layout.fillWidth: true
        AppButton {
            objectName: "Cs.display.addPage"
            text: qsTr("Add Page")
            iconName: "plus"
            enabled: section.disp.canAddPage === true
            onClicked: CsController.addPage()
        }
        Item { implicitHeight: 1; Layout.fillWidth: true }
        Text {
            visible: section.disp.canAddPage !== true && (section.disp.pages || []).length > 0
            text: qsTr("All %1 page slots are in use.").arg(section.disp.maxPages || 0)
            font: Theme.fontSmall
            color: Theme.textMuted
        }
    }
}
