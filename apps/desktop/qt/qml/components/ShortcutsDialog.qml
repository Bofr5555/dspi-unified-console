import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP UA5 -- Help › Keyboard Shortcuts (the action help.shortcuts, Ctrl+/):
// every key of the app on one sheet. First the Modern layout's own keys for
// moving around (the palette, F6 through the areas, the arrows in the rail and
// the tab row, Ctrl+PageDown/PageUp through the tabs), then each menu of the
// menu table with its rows that have a key, in the table's words and order. A
// submenu whose rows all have keys is one line ("Recall Preset", Ctrl+1 …
// Ctrl+9). Read and closed: Close or Escape.
//
//     ShortcutsDialog { id: keys }
//     keys.open()
Dialog {
    id: dialog
    objectName: "ShortcutsDialog"
    title: qsTr("Keyboard Shortcuts")

    // The Modern layout's keys that are no menu row's
    readonly property var movingRows: [
        { id: "palette", text: qsTr("Search settings, channels and commands"), keys: PaletteController.shortcut },
        { id: "area", text: qsTr("Next area: top bar, sections, page, inspector, dock"), keys: "F6" },
        { id: "areaBack", text: qsTr("Previous area"), keys: "Shift+F6" },
        { id: "arrows", text: qsTr("Another section or tab, in the section rail or the tab row"), keys: qsTr("Arrow keys") },
        { id: "tabNext", text: qsTr("Next tab of the page"), keys: "Ctrl+PgDown" },
        { id: "tabBack", text: qsTr("Previous tab of the page"), keys: "Ctrl+PgUp" },
        { id: "shortcuts", text: qsTr("This list"), keys: "Ctrl+/" }
    ]

    // [{ heading } | { id, text, keys }], in order
    function rowsNow() {
        const out = [{ heading: qsTr("Moving Around") }].concat(movingRows)
        const shown = r => r.shortcut && (!r.optional || Shell.handles(r.id))
                           && (!r.platformHidden || DeviceModel.platformName !== r.platformHidden)
        for (const m of MenuTable.menus) {
            const rows = []
            const walk = list => {
                for (const r of list) {
                    if (r.rows) {
                        const keyed = r.rows.filter(shown)
                        if (keyed.length && keyed.length === r.rows.filter(x => x.id).length && keyed.length > 2)
                            rows.push({ id: keyed[0].id, text: r.submenu, keys: keyed[0].shortcut + " … " + keyed[keyed.length - 1].shortcut })
                        else walk(r.rows)
                    } else if (r.id && shown(r)) {
                        rows.push({ id: r.id, text: r.text, keys: r.shortcut })
                    }
                }
            }
            walk(m.rows)
            if (rows.length) out.push({ heading: m.title.replace("&", "") }, ...rows)
        }
        return out
    }
    property var rows: []
    onAboutToShow: rows = rowsNow()
    onOpened: closeButton.forceActiveFocus()

    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    // Centred in the window it belongs to
    readonly property real hostWidth: parent ? parent.width : 560
    readonly property real hostHeight: parent ? parent.height : 600
    width: Math.min(560, hostWidth - 2 * Theme.spacingLarge)
    height: Math.min(implicitHeight, hostHeight - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: Math.max(Theme.spacingLarge, (hostHeight - height) / 3)
    padding: Theme.spacingLarge
    topPadding: Theme.spacingSmall

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.width: Theme.dark ? 1 : 0
        border.color: Theme.separator
    }

    header: Text {
        leftPadding: Theme.spacingLarge
        rightPadding: Theme.spacingLarge
        topPadding: Theme.spacingLarge
        textFormat: Text.PlainText
        text: dialog.title
        font: Theme.fontTitle
        color: Theme.text
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }

    contentItem: ListView {
        id: list
        objectName: "ShortcutsDialog.list"
        implicitHeight: contentHeight
        clip: true
        model: dialog.rows
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: AppScrollBar {}
        Accessible.role: Accessible.List
        Accessible.name: dialog.title
        delegate: Item {
            id: row
            required property var modelData
            required property int index
            readonly property bool heading: modelData.heading !== undefined
            objectName: heading ? "" : "ShortcutsDialog.row." + modelData.id
            width: ListView.view.width
            implicitHeight: heading ? (index === 0 ? 20 : 20 + Theme.spacingLarge) : 26
            Text {
                visible: row.heading
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 4
                textFormat: Text.PlainText
                text: row.heading ? row.modelData.heading : ""
                font: Theme.fontHeading
                color: Theme.textMuted
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            RowLayout {
                visible: !row.heading
                anchors.fill: parent
                anchors.rightMargin: Theme.spacingSmall
                spacing: Theme.spacing
                Text {
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: row.heading ? "" : row.modelData.text
                    font: Theme.fontBody
                    color: Theme.text
                    elide: Text.ElideRight
                }
                Text {
                    objectName: row.heading ? "" : "ShortcutsDialog.key." + row.modelData.id
                    textFormat: Text.PlainText
                    text: row.heading ? "" : row.modelData.keys
                    font: Theme.fontMono
                    color: Theme.textMuted
                }
            }
            Accessible.role: heading ? Accessible.NoRole : Accessible.ListItem
            Accessible.name: heading ? "" : modelData.text + ", " + modelData.keys
        }
    }

    footer: Item {
        implicitHeight: closeButton.implicitHeight + 2 * Theme.spacing
        AppButton {
            id: closeButton
            objectName: "ShortcutsDialog.close"
            x: parent.width - width - Theme.spacingLarge
            y: Theme.spacing
            text: qsTr("Close")
            primary: true
            onClicked: dialog.close()
            Keys.onReturnPressed: dialog.close()
            Keys.onEnterPressed: dialog.close()
        }
    }
}
