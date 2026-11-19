#ifndef TUNECONTROLLER_H
#define TUNECONTROLLER_H

// -- WP P5QUI -- The Measure window's Auto-tune tab (PLAN-P5 section 1.4, T10;
// the web's P5WAT wizard on Qt), over the core's dspi_measure_tune_*: the
// layout with its alignment groups and programme feeds, the proposal of a
// measurement (MeasureController's result) with every write in words, what it
// was held to and what is withdrawn and why, the mandatory summed check, the
// guarded apply with its countdown, the verify, Keep or Revert, and the A/B
// compare of the "before" and "after" snapshots. Every rule is the core's.
//
// Keep after a verify that did not pass (WP P5TRUST): the core refuses the
// first Keep (`measure.tune.keep_unverified`); this controller then asks the
// person again (keepConfirmNeeded, the tab's question), and only their second,
// explicit answer calls Keep again (confirmKeep). Nothing here ever confirms
// on its own: confirmKeep does nothing unless the core refused a Keep and
// the question is open.
//
// Threads as MeasureController's: the runs (the summed check, the verify)
// and Keep on its measurement loop, which also ticks the tune (its runs and
// the guard's timer); the writes (apply, revert, the A/B) through
// DeviceModel::write.

#include <QElapsedTimer>
#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "Registrar.h"

class MeasureController;
class QJSEngine;
class QQmlEngine;
struct FfiCore;

class TuneController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

    // The tune's stage ("idle", "proposed", "sumCheck", "applied", "verifying",
    // "verified", "kept", "reverted") and what it means in words
    Q_PROPERTY(QString stage READ stage NOTIFY stateChanged)
    Q_PROPERTY(QString stageText READ stageText NOTIFY stateChanged)
    // The summed check's or the verify's runs play
    Q_PROPERTY(bool running READ running NOTIFY stateChanged)
    Q_PROPERTY(QString runText READ runText NOTIFY stateChanged)
    Q_PROPERTY(int progress READ progress NOTIFY stateChanged)
    // Why the last step stopped, in words ("" none)
    Q_PROPERTY(QString issueText READ issueText NOTIFY stateChanged)

    // ---- The layout ----
    // [{ output, label, on, role, speaker, group, feed (words) }]
    Q_PROPERTY(QVariantList layout READ layout NOTIFY layoutChanged)
    Q_PROPERTY(QString layoutNote READ layoutNote NOTIFY layoutChanged)
    // The options ("keep the crossover, align only" and its kin)
    Q_PROPERTY(bool keepCrossover READ keepCrossover WRITE setKeepCrossover NOTIFY optionsChanged)
    Q_PROPERTY(bool keepDelays READ keepDelays WRITE setKeepDelays NOTIFY optionsChanged)
    Q_PROPERTY(bool keepLevels READ keepLevels WRITE setKeepLevels NOTIFY optionsChanged)
    Q_PROPERTY(bool keepEq READ keepEq WRITE setKeepEq NOTIFY optionsChanged)

    // ---- The proposal ----
    Q_PROPERTY(bool hasProposal READ hasProposal NOTIFY proposalChanged)
    Q_PROPERTY(int proposalId READ proposalId NOTIFY proposalChanged)
    // [{ words, timing, kind, output }]
    Q_PROPERTY(QVariantList writes READ writes NOTIFY proposalChanged)
    Q_PROPERTY(QStringList held READ held NOTIFY proposalChanged)
    // In words, with their reasons
    Q_PROPERTY(QStringList withdrawn READ withdrawn NOTIFY proposalChanged)
    Q_PROPERTY(QStringList warnings READ warnings NOTIFY proposalChanged)
    Q_PROPERTY(QStringList notes READ notes NOTIFY proposalChanged)
    Q_PROPERTY(bool timingReliable READ timingReliable NOTIFY proposalChanged)
    // "notNeeded", "pending", "passed", "failed", and in words
    Q_PROPERTY(QString sumCheck READ sumCheck NOTIFY proposalChanged)
    Q_PROPERTY(QString sumCheckText READ sumCheckText NOTIFY proposalChanged)
    // The proposal has timing writes that wait for the summed check
    Q_PROPERTY(bool needsSumCheck READ needsSumCheck NOTIFY proposalChanged)
    // The graph: each speaker's before (solid), predicted (dotted) and target (dashed)
    Q_PROPERTY(QVariantMap graph READ graph NOTIFY proposalChanged)
    Q_PROPERTY(QString measurementName READ measurementName NOTIFY proposalChanged)

    // ---- The guard ----
    Q_PROPERTY(bool applied READ applied NOTIFY stateChanged)
    Q_PROPERTY(int guardRemainingS READ guardRemainingS NOTIFY stateChanged)
    Q_PROPERTY(QString guardText READ guardText NOTIFY stateChanged)
    // The verify's verdict in words ("" none yet) and whether it passed
    Q_PROPERTY(QString verifyText READ verifyText NOTIFY stateChanged)
    Q_PROPERTY(bool verifyPassed READ verifyPassed NOTIFY stateChanged)
    Q_PROPERTY(bool verifyDone READ verifyDone NOTIFY stateChanged)
    // The core refused Keep after a verify that did not pass: the question is
    // open (the tab asks; only the person's answer keeps)
    Q_PROPERTY(bool keepConfirmNeeded READ keepConfirmNeeded NOTIFY keepConfirmChanged)
    Q_PROPERTY(QString keepConfirmText READ keepConfirmText NOTIFY keepConfirmChanged)
    // How many Keeps the core refused so far (the smoke driver)
    Q_PROPERTY(int keepRefusals READ keepRefusals NOTIFY keepConfirmChanged)

    // ---- After Keep ----
    Q_PROPERTY(QString keptText READ keptText NOTIFY stateChanged)
    Q_PROPERTY(bool abActive READ abActive NOTIFY abChanged)
    Q_PROPERTY(QString abSide READ abSide NOTIFY abChanged)
    Q_PROPERTY(QString abText READ abText NOTIFY abChanged)

    Q_PROPERTY(QString error READ error NOTIFY errorChanged)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)

