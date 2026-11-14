#ifndef SYSTEMCONTROLLER_H
#define SYSTEMCONTROLLER_H

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <atomic>
#include <functional>

#include "Registrar.h"

class DeviceModel;
class QJSEngine;
class QQmlEngine;
struct FfiCore;

// -- WP LKUI1 -- The audio link's installation on the hub this console is on
// (the web console's System view, SW3): its elements with their role and alink
// state, the scene bar, the Clock-and-audio-links editor and a one-line status. The
// core turns the hub's system.* answers into rows (dspi_alink_system_view and
// friends, dspi-core alink/system.rs); this only keeps them for QML, keeps the
// editor's draft and runs the calls.
//
// The calls ask the hub over the network (a status read touches every unit on
// the hub's side), so they run on a thread of this controller's own, as
// LinkController's do, and never hold up the core worker's device reads. The
// view is read every 3 s while a hub's device is open (the hub answers a status
// at most once a second per session).
//
// Roles: the core's mayEdit/mayRecall decide what QML offers (a Viewer gets
// nothing that writes); the hub judges every call again. No PIN exists here,
// so nothing is locked (docs/UI-MODES.md §1).
//
// The draft: edits are kept with the revision they were made on. Once the
// hub's revision moves past it, `stale` withdraws Save and QML offers Reload;
// a Save the hub refuses as a conflict does the same.
class SystemController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // A hub session answered with a view
    Q_PROPERTY(bool available READ available NOTIFY viewChanged)
    Q_PROPERTY(bool installed READ installed NOTIFY viewChanged)
    Q_PROPERTY(QString role READ role NOTIFY viewChanged)
    Q_PROPERTY(bool mayEdit READ mayEdit NOTIFY viewChanged)
    Q_PROPERTY(bool mayRecall READ mayRecall NOTIFY viewChanged)
    Q_PROPERTY(QString editNote READ editNote NOTIFY viewChanged)
    Q_PROPERTY(QString recallNote READ recallNote NOTIFY viewChanged)
    // "Audio link: 4 of 4 locked", and its meaning: ok, warning, problem, none
    Q_PROPERTY(QString summary READ summary NOTIFY viewChanged)
    Q_PROPERTY(QString health READ health NOTIFY viewChanged)
    Q_PROPERTY(int revision READ revision NOTIFY viewChanged)
    // The core's records as maps (camelCase): units, rooms, scenes, alarms.
    // Each list notifies only when it changed, so a poll does not rebuild
    // rows (or take the focus from a field being typed in)
    Q_PROPERTY(QVariantList units READ units NOTIFY unitsChanged)
    Q_PROPERTY(QVariantList rooms READ rooms NOTIFY roomsChanged)
    Q_PROPERTY(QVariantList scenes READ scenes NOTIFY scenesChanged)
    Q_PROPERTY(QVariantList alarms READ alarms NOTIFY viewChanged)
    // The units the editor lists: { id, name, latencyUs }, changing only with
    // the model
    Q_PROPERTY(QVariantList editUnits READ editUnits NOTIFY editUnitsChanged)
    Q_PROPERTY(QStringList issues READ issues NOTIFY viewChanged)
    Q_PROPERTY(QVariant systemMute READ systemMute NOTIFY viewChanged)
    Q_PROPERTY(int systemDelayFrames READ systemDelayFrames NOTIFY viewChanged)
    Q_PROPERTY(double systemDelayMs READ systemDelayMs NOTIFY viewChanged)
    Q_PROPERTY(bool systemDelayAuto READ systemDelayAuto NOTIFY viewChanged)
    Q_PROPERTY(int rateHz READ rateHz NOTIFY viewChanged)
    Q_PROPERTY(int pendingUnits READ pendingUnits NOTIFY viewChanged)
    // Why the view could not be read ("" when it was)
    Q_PROPERTY(QString error READ error NOTIFY viewChanged)
    // A recall, save or alignment is on its way
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    // The last recall's outcome: { scene, name, taken, message, writes: [...] },
    // or a refusal by role: { name, taken: false, message } with the hub's words
    Q_PROPERTY(QVariantMap lastRecall READ lastRecall NOTIFY recallChanged)
    // The last scene actually taken, by name ("" for none yet)
    Q_PROPERTY(QString recalledScene READ recalledScene NOTIFY recallChanged)
    // The last Save or alignment, in words; and whether it went wrong
    Q_PROPERTY(QString actionMessage READ actionMessage NOTIFY actionChanged)
    Q_PROPERTY(bool actionFailed READ actionFailed NOTIFY actionChanged)
    // The editor's draft
    Q_PROPERTY(bool dirty READ dirty NOTIFY draftChanged)
    Q_PROPERTY(bool stale READ stale NOTIFY draftChanged)
    Q_PROPERTY(bool draftDelayAuto READ draftDelayAuto NOTIFY draftChanged)
    Q_PROPERTY(int draftDelayFrames READ draftDelayFrames NOTIFY draftChanged)
    // -- WP AUSB2 -- the USB-direct units the editor lists ({ id, name }), and
    // "Keep USB-direct units aligned" as drafted, else as the hub has it
    Q_PROPERTY(QVariantList usbUnits READ usbUnits NOTIFY editUnitsChanged)
    Q_PROPERTY(bool keepUsbAligned READ keepUsbAligned NOTIFY draftChanged)
