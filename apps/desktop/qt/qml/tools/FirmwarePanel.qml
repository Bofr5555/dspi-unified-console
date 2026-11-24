import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP16 -- Firmware Update (the Mac's FirmwareUpdateView, 460×470): the
// versions in play and the image, the four steps, one card for the state the
// installer is in, the BOOTSEL hints, and the buttons. FirmwareController
// does the work; nothing is written before Update Firmware.
// -- WP LAY1b -- A panel (Panel.qml): PanelTable's row for device.firmwareUpdate gives
// its window's title and sizes.
Panel {
    id: panel
    objectName: "FirmwarePanel"
    // -- WP PGQT -- room for the image note or the generation change when one shows
    minimumHeight: 470 + (fw.generationChange.length > 0 || fw.imageNote.length > 0 ? 64 : 0)

    readonly property var fw: FirmwareController

    // -- WP DL3b -- On Modern's page (DESIGN-LANGUAGE §3, §15, §16): no rules,
    // the state the installer is in without a box and right under the steps,
    // then the one primary action (no Cancel or Done: the page is no dialog);
    // Export Configuration… in the row over the page. The window keeps its
    // dialog layout.
    readonly property bool paged: inPage
    readonly property Component pageTools: Component {
        AppButton {
            objectName: "FirmwareTools.export"
            visible: panel.fw.showExport && panel.fw.stage !== "verified"
            enabled: !DeviceInfoController.sender
            text: qsTr("Export Configuration…")
            onClicked: FirmwareController.exportConfiguration()
        }
    }

    onShownChanged: {
        FirmwareController.windowShown(shown)
        // A question asked before the window showed (the wizard's run)
        if (shown && FirmwareController.guardAsking) guard.open()
    }
    Component.onDestruction: if (shown) FirmwareController.windowShown(false)   // -- WP LAY1b -- a page host unloaded

    // One "LABEL   value" line with a fixed label column (the Mac's LabeledValueRow)
    component ValueRow: RowLayout {
        id: valueRow
        property string label
        property string value
        property bool muted: false
        spacing: Theme.spacingSmall
        Text {
            Layout.preferredWidth: 120
            text: valueRow.label
            font: Theme.fontHeading
            color: Theme.textMuted
        }
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: valueRow.value
            font: Theme.fontMono
            color: valueRow.muted ? Theme.textMuted : Theme.text
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
                    text: qsTr("Install firmware %1 onto a DSPi board").arg(panel.fw.imageVersion)
                    font: Theme.fontCaption
                    color: Theme.textMuted
                }
            }
            Item { Layout.fillWidth: true }
        }
        Rectangle {
            Layout.fillWidth: true
            visible: !panel.paged   // -- WP DL3b --
            implicitHeight: 1
            color: Theme.separator
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: !panel.paged   // -- WP DL3b --
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
                    value: panel.fw.expectedVersion.length ? panel.fw.expectedVersion : qsTr("unknown")
                }
                ValueRow {
                    objectName: "FirmwareWindow.device"
                    Layout.fillWidth: true
                    label: qsTr("Connected device")
                    value: panel.fw.runningVersion.length ? panel.fw.runningVersion : qsTr("None")
                    muted: panel.fw.runningVersion.length === 0
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                    ValueRow {
                        objectName: "FirmwareWindow.image"
                        Layout.fillWidth: true
                        label: panel.fw.custom ? qsTr("Custom image") : qsTr("Bundled image")
                        value: panel.fw.imageName
                    }
                    Icon {
                        objectName: "FirmwareWindow.imageChecked"
                        visible: panel.fw.imageChecked
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
                        text: qsTr("Choose .uf2…")
                        enabled: !panel.fw.confirmed
                        onClicked: FirmwareController.chooseCustomImage()
                    }
                    AppButton {
                        objectName: "FirmwareWindow.useBundled"
                        visible: panel.fw.custom
                        text: qsTr("Use this Console's firmware instead")
                        enabled: !panel.fw.confirmed
                        onClicked: FirmwareController.useBundledImage()
                    }
                    Item { Layout.fillWidth: true }
                }
                Text {
                    objectName: "FirmwareWindow.imageProblem"
                    Layout.fillWidth: true
                    visible: text.length > 0
                    text: panel.fw.imageProblem
                    font: Theme.fontSmall
                    color: Theme.warning
                    wrapMode: Text.Wrap
                }
                // -- WP SECFIX -- a file that is not this Console's own image (the
                // core's manifest): labelled, in words, and still allowed
                ColumnLayout {
                    objectName: "FirmwareWindow.unverified"
                    Layout.fillWidth: true
                    visible: panel.fw.unverifiedLabel.length > 0
                    spacing: 2
                    Text {
                        objectName: "FirmwareWindow.unverifiedLabel"
                        Layout.fillWidth: true
                        text: panel.fw.unverifiedLabel
                        font: Qt.font({ pixelSize: 13, weight: Font.DemiBold })
                        color: Theme.text
                        wrapMode: Text.Wrap
                    }
                    Text {
                        Layout.fillWidth: true
                        visible: text.length > 0
                        text: panel.fw.unverifiedText
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                }
                RowLayout {
                    objectName: "FirmwareWindow.downgradeNote"
                    Layout.fillWidth: true
                    visible: panel.fw.downgrade && !panel.fw.custom
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
                // -- WP PGQT -- no image for this board (no BOOTSEL), or the
                // sender's own image to choose; and the protocol generation the
                // image brings, as the core's update check says it
                Text {
                    objectName: "FirmwareWindow.imageNote"
                    Layout.fillWidth: true
                    visible: text.length > 0
                    text: panel.fw.imageNote
                    font: Theme.fontSmall
                    color: Theme.warning
                    wrapMode: Text.Wrap
                }
                Text {
                    objectName: "FirmwareWindow.generationChange"
                    Layout.fillWidth: true
                    visible: text.length > 0
                    text: panel.fw.generationChange
                    font: Theme.fontSmall
                    color: Theme.text
                    wrapMode: Text.Wrap
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
                opacity: panel.fw.failed ? 0.4 : 1
                readonly property var labels: [qsTr("Prepare"), qsTr("Write"), qsTr("Verify"), qsTr("Done")]
                Repeater {
                    model: steps.labels
                    RowLayout {
                        id: stepItem
                        required property int index
                        required property string modelData
                        readonly property int current: panel.fw.step
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
                                textFormat: Text.PlainText
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
                Layout.fillHeight: !panel.paged   // -- WP DL3b --
                Layout.topMargin: panel.paged ? Theme.spacingLarge : 0
                padding: 14
                color: panel.paged ? "transparent" : Theme.panel   // -- WP DL3b -- unboxed on the page
                Item { Layout.fillHeight: true }
                Item {
                    Layout.alignment: Qt.AlignHCenter
                    implicitWidth: 28
                    implicitHeight: 28
                    Icon {
                        id: statusIcon
                        objectName: "FirmwareWindow.statusIcon"
                        anchors.centerIn: parent
                        size: panel.fw.stage === "verified" ? 32 : 28
                        name: panel.fw.statusBusy ? "loader-circle" : panel.fw.statusIcon
                        color: panel.fw.statusTint === "success" ? Theme.success
                             : panel.fw.statusTint === "warning" ? Theme.warning
                             : panel.fw.statusTint === "muted" ? Theme.textMuted
                             : Theme.accentText
                        RotationAnimator on rotation {
                            running: panel.fw.statusBusy && panel.shown
                            from: 0
                            to: 360
                            duration: 1100
                            loops: Animation.Infinite
                        }
                        onNameChanged: if (!panel.fw.statusBusy) rotation = 0
                    }
                }
                Text {
                    objectName: "FirmwareWindow.statusTitle"
                    Layout.alignment: Qt.AlignHCenter
                    textFormat: Text.PlainText
                    text: panel.fw.statusTitle
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
                    textFormat: Text.PlainText
                    text: panel.fw.statusMessage
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
                ColumnLayout {
                    objectName: "FirmwareWindow.writing"
                    visible: panel.fw.stage === "writing"
                    Layout.fillWidth: true
                    spacing: 4
                    ProgressBar {
                        Layout.fillWidth: true
                        indeterminate: true
                        Accessible.name: qsTr("Writing firmware")
                    }
                    ValueRow {
                        Layout.fillWidth: true
                        visible: panel.fw.boardName.length > 0
                        label: qsTr("Board")
                        value: panel.fw.boardName
                    }
                    ValueRow {
                        Layout.fillWidth: true
                        label: qsTr("Firmware")
                        value: panel.fw.imageVersion
                    }
                }
                Item { Layout.fillHeight: true }
                Accessible.role: Accessible.StatusBar
                Accessible.name: panel.fw.statusTitle
                Accessible.description: panel.fw.statusMessage
            }

            // ---- BOOTSEL by hand ----
            RowLayout {
                objectName: "FirmwareWindow.bootselHint"
                Layout.fillWidth: true
                visible: panel.fw.showBootselHint
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
                visible: !panel.fw.showBootselHint && panel.fw.showManualHint && !panel.fw.noUpdate   // -- WP PGQT --
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
            visible: !panel.paged   // -- WP DL3b --
            implicitHeight: 1
            color: Theme.separator
        }

        // ---- Buttons ----
        RowLayout {
            // -- WP DL3b -- on the page: under the state, centred as it is
            Layout.fillWidth: !panel.paged
            Layout.alignment: panel.paged ? Qt.AlignHCenter : Qt.AlignLeft
            Layout.leftMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
            Layout.topMargin: panel.paged ? 0 : Theme.spacing
            Layout.bottomMargin: Theme.spacing
            spacing: Theme.spacingSmall
            readonly property bool done: panel.fw.stage === "verified"
            AppButton {
                objectName: "FirmwareWindow.another"
                visible: parent.done
                text: qsTr("Update Another Board")
                onClicked: FirmwareController.resetRun()
            }
            AppButton {
                objectName: "FirmwareWindow.cancel"
                visible: !parent.done && !panel.paged   // -- WP DL3b --
                text: qsTr("Cancel")
                onClicked: panel.close()
            }
            AppButton {
                objectName: "FirmwareWindow.export"
                visible: !parent.done && panel.fw.showExport && !panel.paged   // -- WP DL3b -- the page's tools
                enabled: !DeviceInfoController.sender   // -- WP QTSENDER -- the sender has no configuration to export
                text: qsTr("Export Configuration…")
                onClicked: FirmwareController.exportConfiguration()
            }
            Item { Layout.fillWidth: true; visible: !panel.paged }   // -- WP DL3b --
            AppButton {
                objectName: "FirmwareWindow.done"
                visible: parent.done && !panel.paged   // -- WP DL3b --
                primary: true
                text: qsTr("Done")
                onClicked: panel.close()
            }
            AppButton {
                objectName: "FirmwareWindow.primary"
                visible: !parent.done && panel.fw.primaryText.length > 0
                primary: true
                text: panel.fw.primaryText
                enabled: panel.fw.failed || (!panel.fw.confirmed && !panel.fw.guardAsking
                                              && (!panel.fw.custom || panel.fw.imageProblem.length === 0)
                                              && panel.fw.imageNote.length === 0)   // -- WP PGQT --
                onClicked: panel.fw.failed ? FirmwareController.resetRun() : FirmwareController.start()
            }
        }
        // -- WP DL3b -- on the page the content stays at the top
        Item {
            Layout.fillHeight: true
            visible: panel.paged
        }
    }

    Shortcut {
        sequences: ["Enter", "Return"]
        enabled: panel.shown && !guard.visible   // -- WP LAY1b -- only while it shows
        onActivated: {
            if (panel.fw.stage === "verified") panel.close()
            else if (panel.fw.failed) FirmwareController.resetRun()
            else if (panel.fw.primaryText.length && !panel.fw.confirmed) FirmwareController.start()
        }
    }

    // ---- The firmware guard: a typed confirmation (the core's text and words) ----
    Dialog {
        id: guard
        objectName: "FirmwareWindow.guard"
        parent: Overlay.overlay
        modal: true
        closePolicy: Popup.CloseOnEscape
        width: Math.min(400, panel.width - 2 * Theme.spacingLarge)
        x: (panel.width - width) / 2
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
                textFormat: Text.PlainText
                text: FirmwareController.guardMessage
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.Wrap
            }
            // -- WP PGQT -- the generation change, beside the question it is part of
            Text {
                objectName: "FirmwareWindow.guardGeneration"
                Layout.fillWidth: true
                visible: text.length > 0
                text: FirmwareController.generationChange
                font: Theme.fontSmall
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
                // -- WP PGQT -- the field starts empty and hints, never holding the words
                placeholderText: qsTr("Type the words above")
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
