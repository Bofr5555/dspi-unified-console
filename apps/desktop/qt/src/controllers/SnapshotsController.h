#ifndef SNAPSHOTSCONTROLLER_H
#define SNAPSHOTSCONTROLLER_H

#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "Poller.h"
#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP P1QT1 -- Device > Snapshots… and the safe apply (PLAN-CLIENTS 5.9:
// P1SN, P1AB, P1SA). The core keeps the snapshots (dspi_snapshot_*, one
// folder per serial under the app's data folder), runs the A/B compare
// (dspi_ab_*) and the guard (dspi_apply_*); this window lists, names and
// recalls them, compares two, and shows the countdown bar (SafeApplyBanner)
// with Keep and Revert for every guarded apply, a document's import
// (FilesController) included.
//
// - A recall is guarded when the core says so by default (DSPi Link to a hub
//   with the guard: dspi_apply_guard_default, owner OD-P1g) or when the
//   person asks for it ("Recall with rollback", USB).
// - The A/B toggle is never guarded, and is off while a guard waits; a guard
//   that ends other than kept ends the compare (the core's rule), so the
//   compare is read again afterwards.
// - Recalls, toggles and guarded applies are writes (DeviceModel::write); the
//   store's own calls (capture, rename, note, delete, export, import) run on
//   the worker. A success is the change itself or a notice, never a modal.
class SnapshotsController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The open device's snapshots, newest first: [{ id, name, note, saved,
    // savedMs, platform, inputs, outputs }]
    Q_PROPERTY(QVariantList snapshots READ snapshots NOTIFY snapshotsChanged)
    // A store call or a write of this controller is under way
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    // A recall is guarded without asking (over DSPi Link to a hub with the guard)
    Q_PROPERTY(bool guardDefault READ guardDefault NOTIFY guardChanged)
    // The person's choice for a recall on USB: with rollback (Keep or Revert)
    Q_PROPERTY(bool rollback READ rollback WRITE setRollback NOTIFY rollbackChanged)
    // ---- A/B ----
    Q_PROPERTY(bool abActive READ abActive NOTIFY abChanged)
    // "a" or "b": the side the device holds now
    Q_PROPERTY(QString abSide READ abSide NOTIFY abChanged)
    Q_PROPERTY(QString abAName READ abAName NOTIFY abChanged)
    Q_PROPERTY(QString abBName READ abBName NOTIFY abChanged)
    // What differs from A to B: [{ block, section, item, before, after }]
    Q_PROPERTY(QVariantList abRows READ abRows NOTIFY abChanged)
    // The toggle works now: a compare runs, no guard waits, nothing is under way
    Q_PROPERTY(bool canToggle READ canToggle NOTIFY abChanged)
    // ---- The safe apply ----
    // "idle", "waiting", "kept", "reverted", "revert_failed", "not_written", "lost"
    Q_PROPERTY(QString guardPhase READ guardPhase NOTIFY guardChanged)
    Q_PROPERTY(bool guardWaiting READ guardWaiting NOTIFY guardChanged)
    Q_PROPERTY(QString guardLabel READ guardLabel NOTIFY guardChanged)
    Q_PROPERTY(QString guardMessage READ guardMessage NOTIFY guardChanged)
    Q_PROPERTY(bool guardHub READ guardHub NOTIFY guardChanged)
    Q_PROPERTY(int guardTimeoutMs READ guardTimeoutMs NOTIFY guardChanged)
    Q_PROPERTY(int guardRemainingMs READ guardRemainingMs NOTIFY guardChanged)
    // What the guarded apply changed, in the unsaved-changes list's words
    Q_PROPERTY(QStringList guardChanges READ guardChanges NOTIFY guardChanged)
    // -- WP P1DCFIX --
    // The countdown's sentence, the core's (notice `guard.waiting`): `{s}` is
    // the whole seconds left
    Q_PROPERTY(QString guardWaitingText READ guardWaitingText NOTIFY guardWaitingTextChanged)
    // How long a guarded apply (a recall, an import) waits for Keep, in
    // seconds: 30 unless set (Advanced), 5 to 300; remembered on this console
    Q_PROPERTY(int guardTimeoutS READ guardTimeoutS WRITE setGuardTimeoutS NOTIFY guardTimeoutSChanged)
