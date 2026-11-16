#include "AudioCapture.h"

// -- WP P5QCAP -- see AudioCapture.h. Capture only: no audio output class is
// used here (cmake/wp/p5qcap.cmake checks).

#include <QAudioDevice>
#include <QAudioSource>
#include <QIODevice>
#include <QMediaDevices>
#include <QThread>
#include <cmath>
#include <cstring>

using namespace dspi_capture;

namespace {

// The QAudioSource reader's buffer: room for a slow reader thread (a held
// scheduler, a busy machine) before the backend overruns
constexpr qint64 kBufferUs = 250000;
// Frames the backend may have counted as processed but not yet handed over,
// beyond its buffer: not taken as lost
constexpr qint64 kLeadSlackUs = 100000;

QAudioDevice findInput(const QByteArray &id) {
    if (id.isEmpty()) return QMediaDevices::defaultAudioInput();
    const QList<QAudioDevice> list = QMediaDevices::audioInputs();
    for (const QAudioDevice &d : list) {
        if (d.id() == id) return d;
    }
    return QAudioDevice();
}

bool convertible(QAudioFormat::SampleFormat f) {
    return f == QAudioFormat::UInt8 || f == QAudioFormat::Int16 || f == QAudioFormat::Int32 ||
           f == QAudioFormat::Float;
}

// The microphone through Qt Multimedia's native backend (PulseAudio or PipeWire
// on Linux, WASAPI on Windows; no media plugin is involved).
class QtInput final : public Source {
public:
    QtInput(QAudioDevice device, int rateHz) : m_device(std::move(device)), m_rateHz(rateHz) {}
    ~QtInput() override { close(); }

    bool open(Failure *failure, QString *detail) override {
        if (m_device.isNull()) {
            *failure = Failure::NoDevice;
            *detail = QStringLiteral("no such audio input");
            return false;
        }
        if (!chooseFormat()) {
            *failure = Failure::FormatUnsupported;
            *detail = QStringLiteral("the input offers no PCM or float format");
            return false;
        }
        m_source = std::make_unique<QAudioSource>(m_device, m_format);
        m_source->setBufferSize(static_cast<qsizetype>(m_format.bytesForDuration(kBufferUs)));
        QObject::connect(m_source.get(), &QAudioSource::stateChanged, m_source.get(), [this](QtAudio::State state) {
            if (state == QtAudio::StoppedState && m_source && m_source->error() != QtAudio::NoError && !m_closing) {
                fail(Failure::DeviceLost,
                     QStringLiteral("the audio input stopped (error %1)").arg(int(m_source->error())));
            }
        });
        m_io = m_source->start();
        if (!m_io || m_source->error() != QtAudio::NoError) {
            *failure = Failure::OpenFailed;
            *detail = QStringLiteral("the audio input did not start (error %1)").arg(int(m_source->error()));
            m_source.reset();
            m_io = nullptr;
            return false;
        }
        // What the backend actually buffers, once started
        m_slackFrames = m_format.framesForBytes(m_source->bufferSize()) + m_format.framesForDuration(kLeadSlackUs);
        return true;
    }
    QIODevice *device() override { return m_io; }
    QAudioFormat format() const override { return m_format; }
    qint64 producedFrames() const override {
        if (!m_source) return -1;
        return m_format.framesForDuration(m_source->processedUSecs());
    }
    qint64 leadToleranceFrames() const override { return m_slackFrames; }
    QString name() const override { return m_device.description(); }
    void close() override {
        m_closing = true;
        if (m_source) m_source->stop();
        m_source.reset();
        m_io = nullptr;
    }

private:
    // The rate asked for, else the device's own; mono if it can, else its
    // channels (channel 0 is read); float, then 32-, 16- and 8-bit PCM
    bool chooseFormat() {
        const QAudioFormat preferred = m_device.preferredFormat();
        QList<int> rates{ m_rateHz };
        if (preferred.sampleRate() > 0 && preferred.sampleRate() != m_rateHz) rates << preferred.sampleRate();
        QList<int> channels{ 1 };
        if (preferred.channelCount() > 1) channels << preferred.channelCount();
        const QAudioFormat::SampleFormat formats[] = { QAudioFormat::Float, QAudioFormat::Int32, QAudioFormat::Int16,
                                                       QAudioFormat::UInt8 };
        for (int rate : rates) {
            for (int ch : channels) {
                for (QAudioFormat::SampleFormat sf : formats) {
                    QAudioFormat f;
                    f.setSampleRate(rate);
                    f.setChannelCount(ch);
                    f.setSampleFormat(sf);
                    if (m_device.isFormatSupported(f)) {
                        m_format = f;
                        return true;
                    }
                }
            }
        }
        if (preferred.isValid() && convertible(preferred.sampleFormat())) {
            m_format = preferred;
            return true;
        }
        return false;
    }

