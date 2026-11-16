// -- WP P5QCAP -- The microphone reader (src/platform/AudioCapture), fed from WAV
// files through its test seam (AudioCapture::startWith) instead of an audio
// device: mono f32 blocks of 20 ms with their first frame, the rate, the clip
// flag; channel 0 of a stereo file; a dropped stretch is a jump in the frame
// index and never filled in; a source that fails mid-stream ends the capture
// with DeviceLost; stop() delivers the last partial block; the reading happens
// off the GUI thread. Then the development switches --audio-inputs and
// --measure-spike, as subprocesses of this binary. Nothing is played: no audio
// device is opened by the WAV checks, and the spike captures only.
#include "SmokeDriver.h"

#include "AudioCapture.h"
#include "MeasureSpike.h"

#include <QCoreApplication>
#include <QDataStream>
#include <QFile>
#include <QIODevice>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QThread>
#include <QTimer>
#include <QtEndian>
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstring>
#include <memory>
#include <vector>

namespace {

using dspi_capture::Block;
using dspi_capture::Failure;

// ---- WAV files ----------------------------------------------------------------

struct Wav {
    int rate = 0;
    int channels = 0;
    QAudioFormat::SampleFormat format = QAudioFormat::Unknown;
    QByteArray data;   // interleaved frames
    qint64 frames() const {
        const int bpf = channels * (format == QAudioFormat::Int16 ? 2 : 4);
        return bpf ? data.size() / bpf : 0;
    }
};

void put16(QByteArray &b, quint16 v) { b.append(char(v & 0xff)).append(char(v >> 8)); }
void put32(QByteArray &b, quint32 v) {
    for (int i = 0; i < 4; ++i) b.append(char((v >> (8 * i)) & 0xff));
}

// A canonical RIFF/WAVE file: PCM 16-bit (format 1) or IEEE float 32-bit (3)
bool writeWav(const QString &path, int rate, int channels, bool isFloat, const QByteArray &frames) {
    const int bits = isFloat ? 32 : 16;
    QByteArray b("RIFF");
    put32(b, quint32(36 + frames.size()));
    b.append("WAVEfmt ");
    put32(b, 16);
    put16(b, isFloat ? 3 : 1);
    put16(b, quint16(channels));
    put32(b, quint32(rate));
    put32(b, quint32(rate * channels * bits / 8));
    put16(b, quint16(channels * bits / 8));
    put16(b, quint16(bits));
    b.append("data");
    put32(b, quint32(frames.size()));
    b.append(frames);
    QFile f(path);
    return f.open(QIODevice::WriteOnly) && f.write(b) == b.size();
}

quint32 le32(const char *p) {
    quint32 v;
    std::memcpy(&v, p, 4);
    return qFromLittleEndian(v);
}
quint16 le16(const char *p) {
    quint16 v;
    std::memcpy(&v, p, 2);
    return qFromLittleEndian(v);
}

// The reader's half of the seam: PCM 16-bit and float 32-bit, chunks walked
// with bounds checks
bool readWav(const QString &path, Wav *out) {
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) return false;
    const QByteArray b = f.readAll();
    if (b.size() < 12 || !b.startsWith("RIFF") || b.mid(8, 4) != "WAVE") return false;
    qsizetype at = 12;
    bool haveFmt = false;
    while (at + 8 <= b.size()) {
        const QByteArray id = b.mid(at, 4);
        const quint32 len = le32(b.constData() + at + 4);
        const qsizetype body = at + 8;
        if (len > quint32(b.size() - body)) return false;
        if (id == "fmt " && len >= 16) {
            const quint16 tag = le16(b.constData() + body);
            out->channels = le16(b.constData() + body + 2);
            out->rate = int(le32(b.constData() + body + 4));
            const quint16 bits = le16(b.constData() + body + 14);
            if (tag == 1 && bits == 16) out->format = QAudioFormat::Int16;
            else if (tag == 3 && bits == 32) out->format = QAudioFormat::Float;
            else return false;
            haveFmt = true;
        } else if (id == "data" && haveFmt) {
            out->data = b.mid(body, len);
            return out->channels > 0 && out->rate > 0;
        }
        at = body + len + (len & 1);
    }
    return false;
}

// ---- The seam's source ----------------------------------------------------------

