#ifndef MEASURECONTROLLER_H
#define MEASURECONTROLLER_H

// -- WP P5QUI -- Tools > Measure… (PLAN-P5 T1, T5, T7 to T9, T11; the web's
// P5WMS screens on Qt): the setup and the run, the results, the live view with
// its direct adjustments and the microphone RTA, and the documents. Every rule
// is the core's (dspi_measure_*: the preflight, the ramp, the aborts, the
// documents); this controller only drives the calls and shows what they say.
//
// Nothing is ever played by the host (S1): the stimulus is the DSPi's own
// generator. The microphone is P5QCAP's AudioCapture (capture only), or the
// demo DSPi's simulated microphone (P5DEMO), which is arithmetic in the core.
//
// Threads:
// - The measurement loop (MeasureLoop) is a thread of its own with a 50 ms
//   timer: it ticks the session, the live view or the tune (dspi_measure_tick,
//   _live_tick, _tune_tick), pushes the microphone's blocks
//   (dspi_measure_capture_push, in order, straight from the capture) and runs
//   the session's own calls (start, stop, next position, the summed check, the
//   verify, Keep). The core is internally locked; these calls are documented
//   "off the UI thread".
// - Writes that change the device's parameters (a live adjustment, a tune's
//   apply, its revert, the A/B) go through DeviceModel::write on the core
//   worker, as every write does, so the snapshot and the history follow.
// - The store's calls (documents, imports, export) run on the core worker.
// The GUI thread never waits on any of them.
//
// Stop is always offered while anything runs; leaving the app stops what this
// controller started (one STOP_NOW in the core) before the core goes.

#include <QByteArray>
#include <QJsonObject>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <atomic>
#include <functional>

#include "AudioCapture.h"
#include "Registrar.h"

class QJSEngine;
class QQmlEngine;
class QThread;
class QTimer;
struct FfiCore;

// The measurement thread's object: the tick, the pushes and the session's calls
class MeasureLoop : public QObject {
    Q_OBJECT
public:
    // What the timer drives (set from any thread)
    std::atomic<bool> session{ false };   // dspi_measure_tick while active
    std::atomic<bool> live{ false };      // dspi_measure_live_tick instead
    std::atomic<bool> tune{ false };      // dspi_measure_tune_tick (runs and the guard)

    explicit MeasureLoop(FfiCore *core) : m_core(core) {}
    FfiCore *core() const { return m_core; }
    // How often the timer ticks
    static constexpr int kTickMs = 50;

public slots:
    void begin();
    void push(const dspi_capture::Block &block);
    // Asks for the live view's frame on the next tick (after a proposal)
    void refreshFrame() { m_frameWanted = true; }
    // The end: whatever runs is stopped (one STOP_NOW each), the timer too
    void finish();

signals:
    // Each a JSON reply, emitted when it changed (at most about 5 a second
    // while only the progress moves)
    void sessionState(const QByteArray &json);
    void liveState(const QByteArray &json);
    void liveFrame(const QByteArray &json);
    void tuneState(const QByteArray &json);
    // A push the core refused (a null pointer, a block over its bound)
    void pushRefused(const QByteArray &json);

private:
    void tick();
    bool due(QByteArray *last, qint64 *at, const QByteArray &now, const char *key);

    FfiCore *m_core;
    QTimer *m_timer = nullptr;
    QByteArray m_lastSession, m_lastLive, m_lastTune;
    qint64 m_sessionAt = 0, m_liveAt = 0, m_tuneAt = 0;
    QByteArray m_liveKey;   // seq, pending, active, runs of the last frame read
    bool m_frameWanted = false;
};

