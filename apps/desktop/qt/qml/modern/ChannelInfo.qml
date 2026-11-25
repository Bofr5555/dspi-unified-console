import QtQuick
import UConsole

// -- WP LAY4 -- What a wire channel is (DeviceModel.channels' row for it).
QtObject {
    property int channel: -1
    readonly property int row: DeviceModel.revision < 0 || channel < 0 ? -1 : DeviceModel.channels.rowOfChannel(channel)
    readonly property bool known: row >= 0
    readonly property bool isOutput: known && DeviceModel.channels.get(row, "kind") === "output"
    // The input's or the output's number, from 0
    readonly property int number: known ? (DeviceModel.channels.get(row, "number") ?? -1) : -1
    readonly property bool pdm: known && DeviceModel.channels.get(row, "pdm") === true
    readonly property string label: field("label")
    // -- WP SHOWFIX -- the core's name for it: the speaker layout's "Centre woofer", else the label
    readonly property string displayName: field("displayName")
    readonly property string descriptor: field("descriptor")
    // -- WP UA1 -- (UXP A1) one name per output in Modern: the name first, its
    // number second ("Left woofer · 1"); a name that says its number already
    // ("Out 1 · L", "3 · None") stands alone. Nothing routed to it: `unused`.
    readonly property string name: displayName || label
    readonly property string numberText: isOutput ? numberOf(name, number) : ""
    readonly property string title: titleOf(channel)
    readonly property bool unused: DeviceModel.revision >= 0 && isOutput && DeviceModel.channels.get(row, "unused") === true
    // What the command palette finds it by: its names and their words, the
    // speaker's side and role, its number ("tweeter", "left", "Out 2", "2")
    readonly property var keywords: {
        if (!known) return []
        const words = [name, label, descriptor, title]
        if (isOutput) words.push(qsTr("Out %1").arg(number + 1), String(number + 1))
        for (const w of [name, label]) for (const part of w.split(/[\s·]+/)) words.push(part)
        return words.filter((w, i) => w.length > 0 && words.indexOf(w) === i)
    }
    readonly property color tint: known ? ChannelPalette.forRow(isOutput ? "output" : "input", number, pdm) : Theme.textMuted

    // Named here too: a row keeps its place while its name changes
    // (-- WP LAY5a -- or goes, with its device, before the revision says so)
    function field(name) { return DeviceModel.revision < 0 || !known ? "" : DeviceModel.channels.get(row, name) || "" }

    // -- WP UA1 -- "" when `name` starts with output `n`'s number (from 0), else "2"
    function numberOf(name, n) {
        const shown = String(n + 1)
        const own = name.match(/^(?:Out )?(\d+)(?!\d)/)
        return n < 0 || (own && own[1] === shown) ? "" : shown
    }
    // Any channel's title, for a list of others (the Routing inspector's chips)
    function titleOf(ch) {
        const r = DeviceModel.revision < 0 || ch < 0 ? -1 : DeviceModel.channels.rowOfChannel(ch)
        if (r < 0) return ""
        const shownName = DeviceModel.channels.get(r, "displayName") || DeviceModel.channels.get(r, "label") || ""
        if (DeviceModel.channels.get(r, "kind") !== "output") return shownName
        const n = numberOf(shownName, DeviceModel.channels.get(r, "number") ?? -1)
        //: An output's name and its number, "Left woofer · 1"
        return n.length ? qsTr("%1 · %2").arg(shownName).arg(n) : shownName
    }
}
