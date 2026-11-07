#include "HostAudioFormat.h"
#include "DemoDevice.h"
#include "DSPiBridge.h"
#include "Registrar.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>
#include <QRunnable>
#include <QtGlobal>

static const Registrar::Controller<HostAudioFormatController> hostAudioRegistered("hostAudio");

namespace HostAudio {

namespace {
// The USB product string is "Weeb Labs DSPi"
const QLatin1String kToken("DSPi");
// DSPiBridge's demo device, which no audio system knows of
const QLatin1String kDemoSerial(DemoDevice::kSerial);
// How long changes gather before one read: a playing stream changes often
constexpr int kSettleMs = 500;
constexpr int kMatrixInputs = 2;
constexpr int kAdatInputs = 8;
// INPUT_SOURCE_*, Constants.swift
constexpr int kSourceSpdif = 1, kSourceI2s = 2, kSourceAdat = 3, kSourceSpdif2 = 4, kSourceSpdif3 = 5, kSourceSpdif4 = 6;
}

bool isDspi(const Endpoint &endpoint) {
    return endpoint.text.contains(kToken, Qt::CaseInsensitive);
}

int resolve(const std::vector<Endpoint> &endpoints, const QString &serial) {
    int agreed = 0;
    bool differ = false;
    for (const Endpoint &e : endpoints) {
        if (e.channels <= 0 || !isDspi(e)) continue;
        if (!serial.isEmpty() && e.serialText.contains(serial, Qt::CaseInsensitive)) return e.channels;
        if (agreed != 0 && agreed != e.channels) differ = true;
        agreed = e.channels;
    }
    return differ ? 0 : agreed;
}

int effectiveInputCount(int inputSource, int i2sChannels, int activeInputs, int hostChannels, int deviceInputs) {
    int base = 0;
    switch (inputSource) {
    case kSourceSpdif:
    case kSourceSpdif2:
    case kSourceSpdif3:
    case kSourceSpdif4:
        base = kMatrixInputs;
        break;
    case kSourceI2s:
        base = i2sChannels;
        break;
    case kSourceAdat:
        base = kAdatInputs;
        break;
    default:    // USB, and firmware without input switching, which reports USB
        base = hostChannels > 0 ? hostChannels : activeInputs;
        break;
    }
    return qMin(deviceInputs, qMax(kMatrixInputs, base));
}

bool phaseHasHub(const QString &phase) {
    return phase != QLatin1String("idle") && phase != QLatin1String("closed") && phase != QLatin1String("refused");
}

int waveFormatChannels(const char *blob, qint64 size) {
    if (!blob || size < 4) return 0;
    return int(uint8_t(blob[2])) | (int(uint8_t(blob[3])) << 8);
}

#if !defined(Q_OS_LINUX) && !defined(Q_OS_WIN)
namespace {
// No host API here: always the device's report. The Mac's is CoreAudio, in Swift.
class NoBackend : public Backend {
public:
    std::vector<Endpoint> read() override { return {}; }
    void startWatching(std::function<void()>) override {}
    void stopWatching() override {}
};
}
std::unique_ptr<Backend> makeBackend() { return std::make_unique<NoBackend>(); }
#endif

} // namespace HostAudio

namespace {
bool hubSessionUp(FfiCore *core) {
    QByteArray buf(4 * 1024, '\0');
    uint32_t needed = dspi_link_status(core, buf.data(), uint32_t(buf.size()));
    if (needed > uint32_t(buf.size())) {
        buf.resize(int(needed));
        needed = dspi_link_status(core, buf.data(), uint32_t(buf.size()));
    }
    // A status it can't read: this host's format could be another device's, so don't use it
    if (needed == 0 || needed > uint32_t(buf.size())) return true;
    buf.truncate(int(needed - 1));
    return HostAudio::phaseHasHub(QJsonDocument::fromJson(buf).object().value(QLatin1String("phase")).toString());
}
}

