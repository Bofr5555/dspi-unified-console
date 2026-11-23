#include "HostAudioController.h"

#include "AiController.h"
#include "Registrar.h"
#include "SettingsController.h"

#include <QCoreApplication>
#include <QJSEngine>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPointer>

extern "C" {
#include "dspi_core.h"
}

static const Registrar::Controller<HostAudioController> hostAudioController;
// Settings > This Computer (Application, after AI Agents)
static const SettingsRegistry::Page hostAudioPage{
    "hostaudio", SettingsRegistry::Application, 48, QT_TRANSLATE_NOOP("SettingsPages", "This Computer"), "monitor", "HostAudioPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Sound output"), QT_TRANSLATE_NOOP("SettingsPages", "Default output"),
      QT_TRANSLATE_NOOP("SettingsPages", "System volume"), QT_TRANSLATE_NOOP("SettingsPages", "What plays"),
      QT_TRANSLATE_NOOP("SettingsPages", "Host audio") } };

HostAudioController::HostAudioController(const AppContext &, QObject *parent) : QObject(parent) {
    m_pool.setMaxThreadCount(1);
    m_pool.setExpiryTimeout(-1);
    connect(qApp, &QCoreApplication::aboutToQuit, this, [this] { m_shutDown = true; });
}

HostAudioController::~HostAudioController() {
    m_shutDown = true;
    m_pool.waitForDone();
}

HostAudioController *HostAudioController::create(QQmlEngine *, QJSEngine *) {
    HostAudioController *self = Registrar::instance<HostAudioController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

QString HostAudioController::howToDefault() const {
#if defined(Q_OS_WIN)
    return tr("Open Settings > System > Sound and choose the DSPi under Output. One app alone: Settings > System > Sound > Volume mixer, its Output device.");
#elif defined(Q_OS_MACOS)
    return tr("Open System Settings > Sound > Output and choose the DSPi. macOS sends every app to the same output.");
#else
    return tr("Choose the DSPi as the output in your desktop's sound settings (or run wpctl set-default with its number from wpctl status). One app alone: its Playback row in the sound settings or pavucontrol.");
#endif
}

void HostAudioController::refresh() {
    AiController *ai = Registrar::instance<AiController>();
    if (!ai || m_busy || m_shutDown) return;
    // -- WP INTG -- (review RE20) held until the read is done: AiController
    // frees the handle at quit only after
    FfiMcp *mcp = ai->holdHandle();
    if (!mcp) return;
    m_busy = true;
    emit busyChanged();
    QPointer<HostAudioController> self(this);
    m_pool.start([self, ai, mcp] {
        QByteArray buf(64 * 1024, '\0');
        uint32_t need = dspi_mcp_host_audio(mcp, buf.data(), uint32_t(buf.size()));
        if (need > uint32_t(buf.size())) {
            buf.resize(int(need));
            need = dspi_mcp_host_audio(mcp, buf.data(), need);
        }
        ai->releaseHandle();
        QVariantMap reading;
        if (need > 0 && need <= uint32_t(buf.size())) {
            buf.truncate(int(need) - 1);
            reading = QJsonDocument::fromJson(buf).object().toVariantMap();
        }
        QMetaObject::invokeMethod(
            QCoreApplication::instance(),
            [self, reading] {
                if (!self) return;
                self->m_busy = false;
                emit self->busyChanged();
                self->m_reading = reading;
                emit self->readingChanged();
            },
            Qt::QueuedConnection);
    });
}
