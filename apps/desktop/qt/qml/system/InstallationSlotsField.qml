import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP FEATUI -- A unit's slots (FEAT1): which audio link channel each of
// its 8 inputs plays, "-" for silent. Locked while a zone's source owns them
// (the zone's source routes them); "Its own" clears hand-made ones.
InstallationField {
    id: field
    required property string unitId
    property bool editable: false
    readonly property var unit: InstallationController.element(unitId, InstallationController.generation)
    readonly property var slots: unit.slots || []
    readonly property bool locked: (unit.slotsZone || "").length > 0
    readonly property string shown: slots.length === 0 ? "" : slots.map(n => n === 255 ? "-" : String(n)).join(" ")
    title: qsTr("Audio link channel on each input (- for none)")
    issue: InstallationController.issueOf(unitId, "slots", InstallationController.generation)
    issueName: "Installation.slotsIssue." + unitId

    // "8 9 - -" as 8 entries (channels 0-63, 255 for silent); null for text that is not that
    function parsed(text) {
        const parts = text.trim().length === 0 ? [] : text.trim().split(/[\s,;]+/)
        if (parts.length > 8) return null
        const out = []
        for (const p of parts) {
            if (p === "-") { out.push(255); continue }
            const n = Number(p)
            if (!Number.isInteger(n) || n < 0 || n > 63) return null
            out.push(n)
        }
        while (out.length > 0 && out.length < 8) out.push(255)
        return out
    }

    BorderedField {
        id: entry
        objectName: "Installation.slots." + field.unitId
        implicitWidth: 180
        enabled: field.editable && !field.locked
        label: qsTr("Channels on the inputs of %1").arg(field.unit.name || "")
        placeholderText: qsTr("Its own")
        function sync() { if (!activeFocus) text = field.shown }
        Component.onCompleted: sync()
        Connections {
            target: InstallationController
            function onDraftChanged() { entry.sync() }
        }
        onCommitted: t => {
            const list = field.parsed(t)
            if (list === null || !InstallationController.setSlots(field.unitId, list)) text = field.shown
        }
    }
    AppButton {
        objectName: "Installation.ownSlots." + field.unitId
        visible: field.editable && !field.locked && field.slots.length > 0
        text: qsTr("Its Own")
        toolTip: qsTr("The unit plays its own slots again, as set on it")
        onClicked: InstallationController.setSlots(field.unitId, [])
    }
    Text {
        objectName: "Installation.slotsOwner." + field.unitId
        visible: field.locked
        Layout.maximumWidth: 260
        //: %1 a zone, %2 the audio link source it plays
        text: qsTr("Set by %1, which plays %2").arg(field.unit.slotsZone || "").arg(field.unit.slotsSource || "")
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
        Accessible.role: Accessible.StaticText
        Accessible.name: text
    }
}