HostAudioFormatController::HostAudioFormatController(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge), m_core(bridge->core()), m_backend(HostAudio::makeBackend())
{
    m_worker.setMaxThreadCount(1);
    m_settle.setSingleShot(true);
    m_settle.setInterval(HostAudio::kSettleMs);
    connect(&m_settle, &QTimer::timeout, this, &HostAudioFormatController::requestRead);
    connect(m_bridge, &DSPiBridge::stateChanged, this, &HostAudioFormatController::sync);
    connect(m_bridge, &DSPiBridge::statusChanged, this, &HostAudioFormatController::sync);
    connect(m_bridge, &DSPiBridge::devicesChanged, this, [this]() { if (m_hostSide) m_settle.start(); });
    sync();
}

HostAudioFormatController::~HostAudioFormatController() {
    m_generation++;
    setWatching(false);
    m_worker.waitForDone();
}

void HostAudioFormatController::setBackendForTest(std::unique_ptr<HostAudio::Backend> backend, bool useOnDemo) {
    m_generation++;
    setWatching(false);
    m_worker.waitForDone();
    m_settle.stop();
    m_backend = std::move(backend);
    m_useOnDemo = useOnDemo;
    m_hostSide = false;
    m_reading = false;
    m_readAgain = false;
    m_serial.clear();   // sync() starts over
    m_host = 0;
    sync();
    updateCount();
}

void HostAudioFormatController::sync() {
    const QString serial = m_bridge->connected() ? m_bridge->selectedSerial() : QString();
    const int active = m_bridge->activeInputChannels();
    if (serial != m_serial) {
        m_serial = serial;
        m_generation++;
        m_settle.stop();
        m_host = 0;
        m_hostSide = !serial.isEmpty() && (m_useOnDemo || serial != HostAudio::kDemoSerial);
        // The first read says whether it is this host's device: then it starts watching
        if (!m_hostSide) setWatching(false);
        else requestRead();
    } else if (active != m_active && m_hostSide) {
        // The stream opened or closed: under ALSA that is the only sign there is
        m_settle.start();
    }
    m_active = active;
    updateCount();
}

void HostAudioFormatController::setWatching(bool on) {
    if (on == m_watching) return;
    m_watching = on;
    if (on) {
        m_backend->startWatching([this]() {
            QMetaObject::invokeMethod(this, [this]() {
                if (m_hostSide && !m_settle.isActive()) m_settle.start();
            }, Qt::QueuedConnection);
        });
    } else {
        m_backend->stopWatching();
    }
}

void HostAudioFormatController::requestRead() {
    if (!m_hostSide) return;
    if (m_reading) {
        m_readAgain = true;
        return;
    }
    startRead();
}

void HostAudioFormatController::startRead() {
    m_reading = true;
    m_readAgain = false;
    const unsigned generation = m_generation;
    const QString serial = m_serial;
    FfiCore *core = m_core;
    HostAudio::Backend *backend = m_backend.get();
    m_worker.start(QRunnable::create([this, core, backend, serial, generation]() {
        int channels = 0;
        // A remote device's format is on its own host
        const bool hub = m_generation != generation || hubSessionUp(core);
        if (!hub && m_generation == generation) channels = HostAudio::resolve(backend->read(), serial);
        QMetaObject::invokeMethod(this, [this, channels, hub, generation]() { finishRead(channels, hub, generation); }, Qt::QueuedConnection);
    }));
}

void HostAudioFormatController::finishRead(int channels, bool hub, unsigned generation) {
    m_reading = false;
    if (generation == m_generation) {
        setWatching(m_hostSide && !hub);
        if (channels != m_host) qInfo("Host audio format: %d channels (0: use the device's report)", channels);
        m_host = channels;
        updateCount();
    } else {
        m_readAgain = m_hostSide;   // that read was for another device
    }
    if (m_readAgain) startRead();
}

void HostAudioFormatController::updateCount() {
    const DspState *s = dspi_get_state(m_core);
    const int count = HostAudio::effectiveInputCount(s->input_source, s->i2s_input_channels,
                                                     m_bridge->activeInputChannels(), m_host, s->num_input_channels);
    if (count == m_count && m_host == m_shownHost) return;
    m_count = count;
    m_shownHost = m_host;
    emit countChanged();
}
