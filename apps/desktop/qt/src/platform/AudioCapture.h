#ifndef AUDIOCAPTURE_H
#define AUDIOCAPTURE_H

// -- WP P5QCAP -- The microphone, for measurements (PLAN-P5 section 1.1 step 2,
// OD-P5e, OD-P5s): a QAudioSource read on a thread of its own, delivered as
// mono f32 blocks of about 20 ms, each with the index of its first frame, the
// rate and a clip flag; the core's measure_capture_push takes them as they are.
//
// Capture only. Nothing here, or anywhere in the app, opens an audio output:
// the stimulus is the DSPi's own generator (S1). cmake/wp/p5qcap.cmake fails the
// configure step if a source file names a Qt audio or media output class.
//
// Frame indices: the first block starts at frame 0 and every frame the source
// produced counts, including frames that were lost before this reader saw them
// (an overrun of the backend's buffer, detected from the source's processed
// count). A lost stretch is therefore a jump in firstFrame, which the core's
// continuity check turns into an abort (S5); nothing is ever filled in.
//
// The source is behind CaptureSource so the smoke test can feed the same reader
// from a WAV file (smoke/SmokeCapture.cpp); the app uses the QAudioSource one.

#include <QAudioFormat>
#include <QByteArray>
#include <QList>
#include <QMetaType>
#include <QObject>
#include <QString>
#include <QVector>
#include <functional>
#include <memory>

class QIODevice;
class QMediaDevices;
class QThread;

namespace dspi_capture {

// One block for the core: mono samples, full scale = 1.0
struct Block {
    qint64 firstFrame = 0;   // index of samples[0] since the capture started
    QVector<float> samples;
    int rateHz = 0;
    bool clipped = false;    // a source sample at the format's full scale (or |x| >= 1 for float)
    // Diagnostics (the spike's report): frames the source said it had produced
    // beyond those this reader had taken in when the block began; -1 unknown
    qint64 sourceLeadFrames = -1;
};

// An input the user can choose (QMediaDevices::audioInputs())
struct InputDevice {
    QByteArray id;
    QString name;
    bool isDefault = false;
};

// Why a capture ended other than by stop(). Stable codes: the screens map them to
// their own text (no user text here)
enum class Failure {
    None = 0,
    NoDevice,          // the device id is not (or no longer) in the list
    FormatUnsupported, // no format the reader can convert (PCM 8/16/32-bit or float)
    OpenFailed,        // the backend refused to start
    DeviceLost,        // the device went away or the backend stopped with an error
};

// Where the frames come from. Created, used and destroyed on the reader thread.
class Source {
public:
    virtual ~Source() = default;
    // Opens the source. On failure returns false and sets `failure` and `detail`
    // (a developer's text, not for users).
    virtual bool open(Failure *failure, QString *detail) = 0;
    // The data, readable once open() succeeded; readyRead() is emitted on the
    // reader thread
    virtual QIODevice *device() = 0;
    virtual QAudioFormat format() const = 0;
    // The frames the source has produced since open(), including any it lost;
    // -1 if it cannot tell. Used only to find gaps, never to pace.
    virtual qint64 producedFrames() const = 0;
    // How far producedFrames() may run ahead of what device() has made readable
    // without anything being lost (a backend's own buffering)
    virtual qint64 leadToleranceFrames() const { return 0; }
    virtual QString name() const = 0;
    virtual void close() = 0;
    // The reader calls this once after open(); the source calls it when it fails
    // later (device unplugged, backend error), at most once
    void setFailureHandler(std::function<void(Failure, const QString &)> handler) { m_onFailure = std::move(handler); }

protected:
    void fail(Failure failure, const QString &detail) {
        if (m_onFailure) {
            auto f = std::move(m_onFailure);
            m_onFailure = nullptr;
            f(failure, detail);
        }
    }

private:
    std::function<void(Failure, const QString &)> m_onFailure;
};

// Makes a source on the reader thread
using SourceFactory = std::function<std::unique_ptr<Source>()>;

class Reader;

} // namespace dspi_capture

Q_DECLARE_METATYPE(dspi_capture::Block)
Q_DECLARE_METATYPE(dspi_capture::Failure)

