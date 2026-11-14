import QtQuick
import UConsole

// -- WP P1QT1 -- An offline document is open (File > Open File…, P1OF): every
// screen edits the file, nothing reaches a DSPi until it is imported there.
Banner {
    objectName: "FileBanner"
    kind: "info"
    iconName: "folder-open"
    visible: FilesController.offline
    //: %1 is the open file's name
    text: qsTr("File: %1. You are editing a file, not a device; nothing here reaches a DSPi.").arg(FilesController.offlineFile)

    AppButton {
        objectName: "FileBanner.save"
        text: qsTr("Save File")
        onClicked: Shell.trigger("file.saveFile")
    }
    AppButton {
        objectName: "FileBanner.saveTemplate"
        text: qsTr("Save as Template…")
        onClicked: Shell.trigger("file.saveAsTemplate")
    }
}