class MeasureController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

    // ---- The device and who may play the generator ----
    Q_PROPERTY(bool connected READ connected NOTIFY deviceChanged)
    // Why this connection may not play the generator (read-only), or "":
    // the passive views and the documents only (PLAN-P5 section 1.5)
    Q_PROPERTY(QString playBlocker READ playBlocker NOTIFY deviceChanged)
    // The device's outputs: [{ output, label }]
    Q_PROPERTY(QVariantList outputs READ outputs NOTIFY deviceChanged)

    // ---- The microphone ----
    // The demo's simulated microphone is offered (the demo DSPi only)
    Q_PROPERTY(bool simAvailable READ simAvailable NOTIFY micChanged)
    Q_PROPERTY(QString simLabel READ simLabel NOTIFY micChanged)
    // The choice: "sim", or an input's id in hex ("" the system's default)
    Q_PROPERTY(QString micChoice READ micChoice WRITE setMicChoice NOTIFY micChanged)
    // The inputs: [{ id (hex), name, isDefault }]
    Q_PROPERTY(QVariantList inputs READ inputs NOTIFY micChanged)
    // The microphone is open now (a capture, or the simulated one on)
    Q_PROPERTY(bool micOpen READ micOpen NOTIFY micChanged)
    // The calibration (`mic` document) in use and its summary ("" none)
    Q_PROPERTY(QString micSummary READ micSummary NOTIFY micChanged)
    Q_PROPERTY(bool micCalibrated READ micCalibrated NOTIFY micChanged)

    // ---- The setup ----
    // [{ output, label, on, role, speaker, group }]
    Q_PROPERTY(QVariantList setupOutputs READ setupOutputs NOTIFY setupChanged)
    Q_PROPERTY(QString placement READ placement WRITE setPlacement NOTIFY setupChanged)
    Q_PROPERTY(int count READ count WRITE setCount NOTIFY setupChanged)
    Q_PROPERTY(QString coupler READ coupler WRITE setCoupler NOTIFY setupChanged)
    Q_PROPERTY(int sweepMs READ sweepMs WRITE setSweepMs NOTIFY setupChanged)
    Q_PROPERTY(int passes READ passes WRITE setPasses NOTIFY setupChanged)
    Q_PROPERTY(QString runName READ runName WRITE setRunName NOTIFY setupChanged)
    // What is wrong with the setup before the core is asked, or ""
    Q_PROPERTY(QString setupProblem READ setupProblem NOTIFY setupChanged)
    // The last preflight: [{ kind ("blocker", "warning"), key, text }], and clear
    Q_PROPERTY(QVariantList preflightLines READ preflightLines NOTIFY preflightChanged)
    Q_PROPERTY(bool preflightDone READ preflightDone NOTIFY preflightChanged)
    Q_PROPERTY(bool preflightClear READ preflightClear NOTIFY preflightChanged)

    // ---- The run ----
    // The session's phase ("idle", "preRoll", "ramp", "run", "analysing",
    // "waitPosition", "done", "aborted", "failed") and where it stands in words
    Q_PROPERTY(QString phase READ phase NOTIFY stateChanged)
    Q_PROPERTY(QString phaseText READ phaseText NOTIFY stateChanged)
    Q_PROPERTY(int progress READ progress NOTIFY stateChanged)
    Q_PROPERTY(bool measuring READ measuring NOTIFY stateChanged)
    Q_PROPERTY(bool waitingPosition READ waitingPosition NOTIFY stateChanged)
    // Why the run stopped, in words ("" none)
    Q_PROPERTY(QString issueText READ issueText NOTIFY stateChanged)
    // The run's timing verdict in words ("" before a walked run's analysis)
    // and whether it was reliable
    Q_PROPERTY(QString timingText READ timingText NOTIFY stateChanged)
    Q_PROPERTY(bool timingReliable READ timingReliable NOTIFY stateChanged)
    // Something of this window plays or listens: the Stop bar shows
    Q_PROPERTY(bool running READ running NOTIFY runningChanged)
    Q_PROPERTY(QString runningText READ runningText NOTIFY runningChanged)

    // ---- The result (a run's, or a file's / the store's) ----
    Q_PROPERTY(bool hasResult READ hasResult NOTIFY resultChanged)
    Q_PROPERTY(QString resultName READ resultName NOTIFY resultChanged)
    // Placement, positions, microphone, level, serial in one line
    Q_PROPERTY(QString resultSummary READ resultSummary NOTIFY resultChanged)
    // "timing reliable" / "timing unreliable" in words, "" without timing
    Q_PROPERTY(QString resultTiming READ resultTiming NOTIFY resultChanged)
    // true, false, or undefined (no timing: a single sweep)
    Q_PROPERTY(QVariant resultTimingReliable READ resultTimingReliable NOTIFY resultChanged)
    // [{ output, label, role, speaker, arrivalMs, colour, shown }]
    Q_PROPERTY(QVariantList resultOutputs READ resultOutputs NOTIFY resultChanged)
    // The graph: { series: [...], yMin, yMax, unit } (MeasureGraph.qml)
    Q_PROPERTY(QVariantMap resultGraph READ resultGraph NOTIFY resultChanged)
    // Where it came from ("run", a file's name, "store", "hub")
    Q_PROPERTY(QString resultFrom READ resultFrom NOTIFY resultChanged)

    // ---- The live view ----
    Q_PROPERTY(bool liveActive READ liveActive NOTIFY liveChanged)
    Q_PROPERTY(QString liveText READ liveText NOTIFY liveChanged)
    Q_PROPERTY(int liveSeq READ liveSeq NOTIFY liveChanged)
    Q_PROPERTY(QString liveMode READ liveMode NOTIFY liveChanged)
    Q_PROPERTY(int liveOutput READ liveOutput NOTIFY liveChanged)
    Q_PROPERTY(QString liveIssueText READ liveIssueText NOTIFY liveChanged)
    Q_PROPERTY(QVariantMap liveGraph READ liveGraph NOTIFY liveChanged)
    // The RTA's compare with its held copy, a graph like liveGraph (empty: none)
    Q_PROPERTY(QVariantMap compareGraph READ compareGraph NOTIFY liveChanged)
    Q_PROPERTY(bool rtaHeld READ rtaHeld NOTIFY liveChanged)
    Q_PROPERTY(bool rtaReady READ rtaReady NOTIFY liveChanged)
    Q_PROPERTY(int rtaFrames READ rtaFrames NOTIFY liveChanged)
    Q_PROPERTY(bool liveMeasured READ liveMeasured NOTIFY liveChanged)
    // The highest trusted point of the measured curve, 100 Hz to 5 kHz (0 none)
    Q_PROPERTY(double livePeakHz READ livePeakHz NOTIFY liveChanged)
    // The pending change: { id, label, changes: [words], gainText, rmsText,
    // reason ("" when it changes something) }; empty when none
    Q_PROPERTY(QVariantMap proposal READ proposal NOTIFY proposalChanged)
    // The last thing an adjustment said ("Applied: ...")
    Q_PROPERTY(QString liveSaid READ liveSaid NOTIFY proposalChanged)
    // The imported target in use ("" the placement's default, our own curve)
    Q_PROPERTY(QString targetName READ targetName NOTIFY targetChanged)

    // ---- The documents ----
    // [{ id, kind, name, note, serial, saved, where ("local", "hub", "both") }]
    Q_PROPERTY(QVariantList documents READ documents NOTIFY documentsChanged)
    Q_PROPERTY(bool hubDocuments READ hubDocuments NOTIFY documentsChanged)
    Q_PROPERTY(QString documentsNote READ documentsNote NOTIFY documentsChanged)

    // ---- Words ----
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)
    Q_PROPERTY(QString said READ said NOTIFY errorChanged)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)

