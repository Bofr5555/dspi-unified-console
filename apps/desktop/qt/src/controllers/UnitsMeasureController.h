#ifndef UNITSMEASURECONTROLLER_H
#define UNITSMEASURECONTROLLER_H

// -- WP ATQT -- Tools > Measure…'s "Several units" tab (PLAN-AT 1.4 to 1.6,
// OD-ATh, OD-ATl; the web's ATWEB step on Qt): an alignment group across
// units of our hub, measured with one microphone, each unit's own generator
// started on the shared time base. Every rule is the core's
// (dspi_measure_system_*) and the hub's (system.measure.*); this controller
// only gathers the form, drives the calls and words what they say.
//
// The session runs on this console's own core (its DSPi Link session
// reaches the hub). Every unit of the group is judged by its **own** core:
// a connection of its own to the same hub (dspi_core_new, the session's
// address and credential, bound to that unit), opened when the unit is
// chosen, whose dspi_measure_system_unit_check is asked again right before
// each call that takes checks (the preflight, the start, the summed check,
// the verify: review AT-C F2). This console's own DSPi is judged by its core.
//
// Rules this adds (the coordinator's): nothing plays before the person chose
// where the microphone is, and across units only the listening position is
// taken; Stop is always offered while anything plays (the Measure window's
// Stop bar too), and the settle wait is said in words; Apply and Revert are
// Admin's only (Control measures and sees, nothing is sent for it); Keep is
// never pressed by this code: after a verify that did not pass the core
// refuses the first Keep (its words, `applied.message`, are the question),
// and only the person's own second press, at least a second later, keeps.
// A Keep asked for sooner waits for that second and is withdrawn by Revert,
// Stop or a new apply or verify.
//
// The hub's guard of a system apply (WP ATGUARD): the proposal's
// `guardNote` is shown (on a hub without the guard it says nothing reverts
// by itself), the guard's countdown from `guardS` after the apply, and a
// Keep the hub has already reverted shows the hub's words.
//
// Threads: the main core's calls on the measurement loop (MeasureController:
// it also ticks the session and pushes the microphone's blocks); the units'
// connections open and close on a thread of this controller's own. The GUI
// thread never waits on either.

#include <QElapsedTimer>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <functional>
#include <memory>

#include "Registrar.h"

class MeasureController;
class QJSEngine;
class QQmlEngine;
struct FfiCore;

