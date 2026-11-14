#ifndef CONFIRM_H
#define CONFIRM_H

#include <QByteArray>
#include <QString>
#include <cstdint>
#include <functional>

#include "CoreApi.h"

// -- WP P1QT2 -- Confirmation tokens for the dangerous operations (P1TK,
// PRO1 W9). The app turns the core's requirement on at start
// (LimitsController), so every gated call goes through here: once the person
// has confirmed (the question answered, Save pressed, the word typed), the
// worker mints a token for that one operation and passes it to the
// `_confirmed` call at once. A token is single use and lives 10 s on the
// connection it was minted on; it is a discipline, not a boundary.
//
// Worker thread only, like the calls it wraps. A refusal whose notice key is
// one of NoticeCenter::gateKey's (a missing token, the installer's lock, a
// limit) is also posted as the core's notice, on the GUI thread.
namespace Confirm {

// The operations, by the core's keys (ConfirmOp::key)
constexpr const char *kSavePreset = "save_preset";
constexpr const char *kCopyPreset = "copy_preset";
constexpr const char *kDeletePreset = "delete_preset";
constexpr const char *kFactoryReset = "factory_reset";
constexpr const char *kSaveOutputConfig = "save_output_config";
constexpr const char *kEnterBootloader = "enter_bootloader";
constexpr const char *kConfigImport = "config_import";
constexpr const char *kVregSave = "vreg_save";
constexpr const char *kFirmwareFlash = "firmware_flash";

// A reply of a `_confirmed` call
struct Reply {
    bool ok = false;
    QString code, error, noticeKey;
    QByteArray json;   // the whole reply (the import's carries its lines)
};

// A fresh token for `op`; "" when the core gave none
QByteArray token(FfiCore *c, const char *op);

// Mints a token for `op` and runs `call(token, buf, len)` once, with a buffer
// large enough for any of these replies (a `_confirmed` call has happened
// whatever the buffer, so it is never called twice)
Reply run(FfiCore *c, const char *op, const std::function<uint32_t(const char *token, char *buf, uint32_t len)> &call,
          uint32_t bytes = 64 * 1024);

// The PRESET_* status (or status byte) the plain call would have answered:
// 0 when done, the device's own status for its refusal, 0xFE when another
// console holds the device, 0xFF for anything else
uint8_t status(const Reply &r);

// The gated calls with the plain calls' answers
uint8_t savePreset(FfiCore *c, uint8_t slot);
uint8_t copyPreset(FfiCore *c, uint8_t source, uint8_t dest);
uint8_t deletePreset(FfiCore *c, uint8_t slot);
uint8_t factoryReset(FfiCore *c);
uint8_t saveOutputConfig(FfiCore *c);
bool enterBootloader(FfiCore *c);
uint8_t vregCommit(FfiCore *c, const char *typed);
uint8_t vregManualSet(FfiCore *c, uint8_t value, const char *typed);

} // namespace Confirm

#endif // CONFIRM_H
