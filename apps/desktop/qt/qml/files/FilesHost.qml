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
    // -- WP P1QT1 --
    TemplatePicker {
        id: templates
    }
    ImportProgress {
        visible: FilesController.importing
    }
    Connections {
        target: FilesController
        function onFiltersPicker(prompt, channels) { filters.ask(prompt, channels) }
        function onConfigurationOptions(prompt, preview, guardDefault) { config.ask(prompt, preview, guardDefault) }   // -- WP P1QT1 --
        function onTemplatesPicker(list) { templates.ask(list) }   // -- WP P1QT1 --
        function onNotice(title, text, error) { notice.show(title, text, error) }
        // -- WP QPI2 --
        function onDeviceQuestion(op, title, text, acceptText, destructive) {
            device.op = op
            device.title = title
            device.text = text
            device.acceptText = acceptText
            device.destructive = destructive
            device.open()
        }
    }

    // -- WP QPI2 -- Commit Parameters, Revert to Saved, Factory Reset
    QuestionDialog {
        id: device
        objectName: "FilesHost.deviceQuestion"
        property string op
        rejectText: qsTr("Cancel")
        onAnswered: answer => FilesController.answerDevice(op, answer)
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
    // -- WP QP16 -- Settings' unsaved changes before a device switch (F8)
    QuestionDialog {
        id: settingsQuestion
        objectName: "FilesHost.settingsQuestion"
        acceptText: qsTr("Discard and Switch")
        rejectText: qsTr("Cancel")
        destructive: true
        onAnswered: answer => UnsavedGuard.answerSettings(answer)
    }
    Connections {
        target: UnsavedGuard
        // -- WP QP16 --
        function onSettingsQuestion(title, text) {
            settingsQuestion.title = title
            settingsQuestion.text = text
            settingsQuestion.open()
        }
        function onQuestion(op, summary) {
            // -- WP MDQT2b -- with several devices open, the one asked about
            unsaved.text = UnsavedGuard.deviceTitle !== ""
                ? qsTr("The current preset of %1 has unsaved changes:\n\n%2\n\nSave before continuing?").arg(UnsavedGuard.deviceTitle).arg(summary)
                : qsTr("The current preset has unsaved changes:\n\n%1\n\nSave before continuing?").arg(summary)
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
            if (answer === "accept") AutoEqController.rebuildFromGitHub()
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
