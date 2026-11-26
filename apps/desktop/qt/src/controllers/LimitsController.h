#ifndef LIMITSCONTROLLER_H
#define LIMITSCONTROLLER_H

#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <cstdint>
#include <functional>
#include <memory>
#include <vector>

#include "CoreApi.h"
#include "Registrar.h"

class DeviceSession;
class QJSEngine;
class QQmlEngine;

// -- WP P1QT2 -- Settings > Installer (OD-P1e: the Mode menu stays hidden):
// the limits profile (P1LM), the installer's PIN and the lock (P1LK), and the
// locked-user view's state. Every rule is the core's; this class only asks
// and shows (docs/integrator/limits-profiles.md, installer-lock.md).
//
// - Changes apply at once: each edit of a limit sends the whole profile
//   (`dspi_limits_apply_profile`), which holds the device to it at once; what
//   it held is shown as a notice, in the core's words.
// - With no PIN set nothing is locked and nothing asks for a PIN.
// - The lock is one lock: this console's, or the hub's when the device is
//   reached through a hub that holds it (`heldBy`), with one prompt.
// - Every PIN call is slow (PBKDF2): all of them run on the core's worker.
// - At start it names the limits folder (the app's data folder, before any
//   device opens) and turns the core's confirmation tokens on (P1TK: every
//   dangerous operation goes through Confirm.h).
class LimitsController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool connected READ connected NOTIFY changed)
    // A PIN call or a profile write is on its way
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)

    // ---- The limits profile ----
    // A profile is in force for the open device
    Q_PROPERTY(bool active READ active NOTIFY changed)
    // `none`, `ok`, `fell_back`, `unreadable` (limits_status)
    Q_PROPERTY(QString profileState READ profileState NOTIFY changed)
    // The standing notice of an unreadable profile, in one line ("" with none)
    Q_PROPERTY(QString standingNotice READ standingNotice NOTIFY changed)
    // Per output: { output, name, maxGainDb, limiterCeilingDb, highpassMinHz,
    // highpassMinSlopeDb, delayMinMs, delayMaxMs } (a field left out limits nothing)
    Q_PROPERTY(QVariantList outputs READ outputs NOTIFY changed)
    // The profile's own fields: maxPeqBoostDb, masterVolumeMaxDb, note, contact
    Q_PROPERTY(QVariantMap limits READ limits NOTIFY changed)
    // What the last hold changed (limits_status().lastHeld)
    Q_PROPERTY(QStringList lastHeld READ lastHeld NOTIFY changed)
    // -- WP P1EFIX -- The core's one range table for the limits editor
    // (`dspi_limits_field_ranges`): field key -> { min, max, step, unit, perOutput }
    Q_PROPERTY(QVariantMap ranges READ ranges CONSTANT)

    // ---- The installer's PIN and the lock ----
    // The core's LockState: { pinSet, unlocked, locked, installerOpen, covers,
    // wrongTries, retryAfterMs, recovered, note, contact, scopeNoticeKey,
    // heldBy, ... }
    Q_PROPERTY(QVariantMap lock READ lock NOTIFY lockChanged)
    Q_PROPERTY(bool pinSet READ pinSet NOTIFY lockChanged)
    // The locked-user state: only volume, mute, source and presets may change
    Q_PROPERTY(bool locked READ locked NOTIFY lockChanged)
    // The Installer page may change things without a prompt
    Q_PROPERTY(bool installerOpen READ installerOpen NOTIFY lockChanged)
    // The lock covers the settings (a PIN alone locks nothing)
    Q_PROPERTY(bool covers READ covers NOTIFY lockChanged)
    // The hub this device is reached through holds the lock
    Q_PROPERTY(bool heldByHub READ heldByHub NOTIFY lockChanged)
    // The unlock dialog's one line: the lock is on this console, the DSPi
    // itself is open (or: the hub holds it)
    Q_PROPERTY(QString scopeText READ scopeText NOTIFY lockChanged)
    // The recovery code, shown once after a PIN is set ("" otherwise)
    Q_PROPERTY(QString recoveryCode READ recoveryCode NOTIFY lockChanged)
    // The last PIN call's refusal, in the core's words ("" with none)
    Q_PROPERTY(QString pinError READ pinError NOTIFY lockChanged)

    // -- WP HUBADMIN -- Which copy the page edits (`dspi_installer_target`):
    // `console` (this console's own: USB, MIDI, the demo), `hub` (the hub's,
    // over DSPi Link: the `dspi_hub_*` calls) or `none`
    Q_PROPERTY(QString copy READ copy NOTIFY changed)
    // The page may change that copy (the hub's: an Admin of the hub only)
    Q_PROPERTY(bool editable READ editable NOTIFY changed)
    // The notice row that says which copy is edited, in one line
    Q_PROPERTY(QString copyText READ copyText NOTIFY changed)
    // Why the page is read-only, in one line ("" when editable)
    Q_PROPERTY(QString readOnlyText READ readOnlyText NOTIFY changed)
    // The hub's pinned mode per role: [{ role, mode }] (mode "" = none); [] off a hub
    Q_PROPERTY(QVariantList modePins READ modePins NOTIFY changed)
    // The notice row that goes with the recovery code on show
    // (`installer_lock.recovery`, or `installer_lock.hub_recovery` for the hub's)
    Q_PROPERTY(QString recoveryKey READ recoveryKey NOTIFY lockChanged)
