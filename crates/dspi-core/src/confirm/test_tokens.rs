//! Test-only calls that go through the token gate with a fresh token (WP P1TZ).
//!
//! With tokens always required (OD-P1f), a test that exercises a gated
//! operation mints a token for it, as a client does after its dialog: each
//! `*_tok` call mints one on the current connection and passes it to the
//! `_confirmed` form, so the real gate accepts and spends it.

use crate::{ConfirmOp, DspiCore, Result};

impl DspiCore {
    fn tok(&self, op: ConfirmOp) -> Option<String> {
        Some(self.confirm_token(op))
    }

    pub(crate) fn save_preset_tok(&self, slot: u8) -> Result<()> {
        self.save_preset_confirmed(slot, self.tok(ConfirmOp::SavePreset))
    }

    pub(crate) fn copy_preset_tok(&self, source: u8, dest: u8) -> Result<()> {
        self.copy_preset_confirmed(source, dest, self.tok(ConfirmOp::CopyPreset))
    }

    pub(crate) fn delete_preset_tok(&self, slot: u8) -> Result<()> {
        self.delete_preset_confirmed(slot, self.tok(ConfirmOp::DeletePreset))
    }

    pub(crate) fn save_params_tok(&self) -> Result<()> {
        self.save_params_confirmed(self.tok(ConfirmOp::SaveParams))
    }

    pub(crate) fn factory_reset_tok(&self) -> Result<()> {
        self.factory_reset_confirmed(self.tok(ConfirmOp::FactoryReset))
    }

    pub(crate) fn save_output_config_tok(&self) -> Result<()> {
        self.save_output_config_confirmed(self.tok(ConfirmOp::SaveOutputConfig))
    }

    pub(crate) fn enter_bootloader_tok(&self) -> Result<()> {
        self.enter_bootloader_confirmed(self.tok(ConfirmOp::EnterBootloader))
    }

    pub(crate) fn config_import_tok(&self, text: String, options: u32) -> String {
        self.config_import_confirmed(text, options, u32::MAX, self.tok(ConfirmOp::ConfigImport))
    }

    pub(crate) fn config_import_selected_tok(
        &self,
        text: String,
        options: u32,
        eq_channels: u32,
    ) -> String {
        self.config_import_confirmed(
            text,
            options,
            eq_channels,
            self.tok(ConfirmOp::ConfigImport),
        )
    }

    pub(crate) fn core_vreg_cal_commit_tok(&self, typed: Option<String>) -> Result<()> {
        self.core_vreg_cal_commit_confirmed(typed, self.tok(ConfirmOp::VregSave))
    }

    pub(crate) fn core_vreg_manual_set_tok(&self, value: u8, typed: Option<String>) -> Result<()> {
        self.core_vreg_manual_set_confirmed(value, typed, self.tok(ConfirmOp::VregSave))
    }
}
