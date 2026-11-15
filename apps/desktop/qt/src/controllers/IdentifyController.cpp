// -- WP P2QT -- Device > Identify (P2CC1's one Identify)
#include "IdentifyController.h"

#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "NoticeCenter.h"
#include "Shell.h"
#include "Snapshot.h"

#include <QJSEngine>
#include <QJsonDocument>
#include <algorithm>
#include <memory>

static const Registrar::Controller<IdentifyController> identifyController;

namespace {
// dspi_hub_identify's tone_output for no tone
constexpr uint8_t kNoTone = 0xFF;

QJsonObject object(const QByteArray &reply) { return QJsonDocument::fromJson(reply).object(); }
bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }
} // namespace

IdentifyController::IdentifyController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    ctx.shell->addHandler(QStringLiteral("device.identify"), this, [this] { identify(kSeconds); });
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &IdentifyController::onSnapshot);
}

IdentifyController *IdentifyController::create(QQmlEngine *, QJSEngine *) {
    IdentifyController *self = Registrar::instance<IdentifyController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

// Whether the open device has Identify: asked again when the device, the
// connection's kind or the read-only state changes
void IdentifyController::onSnapshot() {
    const bool was = available();
    const bool connected = m_ctx.device->connected();
    const QString serial = m_ctx.device->serial();
    const bool remote = m_ctx.device->remote();
    m_readOnly = m_ctx.device->readOnly();
    const bool changed = connected != m_connected || serial != m_serial || remote != m_remote;
    m_connected = connected;
    m_serial = serial;
    m_remote = remote;
    if (changed) {
        m_supported = false;
        if (connected) {
            const int probe = ++m_probe;
            m_ctx.core->worker()->call(
                this,
                [](FfiCore *c) {
                    // On a hub: its cap, no I/O. Directly: the device's own
                    // EXT_IDENTIFY answers (one read)
                    const QJsonObject caps = object(readSized([c](char *b, uint32_t n) { return dspi_hub_caps(c, b, n); }));
                    if (caps.value(QLatin1String("onHub")).toBool()) return caps.value(QLatin1String("identify")).toBool();
                    return isOk(object(readSized([c](char *b, uint32_t n) { return dspi_fetch_identify(c, b, n); })));
                },
                [this, probe](bool supported) {
                    if (probe != m_probe) return;
                    const bool before = available();
                    m_supported = supported;
                    if (available() != before) emit availableChanged();
                });
        }
    }
    if (available() != was) emit availableChanged();
}

void IdentifyController::identify(int seconds) {
    auto *notices = Registrar::instance<NoticeCenter>();
    if (!available()) {
        if (notices) notices->postKey(QStringLiteral("unsupported"), tr("This DSPi cannot identify itself."));
        return;
    }
    const uint8_t s = uint8_t(std::clamp(seconds, 0, 120));
    auto reply = std::make_shared<QJsonObject>();
    m_busy = true;
    emit busyChanged();
    m_ctx.device->write(
        QStringLiteral("identify %1").arg(++m_writes),
        [reply, s](FfiCore *c) {
            *reply = object(readSized([&](char *b, uint32_t n) { return dspi_hub_identify(c, s, kNoTone, b, n); }));
            return isOk(*reply);
        },
        [this, reply, s](bool ok) {
            m_busy = false;
            emit busyChanged();
            auto *notices = Registrar::instance<NoticeCenter>();
            if (notices && ok) {
                notices->post(s == 0 ? tr("Identify stopped.")
                                     //: %1 is a number of seconds
                                     : tr("The DSPi blinks its LED for %1 s.").arg(s));
            } else if (notices) {
                const QString key = reply->value(QLatin1String("noticeKey")).toString();
                const QString error = reply->value(QLatin1String("error")).toString();
                if (!key.isEmpty() && !notices->row(key).isEmpty()) notices->postKey(key, error);
                else notices->post(error.isEmpty() ? tr("The DSPi did not identify itself.") : error, QStringLiteral("error"));
            }
            emit identified(ok, *reply);
        });
}
