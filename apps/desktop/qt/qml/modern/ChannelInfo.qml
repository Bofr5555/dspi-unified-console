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
    readonly property string descriptor: field("descriptor")
    readonly property color tint: known ? ChannelPalette.forRow(isOutput ? "output" : "input", number, pdm) : Theme.textMuted

    // Named here too: a row keeps its place while its name changes
    // (-- WP LAY5a -- or goes, with its device, before the revision says so)
    function field(name) { return DeviceModel.revision < 0 || !known ? "" : DeviceModel.channels.get(row, name) || "" }
}