// A sequential device the source appends to; readyRead() on each append
class PushDevice : public QIODevice {
public:
    PushDevice() { open(QIODevice::ReadOnly); }
    void push(const QByteArray &bytes) {
        m_buf.append(bytes);
        emit readyRead();
    }
    bool isSequential() const override { return true; }
    qint64 bytesAvailable() const override { return m_buf.size() + QIODevice::bytesAvailable(); }

protected:
    qint64 readData(char *data, qint64 max) override {
        const qint64 n = qMin<qint64>(max, m_buf.size());
        std::memcpy(data, m_buf.constData(), size_t(n));
        m_buf.remove(0, n);
        return n;
    }
    qint64 writeData(const char *, qint64) override { return -1; }

private:
    QByteArray m_buf;
};

struct Faults {
    int dropChunk = -1;   // this chunk's frames are counted as produced, never delivered
    int failChunk = -1;   // the source fails (DeviceLost) instead of delivering this chunk
    int intervalMs = 1;   // between chunks (10 ms of audio each)
};

// What the test reads back from the source after the run (written on the
// reader thread, read after stop())
struct Probe {
    std::atomic<QThread *> openThread{ nullptr };
    std::atomic<QThread *> pushThread{ nullptr };
    std::atomic<int> chunks{ 0 };
};

class WavSource final : public dspi_capture::Source {
public:
    WavSource(Wav wav, int chunkFrames, Faults faults, Probe *probe)
        : m_wav(std::move(wav)), m_chunkFrames(chunkFrames), m_faults(faults), m_probe(probe) {}

    bool open(Failure *failure, QString *detail) override {
        m_probe->openThread = QThread::currentThread();
        m_format.setSampleRate(m_wav.rate);
        m_format.setChannelCount(m_wav.channels);
        m_format.setSampleFormat(m_wav.format);
        if (!m_format.isValid() || m_wav.frames() == 0) {
            *failure = Failure::FormatUnsupported;
            *detail = QStringLiteral("empty WAV");
            return false;
        }
        m_device = std::make_unique<PushDevice>();
        m_timer = std::make_unique<QTimer>();
        m_timer->setInterval(m_faults.intervalMs);
        QObject::connect(m_timer.get(), &QTimer::timeout, m_timer.get(), [this] { next(); });
        m_timer->start();
        return true;
    }
    QIODevice *device() override { return m_device.get(); }
    QAudioFormat format() const override { return m_format; }
    qint64 producedFrames() const override { return m_produced; }
    QString name() const override { return QStringLiteral("WAV"); }
    void close() override {
        m_timer.reset();
        m_device.reset();
    }

private:
    void next() {
        const qint64 total = m_wav.frames();
        const qint64 at = qint64(m_chunk) * m_chunkFrames;
        if (at >= total) {
            m_timer->stop();
            return;
        }
        const qint64 n = qMin<qint64>(m_chunkFrames, total - at);
        const int bpf = m_format.bytesPerFrame();
        const int chunk = m_chunk++;
        m_probe->pushThread = QThread::currentThread();
        m_probe->chunks = m_chunk;
        if (chunk == m_faults.failChunk) {
            m_timer->stop();
            fail(Failure::DeviceLost, QStringLiteral("the WAV source failed"));
            return;
        }
        m_produced += n;
        if (chunk == m_faults.dropChunk) return;   // produced, lost before the reader
        m_device->push(m_wav.data.mid(at * bpf, n * bpf));
    }

    Wav m_wav;
    int m_chunkFrames;
    Faults m_faults;
    Probe *m_probe;
    QAudioFormat m_format;
    std::unique_ptr<PushDevice> m_device;
    std::unique_ptr<QTimer> m_timer;
    qint64 m_produced = 0;
    int m_chunk = 0;
};

// ---- A run --------------------------------------------------------------------

struct Run {
    std::vector<Block> blocks;
    int startedRate = 0;
    int startedChannels = 0;
    int stoppedCount = 0;
    Failure failure = Failure::None;
    QThread *openThread = nullptr;
    QThread *pushThread = nullptr;
    int chunks = 0;
    bool timedOut = false;
};

