import QtQuick
import QtQuick.Controls.Basic
import UConsole

// Where the File and AutoEQ menus' questions and alerts live, and the
// unsaved-changes question of the device switch and quit: FilesController
// makes one in the main window once the window exists.
Item {
    id: host
    objectName: "FilesHost"
    anchors.fill: parent

    NoticeDialog {
        id: notice
        objectName: "FilesHost.notice"
    }
    ImportFilters {
        id: filters
    }
    ImportConfig {
        id: config
    }
    ImportProgress {
        visible: FilesController.importing
    }
    Connections {
        target: FilesController
        function onFiltersPicker(prompt, channels) { filters.ask(prompt, channels) }
        function onConfigurationOptions(prompt) { config.ask(prompt) }
        function onNotice(title, text, error) { notice.show(title, text, error) }
    }

    // ---- The unsaved-changes question (UnsavedGuard) ----
    QuestionDialog {
        id: unsaved
        objectName: "FilesHost.unsavedQuestion"
        title: qsTr("Unsaved Changes")
        acceptText: qsTr("Save")
        alternativeText: qsTr("Discard")
        rejectText: qsTr("Cancel")
        onAnswered: answer => UnsavedGuard.answer(answer)
    }
    QuestionDialog {
        id: quitAnyway
        objectName: "FilesHost.saveFailedQuestion"
        acceptText: qsTr("Quit")
        rejectText: qsTr("Cancel")
        onAnswered: answer => UnsavedGuard.answerSaveFailed(answer)
    }
    Connections {
        target: UnsavedGuard
        function onQuestion(op, summary) {
            unsaved.text = qsTr("The current preset has unsaved changes:\n\n%1\n\nSave before continuing?").arg(summary)
            unsaved.open()
        }
        function onSaveFailed(op, title, text) {
            if (op === "quit") {
                quitAnyway.title = title
                quitAnyway.text = text
                quitAnyway.open()
            } else {
                notice.show(title, text, true)
            }
        }
    }

    // ---- AutoEQ ----
    AutoEqUpdateDialog {
        id: update
        onRebuildChosen: rebuildQuestion.open()
    }
    QuestionDialog {
        id: rebuildQuestion
        objectName: "FilesHost.rebuildQuestion"
        title: qsTr("Rebuild AutoEQ Database")
        text: qsTr("You are about to rebuild the AutoEQ database by downloading all profiles from GitHub.\n\nThis requires an internet connection and may take several minutes.\n\nDo you wish to proceed?")
        acceptText: qsTr("Rebuild")
        rejectText: qsTr("Cancel")
        onAnswered: answer => {
            if (answer === "accept") AutoEqController.startRebuild()
        }
    }
    AutoEqRebuild {
        transientParent: host.Window.window
    }
    Connections {
        target: AutoEqController
        function onUpdateRequested() { update.ask() }
        function onNotice(title, text, error) { notice.show(title, text, error) }
    }
}
