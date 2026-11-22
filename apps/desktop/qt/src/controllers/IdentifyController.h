#ifndef IDENTIFYCONTROLLER_H
#define IDENTIFYCONTROLLER_H

#include <QJsonObject>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class DeviceSession;
class QJSEngine;
class QQmlEngine;

// -- WP P2QT -- Device > Identify: the open DSPi blinks its LED, through the
// core's one Identify (dspi_hub_identify, P2CC1): the hub's `device.identify`
// over DSPi Link to a hub with the `identify` cap, the device's own
// EXT_IDENTIFY on a direct connection. Never a tone (the person did not ask
// for sound). The row works when the core says Identify is there: the hub's
// cap, or the device answering EXT_IDENTIFY (dspi_fetch_identify); a
// read-only connection does not identify. It is a write (DeviceModel::write);
// the outcome is a notice.
class IdentifyController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // Identify works for the open device now
    Q_PROPERTY(bool available READ available NOTIFY availableChanged)
    // An Identify is under way
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
public:
    // How long the LED blinks
    static constexpr int kSeconds = 10;

    IdentifyController(const AppContext &ctx, QObject *parent = nullptr);
    static IdentifyController *create(QQmlEngine *, QJSEngine *);

    bool available() const { return m_supported && m_ctx.device && !m_readOnly && m_connected; }
    bool busy() const { return m_busy; }

    // The LED blinks for kSeconds (0 stops it)
    Q_INVOKABLE void identify(int seconds = kSeconds);

signals:
    void availableChanged();
    void busyChanged();
    // An Identify finished: whether it went through and the core's reply
    void identified(bool ok, const QJsonObject &reply);

private:
    void onSnapshot();

    AppContext m_ctx;
    bool m_supported = false;
    bool m_readOnly = true;
    bool m_connected = false;
    bool m_busy = false;
    QString m_serial;
    bool m_remote = false;
    int m_writes = 0;
    int m_probe = 0;
    QPointer<DeviceSession> m_session;   // -- WP MDQT2b -- the device probed
};

#endif // IDENTIFYCONTROLLER_H