public:
    MeasureController(const AppContext &ctx, QObject *parent = nullptr);
    ~MeasureController() override;
    static MeasureController *create(QQmlEngine *, QJSEngine *);

    bool connected() const;
    QString playBlocker() const;
    QVariantList outputs() const { return m_outputs; }

    bool simAvailable() const { return m_simAvailable; }
    QString simLabel() const { return m_simLabel; }
    QString micChoice() const { return m_micChoice; }
    void setMicChoice(const QString &choice);
    QVariantList inputs() const { return m_inputs; }
    bool micOpen() const;
    QString micSummary() const { return m_micSummary; }
    bool micCalibrated() const { return !m_micDocument.isEmpty(); }
    QByteArray micDocument() const { return m_micDocument; }

    QVariantList setupOutputs() const { return m_setupOutputs; }
    QString placement() const { return m_placement; }
    void setPlacement(const QString &p);
    int count() const { return m_count; }
    void setCount(int n);
    QString coupler() const { return m_coupler; }
    void setCoupler(const QString &c);
    int sweepMs() const { return m_sweepMs; }
    void setSweepMs(int ms);
    int passes() const { return m_passes; }
    void setPasses(int n);
    QString runName() const { return m_runName; }
    void setRunName(const QString &n);
    QString setupProblem() const;
    QVariantList preflightLines() const { return m_preflightLines; }
    bool preflightDone() const { return m_preflightDone; }
    bool preflightClear() const { return m_preflightClear; }

    QString phase() const;
    QString phaseText() const;
    int progress() const;
    bool measuring() const;
    bool waitingPosition() const { return phase() == QLatin1String("waitPosition"); }
    QString issueText() const;
    QString timingText() const;
    bool timingReliable() const;
    bool running() const;
    QString runningText() const;

    bool hasResult() const { return !m_result.isEmpty(); }
    QByteArray resultDocument() const { return m_result; }
    QString resultName() const { return m_resultView.value(QStringLiteral("name")).toString(); }
    QString resultSummary() const { return m_resultView.value(QStringLiteral("summary")).toString(); }
    QString resultTiming() const { return m_resultView.value(QStringLiteral("timing")).toString(); }
    QVariant resultTimingReliable() const { return m_resultView.value(QStringLiteral("reliable")); }
    QVariantList resultOutputs() const { return m_resultView.value(QStringLiteral("outputs")).toList(); }
    QVariantMap resultGraph() const { return m_resultGraph; }
    QString resultFrom() const { return m_resultFrom; }

    bool liveActive() const;
    QString liveText() const;
    int liveSeq() const { return m_live.value(QLatin1String("seq")).toInt(); }
    QString liveMode() const { return m_live.value(QLatin1String("mode")).toString(QStringLiteral("sweep")); }
    int liveOutput() const { return m_live.value(QLatin1String("output")).toInt(); }
    QString liveIssueText() const;
    QVariantMap liveGraph() const { return m_liveGraph; }
    QVariantMap compareGraph() const { return m_compareGraph; }
    bool rtaHeld() const { return m_rtaHeld; }
    bool rtaReady() const { return m_rtaReady; }
    int rtaFrames() const { return m_rtaFrames; }
    bool liveMeasured() const { return m_liveMeasured; }
    double livePeakHz() const { return m_livePeakHz; }
    QVariantMap proposal() const { return m_proposal; }
    QString liveSaid() const { return m_liveSaid; }
    QString targetName() const { return m_targetName; }
    QByteArray targetDocument() const { return m_targetDocument; }

    QVariantList documents() const { return m_documents; }
    bool hubDocuments() const { return m_hubDocuments; }
    QString documentsNote() const { return m_documentsNote; }

    QString error() const { return m_error; }
    QString said() const { return m_said; }
    bool busy() const { return m_busy > 0; }

    // ---- The microphone ----
    Q_INVOKABLE void refreshInputs();
    // An open dialog, then importMicFile(); clearMic() goes back to relative dB
    Q_INVOKABLE void chooseMicFile();
    Q_INVOKABLE void importMicFile(const QString &path);
    Q_INVOKABLE void clearMic();

    // ---- The setup and the run ----
    Q_INVOKABLE void setOutputOn(int output, bool on);
    Q_INVOKABLE void setOutputRole(int output, const QString &role);
    Q_INVOKABLE void setOutputSpeaker(int output, const QString &speaker);
    Q_INVOKABLE void setOutputGroup(int output, int group);
    // The layout the device's state suggests (roles, speakers, groups)
    Q_INVOKABLE void proposeLayout();
    Q_INVOKABLE void preflight();
    // Preflight, the microphone, then the run
    Q_INVOKABLE void start();
    Q_INVOKABLE void nextPosition();
    // Stop: the run, the live view and the tune's runs, the microphone
    Q_INVOKABLE void stop();

    // ---- The result ----
    Q_INVOKABLE void setOutputShown(int output, bool shown);
    // An open dialog, then openResultFile()
    Q_INVOKABLE void chooseResultFile();
    Q_INVOKABLE void openResultFile(const QString &path);
    // Files the result in this console's store, or on the hub
    Q_INVOKABLE void saveResult();
    Q_INVOKABLE void saveResultToHub();
    // A save dialog, then saveResultTo() (the document's text)
    Q_INVOKABLE void exportResult();
    Q_INVOKABLE void saveResultTo(const QString &path);
    // Output `output`'s response as REW text: a save dialog, then exportRewTo()
    Q_INVOKABLE void exportRew(int output);
    Q_INVOKABLE void exportRewTo(int output, const QString &path);
    Q_INVOKABLE void clearResult();

    // ---- The live view ----
    // { output, role, mode ("sweep", "pinkRta"), placement, coupler, smoothing,
    // averaging }
    Q_INVOKABLE void liveStart(const QVariantMap &setup);
    Q_INVOKABLE void liveStop();
    Q_INVOKABLE void cutPeak(double freqHz);
    Q_INVOKABLE void fitRange(double f1Hz, double f2Hz);
    // Writes the pending change as one undo step, or drops it
    Q_INVOKABLE void liveApply();
    Q_INVOKABLE void liveDiscard();
    Q_INVOKABLE void liveHold(bool on);
    Q_INVOKABLE void liveAveraging(const QString &averaging);
    // Edit > Undo (HistoryController): takes the last step back
    Q_INVOKABLE void undo();
    // A target file (REW or CSV): an open dialog, then importTargetFile()
    Q_INVOKABLE void chooseTargetFile(const QString &use, const QString &coupler);
    Q_INVOKABLE void importTargetFile(const QString &path, const QString &use, const QString &coupler);
    Q_INVOKABLE void clearTarget();

    // ---- The documents ----
    Q_INVOKABLE void reloadDocuments();
    // Opens document `id` (a measurement as the result, a mic as the
    // calibration, a target as the target); `where` "hub" reads the hub's
    Q_INVOKABLE void openDocument(const QString &id, const QString &where);
    Q_INVOKABLE void renameDocument(const QString &id, const QString &where, const QString &name);
    Q_INVOKABLE void noteDocument(const QString &id, const QString &where, const QString &note);
    Q_INVOKABLE void deleteDocument(const QString &id, const QString &where);

    Q_INVOKABLE void clearError();

    // ---- For TuneController (same thread rules) ----
    MeasureLoop *loop() const { return m_loop; }
    // Opens the chosen microphone; `done(ok, rateHz)` on the GUI thread
    void openMic(std::function<void(bool ok, int rateHz)> done);
    void closeMic();
    // Runs `job` on the measurement thread, then `done(result)` here while
    // `ctx` lives. The job gets the core (never null).
    void onLoop(QObject *ctx, std::function<QByteArray(FfiCore *)> job, std::function<void(const QByteArray &)> done);
    // A JSON reply's failure in words (the notice's title when the table has
    // one, else the core's message)
    static QString failureText(const QJsonObject &reply);
    // An issue {key, output, message} in words
    QString issueWords(const QJsonObject &issue) const;
    QString outputName(int output) const;
    void fail(const QString &text);
    void say(const QString &text);
    void setResultDocument(const QByteArray &text, const QString &from);

    // A call's reply as text, the buffer grown as the call asks
    static QByteArray reply(const std::function<uint32_t(char *, uint32_t)> &call, int first = 1 << 16);
    // A reply's JSON object (empty for one that is not)
    static QJsonObject object(const QByteArray &text);
    // A curve record ({freqsHz, magDb, valid, spl}) as a graph series
    static QVariantMap series(const QJsonObject &curve, const QString &kind, const QString &label, int colour = 0);
    // A stored curve ({grid, mag, valid, spl, spread?}, docs/integrator/measurement.md 7.2)
    // decoded to a curve record; empty when it does not read
    static QJsonObject decodeCurve(const QJsonObject &stored);
    // The graph of `series`: { series, yMin, yMax, unit }
    static QVariantMap graph(const QVariantList &series, const QString &unit);

    // The smoke driver's view of the capture
    AudioCapture *capture() const { return m_capture; }

