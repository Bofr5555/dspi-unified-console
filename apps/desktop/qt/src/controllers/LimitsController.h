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

#include "CoreApi.h"
#include "Registrar.h"

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
    // The locked-user state: only volume, mute and presets may change
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
public:
    // A PIN's length (the core's rule, installer-lock.md)
    static constexpr int kPinMin = 4;
    static constexpr int kPinMax = 64;

    LimitsController(const AppContext &ctx, QObject *parent = nullptr);
    static LimitsController *create(QQmlEngine *, QJSEngine *);

    bool connected() const;
    bool busy() const { return m_busy > 0; }
    bool active() const { return m_status.value(QLatin1String("active")).toBool(); }
    QString profileState() const;
    QString standingNotice() const;
    QVariantList outputs() const;
    QVariantMap limits() const;
    QStringList lastHeld() const;
    QVariantMap ranges() const { return m_ranges; }
    QVariantMap lock() const { return m_lock.toVariantMap(); }
    bool pinSet() const { return m_lock.value(QLatin1String("pinSet")).toBool(); }
    bool locked() const { return m_lock.value(QLatin1String("locked")).toBool(); }
    bool installerOpen() const;
    bool covers() const;
    bool heldByHub() const { return m_lock.value(QLatin1String("heldBy")).toString() == QLatin1String("hub"); }
    QString scopeText() const;
    QString recoveryCode() const { return m_recoveryCode; }
    QString pinError() const { return m_pinError; }

    // One limit of output `output` (a field of `outputs`'s rows); NaN clears it
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
    };
    static Read readAll(FfiCore *c, bool hub);
    void apply(const Read &r);
    void onSnapshot();
    void track(int delta);
    // Sends the profile with `m_limits` as its limits block
    void sendProfile(const QString &what);
    // A PIN call on the worker; its reply is the lock state, or a refusal
    void pinCall(std::function<QByteArray(FfiCore *)> call, std::function<void(const QJsonObject &)> then = {});
    void setPinError(const QJsonObject &reply);
    void postHeld(const QJsonObject &reply);

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
};

#endif // LIMITSCONTROLLER_H