public:
    // A PIN's length (the core's rule, installer-lock.md)
    static constexpr int kPinMin = 4;
    static constexpr int kPinMax = 64;

    LimitsController(const AppContext &ctx, QObject *parent = nullptr);
    static LimitsController *create(QQmlEngine *, QJSEngine *);

    bool connected() const;
    bool busy() const { return m_busy > 0; }
    // -- WP HUBADMIN -- the hub's copy: its profile and its PIN
    bool active() const;
    QString profileState() const;
    QString standingNotice() const;
    QVariantList outputs() const;
    QVariantMap limits() const;
    QStringList lastHeld() const;
    QVariantMap ranges() const { return m_ranges; }
    QVariantMap lock() const { return m_lock.toVariantMap(); }
    bool pinSet() const;
    bool locked() const { return m_lock.value(QLatin1String("locked")).toBool(); }
    bool installerOpen() const;
    bool covers() const;
    bool heldByHub() const { return m_lock.value(QLatin1String("heldBy")).toString() == QLatin1String("hub"); }
    QString scopeText() const;
    QString recoveryCode() const { return m_recoveryCode; }
    QString pinError() const { return m_pinError; }
    // -- WP HUBADMIN --
    ~LimitsController() override;
    QString copy() const;
    bool hubCopy() const { return copy() == QLatin1String("hub"); }
    bool editable() const;
    QString copyText() const;
    QString readOnlyText() const;
    QVariantList modePins() const;
    QString recoveryKey() const;

    // One limit of output `output` (a field of `outputs`'s rows); NaN clears it
    // -- FIX016 -- (REVG F8, D4 in Modern) Whether the menu table's row `action` is
    // one the locked user may not use in Modern: the device writes the top bar
    // hides (Save, Revert, undo and redo, Factory Reset, Bypass, A/B, snapshots,
    // imports, the preset slots' rows, firmware). Volume, mute, preset recall and
    // the input source stay; Classic is not fenced (owner, 2026-11-26). Only the
    // list: the callers read `locked` and the layout themselves, so a binding
    // follows them
    Q_INVOKABLE static bool lockRefuses(const QString &action);
    // The same with the lock and the layout: Shell's gate refuses the action
    bool lockFences(const QString &action) const;
    Q_INVOKABLE void setOutputLimit(int output, const QString &field, double value);
    // maxPeqBoostDb or masterVolumeMaxDb; NaN clears it
    Q_INVOKABLE void setLimit(const QString &field, double value);
    // note or contact
    Q_INVOKABLE void setText(const QString &field, const QString &text);
    // Every limit gone (the PIN stays; with a PIN, what the lock covers too)
    Q_INVOKABLE void clearLimits();

    // The PIN: set (both fields alike), remove, unlock with it or the
    // recovery code, lock again
    Q_INVOKABLE void setPin(const QString &pin, const QString &repeat);
    Q_INVOKABLE void clearPin();
    Q_INVOKABLE void unlock(const QString &pin);
    Q_INVOKABLE void lockNow();
    // The lock covers the settings (every screen but the front page), or nothing
    Q_INVOKABLE void setCovers(bool on);
    // The recovery code has been written down
    Q_INVOKABLE void dismissRecovery();
    // Reads the profile and the lock again (on the worker)
    Q_INVOKABLE void refresh();
    // -- WP HUBADMIN -- Pins `role`'s mode on the hub (`viewer`, `control`,
    // `admin`; mode `simple`, `advanced`, `installer`, or "" for none)
    Q_INVOKABLE void setModePin(const QString &role, const QString &mode);