public:
    // The second Keep counts only from a second after the refusal (the core's
    // rule; a double click is one press): an answer sooner is sent this long
    // after the refusal
    static constexpr int kConfirmDelayMs = 1200;

    TuneController(const AppContext &ctx, QObject *parent = nullptr);
    static TuneController *create(QQmlEngine *, QJSEngine *);

    QString stage() const;
    QString stageText() const;
    bool running() const { return m_state.value(QLatin1String("running")).toBool(); }
    QString runText() const;
    int progress() const;
    QString issueText() const;

    QVariantList layout() const { return m_layout; }
    QString layoutNote() const { return m_layoutNote; }
    bool keepCrossover() const { return m_keepCrossover; }
    void setKeepCrossover(bool on);
    bool keepDelays() const { return m_keepDelays; }
    void setKeepDelays(bool on);
    bool keepLevels() const { return m_keepLevels; }
    void setKeepLevels(bool on);
    bool keepEq() const { return m_keepEq; }
    void setKeepEq(bool on);

    bool hasProposal() const { return !m_proposal.isEmpty(); }
    int proposalId() const { return m_proposal.value(QLatin1String("id")).toInt(); }
    QVariantList writes() const { return m_writes; }
    QStringList held() const { return m_held; }
    QStringList withdrawn() const { return m_withdrawn; }
    QStringList warnings() const { return m_warnings; }
    QStringList notes() const { return m_notes; }
    bool timingReliable() const { return m_proposal.value(QLatin1String("timingReliable")).toBool(); }
    QString sumCheck() const { return m_proposal.value(QLatin1String("sumCheck")).toString(); }
    QString sumCheckText() const;
    bool needsSumCheck() const;
    QVariantMap graph() const { return m_graph; }
    QString measurementName() const { return m_proposal.value(QLatin1String("measurementName")).toString(); }

    bool applied() const;
    int guardRemainingS() const;
    QString guardText() const;
    QString verifyText() const;
    bool verifyPassed() const;
    bool verifyDone() const { return m_state.value(QLatin1String("verify")).isObject(); }
    bool keepConfirmNeeded() const { return m_keepConfirm; }
    QString keepConfirmText() const { return m_keepConfirmText; }
    int keepRefusals() const { return m_keepRefusals; }
    QString keptText() const { return m_keptText; }
    bool abActive() const { return m_ab.value(QLatin1String("active")).toBool(); }
    QString abSide() const { return m_ab.value(QLatin1String("side")).toString(); }
    QString abText() const;
    QString error() const { return m_error; }
    bool busy() const { return m_busy > 0; }

    Q_INVOKABLE void reloadLayout();
    Q_INVOKABLE void setOutputOn(int output, bool on);
    Q_INVOKABLE void setOutputRole(int output, const QString &role);
    Q_INVOKABLE void setOutputSpeaker(int output, const QString &speaker);
    Q_INVOKABLE void setOutputGroup(int output, int group);
    // Proposes a tune of the Measure window's result (nothing is written)
    Q_INVOKABLE void propose();
    // The mandatory summed runs (the microphone opens for them)
    Q_INVOKABLE void sumCheckRun();
    // Applies the proposal guarded (`guardS` 0: the planned verify plus a margin)
    Q_INVOKABLE void apply(int guardS = 0);
    // The verify runs inside the guard
    Q_INVOKABLE void verify();
    // Keep: refused once after a verify that did not pass, then asked again
    Q_INVOKABLE void keep();
    // The person's second answer to that question: keeps the tune anyway.
    // Nothing unless the question is open.
    Q_INVOKABLE void confirmKeep();
    // The person declined the question: the guard runs on (Revert or its timeout)
    Q_INVOKABLE void declineKeep();
    Q_INVOKABLE void revert();
    // Stops the runs (an unapplied tune is forgotten)
    Q_INVOKABLE void stop();
    // The A/B compare of "before" and "after"; toggle; end on a side
    Q_INVOKABLE void abStart();
    Q_INVOKABLE void abToggle();
    Q_INVOKABLE void abKeep(const QString &side);
    Q_INVOKABLE void clearError();

    // The smoke driver's seam: Keep's reply as the core gave it, and the
    // tune's state after it (what keep() does with them)
    void keepAnswered(const QJsonObject &reply, const QJsonObject &state);