    QAudioDevice m_device;
    int m_rateHz;
    QAudioFormat m_format;
    std::unique_ptr<QAudioSource> m_source;
    QIODevice *m_io = nullptr;
    qint64 m_slackFrames = 0;
    bool m_closing = false;
};

} // namespace

// ---- Reader ------------------------------------------------------------------

Reader::Reader(SourceFactory factory, int blockMs) : m_factory(std::move(factory)), m_blockMs(blockMs) {}

Reader::~Reader() = default;

bool Reader::convert(const char *data, qint64 frames, const QAudioFormat &format, QVector<float> *out,
                     bool *clipped) {
    const int channels = format.channelCount();
    const int bps = format.bytesPerSample();
    if (channels < 1 || bps < 1 || frames < 0 || !convertible(format.sampleFormat())) return false;
    const qint64 stride = qint64(channels) * bps;
    bool clip = false;
    out->reserve(out->size() + frames);
    for (qint64 i = 0; i < frames; ++i) {
        const char *p = data + i * stride;   // channel 0 of frame i
        float v = 0.0f;
        switch (format.sampleFormat()) {
        case QAudioFormat::UInt8: {
            const quint8 x = static_cast<quint8>(*p);
            clip = clip || x == 0 || x == 255;
            v = (float(x) - 128.0f) / 128.0f;
            break;
        }
        case QAudioFormat::Int16: {
            qint16 x;
            std::memcpy(&x, p, sizeof x);
            clip = clip || x == 32767 || x == -32768;
            v = float(x) / 32768.0f;
            break;
        }
        case QAudioFormat::Int32: {
            qint32 x;
            std::memcpy(&x, p, sizeof x);
            clip = clip || x == 2147483647 || x == qint32(-2147483647 - 1);
            v = float(double(x) / 2147483648.0);
            break;
        }
        case QAudioFormat::Float: {
            float x;
            std::memcpy(&x, p, sizeof x);
            if (!std::isfinite(x)) {
                // Not a sample: the block is marked so the core stops (S4)
                clip = true;
                x = 0.0f;
            } else if (std::fabs(x) >= 1.0f) {
                clip = true;
            }
            v = x;
            break;
        }
        default:
            return false;
        }
        out->append(v);
    }
    *clipped = *clipped || clip;
    return true;
}

void Reader::open() {
    m_source = m_factory ? m_factory() : nullptr;
    Failure failure = Failure::OpenFailed;
    QString detail = QStringLiteral("no audio source");
    if (!m_source || !m_source->open(&failure, &detail)) {
        m_source.reset();
        emit failed(failure, detail);
        return;
    }
    m_format = m_source->format();
    m_bytesPerFrame = m_format.bytesPerFrame();
    if (m_bytesPerFrame <= 0 || m_format.sampleRate() <= 0 || !convertible(m_format.sampleFormat())) {
        m_source->close();
        m_source.reset();
        emit failed(Failure::FormatUnsupported, QStringLiteral("the source's format cannot be read"));
        return;
    }
    m_blockFrames = qMax(1, int(qint64(m_format.sampleRate()) * m_blockMs / 1000));
    m_source->setFailureHandler([this](Failure f, const QString &why) {
        // Not inside the source's own signal: the handler may close it
        QMetaObject::invokeMethod(
            this,
            [this, f, why] {
                if (!m_open) return;
                readAvailable();
                emitBlock();
                emit failed(f, why);
            },
            Qt::QueuedConnection);
    });
    m_open = true;
    connect(m_source->device(), &QIODevice::readyRead, this, &Reader::readAvailable);
    emit opened(m_format.sampleRate(), m_format.channelCount(), m_source->name());
    readAvailable();
}

void Reader::readAvailable() {
    if (!m_open || !m_source || !m_source->device()) return;
    QIODevice *io = m_source->device();
    // Frames the source produced that this reader never saw (beyond what the
    // source may hold back without losing anything): a jump in the frame index,
    // with the block so far sent first (a block is always contiguous). The jump
    // is the whole lead, so indices never fall behind the true ones afterwards.
    const qint64 produced = m_source->producedFrames();
    const qint64 available = io->bytesAvailable();
    if (produced >= 0) {
        const qint64 visible = m_nextFrame + (available + m_partialFrame.size()) / m_bytesPerFrame;
        if (produced - visible > m_source->leadToleranceFrames()) {
            emitBlock();
            m_partialFrame.clear();
            m_nextFrame = produced - available / m_bytesPerFrame;
        }
    }
    QByteArray data = io->readAll();
    if (data.isEmpty()) return;
    if (!m_partialFrame.isEmpty()) {
        data.prepend(m_partialFrame);
        m_partialFrame.clear();
    }
    const qint64 frames = data.size() / m_bytesPerFrame;
    const qint64 rest = data.size() - frames * m_bytesPerFrame;
    if (rest > 0) m_partialFrame = data.right(rest);
    qint64 done = 0;
    while (done < frames) {
        if (m_block.samples.isEmpty()) {
            m_block.firstFrame = m_nextFrame;
            m_block.rateHz = m_format.sampleRate();
            m_block.clipped = false;
            m_block.sourceLeadFrames = produced >= 0 ? produced - m_nextFrame : -1;
        }
        const qint64 n = qMin<qint64>(frames - done, m_blockFrames - m_block.samples.size());
        if (!convert(data.constData() + done * m_bytesPerFrame, n, m_format, &m_block.samples, &m_block.clipped)) {
            m_open = false;
            emit failed(Failure::FormatUnsupported, QStringLiteral("the source's format cannot be read"));
            return;
        }
        done += n;
        m_nextFrame += n;
        if (m_block.samples.size() >= m_blockFrames) emitBlock();
    }
}

