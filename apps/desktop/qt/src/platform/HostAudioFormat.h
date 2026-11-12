#ifndef HOSTAUDIOFORMAT_H
#define HOSTAUDIOFORMAT_H

#include <QObject>
#include <QString>
#include <QStringList>
#include <QTimer>
#include <QtQml/qqmlregistration.h>
#include <functional>
#include <memory>
#include <vector>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP QP18 -- How many channels the operating system plays to the open DSPi
// (behaviour.md §18, the Mac's HostAudioFormatMonitor): the USB input strips
// follow it (ChannelListModel::effectiveInputs), because the device itself
// reports stereo while nothing plays. Windows reads the render endpoint's
// format (WASAPI) and finds the board by the USB serial above the endpoint in
// the device tree; Linux reads the ALSA card of the board's USB device while a
// stream has it open. With no answer the device's own count is used.
namespace HostAudio {

// One playback endpoint the OS offers
struct Endpoint {
    QString text;         // its names, for matching by name
    int channels = 0;     // 0: not known
    QString serialText;   // the USB serial of the device it belongs to, if found
};

// One per OS. read() blocks and runs on a pool thread; startWatching() calls
// `changed` (from any thread) when the endpoints or their formats change, on
// an OS that tells; otherwise the controller reads again now and then.
class Backend {
public:
    virtual ~Backend() = default;
    virtual std::vector<Endpoint> read() = 0;
    virtual void startWatching(std::function<void()> changed) { Q_UNUSED(changed) }
    virtual void stopWatching() {}
    // The OS tells about changes (no need to read on a timer)
    virtual bool notifies() const { return false; }
};
std::unique_ptr<Backend> makeBackend();

// A WAVEFORMATEX (or WAVEFORMATEXTENSIBLE) blob's channel count, 0 if too short
int waveFormatChannels(const char *data, qint64 size);
// The serial in a chain of Windows device instance ids: the last part of the
// first USB\VID_... id whose last part the device gave (no '&' in it)
QString usbSerialFromInstanceIds(const QStringList &chain);
// The open DSPi's channel count among `endpoints`: the one with its serial,
// else the only one whose names say DSPi; 0 for none
int channelsFor(const std::vector<Endpoint> &endpoints, const QString &serial);

} // namespace HostAudio

class HostAudioFormat : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The OS's channel count for the open DSPi, 0 when it gives none
    Q_PROPERTY(int channels READ channels NOTIFY channelsChanged)
public:
    // How often a backend without notifications is read again
    static constexpr int kRereadMs = 2000;

    HostAudioFormat(const AppContext &ctx, QObject *parent = nullptr);
    ~HostAudioFormat() override;
    static HostAudioFormat *create(QQmlEngine *, QJSEngine *);

    int channels() const { return m_channels; }
    // Tests: as if the OS had said `channels` (0: no answer); reading stops
    void setOverride(int channels);

signals:
    void channelsChanged();

private:
    void read();
    void setChannels(int channels);

    AppContext m_ctx;
    std::shared_ptr<HostAudio::Backend> m_backend;
    QTimer m_timer;
    int m_channels = 0;
    bool m_reading = false;
    bool m_overridden = false;
};

#endif // HOSTAUDIOFORMAT_H
