import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP2 -- One band of a filter list (the Mac's FilterRowView).
//
// PEQ rows: the bypass disc (firmware 1.1.4+), the number, the type picker
// (shapes with two orders open a submenu), then FREQ, GAIN and WIDTH (Q) as
// the type uses them; a Linkwitz Transform shows one button that opens its
// panel instead. Crossover rows: FAMILY, TYPE (low or high pass), SLOPE and
// FREQ. Every edit goes to ChannelController.
//
// For the on-graph editor (QP4) a row takes, from FilterList's
// `graphSelection`, its graph colour (`bandColor`), whether the graph has it
// selected (`selected`) and live values during a drag (`liveFreq`,
// `liveGain`, `liveQ`, NaN when none); it reports a click on its number
// (`numberClicked(band, modifiers)`) and the pointer entering or leaving it
// (`hoverChanged(band, hovering)`).
Rectangle {
    id: row
    property int channel: -1
    property int band: 0
    property bool crossover: false
    property bool includeLinkwitz: false
    property bool bypassSupported: ChannelController.bypassSupported

    // The on-graph editor's hooks (see above)
    property color bandColor: "transparent"
    property bool selected: false
    property real liveFreq: NaN
    property real liveGain: NaN
    property real liveQ: NaN
    signal numberClicked(int band, int modifiers)
    // The row's popups, for tests and the graph's band menu
    readonly property alias typePicker: typeMenu
    readonly property alias linkwitzPopover: linkwitzPanel
    signal hoverChanged(int band, bool hovering)

    // The band as shown: a staged edit at once, then the device's value
    readonly property var params: (ChannelController.revision < 0 ? undefined : ChannelController.band(channel, band, crossover))
    readonly property int type: params.type
    readonly property bool active: type !== 0
    readonly property bool bypassed: params.bypass
    readonly property bool linkwitz: type === ChannelController.linkwitzType
    readonly property bool hasColor: bandColor.a > 0
    readonly property string what: crossover ? qsTr("Crossover band %1").arg(band + 1) : qsTr("Band %1").arg(band + 1)

    objectName: "BandRow." + (crossover ? "xo." : "") + band
    property int rowHeight: 36 // -- WP LAY4 -- Modern's are denser
    implicitHeight: rowHeight
    implicitWidth: line.implicitWidth + 2 * Theme.spacingLarge
    color: selected ? Theme.selection : "transparent"
    enabled: ChannelController.writable

    // Every other row a shade lighter. -- WP DL0 -- Modern: the stripe token
    // (DESIGN-LANGUAGE §9), and the row under the pointer in the hover tone
    Rectangle {
        anchors.fill: parent
        visible: row.band % 2 === 0 && !row.selected
        color: LayoutController.modern ? Theme.stripe : Theme.text
        opacity: LayoutController.modern ? 1 : 0.03
    }
    Rectangle {
        anchors.fill: parent
        visible: LayoutController.modern && rowHover.hovered && !row.selected && row.enabled
        color: Theme.controlHover
        opacity: 0.6
    }

    HoverHandler {
        id: rowHover   // -- WP DL0 --
        onHoveredChanged: if (row.active || !hovered) row.hoverChanged(row.band, hovered)
    }

    // -- WP LAY3b -- Modern: the band's right-click menu, also by the menu key
    TapHandler {
        acceptedButtons: Qt.RightButton
        enabled: LayoutController.modern
        onTapped: point => ContextMenus.band(row.channel, row.band, row.crossover, row, point.position.x, point.position.y)
    }
    Keys.onPressed: event => {
        event.accepted = LayoutController.modern
            && (event.key === Qt.Key_Menu || (event.key === Qt.Key_F10 && event.modifiers === Qt.ShiftModifier))
        if (event.accepted) ContextMenus.band(row.channel, row.band, row.crossover, row, Theme.spacingLarge, row.height)
    }

    function chooseType(t) { ChannelController.setBandType(channel, band, crossover, t) }
    function setValue(field, v) { ChannelController.setBandValue(channel, band, crossover, field, v) }

    RowLayout {
        id: line
        anchors.fill: parent
        anchors.leftMargin: Theme.spacingLarge
        anchors.rightMargin: Theme.spacingLarge
        spacing: Theme.spacing

        // Bypass: a filled disc while the band plays, a ring when bypassed or Off
        AbstractButton {
            id: bypassDisc
            objectName: row.objectName + ".bypass"
            visible: row.bypassSupported
            enabled: row.active && row.enabled
            Layout.preferredWidth: 18
            Layout.preferredHeight: 18
            hoverEnabled: true
            focusPolicy: Qt.TabFocus
            readonly property bool on: row.active && !row.bypassed
            readonly property color ink: row.hasColor && row.active ? row.bandColor : Theme.textMuted
            onClicked: ChannelController.setBandBypass(row.channel, row.band, row.crossover, !row.bypassed)
            contentItem: Item {
                Rectangle {
                    anchors.centerIn: parent
                    width: 12
                    height: 12
                    radius: 6
                    color: bypassDisc.on ? bypassDisc.ink : "transparent"
                    border.width: 1.2
                    border.color: bypassDisc.ink
                    opacity: bypassDisc.enabled ? 1 : 0.35
                    scale: bypassDisc.hovered && bypassDisc.enabled ? 1.1 : 1
                    Behavior on scale { NumberAnimation { duration: 120 } }
                }
            }
            background: Rectangle {
                radius: 9
                color: "transparent"
                border.width: bypassDisc.visualFocus ? 2 : 0
                border.color: Theme.focusRing
            }
            HelpTip {
                text: bypassDisc.on ? qsTr("Bypass this band") : qsTr("Re-enable this band")
                visible: bypassDisc.hovered && bypassDisc.enabled
            }
            Accessible.role: Accessible.CheckBox
            Accessible.name: qsTr("%1 plays").arg(row.what)
            Accessible.checkable: true
            Accessible.checked: on
        }

        // The number: the graph colour where there is no disc to carry it
        Text {
            objectName: row.objectName + ".number"
            Layout.preferredWidth: 24
            text: row.band + 1
            font: Theme.fontBody
            color: !row.active || row.bypassed ? Theme.textDisabled
                 : row.hasColor && !row.bypassSupported ? row.bandColor : Theme.text
            TapHandler {
                enabled: row.active
                onTapped: (point, button) => row.numberClicked(row.band, point.modifiers)
            }
            Accessible.role: Accessible.StaticText
            Accessible.name: row.what
        }

        // ---- PEQ ----
        AbstractButton {
            id: typeButton
            objectName: row.objectName + ".type"
            visible: !row.crossover
            Layout.preferredWidth: 140
            Layout.preferredHeight: 20
            hoverEnabled: true
            focusPolicy: Qt.StrongFocus
            onClicked: typeMenu.popup(typeButton, 0, typeButton.height)
            contentItem: RowLayout {
                spacing: 2
                Text {
                    Layout.fillWidth: true
                    text: ChannelController.typeName(row.type)
                    font: Theme.fontBody
                    color: typeButton.enabled ? Theme.text : Theme.textDisabled
                    opacity: row.bypassed || !row.active ? Theme.dim(0.45) : 1
                    elide: Text.ElideRight
                }
                Icon { name: "chevrons-up-down"; size: 10; color: Theme.textMuted }
            }
            background: Rectangle {
                radius: Theme.radiusSmall
                color: typeButton.hovered ? Theme.controlHover : "transparent"
                border.width: typeButton.visualFocus ? 2 : 0
                border.color: Theme.focusRing
            }
            Accessible.role: Accessible.ButtonMenu
            Accessible.name: qsTr("%1 type").arg(row.what)
            Accessible.description: ChannelController.typeName(row.type)
        }

        // Linkwitz Transform: its four values live in a panel
        Item {
            visible: !row.crossover && row.active && row.linkwitz
            Layout.preferredWidth: 104
            Layout.preferredHeight: 24
            opacity: row.bypassed ? Theme.dim(0.45) : 1
            IconButton {
                id: linkwitzButton
                objectName: row.objectName + ".linkwitz"
                anchors.centerIn: parent
                iconName: "sliders-horizontal"
                iconColor: Theme.textMuted
                toolTip: qsTr("Edit Linkwitz Transform parameters (f0, Q0, fp, Qp)")
                onClicked: linkwitzPanel.open()
            }
            Linkwitz {
                id: linkwitzPanel
                objectName: row.objectName + ".linkwitzPanel"
                y: linkwitzButton.y + linkwitzButton.height + 4
                x: linkwitzButton.x + linkwitzButton.width / 2 - width / 2
                channel: row.channel
                band: row.band
            }
        }

        RowLayout {
            visible: !row.crossover && row.active && !row.linkwitz
            spacing: Theme.spacing
            opacity: row.bypassed ? Theme.dim(0.45) : 1
            ChannelValueField {
                objectName: row.objectName + ".freq"
                label: qsTr("%1 frequency").arg(row.what)
                unit: qsTr("Hz")
                fieldWidth: 80
                scrollStep: 10
                minValue: 10
                value: row.params.freq
                liveValue: row.liveFreq
                onCommitted: v => row.setValue("freq", v)
            }
            ChannelValueField {
                objectName: row.objectName + ".gain"
                visible: ChannelController.usesGain(row.type)
                label: qsTr("%1 gain").arg(row.what)
                unit: qsTr("dB")
                fieldWidth: 60
                maxDecimals: 3
                value: row.params.gain
                liveValue: row.liveGain
                onCommitted: v => row.setValue("gain", v)
            }
            Item { visible: !ChannelController.usesGain(row.type); Layout.preferredWidth: 84; Layout.preferredHeight: 1 }
            ChannelValueField {
                objectName: row.objectName + ".q"
                visible: ChannelController.usesQ(row.type)
                label: qsTr("%1 Q").arg(row.what)
                unit: qsTr("Q")
                fieldWidth: 50
                maxDecimals: 3
                stripTrailingZeros: true
                minValue: 0.1
                value: row.params.q
                liveValue: row.liveQ
                onCommitted: v => row.setValue("q", v)
            }
            Item { visible: !ChannelController.usesQ(row.type); Layout.preferredWidth: 74; Layout.preferredHeight: 1 }
        }

        // ---- Crossover ----
        AppComboBox {
            id: familyBox
            objectName: row.objectName + ".family"
            visible: row.crossover
            Layout.preferredWidth: 130
            label: qsTr("%1 family").arg(row.what)
            model: ChannelController.crossoverFamilies()
            currentIndex: ChannelController.familyOf(row.type)
            opacity: row.bypassed || !row.active ? Theme.dim(0.45) : 1
            onActivated: index => ChannelController.setCrossoverFamily(row.channel, row.band, index)
        }
        RowLayout {
            visible: row.crossover && row.active
            spacing: Theme.spacing
            opacity: row.bypassed ? Theme.dim(0.45) : 1
            AppComboBox {
                objectName: row.objectName + ".direction"
                Layout.preferredWidth: 100
                label: qsTr("%1 type").arg(row.what)
                model: [qsTr("Low pass"), qsTr("High pass")] // -- INT016 -- (A10 one filter vocabulary) as the inspector says
                currentIndex: ChannelController.isLowPass(row.type) ? 0 : 1
                onActivated: index => ChannelController.setCrossoverLowPass(row.channel, row.band, index === 0)
            }
            AppComboBox {
                id: slopeBox
                objectName: row.objectName + ".slope"
                Layout.preferredWidth: 100
                label: qsTr("%1 slope").arg(row.what)
                readonly property var orders: ChannelController.ordersOf(ChannelController.familyOf(row.type))
                model: orders.map(o => qsTr("%1 dB/oct").arg(o * 6))
                currentIndex: orders.indexOf(ChannelController.orderOf(row.type))
                onActivated: index => ChannelController.setCrossoverOrder(row.channel, row.band, orders[index])
            }
            ChannelValueField {
                objectName: row.objectName + ".freq"
                label: qsTr("%1 frequency").arg(row.what)
                unit: qsTr("Hz")
                fieldWidth: 80
                scrollStep: 10
                minValue: 10
                value: row.params.freq
                onCommitted: v => row.setValue("freq", v)
            }
        }

        Item { Layout.fillWidth: true; Layout.preferredHeight: 1 }
    }

    // The type picker: built from the controller each time it opens, so it
    // always offers what the firmware takes
    AppMenu {
        id: typeMenu
        objectName: row.objectName + ".typeMenu"
        property var made: []
        onAboutToShow: rebuild()
        function rebuild() {
            while (count > 0) takeItem(0)
            for (const o of made) o.destroy()
            made = []
            const groups = ChannelController.typeMenu(row.includeLinkwitz)
            for (const g of groups) {
                if (g.variants.length === 1) {
                    const item = typeItem.createObject(null, { text: g.name, type: g.variants[0].type })
                    made.push(item)
                    addItem(item)
                    continue
                }
                const sub = typeSubmenu.createObject(null, { title: g.name })
                made.push(sub)
                for (const v of g.variants) {
                    const item = typeItem.createObject(null, { text: v.label, type: v.type })
                    made.push(item)
                    sub.addItem(item)
                }
                addMenu(sub)
            }
        }
    }
    Component {
        id: typeItem
        AppMenuItem {
            property int type
            checkable: true
            checked: type === row.type
            onTriggered: row.chooseType(type)
        }
    }
    Component {
        id: typeSubmenu
        AppMenu {}
    }

    Accessible.role: Accessible.Row
    Accessible.name: what

    // -- WP AIVIS -- an agent changed it just now
    AiTouched { objectName: row.objectName + ".touched"; path: "eq."; indices: [row.channel, row.band + (row.crossover ? 20 : 0)] }
}