signals:
    void deviceChanged();
    void micChanged();
    void setupChanged();
    void preflightChanged();
    void stateChanged();
    void runningChanged();
    void resultChanged();
    void liveChanged();
    void proposalChanged();
    void targetChanged();
    void documentsChanged();
    void errorChanged();
    void busyChanged();
    // A store or document call finished (the smoke driver): what and whether it went through
    void stored(const QString &op, bool ok);

private:
    void onSnapshot();
    void onSessionState(const QByteArray &json);
    void onLiveState(const QByteArray &json);
    void onLiveFrame(const QByteArray &json);
    void track(int delta);
    void storeCall(const QString &op, std::function<QByteArray(FfiCore *)> job,
                   std::function<void(const QByteArray &)> done = {});
    QByteArray setupJson(int rateHz) const;
    void setOutputField(int output, const QString &field, const QVariant &value);
    void refreshSim();
    void takeResult();
    void fileImported(const QByteArray &doc);
    void propose(std::function<QByteArray(FfiCore *)> job);
    void showProposal(const QJsonObject &proposal);
    void setRunning();
    void shutdown();

    AppContext m_ctx;
    QThread *m_thread = nullptr;
    MeasureLoop *m_loop = nullptr;
    AudioCapture *m_capture = nullptr;
    bool m_shutDown = false;
    quint64 m_stopEpoch = 0;   // review P5-D: counts the Stops (a start begun before one is withdrawn)
    QString m_serial;

    QVariantList m_outputs;
    bool m_simAvailable = false;
    bool m_simOn = false;
    QString m_simLabel;
    QString m_micChoice;
    bool m_micChosen = false;   // the person chose (the demo's default is the simulated one)
    QVariantList m_inputs;
    QByteArray m_micDocument;
    QString m_micSummary;
    std::function<void(bool, int)> m_micOpening;

    QVariantList m_setupOutputs;
    QString m_placement = QStringLiteral("listeningPosition");
    int m_count = 3;
    QString m_coupler = QStringLiteral("iec60318_4");
    int m_sweepMs = 5500;
    int m_passes = 3;
    QString m_runName;
    QVariantList m_preflightLines;
    bool m_preflightDone = false;
    bool m_preflightClear = false;

    QJsonObject m_state;
    bool m_wasActive = false;
    bool m_wasRunning = false;
    bool m_tuneRunning = false;
    // The session's last run was the tune's (its result is not the window's)
    bool m_tuneSession = false;

    QByteArray m_result;
    QString m_resultFrom;
    QVariantMap m_resultView;
    QVariantMap m_resultGraph;
    QList<int> m_hidden;

    QJsonObject m_live;
    QJsonObject m_frame;
    QVariantMap m_liveGraph;
    QVariantMap m_compareGraph;
    bool m_rtaHeld = false;
    bool m_rtaReady = false;
    int m_rtaFrames = 0;
    bool m_liveMeasured = false;
    double m_livePeakHz = 0;
    QJsonObject m_pending;   // the proposal as the core gave it
    QVariantMap m_proposal;
    QString m_liveSaid;
    QByteArray m_targetDocument;
    QString m_targetName;

    QVariantList m_documents;
    bool m_hubDocuments = false;
    QString m_documentsNote;

    QString m_error;
    QString m_said;
    int m_busy = 0;

    friend class TuneController;
};

#endif // MEASURECONTROLLER_H