// Feeds `wav` through the reader; stops after `stopAfterFrames` delivered (0:
// when the file is done) or when the source fails
Run feed(SmokeDriver &d, const Wav &wav, Faults faults, qint64 stopAfterFrames = 0) {
    Run r;
    Probe probe;
    AudioCapture capture;
    QObject::connect(&capture, &AudioCapture::started, [&](int rate, int ch, const QString &) {
        r.startedRate = rate;
        r.startedChannels = ch;
    });
    QObject::connect(&capture, &AudioCapture::block, [&](const Block &b) { r.blocks.push_back(b); });
    QObject::connect(&capture, &AudioCapture::failed, [&](Failure f, const QString &) { r.failure = f; });
    QObject::connect(&capture, &AudioCapture::stopped, [&] { ++r.stoppedCount; });
    const int chunkFrames = wav.rate / 100;   // 10 ms pushes, half a block
    capture.startWith([wav, chunkFrames, faults, &probe]() -> std::unique_ptr<dspi_capture::Source> {
        return std::make_unique<WavSource>(wav, chunkFrames, faults, &probe);
    });
    const qint64 total = wav.frames();
    auto delivered = [&] {
        qint64 n = 0;
        for (const Block &b : r.blocks) n += b.samples.size();
        return n;
    };
    const qint64 lastChunks = (total + chunkFrames - 1) / chunkFrames;
    const bool done = d.waitFor(20000, [&] {
        if (!capture.isRunning()) return true;
        if (stopAfterFrames > 0) return delivered() >= stopAfterFrames;
        return probe.chunks.load() >= lastChunks;
    });
    r.timedOut = !done;
    capture.stop();
    d.waitFor(2000, [&] { return r.stoppedCount > 0; });
    d.spin(50);   // a second stopped() would arrive here
    r.openThread = probe.openThread.load();
    r.pushThread = probe.pushThread.load();
    r.chunks = probe.chunks.load();
    return r;
}

// Each block starts where the one before ended, except at the gaps listed
// (frame index of the first frame after a gap, and its size)
bool contiguous(const Run &r, const std::vector<std::pair<qint64, qint64>> &gaps = {}) {
    qint64 next = 0;
    size_t g = 0;
    for (const Block &b : r.blocks) {
        if (b.firstFrame != next) {
            if (g >= gaps.size() || b.firstFrame != gaps[g].first || b.firstFrame - next != gaps[g].second) return false;
            ++g;
        }
        next = b.firstFrame + b.samples.size();
    }
    return g == gaps.size();
}

// Every delivered sample is channel 0 of the file's frame with the same index
bool samplesMatch(const Run &r, const std::vector<float> &want) {
    for (const Block &b : r.blocks) {
        for (int i = 0; i < b.samples.size(); ++i) {
            const qint64 f = b.firstFrame + i;
            if (f < 0 || f >= qint64(want.size()) || b.samples[i] != want[size_t(f)]) return false;
        }
    }
    return true;
}

qint64 deliveredFrames(const Run &r) {
    qint64 n = 0;
    for (const Block &b : r.blocks) n += b.samples.size();
    return n;
}

// ---- The section ----------------------------------------------------------------

