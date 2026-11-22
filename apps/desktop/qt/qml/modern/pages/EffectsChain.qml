import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5a -- The inspector beside the effects (MODLAY 2): the signal chain,
// every stage in the order the DSPi runs it, the effects with their switches.
// A stage opens its page. The order is the firmware's (audio_pipeline.c):
// crossfeed and loudness work on the outputs where they have an output mask,
// and before the matrix on firmware without one.
Inspector {
    id: inspector
    objectName: "EffectsChain"
    property Item host: null
    property string tab: ""

    Binding {
        target: InspectorState
        property: "windowWidth"
        value: inspector.Window.width
        when: inspector.shown && inspector.Window.width > 0
        restoreMode: Binding.RestoreNone
    }

    // Plain properties, so the stages are made anew only when one changes: a
    // reading of the device must not take the list's items away
    readonly property bool crossfeedOut: EffectsController.crossfeedMaskSupported
    readonly property bool loudnessOut: EffectsController.loudnessMaskSupported
    readonly property bool crossover: ChannelController.crossoverSupported
    readonly property bool limiter: ChannelController.limiterSupported
    // A stage: { id, title, section, page, effect }
    readonly property var stages: {
        const effect = id => {
            const e = EffectsState.available.find(x => x.tab === id)
            return e ? [{ id: id, title: e.title, section: "effects", page: id, effect: true }] : []
        }
        const stage = (id, title, section, page) => [{ id: id, title: title, section: section, page: page, effect: false }]
        return [].concat(
            stage("input", qsTr("Input preamp and EQ"), "equaliser", "peq"),
            effect("leveller"),
            effect("upmixer"),
            crossfeedOut ? [] : effect("crossfeed"),
            loudnessOut ? [] : effect("loudness"),
            stage("matrix", qsTr("Matrix mixer"), "routing", "matrix"),
            crossfeedOut ? effect("crossfeed") : [],
            effect("subharmonic"),
            effect("psybass"),
            effect("tube"),
            crossover ? stage("crossover", qsTr("Crossover"), "equaliser", "crossover") : [],
            stage("output", qsTr("Output EQ"), "equaliser", "peq"),
            stage("gain", qsTr("Gain and volume"), "equaliser", "delay"),
            loudnessOut ? effect("loudness") : [],
            limiter ? stage("limiter", qsTr("Limiter"), "equaliser", "delay") : [],
            stage("delay", qsTr("Delay"), "equaliser", "delay"))
    }

    function open_(stage) { if (host) host.go(stage.section, stage.page) }

    content: Component {
        ColumnLayout {
            spacing: 2

            Text {
                Layout.fillWidth: true
                Layout.rightMargin: 28   // the close button
                text: qsTr("Signal Chain")
                font: Theme.fontTitle
                color: Theme.text
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            Text {
                objectName: "EffectsChain.summary"
                Layout.fillWidth: true
                Layout.topMargin: 4
                Layout.bottomMargin: 6
                text: EffectsState.count === 0 ? qsTr("The order in the DSPi. No effect is on.")
                      : EffectsState.count === 1 ? qsTr("The order in the DSPi. 1 effect is on.")
                      : qsTr("The order in the DSPi. %1 effects are on.").arg(EffectsState.count)
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            Repeater {
                model: inspector.stages
                RowLayout {
                    id: line
                    required property var modelData
                    readonly property bool on: modelData.effect && EffectsState.on(modelData.id) === true
                    readonly property bool current: modelData.effect && modelData.id === inspector.tab
                    Layout.fillWidth: true
                    spacing: 6

                    AbstractButton {
                        id: name
                        objectName: "EffectsChain." + line.modelData.id
                        Layout.fillWidth: true
                        implicitHeight: 28
                        hoverEnabled: true
                        focusPolicy: Qt.TabFocus
                        text: line.modelData.title
                        contentItem: RowLayout {
                            spacing: Theme.spacingSmall
                            Rectangle {
                                Layout.leftMargin: 8
                                implicitWidth: 7
                                implicitHeight: 7
                                radius: 3.5
                                color: line.modelData.effect ? (line.on ? Theme.success : "transparent") : Theme.textMuted
                                border.width: line.modelData.effect && !line.on ? 1 : 0
                                border.color: Theme.textMuted
                            }
                            Text {
                                Layout.fillWidth: true
                                textFormat: Text.PlainText
                                text: name.text
                                font: Theme.fontBody
                                color: line.modelData.effect && !line.on && !line.current ? Theme.textMuted : Theme.text
                                elide: Text.ElideRight
                            }
                            Text {
                                objectName: name.objectName + ".bypassed"
                                // Bypass EQ takes the inputs' alone
                                visible: line.modelData.id === "input" && SidebarController.bypassOn
                                Layout.rightMargin: 8
                                text: qsTr("EQ bypassed")
                                font: Theme.fontSmall
                                color: Theme.warning
                            }
                        }
                        background: Rectangle {
                            radius: Theme.radiusSmall
                            color: line.current ? Theme.selection : name.hovered ? Theme.controlHover : "transparent"
                            border.width: name.visualFocus ? 2 : 0
                            border.color: Theme.focusRing
                        }
                        onClicked: inspector.open_(line.modelData)
                        Accessible.role: Accessible.Button
                        Accessible.name: text
                        Accessible.onPressAction: name.clicked()
                    }
                    AppSwitch {
                        objectName: "EffectsChain." + line.modelData.id + ".on"
                        visible: line.modelData.effect
                        text: ""
                        checked: line.on
                        enabled: EffectsController.writable
                        // The write refused or failed: the device's state again
                        onToggled: {
                            EffectsState.setOn(line.modelData.id, checked)
                            checked = Qt.binding(() => line.on)
                        }
                        Accessible.name: line.modelData.title
                    }
                }
            }
        }
    }
}
