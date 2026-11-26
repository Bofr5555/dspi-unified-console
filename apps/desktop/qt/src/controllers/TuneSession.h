#ifndef TUNESESSION_H
#define TUNESESSION_H

// -- WP MDQT2c -- One device's auto-tune (TuneController until then, now a
// facade over the active device's). The runs and the guard go on when another
// device becomes active. The device closed or replaced inside the guard: the
// tune goes back first, as it would by itself without Keep.
//
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
#include <functional>

class MeasureSession;
struct FfiCore;

class TuneSession : public QObject {
    Q_OBJECT

public:
    // The second Keep counts only from a second after the refusal (the core's
    // rule; a double click is one press): an answer sooner is sent this long
    // after the refusal
    static constexpr int kConfirmDelayMs = 1200;

    explicit TuneSession(MeasureSession *measure);

    // -- WP MDQT2c -- the device is about to be closed or replaced
    void stopForClose();
    // -- WP MCP5 -- An AI agent's summed check or verify (MeasureSession::
    // startForAgent): the microphone, then `job` (dspi_mcp_measure_begin), driven
    // as this tune's own runs; `ended` once, whether they began and why not
    void agentRuns(std::function<QByteArray(FfiCore *)> job, std::function<void(bool, const QString &)> ended) {
        startRuns(QStringLiteral("agent"), std::move(job), std::move(ended));
    }

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

    // -- WP UB7 -- keepChoices: the outputs left out of this tune stay out (the tab shown again)
    void reloadLayout(bool keepChoices = false);
    void takeLayout(const QJsonObject &layout);   // -- WP RCFIX --
    void setOutputOn(int output, bool on);
    void setOutputRole(int output, const QString &role);
    void setOutputSpeaker(int output, const QString &speaker);
    void setOutputGroup(int output, int group);
    // Proposes a tune of the Measure window's result (nothing is written)
    void propose();
    // The mandatory summed runs (the microphone opens for them)
    void sumCheckRun();
    // Applies the proposal guarded (`guardS` 0: the planned verify plus a margin)
    void apply(int guardS = 0);
    // The verify runs inside the guard
    void verify();
    // Keep: refused once after a verify that did not pass, then asked again
    void keep();
    // The person's second answer to that question: keeps the tune anyway.
    // Nothing unless the question is open.
    void confirmKeep();
    // The person declined the question: the guard runs on (Revert or its timeout)
    void declineKeep();
    void revert();
    // Stops the runs (an unapplied tune is forgotten)
    void stop();
    // -- WP AIVIS -- An agent's auto-tune step ran on this device, or the device came
    // to the front: the core's stage and proposal are read again
    void readAgain();
    // The A/B compare of "before" and "after"; toggle; end on a side
    void abStart();
    void abToggle();
    void abKeep(const QString &side);
    void clearError();

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
    MeasureSession *measure() const { return m_measure; }
    void onSnapshot();   // -- WP MDQT2c --
    void onTuneState(const QByteArray &json);
    void setProposal(const QJsonObject &p);
    void readProposal();
    // -- WP MCP5 -- `ended` (may be empty): once, whether the runs began and why not
    void startRuns(const QString &step, std::function<QByteArray(FfiCore *)> job,
                   std::function<void(bool, const QString &)> ended = {});
    void deviceWrite(const QString &step, std::function<QByteArray(FfiCore *)> job, std::function<void(const QJsonObject &)> done);
    void fail(const QString &text);
    void track(int delta);
    // -- WP ROLEQ -- whether it changed (written back to the speaker layout)
    bool setOutputField(int output, const QString &field, const QVariant &value);
    QByteArray layoutJson() const;
    QByteArray optionsJson() const;
    void updateJob();   // -- WP MDQT2c --
    void endRuns();
    bool measured(int output) const;
    void onResult();
    void keepNow();
    void disarm();

    MeasureSession *m_measure;
    QString m_serial;   // -- WP MDQT2c -- the connected device
    QJsonObject m_state;
    QVariantList m_layout;
    quint64 m_layoutEpoch = 0;   // -- WP RCFIX -- a read older than the layout taken is read again
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

#endif // TUNESESSION_H
