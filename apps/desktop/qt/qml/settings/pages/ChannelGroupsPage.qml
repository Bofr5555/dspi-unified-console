import QtQuick 2.15
import QtQuick.Controls 2.15
import "../"
import "../cs"
import "../../components"

// Channel groups: named sets of channels one control can drive together.
SettingsPage {
    id: page
    title: qsTr("Channel Groups")
    subtitle: qsTr("Name a set of channels so one control, remote key, macro step or display page can drive them together.")

    CsHelper { id: cs }

    property var drafts: []
    property var live: []
    property var expanded: ({})
    property var messages: ({})

    function seed() { drafts = cs.copy(cs.groups); live = cs.copy(cs.groups); messages = ({}) }
    function follow() {
        var ng = cs.groups, d = drafts.slice()
        if (d.length !== ng.length) { seed(); return }
        var moved = false
        for (var k = 0; k < ng.length; k++) if (!cs.sameGroup(ng[k], live[k])) moved = true
        if (!moved) return
        for (var g = 0; g < ng.length; g++) if (cs.sameGroup(d[g], live[g])) d[g] = cs.copy(ng[g])
        drafts = d
        live = cs.copy(ng)
    }
    Connections {
        target: controlSurfaces
        function onChanged() { page.follow() }
        function onReloaded() { page.seed() }
    }
    Component.onCompleted: seed()

    function setDraft(g, v) { var d = drafts.slice(); d[g] = v; drafts = d }
    function setMap(name, key, v) {
        var o = Object.assign({}, page[name])
        if (v === undefined) delete o[key]; else o[key] = v
        page[name] = o
    }
    // In use in the editor: a kind or a name before it has members
    function inUse(g) {
        var d = drafts[g]
        return !!d && (d.targetKind !== 0 || d.name !== "" || cs.groupConfigured(live[g]))
    }
    readonly property var visibleGroups: {
        var list = []
        for (var g = 0; g < Math.min(cs.groupCount, drafts.length); g++) if (inUse(g)) list.push(g)
        return list
    }
    readonly property int firstFree: {
        for (var g = 0; g < Math.min(cs.groupCount, drafts.length); g++) if (!inUse(g)) return g
        return -1
    }
    function usedBy(g) {
        var n = 0
        for (var s = 0; s < cs.slotCount; s++) {
            var b = cs.bindings[s]
            if (cs.isConfigured(b) && (b.flags & cs.flagGroup) && b.target === g) n++
        }
        return n
    }
    function summary(grp) {
        if (!cs.groupConfigured(grp)) return qsTr("No channels selected.")
        var names = cs.memberList(grp.memberMask).map(function (ch) { return cs.groupChannelName(grp.targetKind, ch) })
        var head = names.slice(0, 4).join(", ")
        return names.length > 4 ? qsTr("%2 +%1 more").arg(names.length - 4).arg(head) : head
    }
    function addGroup() {
        var g = firstFree
        if (g < 0) return
        setDraft(g, { targetKind: cs.targetOutput, memberMask: 0, name: "Group " + (g + 1) })
        setMap("expanded", g, true)
        setMap("messages", g, undefined)
    }
    function applyGroup(g) {
        var r = controlSurfaces.apply({ op: "group", index: g, group: drafts[g] })
        var st = r && r.status !== undefined ? r.status : 0xFF
        setDraft(g, cs.copy(cs.groups[g]))
        setMap("messages", g, st !== 0 ? cs.statusMessage(st) : undefined)
    }
    function removeGroup(g) {
        setMap("messages", g, undefined)
        setMap("expanded", g, undefined)
        var wasLive = cs.groupConfigured(live[g])
        setDraft(g, { targetKind: 0, memberMask: 0, name: "" })
        if (wasLive) applyGroup(g)
    }

    Column {
        width: parent.width
        spacing: 12

        Text {
            visible: !cs.supported
            text: qsTr("Reading control-surface capabilities from the device...")
            font.pixelSize: 12
            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
        }

        CsEmptyState {
            visible: cs.supported && page.visibleGroups.length === 0
            title: qsTr("No Channel Groups Configured")
            text: qsTr("Name a set of channels so one control can drive them together - a stereo pair, a zone, every output at once.")
            Icon { name: "group"; size: 28; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45) }
            action: CsButton {
                text: qsTr("Add Group"); icon: "plus"; primary: true
                enabled: page.firstFree >= 0 && cs.connected
                onClicked: page.addGroup()
            }
        }

        Repeater {
            model: page.drafts.length
            delegate: CsCard {
                id: card
                readonly property int g: index
                readonly property var d: page.drafts[g] || ({ targetKind: 0, memberMask: 0, name: "" })
                readonly property bool dirty: !cs.sameGroup(d, page.live[g])
                readonly property int health: cs.ext.groupStatus ? cs.ext.groupStatus[g] : 0
                visible: g < cs.groupCount && page.inUse(g)
                expanded: page.expanded[g] === true
                name: d.name
                placeholder: qsTr("Group %1").arg(g + 1)
                summary: page.summary(d)
                removeTip: qsTr("Remove this group")
                onToggle: page.setMap("expanded", g, expanded ? undefined : true)
                onNameEdited: { var nd = cs.copy(card.d); nd.name = text; page.setDraft(card.g, nd) }
                onRemoveClicked: page.removeGroup(g)

                badge: Icon { name: "group"; size: 18; color: isMacOS ? MacColors.accent : "#3a96ff" }
                trailing: Icon {
                    visible: card.health !== 0 && cs.groupConfigured(page.live[card.g])
                    name: "warning"
                    size: 14
                    color: "#ff9f0a"
                    ToolTip.text: cs.statusMessage(card.health)
                    ToolTip.visible: warnMouse.containsMouse
                    MouseArea { id: warnMouse; anchors.fill: parent; hoverEnabled: true }
                }

                CsPickerRow {
                    title: qsTr("Channel Type")
                    detail: qsTr("Which set of channels the members are numbered in.")
                    options: [{ value: 1, text: qsTr("Inputs") }, { value: 2, text: qsTr("Outputs") }, { value: 3, text: qsTr("All Channels") }]
                    value: card.d.targetKind
                    // The mask means different channels under another kind
                    onChosen: { var nd = cs.copy(card.d); nd.targetKind = value; nd.memberMask = 0; page.setDraft(card.g, nd) }
                }
                CsRow {
                    title: qsTr("Members")
                    detail: qsTr("Every channel this group drives together.")
                }
                Item {
                    readonly property int count: cs.groupChannelCount(card.d.targetKind)
                    width: parent.width
                    height: count === 0 ? noChannels.height + 10 : grid.height + 12
                    Text {
                        id: noChannels
                        visible: parent.count === 0
                        x: 14
                        text: qsTr("No channels of this type on the connected device.")
                        font.pixelSize: 11
                        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
                    }
                    Grid {
                        id: grid
                        x: 14
                        width: parent.width - 28
                        columns: 3
                        rowSpacing: 4
                        Repeater {
                            model: parent.parent.count
                            CsCheck {
                                width: grid.width / 3
                                text: cs.groupChannelName(card.d.targetKind, index)
                                checked: ((card.d.memberMask >>> index) & 1) === 1
                                onToggled: {
                                    var nd = cs.copy(card.d)
                                    nd.memberMask = on ? (nd.memberMask | (1 << index)) >>> 0 : (nd.memberMask & ~(1 << index)) >>> 0
                                    page.setDraft(card.g, nd)
                                }
                            }
                        }
                    }
                }
                CsNote {
                    readonly property int n: page.usedBy(card.g)
                    text: n > 0 ? (n === 1 ? qsTr("Used by %1 control. Emptying this group or changing its channel type deactivates them until it fits again.").arg(n) : qsTr("Used by %1 controls. Emptying this group or changing its channel type deactivates them until it fits again.").arg(n)) : ""
                }

                footer: CsApplyRow {
                    visible: card.expanded || card.dirty || page.messages[card.g] !== undefined
                    dirty: card.dirty
                    canApply: card.dirty && cs.groupConfigured(card.d)
                    hint: !cs.groupConfigured(card.d) ? qsTr("Pick at least one channel.") : ""
                    message: page.messages[card.g] || ""
                    onApply: page.applyGroup(card.g)
                    onRevert: { page.setDraft(card.g, cs.copy(page.live[card.g])); page.setMap("messages", card.g, undefined) }
                }
            }
        }

        Item {
            visible: cs.supported && page.visibleGroups.length > 0
            width: parent.width
            height: 30
            CsButton {
                text: qsTr("Add Group"); icon: "plus"
                enabled: page.firstFree >= 0 && cs.connected
                anchors.verticalCenter: parent.verticalCenter
                onClicked: page.addGroup()
            }
            Text {
                visible: page.firstFree < 0
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("All %1 group slots are in use.").arg(cs.groupCount)
                font.pixelSize: 12
                color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
            }
        }
    }

    Text {
        width: parent.width
        wrapMode: Text.WordWrap
        leftPadding: 4
        rightPadding: 4
        text: qsTr("Choose a group in a control's Channel or Group menu. Relative moves (an encoder, a button) step every member from its own value, so the balance between them survives. A knob moves the group's average and keeps those offsets unless \"Match Members Exactly\" is on.\n\nGroups are stored on the device alongside the controls and share their Save and Revert.")
        font.pixelSize: 11
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45)
    }
}
