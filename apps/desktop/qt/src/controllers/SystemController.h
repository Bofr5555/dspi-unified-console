#ifndef SYSTEMCONTROLLER_H
#define SYSTEMCONTROLLER_H

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QPointer>
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
class DeviceRegistry;
class DeviceSession;
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
    // -- WP ATQT -- the S/PDIF transport (PLAN-AT 1.2; the web's SpdifUnits):
    // the S/PDIF units as the core read them ({ id, name, serial, role
    // ("source", "follower"), source, sourceName, txMask, slotsWord, d, unitD,
    // dMin, dMax, connected, state, stateWord, lockError, noTimeBase }), the
    // sources ({ id, name }), the units an Installer may put on S/PDIF (not
    // on it, not USB-direct: { id, name }), and the S/PDIF changes drafted, in
    // words (the rows show the hub's model until Save)
    Q_PROPERTY(QVariantList spdifUnits READ spdifUnits NOTIFY spdifChanged)
    Q_PROPERTY(QVariantList spdifSources READ spdifSources NOTIFY spdifChanged)
    Q_PROPERTY(QVariantList spdifCandidates READ spdifCandidates NOTIFY spdifChanged)
    Q_PROPERTY(QStringList spdifDrafted READ spdifDrafted NOTIFY draftChanged)
    // -- WP XDEV1 -- "Open in tab" (MPLAN 1.2 item 6): offered in Modern, on a
    // hub, for a unit the hub lists; `tabs` is serial -> "active" (the tab
    // shown now) or "open" (a tab of its own), for the units open here
    Q_PROPERTY(bool openInTabOffered READ openInTabOffered NOTIFY tabsChanged)
    Q_PROPERTY(QVariantMap tabs READ tabs NOTIFY tabsChanged)
    // -- WP FEATUI -- A room's or zone's volume (FEAT2, dspi_system_volume_*):
    // the rooms and zones ({ id: "room:<id>" | "zone:<id>", name }), the plan
    // asked for (the core's SystemVolumePlan: { name, units: [{ unitId, name,
    // beforeDb, targetDb, afterDb, louder, state, message }] }; empty for
    // none), whether it has been set, and what came of it
    Q_PROPERTY(QVariantList volumeTargets READ volumeTargets NOTIFY volumeChanged)
    Q_PROPERTY(QVariantMap volumePlan READ volumePlan NOTIFY volumeChanged)
    Q_PROPERTY(bool volumeSet READ volumeSet NOTIFY volumeChanged)
    Q_PROPERTY(QString volumeMessage READ volumeMessage NOTIFY volumeChanged)
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
    // -- WP ATQT --
    QVariantList spdifUnits() const { return m_spdifUnits; }
    QVariantList spdifSources() const;
    QVariantList spdifCandidates() const;
    QStringList spdifDrafted() const;

    // -- WP XDEV1 --
    bool openInTabOffered() const;
    QVariantMap tabs() const;
    // The unit with this serial in a tab: its own when it is open, else a new
    // one through the hub shown here (it becomes the active tab). The serial
    // is matched as the hub lists it (a unit row's `serial`, never a cleaned
    // text). False when not offered (Classic, no hub, a serial the hub does
    // not list); an open that fails is a notice.
    Q_INVOKABLE bool openInTab(const QString &serial);

    // Reads the view now (the poll does every 3 s)
    Q_INVOKABLE void refresh();
    // Scene `id` from `scenes`
    Q_INVOKABLE void recall(const QString &id);
    // Writes the audio link delays the system delay asks for (Admin)
    Q_INVOKABLE void align();
    // -- WP FEATUI --
    QVariantList volumeTargets() const { return m_volumeTargets; }
    QVariantMap volumePlan() const { return m_volumePlan; }
    bool volumeSet() const { return m_volumeSet; }
    QString volumeMessage() const { return m_volumeMessage; }
    // The rooms and zones, read from the hub's installation
    Q_INVOKABLE void loadVolumeTargets();
    // Plan `target` ("room:<id>", "zone:<id>") to `db` (a level, or a step
    // when `step`) of the user volume, or the master volume: nothing written
    Q_INVOKABLE void planVolume(const QString &target, bool step, double db, bool master);
    // Write the plan's ready units
    Q_INVOKABLE void applyVolume();
    Q_INVOKABLE void dropVolume();
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
    // -- WP ATQT -- the S/PDIF edits (Admin), staged in the draft and saved
    // with it in one system.set: a follower's source, a source's time-base
    // slot (`slot` 0 to 3: outputs 2n + 1 and 2n + 2) and its D ("" the
    // hub's; 0 to 65535, the hub holds it to the range), a unit put on
    // S/PDIF `as` "source" or as a follower of source `as`, a unit taken off
    // S/PDIF (back on the audio link). False when not taken.
    Q_INVOKABLE bool setSpdifSource(const QString &unitId, const QString &sourceId);
    Q_INVOKABLE bool setSpdifSlot(const QString &unitId, int slot, bool on);
    Q_INVOKABLE bool setSpdifD(const QString &unitId, const QString &samples);
    Q_INVOKABLE bool addSpdif(const QString &unitId, const QString &as);
    Q_INVOKABLE bool removeSpdif(const QString &unitId);
    // The draft's value for QML: as drafted, else as the model has it
    Q_INVOKABLE int spdifMaskOf(const QString &unitId) const;
    Q_INVOKABLE QString spdifDText(const QString &unitId) const;
    Q_INVOKABLE QString spdifSourceOf(const QString &unitId) const;
    // A unit fed by S/PDIF (a follower): not on the audio link's tables
    Q_INVOKABLE bool spdifFed(const QString &unitId) const;
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
    void volumeChanged();   // -- WP FEATUI --
    void draftChanged();
    void spdifChanged();   // -- WP ATQT --
    void tabsChanged();    // -- WP XDEV1 --

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
    // -- WP MDQT2b -- the active device's core, taken again on a switch;
    // `m_epoch` counts the switches, and an answer of an older one is dropped
    DeviceRegistry *m_registry;
    QPointer<DeviceSession> m_session;
    quint64 m_epoch = 0;
    void rebind();
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
    // -- WP FEATUI --
    QVariantList m_volumeTargets;
    QVariantMap m_volumePlan;
    bool m_volumeSet = false;
    QString m_volumeMessage;
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
    // -- WP ATQT -- id -> { role, source, txMask, setD, d } (absent: unchanged)
    QHash<QString, QVariantMap> m_draftSpdif;
    QVariantList m_spdifUnits;
    QVariantMap spdifRow(const QString &unitId) const;
    bool spdifEdit(const QString &unitId, const QVariantMap &change);
};

#endif // SYSTEMCONTROLLER_H
