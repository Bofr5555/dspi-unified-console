import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- The rows the core describes for one control-surface record
// (dspi_cs_describe): a pick is a popup, a number a stepper field in the
// row's units, a toggle a switch, a note a line of words. Each row has the
// title and its explanation on the left, as the Mac's settingRow.
// `edited(id, value)` fires on a person's change only.
ColumnLayout {
    id: list
    property var rows: []
    // Row ids not drawn here (a page draws them its own way)
    property var skip: []
    signal edited(string id, var value)

    Layout.fillWidth: true
    spacing: Theme.spacing

    // The rows by id and kind, replaced only when that changes, so the fields
    // stay (with their focus and open popups) while their values change
    property var shown: []
    function rowFor(tag) { return list.rows.find(r => r.id + ":" + r.kind === tag) || ({}) }
    function syncShown() {
        const next = list.rows.filter(r => list.skip.indexOf(r.id) < 0).map(r => r.id + ":" + r.kind)
        if (next.join("|") !== shown.join("|")) shown = next
    }
    onRowsChanged: syncShown()
    onSkipChanged: syncShown()
    Component.onCompleted: syncShown()

    Repeater {
        model: list.shown
        delegate: Loader {
            id: rowLoader
            required property string modelData
            readonly property var row: list.rowFor(modelData)
            Layout.fillWidth: true
            sourceComponent: row.kind === "note" ? noteRow : fieldRow
        }
    }

    Component {
        id: noteRow
        RowLayout {
            readonly property var row: parent ? parent.row : ({})
            objectName: list.objectName.length ? list.objectName + ".row." + row.id : ""
            spacing: 6
            Icon {
                visible: parent.row.warning === true
                name: "triangle-alert"
                size: 12
                color: Theme.warning
                Layout.alignment: Qt.AlignTop
            }
            Text {
                Layout.fillWidth: true
                text: parent.row.detail || ""
                font: Theme.fontSmall
                color: parent.row.warning === true ? Theme.warning : Theme.textMuted
                wrapMode: Text.Wrap
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }
        }
    }

    Component {
        id: fieldRow
        HwRow {
            id: hwRow
            readonly property var row: parent ? parent.row : ({})
            readonly property string base: list.objectName.length ? list.objectName + ".row." + row.id : ""
            objectName: base
            title: row.title || ""
            caption: row.detail || ""

            Loader {
                sourceComponent: hwRow.row.kind === "pick" ? pickField
                               : hwRow.row.kind === "number" ? numberField : toggleField
                Component {
                    id: pickField
                    AppComboBox {
                        objectName: hwRow.base.length ? hwRow.base + ".combo" : ""
                        label: hwRow.row.title || ""
                        // A sectioned list reads "Section: item"; a pin someone
                        // holds names its holder; one not offered says so
                        readonly property var options: hwRow.row.options || []
                        property var texts: []
                        function syncTexts() {
                            const next = options.map(o => (o.section ? o.section + ": " : "") + o.text
                                                          + (o.owner ? " (" + o.owner + ")" : ""))
                            if (next.join("\n") !== texts.join("\n")) texts = next
                        }
                        onOptionsChanged: syncTexts()
                        Component.onCompleted: syncTexts()
                        implicitWidth: 200
                        model: texts
                        currentIndex: options.findIndex(o => o.value === hwRow.row.value)
                        displayText: hwRow.row.text || ""
                        onActivated: index => {
                            const o = options[index]
                            if (!o || o.disabled) {
                                currentIndex = options.findIndex(x => x.value === hwRow.row.value)
                                return
                            }
                            if (o.value !== hwRow.row.value) list.edited(hwRow.row.id, o.value)
                        }
                    }
                }
                Component {
                    id: numberField
                    AppSpinBox {
                        objectName: hwRow.base.length ? hwRow.base + ".spin" : ""
                        label: hwRow.row.title || ""
                        implicitWidth: 130
                        realFrom: hwRow.row.min
                        realTo: hwRow.row.max
                        realStep: hwRow.row.step > 0 ? hwRow.row.step : 1
                        decimals: hwRow.row.decimals || 0
                        unit: hwRow.row.unit || ""
                        realValue: hwRow.row.value
                        onCommitted: v => list.edited(hwRow.row.id, v)
                    }
                }
                Component {
                    id: toggleField
                    AppSwitch {
                        objectName: hwRow.base.length ? hwRow.base + ".switch" : ""
                        checked: hwRow.row.on === true
                        enabled: hwRow.row.enabled !== false || hwRow.row.on === true
                        onToggled: list.edited(hwRow.row.id, checked)
                        Accessible.name: hwRow.row.title || ""
                        Accessible.description: hwRow.row.detail || ""
                    }
                }
            }
        }
    }
}
