#ifndef TUNECONTROLLER_H
#define TUNECONTROLLER_H

// -- WP MDQT2c -- The Measure window's Auto-tune tab as QML sees it: a facade
// over the active device's TuneSession, where it all happens (TuneSession.h).

#include <QObject>
#include <QPointer>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"
#include "TuneSession.h"

class MeasureSession;
class QJSEngine;
class QQmlEngine;

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
    static constexpr int kConfirmDelayMs = TuneSession::kConfirmDelayMs;

    TuneController(const AppContext &ctx, QObject *parent = nullptr);
    static TuneController *create(QQmlEngine *, QJSEngine *);
    // The active device's
    TuneSession *session() const;

    QString stage() const { return session()->stage(); }
    QString stageText() const { return session()->stageText(); }
    bool running() const { return session()->running(); }
    QString runText() const { return session()->runText(); }
    int progress() const { return session()->progress(); }
    QString issueText() const { return session()->issueText(); }

    QVariantList layout() const { return session()->layout(); }
    QString layoutNote() const { return session()->layoutNote(); }
    bool keepCrossover() const { return session()->keepCrossover(); }
    void setKeepCrossover(bool on) { session()->setKeepCrossover(on); }
    bool keepDelays() const { return session()->keepDelays(); }
    void setKeepDelays(bool on) { session()->setKeepDelays(on); }
    bool keepLevels() const { return session()->keepLevels(); }
    void setKeepLevels(bool on) { session()->setKeepLevels(on); }
    bool keepEq() const { return session()->keepEq(); }
    void setKeepEq(bool on) { session()->setKeepEq(on); }

    bool hasProposal() const { return session()->hasProposal(); }
    int proposalId() const { return session()->proposalId(); }
    QVariantList writes() const { return session()->writes(); }
    QStringList held() const { return session()->held(); }
    QStringList withdrawn() const { return session()->withdrawn(); }
    QStringList warnings() const { return session()->warnings(); }
    QStringList notes() const { return session()->notes(); }
    bool timingReliable() const { return session()->timingReliable(); }
    QString sumCheck() const { return session()->sumCheck(); }
    QString sumCheckText() const { return session()->sumCheckText(); }
    bool needsSumCheck() const { return session()->needsSumCheck(); }
    QVariantMap graph() const { return session()->graph(); }
    QString measurementName() const { return session()->measurementName(); }

    bool applied() const { return session()->applied(); }
    int guardRemainingS() const { return session()->guardRemainingS(); }
    QString guardText() const { return session()->guardText(); }
    QString verifyText() const { return session()->verifyText(); }
    bool verifyPassed() const { return session()->verifyPassed(); }
    bool verifyDone() const { return session()->verifyDone(); }
    bool keepConfirmNeeded() const { return session()->keepConfirmNeeded(); }
    QString keepConfirmText() const { return session()->keepConfirmText(); }
    int keepRefusals() const { return session()->keepRefusals(); }
    QString keptText() const { return session()->keptText(); }
    bool abActive() const { return session()->abActive(); }
    QString abSide() const { return session()->abSide(); }
    QString abText() const { return session()->abText(); }
    QString error() const { return session()->error(); }
    bool busy() const { return session()->busy(); }

    Q_INVOKABLE void reloadLayout() { session()->reloadLayout(); }
    Q_INVOKABLE void setOutputOn(int output, bool on) { session()->setOutputOn(output, on); }
    Q_INVOKABLE void setOutputRole(int output, const QString &role) { session()->setOutputRole(output, role); }
    Q_INVOKABLE void setOutputSpeaker(int output, const QString &speaker) { session()->setOutputSpeaker(output, speaker); }
    Q_INVOKABLE void setOutputGroup(int output, int group) { session()->setOutputGroup(output, group); }
    // Proposes a tune of the Measure window's result (nothing is written)
    Q_INVOKABLE void propose() { session()->propose(); }
    // The mandatory summed runs (the microphone opens for them)
    Q_INVOKABLE void sumCheckRun() { session()->sumCheckRun(); }
    // Applies the proposal guarded (`guardS` 0: the planned verify plus a margin)
    Q_INVOKABLE void apply(int guardS = 0) { session()->apply(guardS); }
    // The verify runs inside the guard
    Q_INVOKABLE void verify() { session()->verify(); }
    // Keep: refused once after a verify that did not pass, then asked again
    Q_INVOKABLE void keep() { session()->keep(); }
    // The person's second answer to that question: keeps the tune anyway.
    // Nothing unless the question is open.
    Q_INVOKABLE void confirmKeep() { session()->confirmKeep(); }
    // The person declined the question: the guard runs on (Revert or its timeout)
    Q_INVOKABLE void declineKeep() { session()->declineKeep(); }
    Q_INVOKABLE void revert() { session()->revert(); }
    // Stops the runs (an unapplied tune is forgotten)
    Q_INVOKABLE void stop() { session()->stop(); }
    // The A/B compare of "before" and "after"; toggle; end on a side
    Q_INVOKABLE void abStart() { session()->abStart(); }
    Q_INVOKABLE void abToggle() { session()->abToggle(); }
    Q_INVOKABLE void abKeep(const QString &side) { session()->abKeep(side); }
    Q_INVOKABLE void clearError() { session()->clearError(); }

    // The smoke driver's seam: Keep's reply as the core gave it, and the
    // tune's state after it (what keep() does with them)
    void keepAnswered(const QJsonObject &reply, const QJsonObject &state) { session()->keepAnswered(reply, state); }

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
    void follow(MeasureSession *measure);
    void onSwitched();

    AppContext m_ctx;
    QPointer<TuneSession> m_announced;
};

#endif // TUNECONTROLLER_H