class UnitsMeasureController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

    // Why the step cannot run here at all ("" it can) and its notice key
    // (`measure.system.needs_hub`): not on our hub, or the hub without
    // `system_measure`
    Q_PROPERTY(QString unavailable READ unavailable NOTIFY availabilityChanged)
    Q_PROPERTY(QString unavailableKey READ unavailableKey NOTIFY availabilityChanged)
    // The hub's role for this console ("admin", "control", "viewer")
    Q_PROPERTY(QString role READ role NOTIFY availabilityChanged)
    Q_PROPERTY(bool admin READ admin NOTIFY availabilityChanged)

    // ---- The form ----
    // "" until the person chose; "listeningPosition" is the only one taken
    Q_PROPERTY(QString placement READ placement WRITE setPlacement NOTIFY formChanged)
    Q_PROPERTY(QString placementProblem READ placementProblem NOTIFY formChanged)
    // [{ serial, name, online, own, chosen, opening, error, outputs: [{ output,
    // label, on, role }] }]
    Q_PROPERTY(QVariantList units READ units NOTIFY formChanged)
    Q_PROPERTY(int sweepMs READ sweepMs WRITE setSweepMs NOTIFY formChanged)
    // 0: the plan's (5, or 3 for a group that needs several runs)
    Q_PROPERTY(int passes READ passes WRITE setPasses NOTIFY formChanged)
    // "serial:output", or "" (the first unit's first output); and the choices
    // [{ id, label }]
    Q_PROPERTY(QString anchor READ anchor WRITE setAnchor NOTIFY formChanged)
    Q_PROPERTY(QVariantList anchors READ anchors NOTIFY formChanged)
    Q_PROPERTY(QString runName READ runName WRITE setRunName NOTIFY formChanged)
    // What keeps the form from being checked or started ("" nothing)
    Q_PROPERTY(QString blocker READ blocker NOTIFY formChanged)

    // ---- The check ----
    // [{ kind ("ready", "blocker", "warning"), key, serial, text }]
    Q_PROPERTY(QVariantList preflightLines READ preflightLines NOTIFY preflightChanged)

    // ---- The run ----
    // A run, a summed check or a verify holds the hub's session (or starts)
    Q_PROPERTY(bool running READ running NOTIFY stateChanged)
    Q_PROPERTY(QString phase READ phase NOTIFY stateChanged)
    Q_PROPERTY(QString purpose READ purpose NOTIFY stateChanged)
    // Where it is, in words, with the settle wait in the core's words
    Q_PROPERTY(QString phaseText READ phaseText NOTIFY stateChanged)
    Q_PROPERTY(int progress READ progress NOTIFY stateChanged)
    // "Needs N runs" while a chained group runs ("" otherwise)
    Q_PROPERTY(QString runsText READ runsText NOTIFY stateChanged)
    // Why the last run stopped, in words, and its notice key ("" none)
    Q_PROPERTY(QString issueText READ issueText NOTIFY stateChanged)
    Q_PROPERTY(QString issueKey READ issueKey NOTIFY stateChanged)

    // ---- The result ----
    Q_PROPERTY(bool hasResult READ hasResult NOTIFY resultChanged)
    Q_PROPERTY(bool timingReliable READ timingReliable NOTIFY resultChanged)
    Q_PROPERTY(QString timingText READ timingText NOTIFY resultChanged)
    // [{ label, arrival, offered }], [{ from, to, delay, offered }], words
    Q_PROPERTY(QVariantList arrivals READ arrivals NOTIFY resultChanged)
    Q_PROPERTY(QVariantList delays READ delays NOTIFY resultChanged)
    Q_PROPERTY(QStringList reasons READ reasons NOTIFY resultChanged)
    // Each unit's measured responses (MeasureGraph.qml)
    Q_PROPERTY(QVariantMap graph READ graph NOTIFY resultChanged)

    // ---- The proposal ----
    Q_PROPERTY(bool hasProposal READ hasProposal NOTIFY proposalChanged)
    Q_PROPERTY(QStringList proposalArrivals READ proposalArrivals NOTIFY proposalChanged)
    Q_PROPERTY(QStringList withheld READ withheld NOTIFY proposalChanged)
    // [{ text, failed }]
    Q_PROPERTY(QVariantList transitions READ transitions NOTIFY proposalChanged)
    Q_PROPERTY(QString proposalNotice READ proposalNotice NOTIFY proposalChanged)
    // The hub's guard in words (WP ATGUARD): how long, or that nothing
    // reverts by itself on a hub without it
    Q_PROPERTY(QString guardNote READ guardNote NOTIFY proposalChanged)
    Q_PROPERTY(bool sumCheckNeeded READ sumCheckNeeded NOTIFY proposalChanged)
    // Apply is offered (Admin, offered, the summed check passed); else why not
    Q_PROPERTY(bool mayApply READ mayApply NOTIFY proposalChanged)
    Q_PROPERTY(QString applyWhy READ applyWhy NOTIFY proposalChanged)
    Q_PROPERTY(QString applyWhyKey READ applyWhyKey NOTIFY proposalChanged)

    // ---- After the apply ----
    // "none", "applied", "verified", "kept", "reverted"
    Q_PROPERTY(QString stage READ stage NOTIFY appliedChanged)
    Q_PROPERTY(QString appliedText READ appliedText NOTIFY appliedChanged)
    Q_PROPERTY(bool verifyFailed READ verifyFailed NOTIFY appliedChanged)
    // The hub's guard: seconds left (0 none), and in words
    Q_PROPERTY(int guardRemainingS READ guardRemainingS NOTIFY guardChanged)
    Q_PROPERTY(QString guardText READ guardText NOTIFY guardChanged)
    // The second Keep's question: open, its words, may be answered now (a
    // second after the refusal), and the seconds left until then
    Q_PROPERTY(bool keepQuestion READ keepQuestion NOTIFY keepChanged)
    Q_PROPERTY(QString keepQuestionText READ keepQuestionText NOTIFY keepChanged)
    Q_PROPERTY(bool keepReady READ keepReady NOTIFY keepChanged)
    Q_PROPERTY(int keepWaitS READ keepWaitS NOTIFY keepChanged)
    // How many Keeps the core refused (the smoke driver)
    Q_PROPERTY(int keepRefusals READ keepRefusals NOTIFY keepChanged)

    // -- review AT-D2 -- the demo hub's simulated microphone (WP ATDEMO): offered
    // only by `uconsole --demo hub --alink --with-measure-units` (the core's
    // word for it; the hub answers only for its own demo units). On, the core
    // hears the hub's room and no microphone is opened here.
    Q_PROPERTY(bool simAvailable READ simAvailable NOTIFY simChanged)
    Q_PROPERTY(bool useSim READ useSim WRITE setUseSim NOTIFY simChanged)
    Q_PROPERTY(bool simOn READ simOn NOTIFY simChanged)
    Q_PROPERTY(QString simLabel READ simLabel NOTIFY simChanged)

    // ---- Words ----
    Q_PROPERTY(QString error READ error NOTIFY wordsChanged)
    Q_PROPERTY(QString errorKey READ errorKey NOTIFY wordsChanged)
    Q_PROPERTY(QString said READ said NOTIFY wordsChanged)
    // What is on its way ("" nothing): "Checking every unit", "Starting"...
    Q_PROPERTY(QString busy READ busy NOTIFY wordsChanged)