signals:
    void changed();
    void lockChanged();
    void busyChanged();
    // After unlock(): whether it opened
    void unlockFinished(bool ok);

private:
    struct Read {
        QJsonObject status, lock, doc;
        QStringList screens;
        // -- WP HUBADMIN -- the target, and the hub's state when it is the hub's copy
        QJsonObject target, hub;
        bool hubRead = false;
    };
    // -- WP HUBADMIN -- `hubState`: the hub's copy asked of the hub (a round
    // trip: on a new connection, a refresh, after a call or an event; not on
    // every snapshot)
    static Read readAll(FfiCore *c, bool hub, bool hubState = true);
    void apply(const Read &r);
    void onSnapshot();
    void track(int delta);
    // Sends the profile with `m_limits` as its limits block
    void sendProfile(const QString &what);
    // A PIN call on the worker; its reply is the lock state, or a refusal
    void pinCall(std::function<QByteArray(FfiCore *)> call, std::function<void(const QJsonObject &)> then = {});
    void setPinError(const QJsonObject &reply);
    void postHeld(const QJsonObject &reply);
    // -- WP HUBADMIN -- A call on the worker, then a read; a refusal as a
    // notice (or, `asPinError`, as the page's PIN line)
    void adminCall(std::function<QByteArray(FfiCore *)> call, std::function<void(const QJsonObject &)> then, bool asPinError);
    // The hub's admin events, on a core thread: re-read on the GUI thread's say
    static void hubEventThunk(const char *kind, const char *eventJson, void *self);
    void onHubEvent(const QJsonObject &event);
    // -- WP MDQT2b -- One listener per open device's core (events of the
    // active one only are acted on); `asked` the session a read was asked of:
    // its answer counts while that one is still active
    struct HubListener {
        LimitsController *self;
        DeviceSession *session;   // compared, never followed off the GUI thread
    };
    void listen(DeviceSession *session);
    void unlisten(DeviceSession *session);
    bool current(const DeviceSession *asked) const { return asked == m_ctx.activeSession(); }
    std::vector<std::unique_ptr<HubListener>> m_listeners;
    bool m_lockDue = false;   // a new hub connection: its first read asks the hub for the lock

    AppContext m_ctx;
    int m_busy = 0;
    quint64 m_generation = 0;
    bool m_reading = false, m_readAgain = false;
    bool m_primeHeld = false;
    QJsonObject m_status, m_lock, m_limits;
    QStringList m_screens;   // the lock's cover: the manifest's screens past the front page
    QStringList m_heldSeen;  // the last hold already shown
    QString m_recoveryCode;
    QString m_pinError;
    QVariantMap m_ranges; // -- WP P1EFIX --
    // -- WP HUBADMIN --
    QJsonObject m_target, m_hub;
    bool m_hubRecovery = false;   // the recovery code on show is the hub's
    bool m_hubDue = false;        // a new connection: its first read asks the hub
};

#endif // LIMITSCONTROLLER_H