public:
    SystemController(const AppContext &ctx, QObject *parent = nullptr);
    ~SystemController() override;
    static SystemController *create(QQmlEngine *, QJSEngine *);

    bool available() const { return m_available; }
    bool installed() const { return bool(m_view.value(QLatin1String("installed")).toBool()); }
    QString role() const { return text("role"); }
    // -- fixup lk1 -- the hub's role, and this console's read-only state (G1, as the
    // other writing controllers and Android's bridge): a device held read only offers
    // no System write, and the three calls are refused before anything is sent
    bool mayEdit() const { return writable() && m_view.value(QLatin1String("mayEdit")).toBool(); }
    bool mayRecall() const { return writable() && m_view.value(QLatin1String("mayRecall")).toBool(); }
    QString editNote() const;
    QString recallNote() const;
    bool writable() const;
    QString summary() const { return text("summary"); }
    QString health() const { return m_available ? text("health") : QStringLiteral("none"); }
    int revision() const { return m_view.value(QLatin1String("revision")).toInt(); }
    QVariantList units() const { return m_units; }
    QVariantList rooms() const { return m_rooms; }
    QVariantList scenes() const { return m_scenes; }
    QVariantList editUnits() const { return m_editUnits; }
    QVariantList alarms() const { return list("alarms"); }
    QStringList issues() const;
    QVariant systemMute() const { return m_view.value(QLatin1String("systemMute")).toVariant(); }
    int systemDelayFrames() const { return m_view.value(QLatin1String("systemDelayFrames")).toInt(); }
    double systemDelayMs() const { return m_view.value(QLatin1String("systemDelayMs")).toDouble(); }
    bool systemDelayAuto() const { return m_view.value(QLatin1String("systemDelayAuto")).toBool(); }
    int rateHz() const { return m_view.value(QLatin1String("rateHz")).toInt(); }
    int pendingUnits() const { return m_view.value(QLatin1String("pending")).toInt(); }
    QString error() const { return m_error; }
    bool busy() const { return m_busy > 0; }
    QVariantMap lastRecall() const { return m_lastRecall; }
    QString recalledScene() const { return m_recalledScene; }
    QString actionMessage() const { return m_actionMessage; }
    bool actionFailed() const { return m_actionFailed; }
    bool dirty() const { return m_dirty; }
    bool stale() const { return m_stale; }
    bool draftDelayAuto() const { return m_draftAuto; }
    int draftDelayFrames() const { return m_draftFrames; }
    QVariantList usbUnits() const { return m_usbUnits; }
    bool keepUsbAligned() const;

    // Reads the view now (the poll does every 3 s)
    Q_INVOKABLE void refresh();
    // Scene `id` from `scenes`
    Q_INVOKABLE void recall(const QString &id);
    // Writes the audio link delays the system delay asks for (Admin)
    Q_INVOKABLE void align();
    // The draft: the system delay (automatic, or frames), a room's lip sync
    // and a unit's measured latency in µs ("" clears it: the model's own).
    // False for text that is not a whole number in range.
    Q_INVOKABLE void setSystemDelay(bool automatic, int frames);
    Q_INVOKABLE bool setRoomLipSync(const QString &roomId, const QString &us);
    Q_INVOKABLE bool setUnitLatency(const QString &unitId, const QString &us);
    // The draft's value for QML: what was typed, else the view's
    Q_INVOKABLE QString roomLipSyncText(const QString &roomId) const;
    Q_INVOKABLE QString unitLatencyText(const QString &unitId) const;
    // -- WP AUSB2 -- a USB-direct unit's mode ("off", "rate_lock", "phase_lock",
    // or "" for the unit's own), what it does when it can't lock ("mute" or
    // "play_unaligned"), and keeping the USB-direct units aligned (Admin).
    // False when not taken.
    Q_INVOKABLE bool setUsbMode(const QString &unitId, const QString &mode);
    Q_INVOKABLE bool setOnLockError(const QString &unitId, const QString &action);
    Q_INVOKABLE bool setKeepUsbAligned(bool on);
    // The draft's value for QML: as drafted, else as the installation asks
    Q_INVOKABLE QString usbModeOf(const QString &unitId) const;
    Q_INVOKABLE QString onLockErrorOf(const QString &unitId) const;
    // Sends the draft (only while dirty and not stale)
    Q_INVOKABLE void save();
    // Throws the draft away and reads the view again
    Q_INVOKABLE void reload();

    // For the smoke checks: the poll's interval, and the view as the core gave it
    void setPollInterval(int ms) { m_timer.setInterval(ms); }
    // -- fixup lk1 -- the device taken as read only, as a mismatched firmware makes it
    // (the demo hub's units are all current, so the smoke stands one in this way)
    void setReadOnlyForTest(bool readOnly) {
        m_readOnlyForTest = readOnly;
        emit viewChanged();
    }
    QJsonObject view() const { return m_view; }

