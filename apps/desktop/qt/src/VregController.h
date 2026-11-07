#ifndef VREGCONTROLLER_H
#define VREGCONTROLLER_H

#include <QByteArray>
#include <QObject>
#include <QString>
#include <QThreadPool>
#include <QTimer>
#include <QVariantMap>
#include <atomic>
#include <functional>
#include <utility>

extern "C" {
#include "dspi_core.h"
}

class DSPiBridge;

// WP QD28: core-voltage calibration and the manual requests, as QML sees them:
// `vreg`, for Settings > Advanced > Core Voltage only. A thin layer over the
// core's flow (CG11, dspi_core_vreg_cal_*): the limits, the OVERVOLT rule (CG14),
// the messages and the warning all come from the core's view; nothing is decided
// here.
//
// Reads: dspi_core_vreg_cal_poll, which reads the status and advances the flow,
// about once a second while `watching` (the page shows), and once on every
// connect, as CG11 asks, so a trial left running is found. Its view carries the
// status, so the page needs no second read: ExtController's (QD3) runs only for
// Overview.
//
// Writes: none goes out without two steps. request() stages one and says what it
// is (`pending`); only confirm() sends it, with what the user typed when the view
// says the word is needed. The one exception is endTrial(), the abort, which
// returns the device to its saved voltage and must work at any moment. Every call
// to the core goes through apply(), on one worker, in order; a call queued for one
// device never reaches the next.
class VregController : public QObject
{
    Q_OBJECT
    // The core's VregCalView (camelCase, as dspi_core_vreg_cal_view gives it)
    Q_PROPERTY(QVariantMap view READ view NOTIFY viewChanged)
    // The page has something to show: the flow is usable, or a run is in a phase
    Q_PROPERTY(bool available READ available NOTIFY viewChanged)
    Q_PROPERTY(QString phase READ phase NOTIFY viewChanged)
    // At least one poll answered since the device connected
    Q_PROPERTY(bool polled READ polled NOTIFY viewChanged)
    // A staged request: {action, value, soakS, needsWord, title, detail}; empty if none
    Q_PROPERTY(QVariantMap pending READ pending NOTIFY pendingChanged)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    // What the last request ended with: the core's sentence, or one from here
    Q_PROPERTY(QString lastError READ lastError NOTIFY viewChanged)
    Q_PROPERTY(bool watching READ watching WRITE setWatching NOTIFY watchingChanged)

public:
    explicit VregController(DSPiBridge *bridge, QObject *parent = nullptr);
    ~VregController() override;

    QVariantMap view() const { return m_view; }
    bool available() const;
    QString phase() const { return m_view.value(QStringLiteral("phase"), QStringLiteral("idle")).toString(); }
    bool polled() const { return m_polled; }
    QVariantMap pending() const { return m_pending; }
    bool busy() const { return m_calls > 0; }
    QString lastError() const { return m_lastError; }
    bool watching() const { return m_watching; }
    void setWatching(bool on);

    // Calls that send nothing to the device: open shows the warning and the
    // record, close leaves, keep counts a soak on a trial found running.
    Q_INVOKABLE void openFlow();
    Q_INVOKABLE void closeFlow();
    Q_INVOKABLE void keepTrial(int soakS);
    // The abort: TRIAL_END at once, with no confirmation step
    Q_INVOKABLE void endTrial();
    // Stage a write: "start" (value: VREG code, soakS), "commit", "set" (value),
    // "reset", "trial" (value), "auto" (value 0/1), "clear". False with lastError
    // set if the view doesn't offer it now. Sends nothing.
    Q_INVOKABLE bool request(const QString &action, int value = 0, int soakS = 0);
    Q_INVOKABLE void cancel();
    // Send the staged request. `typed` goes to the core when the request needs the
    // word; the core judges it and refuses with nothing written.
    Q_INVOKABLE bool confirm(const QString &typed = QString());
    // A poll now, as the timer would make it
    Q_INVOKABLE void refresh();
    // Volts for a VREG code: "1.20 V"
    Q_INVOKABLE QString volts(int code) const;

    // For the smoke driver. A paused poll only runs when refresh() asks; the view
    // override stands in for the core's view, for phases the demo device can't reach.
    void setPollPaused(bool on);
    void setViewOverride(const QByteArray &json);

signals:
    void viewChanged();
    void pendingChanged();
    void busyChanged();
    void watchingChanged();
    // A request ended: true if the core took it
    void applied(const QString &action, bool ok);

private:
    struct Call {
        QString action;
        int value = 0;
        int soakS = 0;
        QByteArray typed;   // empty: no word passed
        bool sendsWord = false;
    };

    void sync();
    void syncTimer();
    void poll();
    void apply(const Call &call);
    void setView(const QByteArray &json, bool fromPoll);
    void setLastError(const QString &error);
    void clearPending();
    bool offered(int code, const char *flag, bool *needsWord) const;

    DSPiBridge *m_bridge;
    FfiCore *m_core;
    QThreadPool m_worker;
    QTimer m_pollTimer;
    QString m_serial;
    QVariantMap m_view;
    QVariantMap m_pending;
    QString m_lastError;
    QByteArray m_viewOverride;
    // Bumped when the device changes: older calls don't go out, older views are dropped
    std::atomic<quint64> m_generation { 0 };
    int m_calls = 0;
    bool m_connected = false;
    bool m_watching = false;
    bool m_pollPending = false;
    bool m_pollPaused = false;
    bool m_polled = false;
    bool m_reconnecting = false;
};

#endif // VREGCONTROLLER_H
