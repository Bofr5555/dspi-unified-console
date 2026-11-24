pragma Singleton
import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY3b -- The Modern layout's right-click menus (MODLAY 3.1): a channel's,
// a band's (its row or its dot), the graph's and a preset slot's. The rows are
// MenuTable's (places channel, band, graph, slot); what they do to the thing
// under the pointer is here. Whatever shows such a thing opens its menu:
//     ContextMenus.channel(channel, item, x, y, rename)
// In Classic the components open their own menus, as before.
// -- WP DL5 -- The top bar's preset popover (PresetPopover) opens a slot's menu
// from its rows; showAi() asks the top bar for the AI popover (the AI strip's
// grants and their Revoke are there).
QtObject {
    id: menus

    // -- WP DL5 --
    signal aiRequested()
    function showAi() { aiRequested() }

    // ---- Opening them, at a point of `item`
    // Wire channel `channel`; `rename` is the function that begins its renaming
    // where it is shown (none: no Rename row)
    function channel(channel, item, x, y, rename) {
        const row = DeviceModel.channels.rowOfChannel(channel)
        if (row < 0) return
        channelMenu.popupFor({
            channel: channel, isOutput: DeviceModel.channels.get(row, "kind") === "output",
            number: DeviceModel.channels.get(row, "number"), rename: rename || null
        }, item, x, y)
    }
    // Band `band` of `channel`: with the bands selected on the graph, when it is
    // one of them. `overlay` is the graph's editor when its dot was clicked.
    function band(channel, band, crossover, item, x, y, overlay) {
        const onGraph = !crossover && PeqEditorController.editedChannel === channel
        // The row under the pointer is selected, as its dot is
        if (onGraph && !overlay && !PeqEditorController.isSelected(band) && ChannelController.band(channel, band, false).type !== 0)
            PeqEditorController.rowClicked(band, 0)
        const selected = onGraph ? PeqEditorController.selected : []
        bandMenu.popupFor({
            channel: channel, band: band, crossover: crossover, overlay: overlay || null,
            bands: selected.indexOf(band) >= 0 ? selected.slice().sort((a, b) => a - b) : [band]
        }, item, x, y)
    }
    // The graph `plot`; `overlay` is its editor while it edits
    function graph(plot, overlay, item, x, y) {
        graphMenu.popupFor({ plot: plot, overlay: overlay || null, model: overlay ? overlay.menuModel() : ({}) }, item, x, y)
    }
    // Preset slot `slot`; `dialogs` is the shell's PresetDialogs
    function slot(slot, dialogs, item, x, y) {
        if (slot < 0 || !DeviceModel.connected) return
        slotMenu.popupFor({ slot: slot, dialogs: dialogs, active: slot === DeviceModel.presets.activeSlot,
                            occupied: SidebarController.presetOccupied(slot) }, item, x, y)
    }
    function closeAll() {
        for (const m of [channelMenu, bandMenu, graphMenu, slotMenu]) m.close()
        if (clearQuestion && clearQuestion.visible) clearQuestion.finish("reject")
    }

    // What they are about is the active device's
    readonly property Connections leaving: Connections {
        target: DeviceRegistry
        function onActiveLeaving() { menus.closeAll() }
    }
    readonly property Connections switching: Connections {
        target: LayoutController
        function onLayoutChanging() { menus.closeAll() }
    }
    readonly property bool writable: DeviceModel.connected && !DeviceModel.readOnly

    // ---- A channel
    readonly property ActionMenu channelMenu: ActionMenu {
        objectName: "ContextMenus.channel"
        place: "channel"
        handlers: ({
            "channel.rename": {
                shown: t => t.rename !== null,
                enabled: t => menus.writable,
                run: t => t.rename()
            },
            "channel.curve": {
                on: t => GraphController.isCurveVisible(t.channel),
                run: t => GraphController.setCurveVisible(t.channel, !GraphController.isCurveVisible(t.channel))
            },
            "channel.copy": {
                keys: t => t.channel === SidebarController.selectedChannel,
                run: t => SidebarController.copyChannel(t.channel)
            },
            "channel.paste": {
                keys: t => t.channel === SidebarController.selectedChannel,
                enabled: t => SidebarController.hasClipboard && menus.writable,
                run: t => SidebarController.pasteChannel(t.channel)
            },
            "channel.identify": {
                shown: t => t.isOutput && DeviceModel.connected && SidebarController.identifySupported,
                enabled: t => menus.writable,
                run: t => SidebarController.identifyOutput(t.number)
            },
            "channel.mute": {
                shown: t => t.isOutput,
                enabled: t => ChannelController.writable && ChannelController.output(t.number).enabled === true,
                on: t => ChannelController.output(t.number).muted === true,
                run: t => ChannelController.setOutputMute(t.number, ChannelController.output(t.number).muted !== true)
            },
            "channel.routing": { shown: t => t.isOutput },
            "channel.headphone": {
                shown: t => !t.isOutput,
                run: t => {
                    AutoEqController.pair = t.number >> 1
                    Shell.trigger("autoeq.browse")
                }
            },
            "channel.clear": {
                enabled: t => ChannelController.writable,
                run: t => menus.askClear(t)
            }
        })
    }
    property QuestionDialog clearQuestion: null
    readonly property Component clearComponent: Component {
        QuestionDialog {
            objectName: "ContextMenus.clearQuestion"
            ofDevice: true
            property var target: ({})
            readonly property bool linked: !target.isOutput && ChannelController.linkedPartner(target.channel) >= 0
            title: qsTr("Clear All Bands?")
            text: linked ? qsTr("Every band on both channels of the linked pair will be reset to its default (flat) state. Undo puts them back.")
                         : qsTr("Every band of this channel will be reset to its default (flat) state. Undo puts them back.")
            acceptText: qsTr("Clear All")
            onAnswered: answer => {
                if (answer !== "accept") return
                if (target.isOutput) ChannelController.clearBands(target.channel, false)
                else ChannelController.clearInputPeq(target.channel)
            }
        }
    }
    function askClear(t) {
        const root = channelMenu.window ? channelMenu.window.contentItem : null
        if (!root) return
        if (!clearQuestion) clearQuestion = clearComponent.createObject(root)
        clearQuestion.parent = root.Overlay.overlay
        clearQuestion.target = t
        clearQuestion.open()
    }

    // ---- A band
    readonly property ActionMenu bandMenu: ActionMenu {
        objectName: "ContextMenus.band"
        place: "band"
        readonly property var params: t => ChannelController.band(t.channel, t.band, t.crossover)
        handlers: ({
            "band.shape": {
                shown: t => !t.crossover,
                enabled: t => ChannelController.writable,
                rows: t => menus.shapeRows(t),
                run: (t, value) => PeqEditorController.setShape(t.channel, t.bands, value, menus.orderFor(t, value))
            },
            "band.slope": {
                shown: t => !t.crossover,
                enabled: t => ChannelController.writable,
                text: t => PeqEditorController.slopeTitle(bandMenu.params(t).type),
                rows: t => PeqEditorController.slopes(bandMenu.params(t).type),
                run: (t, value) => PeqEditorController.setShape(t.channel, t.bands, value.shape, value.order)
            },
            "band.bypass": {
                shown: t => ChannelController.bypassSupported,
                enabled: t => ChannelController.writable && bandMenu.params(t).type !== 0,
                on: t => bandMenu.params(t).bypass === true,
                run: t => {
                    if (t.overlay) t.overlay.menuAction("bypass")
                    else PeqEditorController.bypassBands(t.channel, t.bands, t.crossover, bandMenu.params(t).bypass !== true)
                }
            },
            "band.invert": {
                shown: t => !t.crossover && ChannelController.usesGain(bandMenu.params(t).type),
                enabled: t => ChannelController.writable,
                run: t => {
                    if (t.overlay) t.overlay.menuAction("invert")
                    else PeqEditorController.invertGain(t.channel, t.bands)
                }
            },
            "band.copy": {
                shown: t => !t.crossover,
                enabled: t => bandMenu.params(t).type !== 0,
                run: t => PeqEditorController.copyBand(t.channel, t.band)
            },
            "band.paste": {
                shown: t => !t.crossover,
                enabled: t => ChannelController.writable && PeqEditorController.hasBand,
                run: t => PeqEditorController.pasteBand(t.channel, t.bands)
            },
            "band.reset": {
                //: %1 is how many bands are selected
                text: t => t.bands.length > 1 ? qsTr("Reset %1 Bands").arg(t.bands.length) : qsTr("Reset Band"),
                enabled: t => ChannelController.writable && (t.bands.length > 1 || bandMenu.params(t).type !== 0),
                run: t => {
                    if (t.overlay) t.overlay.menuAction("delete")
                    else PeqEditorController.resetBands(t.channel, t.bands, t.crossover)
                }
            }
        })
    }
    function shapeRows(t) {
        const now = PeqEditorController.shapeOrder(ChannelController.band(t.channel, t.band, false).type)
        return PeqEditorController.shapes().map(s => ({ name: s.shape, text: s.title, value: s.shape, checked: s.shape === now.shape }))
    }
    // The slope a shape starts with: the band's own where the shape has it
    function orderFor(t, shape) {
        const now = PeqEditorController.shapeOrder(ChannelController.band(t.channel, t.band, false).type)
        return now.shape === shape ? now.order : 2
    }

    // ---- The graph
    readonly property ActionMenu graphMenu: ActionMenu {
        objectName: "ContextMenus.graph"
        place: "graph"
        handlers: ({
            "graph.addBand": {
                shown: t => t.overlay !== null,
                enabled: t => t.model.addBand === true,
                run: t => t.overlay.menuAction("addBand")
            },
            "graph.selectAll": {
                shown: t => t.overlay !== null,
                enabled: t => t.model.selectAll === true,
                run: t => t.overlay.menuAction("selectAll")
            },
            "graph.deselect": {
                shown: t => t.overlay !== null,
                enabled: t => t.model.deselect === true,
                run: t => t.overlay.menuAction("deselect")
            },
            "graph.delete": {
                shown: t => (t.model.delete || "").length > 0,
                text: t => t.model.delete,
                run: t => t.overlay.menuAction("deleteSelected")
            },
            "graph.rta": {
                shown: t => RtaController.available,
                on: t => RtaController.showGraph,
                run: t => RtaController.showGraph = !RtaController.showGraph
            },
            "graph.phase": {
                on: t => GraphController.showPhase,
                run: t => GraphController.showPhase = !GraphController.showPhase
            },
            "window.popOutGraph": { shown: t => !GraphController.poppedOut },
            "graph.settings": { run: t => SettingsController.showPage("graphing") }
        })
    }

    // ---- A preset slot
    readonly property ActionMenu slotMenu: ActionMenu {
        objectName: "ContextMenus.slot"
        place: "slot"
        readonly property bool free: menus.writable && !SidebarController.presetBusy
        handlers: ({
            "slot.recall": {
                enabled: t => slotMenu.free && !t.active,
                run: t => SidebarController.requestPresetSwitch(t.slot)
            },
            "slot.save": {
                enabled: t => slotMenu.free,
                // Into another slot the device copies the active one
                text: t => t.active ? MenuTable.row("slot.save").text
                          //: %1 is the active slot's label, "3: Living Room"
                         : qsTr("Copy \"%1\" Here").arg(SidebarController.presetLabel(DeviceModel.presets.activeSlot)),
                run: t => {
                    if (t.active) SidebarController.savePreset()
                    else SidebarController.requestPresetCopy(t.slot)
                }
            },
            "slot.rename": {
                enabled: t => slotMenu.free && (t.active || t.occupied),
                run: t => t.dialogs.askRename(t.slot)
            },
            "slot.startup": {
                enabled: t => slotMenu.free && (t.active || t.occupied)
                    && !(DeviceModel.presets.startupMode === 0 && DeviceModel.presets.defaultSlot === t.slot),
                run: t => SidebarController.setStartupPreset(t.slot)
            },
            "slot.copyTo": {
                // The device copies the active slot
                shown: t => t.active,
                enabled: t => slotMenu.free,
                rows: t => SidebarController.presetTitles.map((title, i) => ({ name: i, text: SidebarController.presetLabel(i), value: i }))
                                                         .filter(r => r.value !== t.slot),
                run: (t, value) => SidebarController.requestPresetCopy(value)
            },
            "slot.clear": {
                shown: t => t.occupied,
                enabled: t => slotMenu.free,
                //: %1 is a slot's label, "3: Living Room"
                text: t => qsTr("Clear \"%1\"…").arg(SidebarController.presetLabel(t.slot)),
                run: t => t.dialogs.askClear(t.slot)
            },
            "slot.clearAll": {
                shown: t => SidebarController.presetTitles.some((title, i) => SidebarController.presetOccupied(i)),
                enabled: t => slotMenu.free,
                run: t => t.dialogs.askClearAll()
            }
        })
    }
}
