#ifndef COREVOLTAGECONTROLLER_H
#define COREVOLTAGECONTROLLER_H

#include <QByteArray>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

#include "Poller.h"
#include "Registrar.h"

class DeviceSession;
class ExtController;
class QJSEngine;

// -- WP QP23 -- The Core Voltage page (behaviour.md §20, PLAN-CLIENTS QD28):
// a thin view of the core's calibration flow and manual requests
// (dspi_core_vreg_*, the core's vreg.rs). Every warning, limit, state text and
// the confirmation word come from the core's view; this class adds only the
// short state names and the durations' wording.
//
// - The flow is read with dspi_core_vreg_cal_poll on every connection to a
//   device with the extension, and once a second while the page shows (Poller).
// - Every request goes through DeviceModel::write(), so a read-only device or
//   none sends nothing; the core judges the limits and the word again.
// - The page is SettingsRegistry page "coreVoltage", group Hidden: only the
//   Advanced page opens it, and it is unavailable without the extension.
// - -- WP MDQT2b -- Several open devices: the page is the active device's (each
//   core runs its own flow). The last phase read of each device stays in its
//   session, so a calibration under way pins its device (calibratingOn,
//   DevicesController: it is not closed or replaced meanwhile); an answer of
//   a device switched away from updates only that.
class CoreVoltageController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The open device has the core voltage extension (the connect probe's
    // answer, ExtController.hasCoreVreg): the page and its entry show only then
    Q_PROPERTY(bool supported READ supported NOTIFY flowChanged)
    // The core's calibration view (dspi_core_vreg_cal_view's fields: phase,
    // message, warning, soakNote, choices, confirmationWord, canStart, ...)
    Q_PROPERTY(QVariantMap view READ view NOTIFY flowChanged)
    // The device's last status (CoreVregInfo's fields: liveVreg, storedSel,
    // hangCount, ...): the view's, or a plain read while the flow is out of reach
    Q_PROPERTY(QVariantMap status READ status NOTIFY flowChanged)
    Q_PROPERTY(QString phase READ phase NOTIFY flowChanged)
    // A calibration run is under way: the manual requests wait (the core's
    // Phase::busy, which the view does not carry)
    Q_PROPERTY(bool calibrating READ calibrating NOTIFY flowChanged)
    // Requests can go to this device: open, writable, the flow in reach
    Q_PROPERTY(bool writable READ writable NOTIFY flowChanged)
    // A request is on its way
    Q_PROPERTY(bool busy READ busy NOTIFY flowChanged)
    // The soak times offered, seconds, the core's suggestion among them
    Q_PROPERTY(QVariantList soakChoices READ soakChoices NOTIFY flowChanged)
    // The page shows: the flow is polled once a second
    Q_PROPERTY(bool pageShown READ pageShown WRITE setPageShown NOTIFY pageShownChanged)
public:
    CoreVoltageController(const AppContext &ctx, QObject *parent);
    static CoreVoltageController *create(QQmlEngine *, QJSEngine *) {
        return Registrar::instance<CoreVoltageController>();
    }

    bool supported() const { return m_supported; }
    QVariantMap view() const { return m_view; }
    QVariantMap status() const { return m_status; }
    QString phase() const;
    bool calibrating() const;
    bool writable() const;
    bool busy() const { return m_busy > 0; }
    QVariantList soakChoices() const;
    bool pageShown() const { return m_pageShown; }
    void setPageShown(bool shown);

    // The short name of phase `key` ("Soaking"); the description is the view's message
    Q_INVOKABLE QString stateName(const QString &key) const;
    // "30 min", "1 h 15 min", "45 s"
    Q_INVOKABLE QString durationLabel(int seconds) const;
    // Whether `action` at VREG code `code` needs the confirmation word now, as
    // the view says: "start" and "trial" (needsWord), "set" (saveNeedsWord),
    // "commit", "reset"; never for the others
    Q_INVOKABLE bool needsWord(const QString &action, int code) const;
    // `typed` is the core's word, exactly
    Q_INVOKABLE bool wordMatches(const QString &typed) const;

    // The calibration flow; `typed` is what the person typed ("" for nothing)
    Q_INVOKABLE void openFlow();
    Q_INVOKABLE void closeFlow();
    Q_INVOKABLE void start(int code, int soakS, const QString &typed);
    Q_INVOKABLE void keep(int soakS);
    Q_INVOKABLE void commit(const QString &typed);
    // Ends a found or passed trial, or aborts a run
    Q_INVOKABLE void endTrial();
    // Manual requests (disabled while a calibration runs); set code 0 is the
    // reset to the build default
    Q_INVOKABLE void manualSet(int code, const QString &typed);
    Q_INVOKABLE void manualTrial(int code, const QString &typed);
    Q_INVOKABLE void manualAuto(bool on);
    Q_INVOKABLE void manualClear();
    // Reads the flow now
    Q_INVOKABLE void refresh();

    // The demo device only (the core refuses anything else): 0 a hang, 1 a
    // reboot, 2 a power cycle, 3 `seconds` pass. For the smoke and demo mode.
    void demoEvent(int event, uint32_t seconds);
    // Requests handed to DeviceModel::write so far (the smoke's checks)
    int requests() const { return m_requests; }
    // -- WP MDQT2b -- a calibration runs on `session`'s device, as last read
    bool calibratingOn(DeviceSession *session) const;

signals:
    void flowChanged();
    void pageShownChanged();
    // A request the core refused; the reason is also the view's lastError
    void refused(const QString &what, const QString &reason);

private:
    // One request: `setter` returns the core's status byte
    void request(const QString &what, std::function<uint8_t(FfiCore *)> setter);
    void apply(const QByteArray &view, const QByteArray &status);
    // -- WP MDQT2b -- apply() when `asked` is the active one; its phase kept either way
    void applyFor(DeviceSession *asked, const QByteArray &view, const QByteArray &status);
    void onState();
    void onExt();
    void updatePage();

    AppContext m_ctx;
    QPointer<ExtController> m_ext;
    Poller::Id m_poll = -1;
    bool m_supported = false;
    bool m_pageShown = false;
    bool m_open = false;
    uint64_t m_generation = 0;
    int m_busy = 0;
    int m_requests = 0;
    QVariantMap m_view;
    QVariantMap m_status;
};

#endif // COREVOLTAGECONTROLLER_H
