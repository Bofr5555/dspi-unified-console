import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P1QT2 -- Settings > Installer (OD-P1e: reached from Settings while
// Window > Mode… stays hidden). The limits profile (P1LM), the installer's PIN
// and the lock (P1LK). Hardware settings apply at once: no Save bar, each
// change is a write. With no PIN set nothing here asks for one; with a PIN
// set and this console locked, the page asks for it first.
SettingsPage {
    id: page
    objectName: "InstallerPage"
    readonly property bool open: LimitsController.installerOpen
    readonly property var limits: LimitsController.limits

    // Nothing open: say so
    SettingsGroup {
        visible: !DeviceModel.connected
        title: qsTr("Installer")
        iconName: "key-round"
        Text {
            Layout.fillWidth: true
            text: qsTr("Connect a DSPi to set its limits and the installer's PIN.")
            font: Theme.fontBody
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    // ---- Locked: the PIN first ----------------------------------------------------
    SettingsGroup {
        objectName: "Installer.unlockGroup"
        visible: DeviceModel.connected && !page.open
        title: LimitsController.heldByHub ? qsTr("Unlock on the hub") : qsTr("Unlock")
        iconName: "lock"
        Text {
            objectName: "Installer.scope"
            Layout.fillWidth: true
            text: LimitsController.scopeText
            font: Theme.fontBody
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            BorderedField {
                id: unlockPin
                objectName: "Installer.unlockPin"
                Layout.fillWidth: true
                label: qsTr("Installer PIN or recovery code")
                placeholderText: qsTr("PIN or recovery code")
                echoMode: TextInput.Password
                enabled: !LimitsController.busy
                onAccepted: unlockButton.clicked()
            }
            AppButton {
                id: unlockButton
                objectName: "Installer.unlock"
                text: qsTr("Unlock")
                primary: true
                enabled: unlockPin.text.length > 0 && !LimitsController.busy
                onClicked: {
                    LimitsController.unlock(unlockPin.text)
                    unlockPin.text = ""
                }
            }
        }
        Text {
            objectName: "Installer.unlockError"
            Layout.fillWidth: true
            visible: text.length > 0
            text: LimitsController.pinError
            font: Theme.fontSmall
            color: Theme.dangerText
            wrapMode: Text.Wrap
        }
    }

    // ---- The limits profile -------------------------------------------------------
    SettingsGroup {
        objectName: "Installer.limits"
        visible: DeviceModel.connected && page.open
        title: qsTr("Limits")
        iconName: "gauge"
        footer: NoticeCenter.line("limits.scope")

        Text {
            Layout.fillWidth: true
            text: qsTr("Limits narrow what this DSPi may be set to from this console. A change applies at once; settings already outside a new limit are brought inside it, and what changed is shown.")
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        RowLayout {
            objectName: "Installer.standingNotice"
            Layout.fillWidth: true
            visible: LimitsController.standingNotice.length > 0
            spacing: Theme.spacingSmall
            Icon {
                name: "circle-alert"
                size: 14
                color: Theme.dangerText
            }
            Text {
                Layout.fillWidth: true
                text: LimitsController.standingNotice
                font: Theme.fontSmall
                color: Theme.dangerText
                wrapMode: Text.Wrap
            }
        }
        LimitField {
            objectName: "Installer.masterVolumeMax"
            title: qsTr("Master volume at most")
            value: page.limits.masterVolumeMaxDb
            key: "masterVolumeMaxDb"; decimals: 1; defaultValue: -10
            enabled: !LimitsController.busy
            onEdited: v => LimitsController.setLimit("masterVolumeMaxDb", v)
        }
        LimitField {
            objectName: "Installer.maxPeqBoost"
            title: qsTr("EQ boost at most, every band")
            value: page.limits.maxPeqBoostDb
            key: "maxPeqBoostDb"; decimals: 1; defaultValue: 6
            enabled: !LimitsController.busy
            onEdited: v => LimitsController.setLimit("maxPeqBoostDb", v)
        }
        // PRO1 S2: an output above -10 dB of gain with no limiter
        RowLayout {
            objectName: "Installer.unprotected"
            Layout.fillWidth: true
            visible: ToolsController.unprotectedText.length > 0
            spacing: Theme.spacingSmall
            Icon {
                name: "triangle-alert"
                size: 14
                color: Theme.warning
            }
            Text {
                Layout.fillWidth: true
                text: ToolsController.unprotectedText
                font: Theme.fontSmall
                color: Theme.text
                wrapMode: Text.Wrap
            }
            AppButton {
                objectName: "Installer.protectAll"
                text: qsTr("Protect all outputs")
                toolTip: qsTr("Every limiter on at -1 dBFS, the stereo pairs linked")
                enabled: ToolsController.writable && ToolsController.limiterSupported
                onClicked: ToolsController.protectAllOutputs()
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "Installer.clearLimits"
                text: qsTr("Remove All Limits")
                enabled: LimitsController.active && !LimitsController.busy
                onClicked: LimitsController.clearLimits()
            }
        }
    }

    // One group per output
    Repeater {
        model: DeviceModel.connected && page.open ? LimitsController.outputs : []
        delegate: SettingsGroup {
            id: outGroup
            required property var modelData
            required property int index
            readonly property int output: modelData.output
            objectName: "Installer.output." + output
            title: modelData.name
            iconName: "volume-2"
            LimitField {
                objectName: "Installer.output." + outGroup.output + ".maxGain"
                title: qsTr("Gain at most")
                value: outGroup.modelData.maxGainDb
                key: "maxGainDb"; decimals: 1; defaultValue: 0
                enabled: !LimitsController.busy
                onEdited: v => LimitsController.setOutputLimit(outGroup.output, "maxGainDb", v)
            }
            LimitField {
                objectName: "Installer.output." + outGroup.output + ".limiter"
                title: qsTr("Limiter on, ceiling at most")
                value: outGroup.modelData.limiterCeilingDb
                key: "limiterCeilingDb"; decimals: 1; defaultValue: -1
                enabled: !LimitsController.busy
                onEdited: v => LimitsController.setOutputLimit(outGroup.output, "limiterCeilingDb", v)
            }
            LimitField {
                objectName: "Installer.output." + outGroup.output + ".highpass"
                title: qsTr("High-pass kept, at or above")
                value: outGroup.modelData.highpassMinHz
                key: "highpassMinHz"; decimals: 0; defaultValue: 80
                enabled: !LimitsController.busy
                onEdited: v => LimitsController.setOutputLimit(outGroup.output, "highpassMinHz", v)
            }
            LimitField {
                objectName: "Installer.output." + outGroup.output + ".slope"
                visible: outGroup.modelData.highpassMinHz !== undefined
                title: qsTr("High-pass slope at least")
                value: outGroup.modelData.highpassMinSlopeDb
                key: "highpassMinSlopeDb"; decimals: 0; defaultValue: 12
                enabled: !LimitsController.busy
                onEdited: v => LimitsController.setOutputLimit(outGroup.output, "highpassMinSlopeDb", v)
            }
            LimitField {
                objectName: "Installer.output." + outGroup.output + ".delayMin"
                title: qsTr("Delay at least")
                value: outGroup.modelData.delayMinMs
                key: "delayMinMs"; decimals: 2; defaultValue: 0
                enabled: !LimitsController.busy
                onEdited: v => LimitsController.setOutputLimit(outGroup.output, "delayMinMs", v)
            }
            LimitField {
                objectName: "Installer.output." + outGroup.output + ".delayMax"
                title: qsTr("Delay at most")
                value: outGroup.modelData.delayMaxMs
                key: "delayMaxMs"; decimals: 2; defaultValue: 10
                enabled: !LimitsController.busy
                onEdited: v => LimitsController.setOutputLimit(outGroup.output, "delayMaxMs", v)
            }
        }
    }

    // ---- The installer's note -----------------------------------------------------
    SettingsGroup {
        objectName: "Installer.noteGroup"
        visible: DeviceModel.connected && page.open
        title: qsTr("Installer's note")
        iconName: "file-text"
        footer: qsTr("Shown to the people who use this system while it is locked.")
        BorderedField {
            objectName: "Installer.note"
            Layout.fillWidth: true
            label: qsTr("Note")
            placeholderText: qsTr("What this system is set up for")
            text: page.limits.note || ""
            enabled: !LimitsController.busy
            onCommitted: t => LimitsController.setText("note", t)
        }
        BorderedField {
            objectName: "Installer.contact"
            Layout.fillWidth: true
            label: qsTr("Contact")
            placeholderText: qsTr("How to reach the installer")
            text: page.limits.contact || ""
            enabled: !LimitsController.busy
            onCommitted: t => LimitsController.setText("contact", t)
        }
    }

    // ---- The PIN and the lock -----------------------------------------------------
    SettingsGroup {
        objectName: "Installer.pinGroup"
        visible: DeviceModel.connected && page.open && !LimitsController.heldByHub
        title: qsTr("Installer PIN")
        iconName: "key-round"
        footer: NoticeCenter.sentence("installer_lock.scope")

        // No PIN: set one (nothing is locked until it covers the settings)
        Text {
            Layout.fillWidth: true
            visible: !LimitsController.pinSet
            text: qsTr("No PIN is set: nothing is locked and nothing asks for one. Set a PIN to lock this console's settings for everyday use; volume, mute and presets stay open.")
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        RowLayout {
            Layout.fillWidth: true
            visible: !LimitsController.pinSet
            spacing: Theme.spacingSmall
            BorderedField {
                id: newPin
                objectName: "Installer.newPin"
                Layout.fillWidth: true
                label: qsTr("New PIN")
                placeholderText: qsTr("New PIN (4 to 64 characters)")
                echoMode: TextInput.Password
                maximumLength: 64
            }
            BorderedField {
                id: repeatPin
                objectName: "Installer.repeatPin"
                Layout.fillWidth: true
                label: qsTr("New PIN again")
                placeholderText: qsTr("The same again")
                echoMode: TextInput.Password
                maximumLength: 64
                onAccepted: setPinButton.clicked()
            }
            AppButton {
                id: setPinButton
                objectName: "Installer.setPin"
                text: qsTr("Set PIN")
                primary: true
                enabled: newPin.text.length >= 4 && repeatPin.text.length > 0 && !LimitsController.busy
                onClicked: {
                    LimitsController.setPin(newPin.text, repeatPin.text)
                    newPin.text = ""
                    repeatPin.text = ""
                }
            }
        }

        // The recovery code, this once
        ColumnLayout {
            objectName: "Installer.recovery"
            Layout.fillWidth: true
            visible: LimitsController.recoveryCode.length > 0
            spacing: Theme.spacingSmall
            Text {
                Layout.fillWidth: true
                text: NoticeCenter.line("installer_lock.recovery", qsTr("Keep the recovery code"))
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.Wrap
            }
            RowLayout {
                Layout.fillWidth: true
                TextEdit {
                    objectName: "Installer.recoveryCode"
                    Layout.fillWidth: true
                    text: LimitsController.recoveryCode
                    readOnly: true
                    selectByMouse: true
                    font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontTitle.pixelSize })
                    color: Theme.accentText
                    selectionColor: Theme.selection
                    Accessible.role: Accessible.StaticText
                    Accessible.name: qsTr("Recovery code")
                }
                AppButton {
                    objectName: "Installer.recoveryDone"
                    text: qsTr("I Have Written It Down")
                    onClicked: LimitsController.dismissRecovery()
                }
            }
            Text {
                Layout.fillWidth: true
                text: NoticeCenter.row("installer_lock.recovery").action || ""
                visible: text.length > 0
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
        }

        // Unlocked with the recovery code: a new PIN is due
        Text {
            objectName: "Installer.recovered"
            Layout.fillWidth: true
            visible: LimitsController.pinSet && LimitsController.lock.recovered === true
            text: qsTr("Unlocked with the recovery code. Set a new PIN: remove this one, then set another.")
            font: Theme.fontBody
            color: Theme.warning
            wrapMode: Text.Wrap
        }

        // A PIN: what the lock covers, lock now, remove
        SettingsToggle {
            objectName: "Installer.covers"
            visible: LimitsController.pinSet
            title: qsTr("Lock the settings")
            caption: qsTr("When locked, only volume, mute and presets can be changed on this console.")
            checked: LimitsController.covers
            enabled: !LimitsController.busy
            onToggled: on => LimitsController.setCovers(on)
        }
        RowLayout {
            Layout.fillWidth: true
            visible: LimitsController.pinSet
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "Installer.lockNow"
                text: qsTr("Lock Now")
                iconName: "lock"
                enabled: LimitsController.covers && !LimitsController.busy
                onClicked: LimitsController.lockNow()
            }
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "Installer.clearPin"
                text: qsTr("Remove PIN…")
                destructive: true
                enabled: !LimitsController.busy
                onClicked: removeQuestion.open()
            }
        }
        Text {
            objectName: "Installer.pinError"
            Layout.fillWidth: true
            visible: text.length > 0 && page.open
            text: LimitsController.pinError
            font: Theme.fontSmall
            color: Theme.dangerText
            wrapMode: Text.Wrap
        }
    }

    // The hub holds the lock: its PIN is set on the hub
    SettingsGroup {
        objectName: "Installer.hubGroup"
        visible: DeviceModel.connected && page.open && LimitsController.heldByHub
        title: qsTr("Installer PIN")
        iconName: "network"
        Text {
            Layout.fillWidth: true
            text: NoticeCenter.line("installer_lock.hub")
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        AppButton {
            objectName: "Installer.hubLockNow"
            text: qsTr("Lock Now")
            iconName: "lock"
            enabled: !LimitsController.busy
            onClicked: LimitsController.lockNow()
        }
    }

    QuestionDialog {
        id: removeQuestion
        objectName: "Installer.removeQuestion"
        parent: Overlay.overlay
        title: qsTr("Remove the installer PIN?")
        text: qsTr("Nothing will be locked on this console any more, and the recovery code stops working. The limits stay.")
        acceptText: qsTr("Remove PIN")
        destructive: true
        onAnswered: answer => { if (answer === "accept") LimitsController.clearPin() }
    }

    SettingsExtensions {
        page: "installer"
    }
}