public:
    // The second Keep counts from a second after the core's refusal (the
    // core's rule; a double click is one press)
    static constexpr int kConfirmMs = 1000;

    // The calls on a core, by name (the smoke driver stands one in for the
    // result, the proposal and what follows): `op` is "unitCheck" (a, b: the
    // setup and the unit), "preflight", "start" (the setup and the checks),
    // "sumCheck", "verify" (the checks), "apply" (a: the proposal's id),
    // "keep", "revert", "applied", "result", "proposal", "state", "stop",
    // "simInfo", "simSet" (a: "1" on, "0" off).
    // Returns the JSON reply.
    using Calls = std::function<QByteArray(FfiCore *core, const QString &op, const QByteArray &a, const QByteArray &b)>;
    static QByteArray coreCall(FfiCore *core, const QString &op, const QByteArray &a, const QByteArray &b);

    UnitsMeasureController(const AppContext &ctx, QObject *parent = nullptr);
    ~UnitsMeasureController() override;
    static UnitsMeasureController *create(QQmlEngine *, QJSEngine *);

    QString unavailable() const { return m_unavailable; }
    QString unavailableKey() const { return m_unavailable.isEmpty() ? QString() : QStringLiteral("measure.system.needs_hub"); }
    QString role() const;
    bool admin() const { return role() == QLatin1String("admin"); }

    QString placement() const { return m_placement; }
    void setPlacement(const QString &p);
    QString placementProblem() const;
    QVariantList units() const { return m_units; }
    int sweepMs() const { return m_sweepMs; }
    void setSweepMs(int ms);
    int passes() const { return m_passes; }
    void setPasses(int n);
    QString anchor() const { return m_anchor; }
    void setAnchor(const QString &a);
    QVariantList anchors() const;
    QString runName() const { return m_runName; }
    void setRunName(const QString &n);
    QString blocker() const;
    QVariantList preflightLines() const { return m_preflightLines; }

    bool running() const { return m_starting || m_state.value(QLatin1String("active")).toBool(); }
    QString phase() const { return m_state.value(QLatin1String("phase")).toString(QStringLiteral("idle")); }
    QString purpose() const { return m_state.value(QLatin1String("purpose")).toString(QStringLiteral("measure")); }
    QString phaseText() const;
    int progress() const;
    QString runsText() const;
    QString issueText() const { return m_issueText; }
    QString issueKey() const { return m_issueKey; }

    bool hasResult() const { return !m_result.isEmpty(); }
    bool timingReliable() const { return m_result.value(QLatin1String("timingReliable")).toBool(); }
    QString timingText() const;
    QVariantList arrivals() const { return m_arrivals; }
    QVariantList delays() const { return m_delays; }
    QStringList reasons() const;
    QVariantMap graph() const { return m_graph; }

    bool hasProposal() const { return !m_proposal.isEmpty(); }
    QStringList proposalArrivals() const;
    QStringList withheld() const;
    QVariantList transitions() const;
    QString proposalNotice() const;
    QString guardNote() const { return m_proposal.value(QLatin1String("guardNote")).toString(); }
    bool sumCheckNeeded() const { return m_proposal.value(QLatin1String("sumCheckNeeded")).toBool(); }
    bool mayApply() const;
    QString applyWhy() const;
    QString applyWhyKey() const;

    QString stage() const { return m_applied.value(QLatin1String("stage")).toString(QStringLiteral("none")); }
    QString appliedText() const;
    bool verifyFailed() const;
    int guardRemainingS() const;
    QString guardText() const;
    bool keepQuestion() const { return m_question; }
    QString keepQuestionText() const { return m_questionText; }
    bool keepReady() const { return m_question && m_refusedAt.isValid() && m_refusedAt.elapsed() >= kConfirmMs; }
    int keepWaitS() const;
    int keepRefusals() const { return m_keepRefusals; }

    // -- review AT-D2 --
    bool simAvailable() const { return m_simAvailable; }
    bool useSim() const { return m_simAvailable && m_useSim; }
    void setUseSim(bool on);
    bool simOn() const { return m_simOn; }
    QString simLabel() const { return m_simLabel; }

    QString error() const { return m_error; }
    QString errorKey() const { return m_errorKey; }
    QString said() const { return m_said; }
    QString busy() const { return m_busy; }

    // The tab is shown: the hub's family and role, its units, what the core
    // still holds. The tab is left: whatever it started is stopped, the
    // units' connections closed.
    Q_INVOKABLE void enter();
    Q_INVOKABLE void leave();
    // The form: a unit in the group (its connection opens; its first output
    // chosen), an output of it and its role
    Q_INVOKABLE void setUnitChosen(const QString &serial, bool on);
    Q_INVOKABLE void setOutputOn(const QString &serial, int output, bool on);
    Q_INVOKABLE void setOutputRole(const QString &serial, int output, const QString &role);
    // Every unit's fresh check and the core's preflight; nothing plays
    Q_INVOKABLE void check();
    // The microphone, every unit's fresh check, the preflight, fresh checks
    // again, then the core's start
    Q_INVOKABLE void start();
    // Stop: system.measure.abort (every unit stopped), the microphone
    Q_INVOKABLE void stop();
    // The summed check (RC5f) and the verify after an apply: fresh checks,
    // the microphone, the runs
    Q_INVOKABLE void sumCheck();
    Q_INVOKABLE void verify();
    // Admin only; nothing is sent for another role
    Q_INVOKABLE void apply();
    Q_INVOKABLE void revert();
    // Keep: the core refuses it once after a verify that did not pass; the
    // question opens. confirmKeep is the person's answer to it (nothing
    // unless it is open); declineKeep closes it (the guard runs on).
    Q_INVOKABLE void keep();
    Q_INVOKABLE void confirmKeep();
    Q_INVOKABLE void declineKeep();
    Q_INVOKABLE void clearError();

    // ---- The smoke driver's seams ----
    // The calls on the cores (null: the core's own)
    void setCallsForTest(Calls calls) { m_calls = calls ? std::move(calls) : Calls(&UnitsMeasureController::coreCall); }
    // A session's state as the loop gives it (what the tick emits)
    void stateArrived(const QByteArray &json);
    // The hub's role as this console sees it ("" the Link session's)
    void setRoleForTest(const QString &role) {
        m_roleForTest = role;
        emit availabilityChanged();
        emit proposalChanged();
    }
    // Whether unit `serial` has its own connection open now
    bool unitOpen(const QString &serial) const { return m_cores.contains(serial); }