// The capture: start, stop, the input list and its changes. Lives on the GUI
// thread; reads on its own thread. Blocks come in order through block(), queued,
// so a slow GUI never holds up the reader (it only keeps more blocks queued).
class AudioCapture : public QObject {
    Q_OBJECT
public:
    // About 20 ms a block, as the web's AudioWorklet blocks (PLAN-P5 P5WCAP)
    static constexpr int kBlockMs = 20;
    // The rate asked for first (RC8); the device's own rate when it refuses it
    static constexpr int kPreferredRateHz = 48000;

    explicit AudioCapture(QObject *parent = nullptr);
    ~AudioCapture() override;

    // The inputs now (QMediaDevices::audioInputs()), the default first
    static QList<dspi_capture::InputDevice> inputs();

    // Starts reading the input `deviceId` (empty: the system's default input) at
    // `rateHz` when the device takes it. False if a capture is running already;
    // a failure to open comes as failed().
    bool start(const QByteArray &deviceId = QByteArray(), int rateHz = kPreferredRateHz);
    // The test seam: reads from any source (smoke/SmokeCapture.cpp's WAV file)
    bool startWith(dspi_capture::SourceFactory factory, const QByteArray &deviceId = QByteArray());
    // Stops, delivers the last partial block, then emits stopped(). Waits for the
    // reader thread (at most about one block's time). Safe when not running.
    void stop();

    bool isRunning() const { return m_thread != nullptr; }
    // The running capture's rate, its device's id and name (empty when not running)
    int rateHz() const { return m_rateHz; }
    QByteArray deviceId() const { return m_deviceId; }
    QString deviceName() const { return m_deviceName; }
    // The reader thread, for checks (null when not running)
    QThread *readerThread() const { return m_thread; }

signals:
    // Opened: the format the reader got (rate, channels, sample format)
    void started(int rateHz, int channels, const QString &deviceName);
    // A block, in order. firstFrame of a block is the previous block's
    // firstFrame + its size unless frames were lost in between.
    void block(const dspi_capture::Block &block);
    // The capture ended by itself: failure is never None. stopped() follows.
    void failed(dspi_capture::Failure failure, const QString &detail);
    // The device the capture reads left the input list (unplugged): failed()
    // with DeviceLost follows, and the capture stops.
    void deviceLost(const QByteArray &deviceId);
    void stopped();
    // The input list changed (a device came or went)
    void inputsChanged();

private:
    void onReaderFailed(dspi_capture::Failure failure, const QString &detail);
    void onReaderOpened(int rateHz, int channels, const QString &name);
    void onInputsChanged();
    void finish();

    QMediaDevices *m_devices = nullptr;
    QThread *m_thread = nullptr;
    dspi_capture::Reader *m_reader = nullptr;
    int m_rateHz = 0;
    QByteArray m_deviceId;
    QString m_deviceName;
    bool m_stopping = false;
};

namespace dspi_capture {

// Converts the source's interleaved frames to mono f32 (channel 0), frames
// blocks and counts frames. Lives on the reader thread. Public for the smoke
// test's unit checks of the conversion.
class Reader : public QObject {
    Q_OBJECT
public:
    Reader(SourceFactory factory, int blockMs);
    ~Reader() override;

    // Channel 0 of `frames` interleaved frames in `format`, appended to `out`
    // as f32; returns whether any sample was at full scale. Formats: UInt8,
    // Int16, Int32, Float. False and nothing appended for any other.
    static bool convert(const char *data, qint64 frames, const QAudioFormat &format, QVector<float> *out,
                        bool *clipped);

public slots:
    void open();
    // Reads what is left, sends the last (partial) block, closes the source
    void close();

signals:
    void opened(int rateHz, int channels, const QString &name);
    void block(const dspi_capture::Block &block);
    void failed(dspi_capture::Failure failure, const QString &detail);
    void closed();

private:
    void readAvailable();
    void emitBlock();

    SourceFactory m_factory;
    std::unique_ptr<Source> m_source;
    QAudioFormat m_format;
    int m_blockMs;
    int m_blockFrames = 0;
    int m_bytesPerFrame = 0;
    qint64 m_nextFrame = 0;   // index of the next frame this reader takes in
    QByteArray m_partialFrame;
    Block m_block;
    bool m_open = false;
};

} // namespace dspi_capture

#endif // AUDIOCAPTURE_H
