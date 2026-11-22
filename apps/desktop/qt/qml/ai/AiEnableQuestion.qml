import QtQuick
import UConsole

// -- WP LAY8 -- What is asked before AI agents are turned on: MCP4's question,
// moved out of AiAgentsPage for the page and Modern's popover.
QuestionDialog {
    property QtObject ai: AiController
    title: qsTr("Allow AI agents?")
    text: [qsTr("An agent on this computer that you allow may read your devices' settings and change them: equaliser, crossovers, gains, routing, effects, presets and more."),
           qsTr("It cannot update firmware, change the limits or the installer's lock, set up speakers, pair hubs or delete presets."),
           qsTr("You can undo each change, pause the agents or stop them at any time."),
           LimitsController.connected && !LimitsController.active
               ? qsTr("This device has no limits profile. You can set one up first; it holds agents too.") : ""]
          .filter(s => s.length).join("\n\n")
    acceptText: qsTr("Allow")
    alternativeText: LimitsController.connected && !LimitsController.active ? qsTr("Set Up Limits…") : ""
    onAnswered: answer => {
        if (answer === "accept") ai.setEnabled(true)
        else if (answer === "alternative") ai.openLimits()
    }
}