void Reader::emitBlock() {
    if (m_block.samples.isEmpty()) return;
    Block out = std::move(m_block);
    m_block = Block();
    emit block(out);
}

void Reader::close() {
    if (m_open) readAvailable();
    emitBlock();
    m_open = false;
    if (m_source) {
        m_source->setFailureHandler(nullptr);
        m_source->close();
        m_source.reset();
    }
    emit closed();
}

// ---- AudioCapture -----------------------------------------------------------

AudioCapture::AudioCapture(QObject *parent) : QObject(parent), m_devices(new QMediaDevices(this)) {
    qRegisterMetaType<dspi_capture::Block>();
    qRegisterMetaType<dspi_capture::Failure>();
    connect(m_devices, &QMediaDevices::audioInputsChanged, this, &AudioCapture::onInputsChanged);
}

AudioCapture::~AudioCapture() {
    stop();
}

QList<InputDevice> AudioCapture::inputs() {
    QList<InputDevice> out;
    const QList<QAudioDevice> list = QMediaDevices::audioInputs();
    for (const QAudioDevice &d : list) {
        InputDevice in{ d.id(), d.description(), d.isDefault() };
        if (in.isDefault) out.prepend(in);
        else out.append(in);
    }
    return out;
}

bool AudioCapture::start(const QByteArray &deviceId, int rateHz) {
    if (m_thread) return false;
    QAudioDevice device = findInput(deviceId);
    const QByteArray id = device.isNull() ? deviceId : device.id();
    const int rate = rateHz > 0 ? rateHz : kPreferredRateHz;
    // The QAudioDevice is a value; the source is made on the reader thread
    return startWith([device, rate]() -> std::unique_ptr<Source> { return std::make_unique<QtInput>(device, rate); },
                     id);
}

bool AudioCapture::startWith(SourceFactory factory, const QByteArray &deviceId) {
    if (m_thread) return false;
    m_deviceId = deviceId;
    m_deviceName.clear();
    m_rateHz = 0;
    m_stopping = false;
    m_thread = new QThread(this);
    m_thread->setObjectName(QStringLiteral("audio-capture"));
    m_reader = new Reader(std::move(factory), kBlockMs);
    m_reader->moveToThread(m_thread);
    connect(m_thread, &QThread::started, m_reader, &Reader::open);
    connect(m_reader, &Reader::opened, this, &AudioCapture::onReaderOpened);
    connect(m_reader, &Reader::block, this, &AudioCapture::block);
    connect(m_reader, &Reader::failed, this, &AudioCapture::onReaderFailed);
    m_thread->start();
    return true;
}

void AudioCapture::onReaderOpened(int rateHz, int channels, const QString &name) {
    if (!m_thread) return;
    m_rateHz = rateHz;
    m_deviceName = name;
    emit started(rateHz, channels, name);
}

void AudioCapture::onReaderFailed(Failure failure, const QString &detail) {
    if (!m_thread || m_stopping) return;
    emit failed(failure, detail);
    finish();
}

void AudioCapture::onInputsChanged() {
    emit inputsChanged();
    if (!m_thread || m_stopping || m_deviceId.isEmpty()) return;
    const QList<QAudioDevice> list = QMediaDevices::audioInputs();
    for (const QAudioDevice &d : list) {
        if (d.id() == m_deviceId) return;
    }
    const QByteArray lost = m_deviceId;
    emit deviceLost(lost);
    if (!m_thread || m_stopping) return;   // a listener stopped it
    emit failed(Failure::DeviceLost, QStringLiteral("the audio input left the list"));
    finish();
}

void AudioCapture::stop() {
    if (!m_thread || m_stopping) return;
    finish();
}

void AudioCapture::finish() {
    m_stopping = true;
    QThread *thread = m_thread;
    Reader *reader = m_reader;
    if (thread->isRunning()) {
        // The last blocks are queued to this thread before close() returns
        QMetaObject::invokeMethod(reader, &Reader::close, Qt::BlockingQueuedConnection);
        thread->quit();
        thread->wait();
    }
    delete reader;
    delete thread;
    m_reader = nullptr;
    m_thread = nullptr;
    m_rateHz = 0;
    m_deviceName.clear();
    m_stopping = false;
    // After the blocks already queued
    QMetaObject::invokeMethod(this, &AudioCapture::stopped, Qt::QueuedConnection);
}