public:
    // How often the guard is driven while it waits (the countdown bar's tick)
    static constexpr int kGuardPollMs = 250;
    // -- WP P1DCFIX -- the safe apply's timeout: the default and its range
    static constexpr int kGuardTimeoutDefaultS = 30;
    static constexpr int kGuardTimeoutMinS = 5;
    static constexpr int kGuardTimeoutMaxS = 300;

    SnapshotsController(const AppContext &ctx, QObject *parent = nullptr);
    static SnapshotsController *create(QQmlEngine *, QJSEngine *);

    QVariantList snapshots() const { return m_snapshots; }
    bool busy() const { return m_busy > 0; }
    bool guardDefault() const { return m_guardDefault; }
    bool rollback() const { return m_rollback; }
    void setRollback(bool on);
    bool abActive() const { return m_abActive; }
    QString abSide() const { return m_abSide; }
    QString abAName() const { return m_abAName; }
    QString abBName() const { return m_abBName; }
    QVariantList abRows() const { return m_abRows; }
    bool canToggle() const { return m_abActive && !guardWaiting() && m_toggling == 0; }
    QString guardPhase() const { return m_guard.value(QLatin1String("phase")).toString(QStringLiteral("idle")); }
    bool guardWaiting() const { return guardPhase() == QLatin1String("waiting"); }
    QString guardLabel() const { return m_guard.value(QLatin1String("label")).toString(); }
    QString guardMessage() const { return m_guard.value(QLatin1String("message")).toString(); }
    bool guardHub() const { return m_guard.value(QLatin1String("hub")).toBool(); }
    int guardTimeoutMs() const { return m_guard.value(QLatin1String("timeoutMs")).toInt(); }
    int guardRemainingMs() const { return m_guard.value(QLatin1String("remainingMs")).toInt(); }
    QStringList guardChanges() const;
    QString guardWaitingText() const { return m_waitingText; }   // -- WP P1DCFIX --
    int guardTimeoutS() const { return m_timeoutS; }
    void setGuardTimeoutS(int seconds);

    // The store (no device write)
    Q_INVOKABLE void reload();
    Q_INVOKABLE void capture(const QString &name, const QString &note);
    Q_INVOKABLE void rename(const QString &id, const QString &name);
    Q_INVOKABLE void setNote(const QString &id, const QString &note);
    Q_INVOKABLE void remove(const QString &id);
    // A save dialog, then exportTo()
    Q_INVOKABLE void exportSnapshot(const QString &id);
    Q_INVOKABLE void exportTo(const QString &id, const QString &path);
    // An open dialog, then importFrom(): a .dspipreset filed as a snapshot
    Q_INVOKABLE void importSnapshot();
    Q_INVOKABLE void importFrom(const QString &path);

    // Recalls snapshot `id`: guarded when guardDefault or rollback
    Q_INVOKABLE void recall(const QString &id);

    // A/B: `aId` "" captures the device as A; `bId` is B
    Q_INVOKABLE void abStart(const QString &aId, const QString &bId);
    Q_INVOKABLE void abToggle();
    // Ends the compare on `side` ("a" or "b")
    Q_INVOKABLE void abKeep(const QString &side);

    // The countdown bar's buttons
    Q_INVOKABLE void keep();
    Q_INVOKABLE void revert();

    // A guarded apply begun elsewhere (FilesController's import): its state
    void guardStarted(const QJsonObject &state);

signals:
    void snapshotsChanged();
    void busyChanged();
    void rollbackChanged();
    void abChanged();
    void guardChanged();
    // A store call finished: what it was ("capture", "rename", "note",
    // "delete", "export", "import") and whether it went through (the smoke driver)
    void stored(const QString &op, bool ok);
    void guardWaitingTextChanged();   // -- WP P1DCFIX --
    void guardTimeoutSChanged();

private:
    // A store call on the worker, its JSON reply on the GUI thread
    void storeCall(const QString &op, std::function<QByteArray(FfiCore *)> job,
                   std::function<void(const QJsonObject &)> done = {});
    // A device write that answers JSON (called once: it changes the device)
    void writeCall(const QString &what, std::function<QByteArray(FfiCore *)> job,
                   std::function<void(bool ok, const QJsonObject &)> done);
    void setGuard(const QJsonObject &state);
    void setAb(const QJsonObject &compare);
    void reloadAb();
    void reloadGuard();
    void onSnapshotApplied();
    void track(int delta);
    void failed(const QJsonObject &reply, const QString &fallback);

    AppContext m_ctx;
    QVariantList m_snapshots;
    int m_busy = 0;
    bool m_guardDefault = false;
    bool m_rollback = false;
    bool m_abActive = false;
    QString m_abSide;
    QString m_abAName;
    QString m_abBName;
    QVariantList m_abRows;
    int m_toggling = 0;
    int m_writes = 0;
    QJsonObject m_guard;
    Poller::Id m_poll = -1;
    QString m_serial;
    bool m_connected = false;
    // -- WP P1DCFIX --
    QString m_waitingText;
    int m_timeoutS = kGuardTimeoutDefaultS;
};

#endif // SNAPSHOTSCONTROLLER_H