signals:
    void stateChanged();
    void layoutChanged();
    void optionsChanged();
    void proposalChanged();
    void keepConfirmChanged();
    void abChanged();
    void errorChanged();
    void busyChanged();
    // A step finished (the smoke driver): "propose", "sumCheck", "apply",
    // "verify", "keep", "revert", "ab", and whether it went through
    void stepDone(const QString &step, bool ok);

private:
    MeasureController *measure() const;
    void onTuneState(const QByteArray &json);
    void setProposal(const QJsonObject &p);
    void readProposal();
    void startRuns(const QString &step, std::function<QByteArray(FfiCore *)> job);
    void deviceWrite(const QString &step, std::function<QByteArray(FfiCore *)> job, std::function<void(const QJsonObject &)> done);
    void fail(const QString &text);
    void track(int delta);
    void setOutputField(int output, const QString &field, const QVariant &value);
    QByteArray layoutJson() const;
    QByteArray optionsJson() const;
    void endRuns();
    bool measured(int output) const;
    void onResult();
    void keepNow();
    void disarm();

    AppContext m_ctx;
    bool m_connected = false;
    QJsonObject m_state;
    QVariantList m_layout;
    QString m_layoutNote;
    bool m_keepCrossover = false;
    bool m_keepDelays = false;
    bool m_keepLevels = false;
    bool m_keepEq = false;
    QJsonObject m_proposal;
    QVariantList m_writes;
    QStringList m_held, m_withdrawn, m_warnings, m_notes;
    QVariantMap m_graph;
    bool m_runs = false;   // our summed check or verify plays (the microphone is open for it)
    bool m_keepConfirm = false;
    QString m_keepConfirmText;
    QElapsedTimer m_refusedAt;   // the core's last keep_unverified refusal
    // The core refused a Keep of this applied tune: every later Keep goes
    // through the question (the core would take any second Keep as the
    // confirmation, so the question must come first)
    bool m_armed = false;
    int m_keepToken = 0;   // review P5-D: which delayed "Keep Anyway" may still be sent
    int m_keepRefusals = 0;
    QString m_keptText;
    QJsonObject m_ab;
    QString m_error;
    int m_busy = 0;
};

#endif // TUNECONTROLLER_H