void captureSection(SmokeDriver &d) {
    const QString dir = d.outDir();
    QThread *gui = QThread::currentThread();

    // A: 48 kHz float mono, 1 s; one full-scale sample at frame 30000
    const int rateA = 48000;
    std::vector<float> a(static_cast<size_t>(rateA));
    for (size_t i = 0; i < a.size(); ++i) a[i] = float(0.5 * std::sin(2.0 * M_PI * 997.0 * double(i) / rateA));
    a[30000] = 1.0f;
    QByteArray bytesA(reinterpret_cast<const char *>(a.data()), qsizetype(a.size() * sizeof(float)));
    const QString pathA = dir + QStringLiteral("/p5qcap-48k-float-mono.wav");
    Wav wavA;
    d.check(QStringLiteral("p5qcap: a float WAV is written and read back"),
            writeWav(pathA, rateA, 1, true, bytesA) && readWav(pathA, &wavA) && wavA.frames() == rateA &&
                wavA.format == QAudioFormat::Float);

    Run r = feed(d, wavA, {});
    d.check(QStringLiteral("p5qcap: the capture opens at the source's rate, mono"),
            r.startedRate == rateA && r.startedChannels == 1);
    d.check(QStringLiteral("p5qcap: every frame comes through, in order, from frame 0"),
            !r.timedOut && deliveredFrames(r) == rateA && contiguous(r) && !r.blocks.empty() &&
                r.blocks.front().firstFrame == 0);
    bool sizes = true;
    for (size_t i = 0; i + 1 < r.blocks.size(); ++i) sizes = sizes && r.blocks[i].samples.size() == rateA / 50;
    d.check(QStringLiteral("p5qcap: blocks are 20 ms (960 frames at 48 kHz) and carry the rate"),
            sizes && std::all_of(r.blocks.begin(), r.blocks.end(), [&](const Block &b) { return b.rateHz == rateA; }));
    d.check(QStringLiteral("p5qcap: float samples arrive unchanged"), samplesMatch(r, a));
    bool clipOk = true;
    for (const Block &b : r.blocks) {
        const bool has = b.firstFrame <= 30000 && 30000 < b.firstFrame + b.samples.size();
        clipOk = clipOk && b.clipped == has;
    }
    d.check(QStringLiteral("p5qcap: only the block with the full-scale sample is flagged clipped"), clipOk);
    d.check(QStringLiteral("p5qcap: the source is opened and read off the GUI thread"),
            r.openThread && r.pushThread && r.openThread != gui && r.pushThread == r.openThread);
    d.check(QStringLiteral("p5qcap: stop() ends with one stopped() and no failure"),
            r.stoppedCount == 1 && r.failure == Failure::None);

    // B: 44.1 kHz 16-bit stereo: channel 0 is read; -32768 is a clip
    const int rateB = 44100;
    QByteArray bytesB;
    std::vector<float> b0(static_cast<size_t>(rateB / 2));
    for (size_t i = 0; i < b0.size(); ++i) {
        qint16 left = qint16(int(i * 37 % 60000) - 30000);
        if (i == 7000) left = -32768;
        const qint16 right = qint16(12345);
        b0[i] = float(left) / 32768.0f;
        put16(bytesB, quint16(left));
        put16(bytesB, quint16(right));
    }
    const QString pathB = dir + QStringLiteral("/p5qcap-44k1-pcm16-stereo.wav");
    Wav wavB;
    d.check(QStringLiteral("p5qcap: a 16-bit stereo WAV is written and read back"),
            writeWav(pathB, rateB, 2, false, bytesB) && readWav(pathB, &wavB) && wavB.channels == 2 &&
                wavB.frames() == qint64(b0.size()));
    r = feed(d, wavB, {});
    d.check(QStringLiteral("p5qcap: 16-bit stereo: channel 0 as f32, every frame, 882-frame blocks"),
            r.startedRate == rateB && r.startedChannels == 2 && deliveredFrames(r) == qint64(b0.size()) &&
                contiguous(r) && samplesMatch(r, b0) && r.blocks.front().samples.size() == rateB / 50);
    bool clipB = false;
    for (const Block &blk : r.blocks) clipB = clipB || (blk.clipped && blk.firstFrame <= 7000 && 7000 < blk.firstFrame + blk.samples.size());
    d.check(QStringLiteral("p5qcap: a 16-bit sample at -32768 flags its block"), clipB);

    // A dropped stretch (chunk 10: frames 4800 to 5279) is a jump in the index
    r = feed(d, wavA, Faults{ 10, -1 });
    d.check(QStringLiteral("p5qcap: a dropped stretch is one jump of its size in firstFrame, never filled"),
            !r.timedOut && contiguous(r, { { 5280, 480 } }) && deliveredFrames(r) == rateA - 480 &&
                samplesMatch(r, a));

    // The source fails mid-stream (device unplugged): DeviceLost, then stopped
    r = feed(d, wavA, Faults{ -1, 20 });
    d.check(QStringLiteral("p5qcap: a failing source ends the capture with DeviceLost and one stopped()"),
            r.failure == Failure::DeviceLost && r.stoppedCount == 1);
    d.check(QStringLiteral("p5qcap: what came before the failure is delivered, contiguous"),
            contiguous(r) && deliveredFrames(r) == 20 * 480 && samplesMatch(r, a));

    // stop() mid-stream delivers the partial block and stops
    r = feed(d, wavA, Faults{ -1, -1, 10 }, 10000);
    d.check(QStringLiteral("p5qcap: stop() mid-stream: contiguous blocks, one stopped()"),
            contiguous(r) && deliveredFrames(r) >= 10000 && deliveredFrames(r) < rateA && r.stoppedCount == 1 &&
                r.failure == Failure::None && samplesMatch(r, a));

    // The conversion, format by format
    {
        QAudioFormat f;
        f.setSampleRate(48000);
        f.setChannelCount(1);
        QVector<float> out;
        bool clipped = false;
        f.setSampleFormat(QAudioFormat::UInt8);
        const char u8[] = { char(0), char(128), char(192) };
        const bool okU8 = dspi_capture::Reader::convert(u8, 3, f, &out, &clipped);
        d.check(QStringLiteral("p5qcap: 8-bit: 0 is -1 and a clip, 128 is 0, 192 is 0.5"),
                okU8 && clipped && out == QVector<float>{ -1.0f, 0.0f, 0.5f });
        out.clear();
        clipped = false;
        f.setSampleFormat(QAudioFormat::Int32);
        const qint32 i32[] = { 1073741824, 2147483647 };
        const bool okI32 = dspi_capture::Reader::convert(reinterpret_cast<const char *>(i32), 2, f, &out, &clipped);
        d.check(QStringLiteral("p5qcap: 32-bit: half scale is 0.5, the maximum is a clip"),
                okI32 && clipped && out.size() == 2 && out[0] == 0.5f);
        out.clear();
        clipped = false;
        f.setSampleFormat(QAudioFormat::Float);
        const float fl[] = { 0.25f, std::nanf(""), -0.5f };
        const bool okF = dspi_capture::Reader::convert(reinterpret_cast<const char *>(fl), 3, f, &out, &clipped);
        d.check(QStringLiteral("p5qcap: float: a NaN becomes 0 and flags the block"),
                okF && clipped && out == QVector<float>{ 0.25f, 0.0f, -0.5f });
        out.clear();
        clipped = false;
        f.setSampleFormat(QAudioFormat::Unknown);
        d.check(QStringLiteral("p5qcap: an unknown format is refused, nothing appended"),
                !dspi_capture::Reader::convert(u8, 3, f, &out, &clipped) && out.isEmpty());
    }

    // The input list (whatever this machine has: offscreen CI usually none)
    const QList<dspi_capture::InputDevice> inputs = AudioCapture::inputs();
    d.check(QStringLiteral("p5qcap: the input list puts the default first"),
            inputs.isEmpty() || inputs.front().isDefault ||
                std::none_of(inputs.begin(), inputs.end(), [](const dspi_capture::InputDevice &i) { return i.isDefault; }));

    // The development switches, as this binary's own launches
    const QString exe = QCoreApplication::applicationFilePath();
    const QString inputsPath = dir + QStringLiteral("/p5qcap-audio-inputs.json");
    QProcess p;
    p.setProcessChannelMode(QProcess::ForwardedChannels);
    p.start(exe, { QStringLiteral("--audio-inputs"), inputsPath });
    const bool listed = p.waitForFinished(30000) && p.exitStatus() == QProcess::NormalExit && p.exitCode() == 0;
    QFile fi(inputsPath);
    const QJsonObject inputsDoc = fi.open(QIODevice::ReadOnly) ? QJsonDocument::fromJson(fi.readAll()).object() : QJsonObject();
    d.check(QStringLiteral("p5qcap: --audio-inputs writes the list and exits 0"),
            listed && inputsDoc.value(QStringLiteral("kind")).toString() == QLatin1String("dspi-audio-inputs") &&
                inputsDoc.value(QStringLiteral("inputs")).isArray());

    // -- Review P5-B (decision a) -- The spike runs the core's session. As a
    // launch it measures the demo DSPi only (never a DSPi on USB from a test);
    // this machine's own input, if it has one, is all it captures.
    const QString spikePath = dir + QStringLiteral("/p5qcap-spike.json");
    QProcess s;
    s.setProcessChannelMode(QProcess::ForwardedChannels);
    s.start(exe, { QStringLiteral("--measure-spike"), spikePath, QStringLiteral("--measure-spike-demo"),
                   QStringLiteral("--measure-spike-seconds"), QStringLiteral("1") });
    const bool ran = s.waitForFinished(30000) && s.exitStatus() == QProcess::NormalExit &&
                     (s.exitCode() == 0 || s.exitCode() == 1);
    QFile fs(spikePath);
    const QJsonObject spike = fs.open(QIODevice::ReadOnly) ? QJsonDocument::fromJson(fs.readAll()).object() : QJsonObject();
    d.check(QStringLiteral("p5qcap: --measure-spike writes its report: the demo DSPi, start level -60 dBFS"),
            ran && spike.value(QStringLiteral("kind")).toString() == QLatin1String("dspi-measure-spike") &&
                spike.value(QStringLiteral("startLevelDb")).toDouble() == -60.0 &&
                spike.value(QStringLiteral("device")).toString() == QLatin1String("demo") &&
                spike.contains(QStringLiteral("session")) && spike.value(QStringLiteral("gaps")).isArray());
    QProcess bad;
    bad.setProcessChannelMode(QProcess::ForwardedChannels);
    bad.start(exe, { QStringLiteral("--measure-spike"), spikePath, QStringLiteral("--measure-spike-demo"),
                     QStringLiteral("--measure-spike-seconds"), QStringLiteral("600") });
    d.check(QStringLiteral("p5qcap: --measure-spike refuses more than 300 s (exit 2)"),
            bad.waitForFinished(30000) && bad.exitCode() == 2);

    // The spike end to end, in this process: the demo DSPi, the microphone a
    // WAV of room noise through the capture's seam at the pace of a real one
    // (10 ms pushes every 10 ms). The noise holds no sweep, so the run ends
    // without timing; what is shown is the whole path: preflight, a start at
    // the start level only, every block pushed with its frame index, the
    // ramp steps and the run played by the generator at -60 dBFS, the
    // analysis, a verdict, the generator idle at the end.
    const int rateN = 48000;
    std::vector<float> noise(static_cast<size_t>(rateN) * 40);
    quint32 seed = 0x2545F491u;
    for (float &v : noise) {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        v = (float(seed >> 8) / float(1u << 24) * 2.0f - 1.0f) * 1.0e-4f;   // about -85 dBFS
    }
    const QString pathN = dir + QStringLiteral("/p5b-noise.wav");
    QByteArray bytesN(reinterpret_cast<const char *>(noise.data()), qsizetype(noise.size() * sizeof(float)));
    Wav wavN;
    const bool wavOk = writeWav(pathN, rateN, 1, true, bytesN) && readWav(pathN, &wavN);
    const QString setupN = QStringLiteral(
        "{\"kind\":\"walked\",\"outputs\":[{\"output\":0,\"role\":\"full\"},{\"output\":1,\"role\":\"full\"}],"
        "\"placement\":\"headphones\",\"coupler\":\"flatPlate\",\"passes\":2,\"sweepMs\":1000,\"gapMs\":300,"
        "\"ceilingDb\":-20}");
    const auto spikeRun = [&](const QString &path, int seconds) {
        const auto probe = std::make_shared<Probe>();   // kept by the factory, so it outlives the reader
        Faults pace;
        pace.intervalMs = 10;
        MeasureSpike::Options o;
        o.path = path;
        o.seconds = seconds;
        o.demo = true;
        o.setupJson = setupN;
        o.source = [wavN, pace, probe]() -> std::unique_ptr<dspi_capture::Source> {
            return std::make_unique<WavSource>(wavN, wavN.rate / 100, pace, probe.get());
        };
        const int code = MeasureSpike::run(o);
        QFile f(path);
        QJsonObject doc = f.open(QIODevice::ReadOnly) ? QJsonDocument::fromJson(f.readAll()).object() : QJsonObject();
        doc.insert(QStringLiteral("exitCode"), code);
        return doc;
    };
    const auto phaseSeen = [](const QJsonObject &doc, const char *phase) {
        const QJsonArray phases = doc.value(QStringLiteral("phases")).toArray();
        return std::any_of(phases.begin(), phases.end(), [phase](const QJsonValue &p) {
            return p.toObject().value(QStringLiteral("phase")).toString() == QLatin1String(phase);
        });
    };
    const QJsonObject full = wavOk ? spikeRun(dir + QStringLiteral("/p5b-spike-run.json"), 60) : QJsonObject();
    const QJsonObject stFull = full.value(QStringLiteral("state")).toObject();
    const QString phaseFull = stFull.value(QStringLiteral("phase")).toString();
    const QString keyFull = stFull.value(QStringLiteral("issue")).toObject().value(QStringLiteral("key")).toString();
    d.check(QStringLiteral("p5b: the spike's preflight on the demo DSPi is clear, and it forces the start level only"),
            full.value(QStringLiteral("preflight")).toObject().value(QStringLiteral("clear")).toBool() &&
                full.value(QStringLiteral("setup")).toObject().value(QStringLiteral("startLevelOnly")).toBool() &&
                full.value(QStringLiteral("setup")).toObject().value(QStringLiteral("captureRateHz")).toInt() == rateN);
    d.check(QStringLiteral("p5b: the session ran its stages on the pushed capture: pre-roll, ramp, run, analysis"),
            phaseSeen(full, "preRoll") && phaseSeen(full, "ramp") && phaseSeen(full, "run") &&
                phaseSeen(full, "analysing"));
    d.check(QStringLiteral("p5b: the generator played a step per output and the run, never above -60 dBFS"),
            stFull.value(QStringLiteral("generatorStarts")).toInt() == 3 &&
                full.value(QStringLiteral("maxLevelDb")).toDouble() == -60.0);
    d.check(QStringLiteral("p5b: every block reached the session in order (no frame fault, no refusal)"),
            stFull.value(QStringLiteral("frameFaults")).toInt() == 0 &&
                full.value(QStringLiteral("pushRefusals")).toInt() == 0 && full.value(QStringLiteral("gaps")).toArray().isEmpty() &&
                stFull.value(QStringLiteral("frames")).toDouble() > 5.0 * rateN);
    // Noise holds no sweep: the session says so (a simulated microphone that
    // answers the generator is P5DEMO's; the timing fields with one are shown
    // by the core's own test wp_p5b_device_a_start_level_only_run_...)
    d.check(QStringLiteral("p5b: the run ends with a verdict in the report, the generator idle"),
            (phaseFull == QLatin1String("failed") || phaseFull == QLatin1String("aborted")) &&
                (keyFull == QLatin1String("measure.no_signal") || keyFull == QLatin1String("measure.abort.cycle_identity")) &&
                full.value(QStringLiteral("verdict")).toString().startsWith(QLatin1String("no timing")) &&
                full.value(QStringLiteral("generatorAfter")).toObject().value(QStringLiteral("state")).toInt(-1) == 0 &&
                full.value(QStringLiteral("exitCode")).toInt() == 1);
    if (phaseFull.isEmpty() || keyFull.isEmpty())
        qWarning("p5b: spike report: %s", QJsonDocument(full).toJson(QJsonDocument::Compact).constData());

    // Stop while the run plays (the time limit, as Ctrl+C): one
    // dspi_measure_stop; the generator was stopped by the host and is idle.
    const QJsonObject cut = wavOk ? spikeRun(dir + QStringLiteral("/p5b-spike-stop.json"), 7) : QJsonObject();
    const QJsonObject stCut = cut.value(QStringLiteral("state")).toObject();
    const QJsonObject genCut = cut.value(QStringLiteral("generatorAfter")).toObject();
    d.check(QStringLiteral("p5b: a stop while the run plays: stopped, the generator idle by the host's stop"),
            cut.value(QStringLiteral("stoppedBy")).toString() == QLatin1String("time-limit") && phaseSeen(cut, "run") &&
                stCut.value(QStringLiteral("phase")).toString() == QLatin1String("aborted") &&
                stCut.value(QStringLiteral("issue")).toObject().value(QStringLiteral("key")).toString() ==
                    QLatin1String("measure.stopped") &&
                genCut.value(QStringLiteral("state")).toInt(-1) == 0 && genCut.value(QStringLiteral("stopReason")).toInt() == 1);
    if (stCut.isEmpty()) qWarning("p5b: stop report: %s", QJsonDocument(cut).toJson(QJsonDocument::Compact).constData());
}

const SmokeDriver::Section capture("p5qcap-capture", 205, &captureSection);

} // namespace
