// -- WP P1QT2 -- Confirmation tokens for the dangerous operations (Confirm.h)
#include "Confirm.h"

#include "NoticeCenter.h"
#include "Registrar.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>
#include <QPointer>
#include <QRegularExpression>

namespace Confirm {

namespace {

// One call, one buffer: `len` bytes, the reply cut at its NUL; "" when it
// did not fit (the call happened all the same)
QByteArray once(const std::function<uint32_t(char *, uint32_t)> &fill, uint32_t len) {
    QByteArray buf(int(len), '\0');
    const uint32_t need = fill(buf.data(), len);
    if (need == 0 || need > len) return QByteArray();
    buf.truncate(int(need) - 1);
    return buf;
}

// The core's notice for a refusal the person should hear about, on the GUI thread
void notice(const Reply &r) {
    if (r.ok || !NoticeCenter::gateKey(r.noticeKey)) return;
    QPointer<NoticeCenter> notices = Registrar::instance<NoticeCenter>();
    if (!notices) return;
    const QString key = r.noticeKey, error = r.error;
    QMetaObject::invokeMethod(
        notices.data(), [notices, key, error] { if (notices) notices->postKey(key, error); }, Qt::QueuedConnection);
}

} // namespace

// -- WP INTI --
uint8_t loadPreset(FfiCore *c, uint8_t slot) {
    const uint8_t status = dspi_load_preset(c, slot);
    const QString text = QString::fromUtf8(once([&](char *b, uint32_t n) { return dspi_preset_load_notice(c, b, n) ? uint32_t(qstrlen(b) + 1) : 0; }, 4096));
    QPointer<NoticeCenter> notices = Registrar::instance<NoticeCenter>();
    if (text.isEmpty() || !notices) return status;
    QMetaObject::invokeMethod(
        notices.data(), [notices, text] { if (notices) notices->post(text, QStringLiteral("warning")); }, Qt::QueuedConnection);
    return status;
}

QByteArray token(FfiCore *c, const char *op) {
    const QJsonObject o = QJsonDocument::fromJson(once([&](char *b, uint32_t n) { return dspi_confirm_token(c, op, b, n); }, 512)).object();
    return o.value(QLatin1String("ok")).toBool() ? o.value(QLatin1String("value")).toString().toUtf8() : QByteArray();
}

Reply run(FfiCore *c, const char *op, const std::function<uint32_t(const char *, char *, uint32_t)> &call, uint32_t bytes) {
    const QByteArray t = token(c, op);
    Reply r;
    r.json = once([&](char *b, uint32_t n) { return call(t.isEmpty() ? nullptr : t.constData(), b, n); }, bytes);
    const QJsonObject o = QJsonDocument::fromJson(r.json).object();
    r.ok = o.value(QLatin1String("ok")).toBool();
    r.code = o.value(QLatin1String("code")).toString();
    r.error = o.value(QLatin1String("error")).toString();
    r.noticeKey = o.value(QLatin1String("noticeKey")).toString();
    if (!r.ok && r.code.isEmpty()) r.code = QStringLiteral("io");
    notice(r);
    return r;
}

uint8_t status(const Reply &r) {
    if (r.ok) return 0;
    if (r.code == QLatin1String("locked")) return 0xFE;
    if (r.code == QLatin1String("refused")) {
        // "device refused the request (status N)"
        static const QRegularExpression number(QStringLiteral("status (\\d+)"));
        const QRegularExpressionMatch m = number.match(r.error);
        if (m.hasMatch()) {
            const uint v = m.captured(1).toUInt();
            if (v > 0 && v < 0xFF) return uint8_t(v);
        }
    }
    return 0xFF;
}

uint8_t savePreset(FfiCore *c, uint8_t slot) {
    return status(run(c, kSavePreset, [&](const char *t, char *b, uint32_t n) { return dspi_save_preset_confirmed(c, slot, t, b, n); }));
}

uint8_t copyPreset(FfiCore *c, uint8_t source, uint8_t dest) {
    return status(run(c, kCopyPreset,
                      [&](const char *t, char *b, uint32_t n) { return dspi_copy_preset_confirmed(c, source, dest, t, b, n); }));
}

uint8_t deletePreset(FfiCore *c, uint8_t slot) {
    return status(run(c, kDeletePreset, [&](const char *t, char *b, uint32_t n) { return dspi_delete_preset_confirmed(c, slot, t, b, n); }));
}

uint8_t factoryReset(FfiCore *c) {
    return status(run(c, kFactoryReset, [&](const char *t, char *b, uint32_t n) { return dspi_factory_reset_confirmed(c, t, b, n); }));
}

uint8_t saveOutputConfig(FfiCore *c) {
    return status(run(c, kSaveOutputConfig, [&](const char *t, char *b, uint32_t n) { return dspi_save_output_config_confirmed(c, t, b, n); }));
}

bool enterBootloader(FfiCore *c) {
    return run(c, kEnterBootloader, [&](const char *t, char *b, uint32_t n) { return dspi_enter_bootloader_confirmed(c, t, b, n); }).ok;
}

uint8_t vregCommit(FfiCore *c, const char *typed) {
    return status(run(c, kVregSave, [&](const char *t, char *b, uint32_t n) { return dspi_core_vreg_cal_commit_confirmed(c, typed, t, b, n); }));
}

uint8_t vregManualSet(FfiCore *c, uint8_t value, const char *typed) {
    return status(
        run(c, kVregSave, [&](const char *t, char *b, uint32_t n) { return dspi_core_vreg_manual_set_confirmed(c, value, typed, t, b, n); }));
}

} // namespace Confirm
