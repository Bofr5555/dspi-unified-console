import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP16 -- Firmware Update (the Mac's FirmwareUpdateView, 460×470): the
// versions in play and the image, the four steps, one card for the state the
// installer is in, the BOOTSEL hints, and the buttons. FirmwareController
// does the work; nothing is written before Update Firmware.
ToolWindow {
    id: window
    objectName: "FirmwareWindow"
    windowId: "firmware"
    defaultWidth: 460
    defaultHeight: 470
    minimumWidth: 460
    minimumHeight: 470
    title: qsTr("Firmware Update")

    readonly property var fw: FirmwareController

    onVisibleChanged: FirmwareController.windowShown(visible)

    // One "LABEL   value" line with a fixed label column (the Mac's LabeledValueRow)
    component ValueRow: RowLayout {
        id: valueRow
        property string label
        property string value
        property bool secondary: false
        spacing: Theme.spacingSmall
        Text {
            Layout.preferredWidth: 120
            text: valueRow.label
            font: Theme.fontHeading
            color: Theme.textMuted
        }
        Text {
            Layout.fillWidth: true
            text: valueRow.value
            font: Theme.fontMono
            color: valueRow.secondary ? Theme.textMuted : Theme.text
            elide: Text.ElideMiddle
        }
        Accessible.role: Accessible.StaticText
        Accessible.name: label + " " + value
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // ---- Header ----
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.spacing
            Layout.leftMargin: Theme.spacingLarge
            spacing: Theme.spacing
            Icon {
                name: "cpu"
                size: 22
                color: Theme.accentText
            }
            ColumnLayout {
                spacing: 2
                Text {
                    text: qsTr("Firmware Update")
                    font: Theme.fontTitle
                    color: Theme.text
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
                Text {
                    objectName: "FirmwareWindow.subtitle"
                    text: qsTr("Install firmware %1 onto a DSPi board").arg(window.fw.imageVersion)
                    font: Theme.fontCaption
                    color: Theme.textMuted
                }
            }
            Item { Layout.fillWidth: true }
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: Theme.spacingLarge
            spacing: Theme.spacing

            // ---- The versions and the image ----
            Card {
                objectName: "FirmwareWindow.summary"
                Layout.fillWidth: true
                padding: 10
                ValueRow {
                    objectName: "FirmwareWindow.expected"
                    Layout.fillWidth: true
                    label: qsTr("This Console")
                    value: window.fw.expectedVersion.length ? window.fw.expectedVersion : qsTr("unknown")
                }
                ValueRow {
                    objectName: "FirmwareWindow.device"
                    Layout.fillWidth: true
                    label: qsTr("Connected device")
                    value: window.fw.deviceVersion.length ? window.fw.deviceVersion : qsTr("None")
                    secondary: window.fw.deviceVersion.length === 0
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                    ValueRow {
                        objectName: "FirmwareWindow.image"
                        Layout.fillWidth: true
                        label: window.fw.custom ? qsTr("Custom image") : qsTr("Bundled image")
                        value: window.fw.imageName
                    }
                    Icon {
                        objectName: "FirmwareWindow.imageChecked"
                        visible: window.fw.imageChecked
                        name: "circle-check"
                        size: 14
                        color: Theme.success
                        accessibleName: qsTr("Checked")
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                    AppButton {
                        objectName: "FirmwareWindow.chooseImage"
                        text: qsTr("Choose .uf2...")
                        enabled: !window.fw.confirmed
                        onClicked: FirmwareController.chooseCustomImage()
                    }
                    AppButton {
                        objectName: "FirmwareWindow.useBundled"
                        visible: window.fw.custom
                        text: qsTr("Use this Console's firmware instead")
                        enabled: !window.fw.confirmed
                        onClicked: FirmwareController.useBundledImage()
                    }
                    Item { Layout.fillWidth: true }
                }
                Text {
                    objectName: "FirmwareWindow.imageProblem"
                    Layout.fillWidth: true
                    visible: text.length > 0
                    text: window.fw.imageProblem
                    font: Theme.fontSmall
                    color: Theme.warning
                    wrapMode: Text.Wrap
                }
                RowLayout {
                    objectName: "FirmwareWindow.downgradeNote"
                    Layout.fillWidth: true
                    visible: window.fw.downgrade && !window.fw.custom
                    spacing: 6
                    Icon {
                        name: "arrow-down-to-line"
                        size: 12
                        color: Theme.warning
                    }
                    Text {
                        Layout.fillWidth: true
                        text: qsTr("The device is newer than this Console, so this would be a downgrade.")
                        font: Theme.fontSmall
                        color: Theme.warning
                        wrapMode: Text.Wrap
                    }
                }
            }

            // ---- The four steps (the Mac's StepDotStrip) ----
            RowLayout {
                id: steps
                objectName: "FirmwareWindow.steps"
                Layout.fillWidth: true
                Layout.leftMargin: Theme.spacingSmall
                Layout.rightMargin: Theme.spacingSmall
                spacing: 0
                opacity: window.fw.failed ? 0.4 : 1
                readonly property var labels: [qsTr("Prepare"), qsTr("Write"), qsTr("Verify"), qsTr("Done")]
                Repeater {
                    model: steps.labels
                    RowLayout {
                        id: stepItem
                        required property int index
                        required property string modelData
                        readonly property int current: window.fw.step
                        readonly property bool last: index === steps.labels.length - 1
                        Layout.fillWidth: index > 0
                        spacing: 0
                        Rectangle {
                            visible: stepItem.index > 0
                            Layout.fillWidth: true
                            Layout.leftMargin: 4
                            Layout.rightMargin: 4
                            implicitHeight: 2
                            color: stepItem.index <= stepItem.current ? Theme.accent : Theme.separator
                        }
                        ColumnLayout {
                            Layout.preferredWidth: 52
                            spacing: 3
                            Rectangle {
                                Layout.alignment: Qt.AlignHCenter
                                implicitWidth: 14
                                implicitHeight: 14
                                radius: 7
                                color: stepItem.index < stepItem.current ? Theme.accent
                                     : stepItem.index === stepItem.current ? (stepItem.last ? Theme.success : Theme.accent)
                                     : Theme.separator
                                Icon {
                                    anchors.centerIn: parent
                                    visible: stepItem.index < stepItem.current || (stepItem.last && stepItem.current === stepItem.index)
                                    name: "check"
                                    size: 9
                                    color: Theme.textOnAccent
                                }
                            }
                            Text {
                                Layout.alignment: Qt.AlignHCenter
                                text: stepItem.modelData
                                font.pixelSize: Theme.fontCaption.pixelSize
                                font.bold: stepItem.index === stepItem.current
                                color: stepItem.index === stepItem.current ? Theme.text : Theme.textMuted
                            }
                        }
                        Accessible.role: Accessible.StaticText
                        Accessible.name: modelData
                    }
                }
            }

            // ---- The state the installer is in ----
            Card {
                objectName: "FirmwareWindow.status"
                Layout.fillWidth: true
                Layout.fillHeight: true
                padding: 14
                Item { Layout.fillHeight: true }
                Item {
                    Layout.alignment: Qt.AlignHCenter
                    implicitWidth: 28
                    implicitHeight: 28
                    Icon {
                        id: statusIcon
                        objectName: "FirmwareWindow.statusIcon"
                        anchors.centerIn: parent
                        size: window.fw.state === "verified" ? 32 : 28
                        name: window.fw.statusBusy ? "loader-circle" : window.fw.statusIcon
                        color: window.fw.statusTint === "success" ? Theme.success
                             : window.fw.statusTint === "warning" ? Theme.warning
                             : window.fw.statusTint === "muted" ? Theme.textMuted
                             : Theme.accentText
                        RotationAnimator on rotation {
                            running: window.fw.statusBusy && window.visible
                            from: 0
                            to: 360
                            duration: 1100
                            loops: Animation.Infinite
                        }
                        onNameChanged: if (!window.fw.statusBusy) rotation = 0
                    }
                }
                Text {
                    objectName: "FirmwareWindow.statusTitle"
                    Layout.alignment: Qt.AlignHCenter
                    text: window.fw.statusTitle
                    font.pixelSize: Theme.fontBody.pixelSize
                    font.bold: true
                    color: Theme.text
                }
                Text {
                    objectName: "FirmwareWindow.statusMessage"
                    Layout.fillWidth: true
                    Layout.maximumWidth: 340
                    Layout.alignment: Qt.AlignHCenter
                    horizontalAlignment: Text.AlignHCenter
                    text: window.fw.statusMessage
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
                ColumnLayout {
                    objectName: "FirmwareWindow.writing"
                    visible: window.fw.state === "writing"
                    Layout.fillWidth: true
                    spacing: 4
                    ProgressBar {
                        Layout.fillWidth: true
                        indeterminate: true
                        Accessible.name: qsTr("Writing firmware")
                    }
                    ValueRow {
                        Layout.fillWidth: true
                        visible: window.fw.boardName.length > 0
                        label: qsTr("Board")
                        value: window.fw.boardName
                    }
                    ValueRow {
                        Layout.fillWidth: true
                        label: qsTr("Firmware")
                        value: window.fw.imageVersion
                    }
                }
                Item { Layout.fillHeight: true }
                Accessible.role: Accessible.StatusBar
                Accessible.name: window.fw.statusTitle
                Accessible.description: window.fw.statusMessage
            }

            // ---- BOOTSEL by hand ----
            RowLayout {
                objectName: "FirmwareWindow.bootselHint"
                Layout.fillWidth: true
                visible: window.fw.showBootselHint
                spacing: Theme.spacingSmall
                Icon {
                    Layout.alignment: Qt.AlignTop
                    name: "circle-dot"
                    size: 12
                    color: Theme.textMuted
                }
                Text {
                    Layout.fillWidth: true
                    text: qsTr("Hold the BOOTSEL button on your Pico-compatible device while plugging it in.")
                    font: Theme.fontCaption
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
            RowLayout {
                objectName: "FirmwareWindow.manualHint"
                Layout.fillWidth: true
                visible: !window.fw.showBootselHint && window.fw.showManualHint
                spacing: 4
                Text {
                    text: qsTr("Flashing a UF2 of your own?")
                    font: Theme.fontCaption
                    color: Theme.textMuted
                }
                Text {
                    objectName: "FirmwareWindow.enterBootloader"
                    text: qsTr("Enter bootloader mode without installing")
                    font: Theme.fontCaption
                    color: Theme.accentText
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: FirmwareController.enterBootloaderOnly()
                    }
                    Accessible.role: Accessible.Link
                    Accessible.name: text
                    Accessible.onPressAction: FirmwareController.enterBootloaderOnly()
                }
                Item { Layout.fillWidth: true }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }

        // ---- Buttons ----
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
            Layout.topMargin: Theme.spacing
            Layout.bottomMargin: Theme.spacing
            spacing: Theme.spacingSmall
            readonly property bool done: window.fw.state === "verified"
            AppButton {
                objectName: "FirmwareWindow.another"
                visible: parent.done
                text: qsTr("Update Another Board")
                onClicked: FirmwareController.resetRun()
            }
            AppButton {
                objectName: "FirmwareWindow.cancel"
                visible: !parent.done
                text: qsTr("Cancel")
                onClicked: window.close()
            }
            AppButton {
                objectName: "FirmwareWindow.export"
                visible: !parent.done && window.fw.showExport
                text: qsTr("Export Configuration...")
                onClicked: FirmwareController.exportConfiguration()
            }
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "FirmwareWindow.done"
                visible: parent.done
                primary: true
                text: qsTr("Done")
                onClicked: window.close()
            }
            AppButton {
                objectName: "FirmwareWindow.primary"
                visible: !parent.done && window.fw.primaryText.length > 0
                primary: true
                text: window.fw.primaryText
                enabled: window.fw.failed || (!window.fw.confirmed && !window.fw.guardAsking
                                              && (!window.fw.custom || window.fw.imageProblem.length === 0))
                onClicked: window.fw.failed ? FirmwareController.resetRun() : FirmwareController.start()
            }
        }
    }

    Shortcut {
        sequences: ["Return", "Enter"]
        enabled: !guard.visible
        onActivated: {
            if (window.fw.state === "verified") window.close()
            else if (window.fw.failed) FirmwareController.resetRun()
            else if (window.fw.primaryText.length && !window.fw.confirmed) FirmwareController.start()
        }
    }

    // ---- The firmware guard: a typed confirmation (the core's text and words) ----
    Dialog {
        id: guard
        objectName: "FirmwareWindow.guard"
        parent: Overlay.overlay
        modal: true
        closePolicy: Popup.CloseOnEscape
        width: Math.min(400, window.width - 2 * Theme.spacingLarge)
        x: (window.width - width) / 2
        y: Theme.spacingLarge
        padding: Theme.spacingLarge
        title: qsTr("Before Installing")
        onOpened: {
            typed.text = ""
            typed.forceActiveFocus()
        }
        onRejected: FirmwareController.cancelGuard()
        Connections {
            target: FirmwareController
            function onGuardChanged() {
                if (FirmwareController.guardAsking) guard.open()
                else guard.close()
            }
        }
        Overlay.modal: Rectangle { color: Theme.scrim }
        background: Rectangle {
            radius: Theme.radius
            color: Theme.panel
            border.color: Theme.border
        }
        header: Text {
            leftPadding: Theme.spacingLarge
            topPadding: Theme.spacingLarge
            text: guard.title
            font: Theme.fontTitle
            color: Theme.text
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        contentItem: ColumnLayout {
            spacing: Theme.spacingSmall
            Accessible.role: Accessible.AlertMessage
            Accessible.name: guard.title
            Accessible.description: FirmwareController.guardMessage
            Text {
                objectName: "FirmwareWindow.guardMessage"
                Layout.fillWidth: true
                text: FirmwareController.guardMessage
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.Wrap
            }
            Text {
                Layout.fillWidth: true
                text: qsTr("To go ahead, type: %1").arg(FirmwareController.guardPhrase)
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            BorderedField {
                id: typed
                objectName: "FirmwareWindow.guardTyped"
                Layout.fillWidth: true
                label: qsTr("Confirmation")
                placeholderText: FirmwareController.guardPhrase
                onAccepted: if (text === FirmwareController.guardPhrase) FirmwareController.answerGuard(text)
            }
        }
        footer: RowLayout {
            spacing: Theme.spacingSmall
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "FirmwareWindow.guardCancel"
                Layout.bottomMargin: Theme.spacing
                text: qsTr("Don't Install")
                onClicked: FirmwareController.cancelGuard()
            }
            AppButton {
                objectName: "FirmwareWindow.guardGo"
                Layout.bottomMargin: Theme.spacing
                Layout.rightMargin: Theme.spacingLarge
                text: qsTr("Go Ahead")
                destructive: true
                // Only the exact words: case and spaces count
                enabled: typed.text === FirmwareController.guardPhrase
                onClicked: FirmwareController.answerGuard(typed.text)
            }
        }
    }
}
