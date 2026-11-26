import QtQuick
import UConsole

// -- WP LAY8 -- What is asked before AI agents are turned on: MCP4's question,
// moved out of AiAgentsPage for the page and Modern's popover.
QuestionDialog {
    property QtObject ai: AiController
    title: qsTr("Allow AI agents?")
    text: [qsTr("An agent on this computer that you allow may read your devices' settings and change them: equaliser, crossovers, gains, routing, effects, presets and more."),
           // -- WP UAG -- (UXP A6, F14) agents may detect speakers and take their roles from your words, with your consent;
           // and what is saved on the device has no Undo
           qsTr("It cannot update firmware, change the limits or the installer's lock, pair hubs or delete presets."),
           qsTr("It detects your speakers or sets their roles from your words only with your consent: it asks you first, or you allow it in Settings › AI Agents."),
           qsTr("You can undo its changes (not what it saves on the device), pause the agents or stop them at any time."),
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
