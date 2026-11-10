#ifndef HOSTAUDIOFORMAT_H
#define HOSTAUDIOFORMAT_H

#include <QByteArray>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QThreadPool>
#include <QTimer>
#include <atomic>
#include <functional>
#include <memory>
#include <vector>

extern "C" {
#include "dspi_core.h"
}

class DSPiBridge;

// How many channels this host is set up to send a DSPi over USB, read from the
// host's audio system. The device's own report collapses to stereo when the host
// idles the stream (USB alt 0), so the input strips would too; the host keeps its
// choice while idle (HostAudioFormatMonitor.swift on the Mac).
namespace HostAudio {

// One output of the host's audio system, as a backend sees it
struct Endpoint {
    QString text;           // names, searched for "DSPi"
    QString serialText;     // text that holds the USB serial, where the OS gives one
    int channels = 0;       // 0: not known
};

bool isDspi(const Endpoint &endpoint);

// The channel count for the unit with this serial, 0 for none: the caller falls
// back to the device's report. The endpoint holding the serial wins. Failing that,
// the DSPi endpoints have to agree: with two units set differently and no serial
// to tell them by, another unit's format must not lay out this one's strips.
int resolve(const std::vector<Endpoint> &endpoints, const QString &serial);

// Input strips to show, as DSPViewModel.effectiveInputChannels: only USB follows
// the host (hostChannels 0: the device's report).
int effectiveInputCount(int inputSource, int i2sChannels, int activeInputs, int hostChannels, int deviceInputs);

// A Link session's phase has a hub's device behind it, not one of this host's
bool phaseHasHub(const QString &phase);

// -- WP UPW2 -- The USB serial in a chain of Windows device instance IDs, from an
// audio endpoint up through its parents: the DSPi's own USB device is
// USB\VID_2E8B&PID_FEAA\<serial> (its interfaces add &MI_xx). "" when none is there,
// or when Windows made the last part up because the serial would not do (it then
// holds '&').
QString usbSerialFromInstanceIds(const QStringList &chain);

// nChannels of a WAVEFORMATEX blob (Windows' PKEY_AudioEngine_DeviceFormat), 0 for
// one too short to hold it
int waveFormatChannels(const char *blob, qint64 size);

// The sinks in `pw-dump` output. ok is false if it isn't a dump at all.
std::vector<Endpoint> parsePipeWireDump(const QByteArray &dump, bool *ok);
// The USB cards under /proc/asound. ALSA has no format of its own while idle: a
// card shows a channel count only while something holds its stream open.
std::vector<Endpoint> readAlsa(const QString &procRoot, const QString &sysClassSound);

class Backend {
public:
    virtual ~Backend() = default;
    // On the controller's worker; may block. Never opens a stream or plays anything.
    virtual std::vector<Endpoint> read() = 0;
    // On the UI thread. `changed` may be called on any thread until stopWatching
    // returns, and only asks for another read().
    virtual void startWatching(std::function<void()> changed) = 0;
    virtual void stopWatching() = 0;
};

// This OS's backend: WASAPI on Windows, PipeWire then ALSA on Linux, none elsewhere.
std::unique_ptr<Backend> makeBackend();

#ifdef Q_OS_LINUX
struct LinuxPaths {
    QString pwDump = QStringLiteral("pw-dump");
    QString procAsound = QStringLiteral("/proc/asound");
    QString sysClassSound = QStringLiteral("/sys/class/sound");
    int timeoutMs = 2000;
};
std::unique_ptr<Backend> makeLinuxBackend(const LinuxPaths &paths);
#endif

} // namespace HostAudio

class HostAudioFormatController : public QObject
{
    Q_OBJECT
    // What the host says, 0 when it says nothing
    Q_PROPERTY(int hostChannels READ hostChannels NOTIFY countChanged)
    // How many input strips to show. Changes only when the number does.
    Q_PROPERTY(int inputCount READ inputCount NOTIFY countChanged)

public:
    explicit HostAudioFormatController(DSPiBridge *bridge, QObject *parent = nullptr);
    ~HostAudioFormatController() override;

    int hostChannels() const { return m_host; }
    int inputCount() const { return m_count; }

    // For the smoke driver. The demo device has no host side, so a backend is only
    // used on it when asked.
    void setBackendForTest(std::unique_ptr<HostAudio::Backend> backend, bool useOnDemo);

signals:
    void countChanged();

private:
    void sync();
    void setWatching(bool on);
    void requestRead();
    void startRead();
    void finishRead(int channels, bool hub, unsigned generation);
    void updateCount();

    DSPiBridge *m_bridge;
    FfiCore *m_core;
    std::unique_ptr<HostAudio::Backend> m_backend;
    QString m_serial;
    bool m_useOnDemo = false;
    bool m_hostSide = false;    // the device open could be one of this host's
    bool m_watching = false;
    bool m_reading = false;
    bool m_readAgain = false;
    int m_active = -1;
    int m_host = 0;
    int m_count = 2;
    int m_shownHost = 0;
    QTimer m_settle;                            // one read for a burst of changes
    std::atomic<unsigned> m_generation { 0 };   // a read in flight stops counting at a change of device
    QThreadPool m_worker;                       // one thread, joined by the destructor
};

#endif