signals:
    void availabilityChanged();
    void formChanged();
    void preflightChanged();
    void stateChanged();
    void resultChanged();
    void proposalChanged();
    void appliedChanged();
    void guardChanged();
    void keepChanged();
    void wordsChanged();
    void simChanged();   // -- review AT-D2 --
    // A step finished (the smoke driver): "enter", "unit", "check", "start",
    // "sumCheck", "verify", "apply", "keep", "revert", and whether it went through
    void stepDone(const QString &step, bool ok);

private:
    struct UnitCore;
    using UnitPtr = std::shared_ptr<UnitCore>;

    MeasureController *measure() const;
    QString ownSerial() const;
    void refreshUnits();
    void openUnit(const QString &serial);
    void closeUnit(const QString &serial);
    void closeAll();
    QVariantMap unitRow(const QString &serial) const;
    void setUnitField(const QString &serial, const QString &field, const QVariant &value);
    QByteArray setupJson(int rateHz) const;
    // Every unit's fresh check, on the measurement loop, then `then(checks)`
    // there; a unit whose check fails names itself (the job's reply is then
    // that failure)
    using Job = std::function<QByteArray(FfiCore *main, const QByteArray &checks)>;
    void withChecks(const QString &step, const QByteArray &setup, Job then, std::function<void(const QJsonObject &)> done);
    void runs(const QString &step, const QString &op);
    void beginSession(const QJsonObject &state);
    void endSession();
    void loadResult(bool quiet);
    void setResult(const QJsonObject &r);
    void setProposal(const QJsonObject &p);
    void setApplied(const QJsonObject &a);
    void requestCurves();
    void keepNow();
    void keepAnswered(const QJsonObject &reply, const QJsonObject &applied);
    void disarm();
    void fail(const QJsonObject &reply, const QString &step = QString());
    void failWords(const QString &text, const QString &key = QString());
    void say(const QString &text);
    void setBusy(const QString &what);
    void clearWords();
    void onLoop(std::function<QByteArray(FfiCore *)> job, std::function<void(const QByteArray &)> done);
    void updateRunning();
    QString issueWords(const QJsonObject &issue) const;
    QString noticeTitle(const QString &key) const;
    QString outputLabel(const QString &serial, int output) const;
    QString arrivalWords(const QJsonObject &a) const;

    AppContext m_ctx;
    Calls m_calls = &UnitsMeasureController::coreCall;
    QThreadPool m_pool;   // the units' connections: open, close
    bool m_shutDown = false;
    bool m_entered = false;
    quint64 m_epoch = 0;   // bumped by Stop and by leaving: a start on its way knows
    QString m_unavailable;
    QString m_roleForTest;

    QString m_placement;
    QVariantList m_units;
    int m_sweepMs = 2000;
    int m_passes = 0;
    QString m_anchor;
    QString m_runName;
    QHash<QString, UnitPtr> m_cores;
    QVariantList m_preflightLines;

    QJsonObject m_state;
    bool m_starting = false;
    bool m_micOurs = false;
    QByteArray m_measured;   // the setup the last measurement ran with
    QString m_issueText, m_issueKey;

    QJsonObject m_result;
    QVariantList m_arrivals, m_delays;
    QVariantMap m_graph;
    quint64 m_curvesAsk = 0;
    QJsonObject m_proposal;
    QJsonObject m_applied;

    QTimer m_guardTimer;
    QElapsedTimer m_guardSince;
    int m_guardS = 0;

    bool m_question = false;
    QString m_questionText;
    QElapsedTimer m_refusedAt;
    QTimer m_keepTimer;   // the question's countdown, a second
    int m_keepToken = 0;  // which delayed Keep may still be sent
    int m_keepRefusals = 0;

    QString m_error, m_errorKey, m_said, m_busy;

    // -- review AT-D2 -- the demo hub's simulated microphone
    // The microphone of a run: the hub's simulated room where it is offered
    // and chosen (nothing opened here), else the Measure window's input
    void openMicForRun(std::function<void(bool, int)> done);
    bool m_simAvailable = false;
    bool m_useSim = true;   // the default where it is offered (as the web's)
    bool m_simOn = false;
    QString m_simLabel;
};

#endif // UNITSMEASURECONTROLLER_H
