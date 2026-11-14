//
//  UserConfirmation.swift
//  DSPi Console
//
//  WP P1SW2F (review p1sw2 F5).  A person's yes to one dangerous operation
//  (P1TK): made only where the person confirmed it (a dialog's OK, the Save
//  item, the unsaved-changes prompt's Save, the Install button), and asked for
//  by every gated view-model call.  The call mints the core's single-use
//  token from it on the core's queue right before the gated call, as Qt's
//  `Confirm` mints on its worker right after the person confirmed; a call
//  without one does not compile, so no code path can mint a token of its own.
//

import dspi_core

struct UserConfirmation: Equatable {
    /// The operation the person confirmed.
    let op: ConfirmOp

    private init(_ op: ConfirmOp) { self.op = op }

    static func savePreset() -> UserConfirmation { UserConfirmation(.savePreset) }
    static func copyPreset() -> UserConfirmation { UserConfirmation(.copyPreset) }
    static func deletePreset() -> UserConfirmation { UserConfirmation(.deletePreset) }
    static func saveParams() -> UserConfirmation { UserConfirmation(.saveParams) }
    static func factoryReset() -> UserConfirmation { UserConfirmation(.factoryReset) }
    static func saveOutputConfig() -> UserConfirmation { UserConfirmation(.saveOutputConfig) }
    static func enterBootloader() -> UserConfirmation { UserConfirmation(.enterBootloader) }
    static func configImport() -> UserConfirmation { UserConfirmation(.configImport) }
    static func firmwareFlash() -> UserConfirmation { UserConfirmation(.firmwareFlash) }

    /// The token for this confirmation, minted now on `core` (the core's
    /// queue, right before the gated call).  `expected`: the call's own
    /// operation; another one is a programming error (a debug assertion), and
    /// the core refuses its token for this call anyway.
    func token(_ core: DspiCore, for expected: ConfirmOp) -> String {
        assert(op == expected, "a confirmation of \(op) used for \(expected)")
        return core.confirmToken(op: op)
    }
}
