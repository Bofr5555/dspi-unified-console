#include "HostAudioFormat.h"

#include "ChannelListModel.h"
#include "DeviceModel.h"
#include "GraphController.h"

#include <QCoreApplication>
#include <QJSEngine>
#include <QPointer>
#include <QThreadPool>
#include <QtEndian>

static const Registrar::Controller<HostAudioFormat> hostAudioFormat;

namespace HostAudio {

int waveFormatChannels(const char *data, qint64 size) {
    // WAVEFORMATEX: wFormatTag (2 bytes), then nChannels (2 bytes)
    if (!data || size < 4) return 0;
    return qFromLittleEndian<quint16>(data + 2);
}

QString usbSerialFromInstanceIds(const QStringList &chain) {
    for (const QString &id : chain) {
        if (!id.startsWith(QLatin1String("USB\\VID_"), Qt::CaseInsensitive)) continue;
        const QString last = id.section(QLatin1Char('\\'), -1);
        // A last part with '&' is one Windows made up: the device gave no serial
        if (!last.isEmpty() && !last.contains(QLatin1Char('&'))) return last;
    }
    return QString();
}

int channelsFor(const std::vector<Endpoint> &endpoints, const QString &serial) {
    if (!serial.isEmpty())
        for (const Endpoint &e : endpoints)
            if (e.channels > 0 && e.serialText.compare(serial, Qt::CaseInsensitive) == 0) return e.channels;
    // The Mac's rule without a serial: an endpoint named DSPi, if it is the only one
    int found = 0, count = 0;
    for (const Endpoint &e : endpoints) {
        if (e.serialText.isEmpty() && e.text.contains(QLatin1String("DSPi"), Qt::CaseInsensitive)) {
            found = e.channels;
            ++count;
        }
    }
    return count == 1 ? found : 0;
}

} // namespace HostAudio

HostAudioFormat::HostAudioFormat(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx), m_backend(HostAudio::makeBackend()) {
    m_timer.setInterval(kRereadMs);
    connect(&m_timer, &QTimer::timeout, this, &HostAudioFormat::read);
    connect(ctx.device, &DeviceModel::snapshotApplied, this, [this] {
        if (m_overridden) return;
        const bool open = m_ctx.device->connected() && !m_ctx.device->demo() && !m_ctx.device->remote();
        if (!open) {
            m_timer.stop();
            setChannels(0);
            return;
        }
        if (!m_backend->notifies() && !m_timer.isActive()) m_timer.start();
        read();
    });
    if (m_backend->notifies()) {
        QPointer<HostAudioFormat> self(this);
        m_backend->startWatching([self] {
            QMetaObject::invokeMethod(self, [self] { if (self) self->read(); }, Qt::QueuedConnection);
        });
    }
}

HostAudioFormat::~HostAudioFormat() {
    if (m_backend) m_backend->stopWatching();
}

HostAudioFormat *HostAudioFormat::create(QQmlEngine *, QJSEngine *) {
    HostAudioFormat *self = Registrar::instance<HostAudioFormat>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

void HostAudioFormat::read() {
    if (m_reading || m_overridden) return;
    m_reading = true;
    const QString serial = m_ctx.device->serial();
    QPointer<HostAudioFormat> self(this);
    std::shared_ptr<HostAudio::Backend> backend = m_backend;
    QThreadPool::globalInstance()->start([self, backend, serial] {
        const int n = HostAudio::channelsFor(backend->read(), serial);
        QMetaObject::invokeMethod(QCoreApplication::instance(), [self, n] {
            if (!self) return;
            self->m_reading = false;
            if (!self->m_overridden) self->setChannels(n);
        }, Qt::QueuedConnection);
    });
}

void HostAudioFormat::setOverride(int channels) {
    m_overridden = true;
    m_timer.stop();
    setChannels(channels);
}

void HostAudioFormat::setChannels(int channels) {
    channels = qMax(0, channels);
    if (channels == m_channels) return;
    m_channels = channels;
    ChannelListModel::setHostInputs(channels);
    emit channelsChanged();
    // The strips follow: the list is made again from a fresh read, and the
    // graph shows the new default curves
    if (m_ctx.device->connected()) m_ctx.device->refresh();
    if (GraphController *graph = Registrar::instance<GraphController>()) graph->resetDefaultVisibility();
}