signals:
    void viewChanged();
    void unitsChanged();
    void roomsChanged();
    void scenesChanged();
    void editUnitsChanged();
    void busyChanged();
    void recallChanged();
    void actionChanged();
    void draftChanged();

private:
    QString text(const char *key) const { return m_view.value(QLatin1String(key)).toString(); }
    QVariantList list(const char *key) const;
    void updatePolling();
    void run(std::function<QJsonObject(FfiCore *)> call, std::function<void(const QJsonObject &)> done);
    void applyView(const QJsonObject &o);
    void setLists(const QJsonObject &o);
    void setAction(const QString &message, bool failed);
    void clearDraft();
    QByteArray draftJson() const;

    FfiCore *m_core;
    DeviceModel *m_device;
    QThreadPool m_pool;
    QTimer m_timer;
    std::atomic<bool> m_reading{ false };
    bool m_shutDown = false;
    bool m_readOnlyForTest = false;   // -- fixup lk1 --
    bool m_wasWritable = true;
    int m_busy = 0;

    bool m_available = false;
    QJsonObject m_view;
    QVariantList m_units, m_rooms, m_scenes, m_editUnits;
    QVariantList m_usbUnits;   // -- WP AUSB2 --
    QString m_error;
    QVariantMap m_lastRecall;
    QString m_recalledScene;
    QString m_actionMessage;
    bool m_actionFailed = false;

    bool m_dirty = false;
    bool m_stale = false;
    int m_draftRevision = 0;
    bool m_draftDelaySet = false;
    bool m_draftAuto = true;
    int m_draftFrames = 0;
    // id -> µs, or an invalid QVariant for "the model's own"
    QHash<QString, QVariant> m_draftRooms;
    QHash<QString, QVariant> m_draftUnits;
    // -- WP AUSB2 -- id -> { mode: "" for the unit's own, onLockError: "" unchanged }
    QHash<QString, QVariantMap> m_draftUsb;
    QVariant m_draftKeep;   // invalid: unchanged
};

#endif // SYSTEMCONTROLLER_H
