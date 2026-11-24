#ifndef MEASURESESSION_H
#define MEASURESESSION_H

// -- WP MDQT2c -- One device's half of the Measure window (MeasureController
// until then, which is now a facade over the active device's): its setup, its
// run, its result, its live view and its own measurement thread, whichever
// device is active. GUI thread only.
//
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
// -- WP MDQT2c -- Another device becoming active: a run, the tune's runs and
// its guard go on here, on this device's core (the tab's busy dot, and the Stop
// bar of whichever device's window shows); what has no end of its own stops
// (the live view, the passive RTA), as the signal generator does.
// The device closed or replaced: whatever plays stops first, and an applied
// tune still in its guard goes back.
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
#include <atomic>
#include <functional>

#include "AudioCapture.h"
#include "ChannelListModel.h"
#include "DeviceSession.h"

class LinkSession;
class MeasureController;
class QThread;
class QTimer;
class SessionModel;
class TuneSession;
struct FfiCore;

// The measurement thread's object: the tick, the pushes and the session's calls
class MeasureLoop : public QObject {
    Q_OBJECT
public:
    // What the timer drives (set from any thread)
    std::atomic<bool> session{ false };   // dspi_measure_tick while active
    std::atomic<bool> live{ false };      // dspi_measure_live_tick instead
    std::atomic<bool> tune{ false };      // dspi_measure_tune_tick (runs and the guard)
    // -- WP P5QUI2 -- the passive microphone RTA: the blocks to
    // dspi_measure_rta_passive_push, its frame read about 4 times a second
    std::atomic<bool> passive{ false };
    // -- WP ATQT -- the several-units step's session on the hub
    // (dspi_measure_system_tick; the blocks to dspi_measure_system_capture_push)
    std::atomic<bool> system{ false };
    // -- WP MCP5 -- the low-end search (dspi_measure_rolloff_tick; its segments'
    // blocks to dspi_measure_capture_push)
    std::atomic<bool> rolloff{ false };
    // -- WP ADETQ -- the speaker detection (dspi_detect_tick; its sessions'
    // blocks to dspi_measure_capture_push)
    std::atomic<bool> detect{ false };

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
    // -- WP P5QUI2 -- the passive RTA's frame ({ok, state, rta})
    void passiveFrame(const QByteArray &json);
    // -- WP ATQT -- the several-units session's state (MeasureSystemState)
    void systemState(const QByteArray &json);
    // -- WP MCP5 -- the low-end search's state (MeasureRolloffState)
    void rolloffState(const QByteArray &json);
    // -- WP ADETQ -- the speaker detection's state (DetectState)
    void detectState(const QByteArray &json);

private:
    void tick();
    bool due(QByteArray *last, qint64 *at, const QByteArray &now, const char *key);

    FfiCore *m_core;
    QTimer *m_timer = nullptr;
    QByteArray m_lastSession, m_lastLive, m_lastTune;
    qint64 m_sessionAt = 0, m_liveAt = 0, m_tuneAt = 0;
    QByteArray m_liveKey;   // seq, pending, active, runs of the last frame read
    bool m_frameWanted = false;
    int m_passiveTicks = 0;   // -- WP P5QUI2 --
    QByteArray m_lastSystem;   // -- WP ATQT --
    qint64 m_systemAt = 0;     // -- WP ATQT --
    QByteArray m_lastRolloff;  // -- WP MCP5 --
    qint64 m_rolloffAt = 0;    // -- WP MCP5 --
    QByteArray m_lastDetect;   // -- WP ADETQ --
    qint64 m_detectAt = 0;     // -- WP ADETQ --
};

class MeasureSession : public QObject {
    Q_OBJECT
public:
    MeasureSession(DeviceSession *session, MeasureController *owner);
    ~MeasureSession() override;

    // -- WP MDQT2c --
    DeviceSession *session() const { return m_session; }
    SessionModel *model() const;
    MeasureController *owner() const { return m_owner; }
    TuneSession *tune() const { return m_tune; }
    // The device's own Link session, null for none
    LinkSession *link() const;
    // It is the active device's
    bool isActive() const;
    // Another device is about to become active: the live view and the passive
    // RTA end, and their starts still on the way
    void leaving();
    // It is the active device again: what others may have changed is read
    void entered();
    // Its device is about to be closed or replaced: whatever plays is stopped,
    // on the device's worker, before that
    void stopForClose();
    // The speaker layout of this device changed elsewhere (ROLEQ)
    void takeLayout(const QJsonObject &layout);
    // What the person chose or imported on another device: the input, the
    // microphone's calibration (the microphone is the console's, not a device's)
    void adoptInput(const QString &choice);
    void adoptCalibration(const QByteArray &document, const QString &summary);
    // The input is open or being opened for this device
    bool capturing() const;
    // It plays or listens, or is about to
    bool underWay() const;
    // The app quits: what it started stops, its thread ends
    void shutdown();

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

    // -- WP DL2 -- (SHOW 3) while an AI agent's measurement runs, the form
    // shows the agent's run (its outputs, placement and sweep), not the
    // person's choices, which stay as they were and come back after it
    QVariantList setupOutputs() const;
    QString placement() const;
    void setPlacement(const QString &p);
    int count() const;
    void setCount(int n);
    QString coupler() const { return m_coupler; }
    void setCoupler(const QString &c);
    int sweepMs() const;
    void setSweepMs(int ms);
    int passes() const;
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
    int resultSmoothing() const { return m_resultSmoothing; }
    void setResultSmoothing(int n);
    bool resultTarget() const { return m_resultTarget; }
    void setResultTarget(bool on);
    int resultCurvesSmoothing() const;

    bool passiveActive() const { return m_passiveOn; }
    QString passiveBlocker() const;
    QVariantMap passiveGraph() const { return m_passiveGraph; }
    QVariantMap passiveCompareGraph() const { return m_passiveCompareGraph; }
    bool passiveHeld() const { return m_passiveHeld; }
    bool passiveReady() const { return m_passiveReady; }
    int passiveFrames() const { return m_passiveFrames; }
    QString passiveAveraging() const { return m_passiveAveraging; }

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
    void refreshInputs();
    // An open dialog, then importMicFile(); clearMic() goes back to relative dB
    void chooseMicFile();
    void importMicFile(const QString &path);
    void clearMic();

    // ---- The setup and the run ----
    void setOutputOn(int output, bool on);
    void setOutputRole(int output, const QString &role);
    void setOutputSpeaker(int output, const QString &speaker);
    void setOutputGroup(int output, int group);
    // The layout the device's state suggests (roles, speakers, groups)
    void proposeLayout();
    // -- WP ROLEQ -- the rows from the stored speaker layout (its outputs
    // chosen), else the proposal; done when a device opens
    void fillFromLayout();
    void preflight();
    // Preflight, the microphone, then the run
    void start();
    void nextPosition();
    // Stop: the run, the live view and the tune's runs, the microphone
    void stop();
    // -- WP MCP5 -- An AI agent's request (a ticket from AiController): the
    // microphone as for start(), then the job (dspi_mcp_measure_begin: a
    // measurement, the low-end search, the tune's runs) driven as this view's
    // own and said as "Requested by AI · name". `done` once: started, or why not.
    void startForAgent(const QJsonObject &ticket, std::function<void(bool started, const QString &note)> done);
    // "AI · name" while an agent's request runs here, "" otherwise
    QString agentLabel() const { return m_agentLabel; }

    // ---- -- WP ADETQ -- The speaker detection (AUTODES) ----
    // The app's own: the microphone as for start(), then dspi_detect_start
    // with the chosen microphone's calibration; false (with error()) when it
    // cannot begin. An agent's comes through startForAgent (loop "detect").
    void startDetect();
    // Why the app's own detection cannot start now, "" when it can
    QString detectBlocker() const;
    bool detecting() const { return m_detectRunning; }
    // Where it stands in words: the Stop bar's ("Detecting the speakers: Out 3,
    // playing from 500 Hz at −50 dBFS."), and the agent strip's shorter one
    // ("Detecting speakers: Out 3 (1 of 2)"); "" while none runs
    QString detectText() const;
    QString detectStripText() const;

    // ---- The result ----
    void setOutputShown(int output, bool shown);
    // An open dialog, then openResultFile()
    void chooseResultFile();
    void openResultFile(const QString &path);
    // Files the result in this console's store, or on the hub
    void saveResult();
    void saveResultToHub();
    // A save dialog, then saveResultTo() (the document's text)
    void exportResult();
    void saveResultTo(const QString &path);
    // Output `output`'s response as REW text: a save dialog, then exportRewTo()
    void exportRew(int output);
    void exportRewTo(int output, const QString &path);
    void clearResult();

    // ---- The live view ----
    // { output, role, mode ("sweep", "pinkRta"), placement, coupler, smoothing,
    // averaging }
    void liveStart(const QVariantMap &setup);
    void liveStop();
    void cutPeak(double freqHz);
    void fitRange(double f1Hz, double f2Hz);
    // Writes the pending change as one undo step, or drops it
    void liveApply();
    void liveDiscard();
    void liveHold(bool on);
    void liveAveraging(const QString &averaging);
    // Edit > Undo (HistoryController): takes the last step back
    void undo();
    // A target file (REW or CSV): an open dialog, then importTargetFile()
    void chooseTargetFile(const QString &use, const QString &coupler);
    void importTargetFile(const QString &path, const QString &use, const QString &coupler);
    void clearTarget();

    // ---- The passive microphone RTA (-- WP P5QUI2 --) ----
    // The chosen input (never the simulated microphone: it hears only the
    // generator), then the core's passive RTA; Stop (or stop()) ends it
    void passiveStart(const QString &averaging);
    void passiveStop();
    void passiveHold(bool on);
    void setPassiveAveraging(const QString &averaging);
    // Reads the client read-only mode again (playBlocker); also on every snapshot
    void refreshAccess();
    // -- WP LAY5b -- Starts the measurement thread and makes its reads of the device
    void wake();
    bool awake() const { return m_awake; }

    // ---- The documents ----
    void reloadDocuments();
    // Opens document `id` (a measurement as the result, a mic as the
    // calibration, a target as the target); `where` "hub" reads the hub's
    void openDocument(const QString &id, const QString &where);
    void renameDocument(const QString &id, const QString &where, const QString &name);
    void noteDocument(const QString &id, const QString &where, const QString &note);
    void deleteDocument(const QString &id, const QString &where);

    void clearError();

    // ---- For TuneSession and UnitsMeasureController (same thread rules) ----
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
    // The graph of `series`: { series, yMin, yMax, unit }
    static QVariantMap graph(const QVariantList &series, const QString &unit);

    // The smoke driver's view of the capture
    AudioCapture *capture() const { return m_capture; }
    // -- WP P5QUI2 -- the smoke driver's seam: the capture reads from this
    // source instead of an input (null: the inputs again)
    void setCaptureSource(dspi_capture::SourceFactory factory) { m_captureSource = std::move(factory); }
    // -- WP ATQT -- the several-units step (UnitsMeasureController): while its
    // session plays, the Stop bar shows its words and Stop (or the window
    // closed) stops it too
    void setSystemRun(bool running, const QString &text, std::function<void()> stop);

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
    void passiveChanged();   // -- WP P5QUI2 --
    // A store or document call finished (the smoke driver): what and whether it went through
    void stored(const QString &op, bool ok);
    // -- WP MDQT2c -- the person chose an input or a calibration here
    void inputChosen(const QString &choice);
    void calibrationChosen();
    // Another device on this session (or none, or one again): what was chosen
    // for the last one is no choice for it
    void deviceSwitched();

private:
    void onSnapshot();
    bool gone() const { return m_shutDown || !m_session; }   // -- WP MDQT2c --
    void fillSetup(bool fromLayout);   // -- WP ROLEQ --
    void onSessionState(const QByteArray &json);
    void onLiveState(const QByteArray &json);
    void onLiveFrame(const QByteArray &json);
    void track(int delta);
    void storeCall(const QString &op, std::function<QByteArray(FfiCore *)> job,
                   std::function<void(const QByteArray &)> done = {});
    QByteArray setupJson(int rateHz) const;
    // -- WP ROLEQ -- whether it changed (a role, speaker or group change is
    // written back to the speaker layout)
    bool setOutputField(int output, const QString &field, const QVariant &value);
    void refreshSim();
    void takeResult();
    void fileImported(const QByteArray &doc);
    void propose(std::function<QByteArray(FfiCore *)> job);
    void showProposal(const QJsonObject &proposal);
    void setRunning();
    void setCalibration(const QByteArray &document, const QString &summary);   // -- WP MDQT2c --
    // -- WP P5QUI2 --
    void openCapture(std::function<void(bool ok, int rateHz)> done);
    void requestCurves();
    void buildResultGraph();
    void onPassiveFrame(const QByteArray &json);
    void passiveEnded();

    // -- WP MDQT2c -- Null once the session is gone
    QPointer<DeviceSession> m_session;
    MeasureController *m_owner;
    TuneSession *m_tune = nullptr;
    ChannelListModel m_channels;   // this device's, as DeviceModel.channels is the active one's
    QString m_lastSerial;          // the last device there was, over a disconnect
    bool m_starting = false;       // a run's start (the tune's too) is on its way
    bool m_liveStarting = false;
    QThread *m_thread = nullptr;
    bool m_awake = false;      // -- WP LAY5b -- m_thread runs (wake())
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
    // -- WP P5FOLLOW -- no default: the person chooses (headphone safety, S3)
    QString m_placement;
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
    // -- WP MCP5 -- the low-end search runs; "AI · name" while an agent's request does
    bool m_rolloffRunning = false;
    QString m_agentLabel;
    QJsonObject m_agentSetup;   // -- WP DL2 -- the agent's measurement's setup while it runs
    QString agentRunWords() const;
    void onRolloffState(const QByteArray &json);
    // -- WP ADETQ -- the speaker detection runs (the app's or an agent's), its last state
    bool m_detectRunning = false;
    QJsonObject m_detect;
    void onDetectState(const QByteArray &json);
    void detectEnded(const QJsonObject &state);
    void openDetectMic(quint64 epoch);

    QByteArray m_result;
    QString m_resultFrom;
    QVariantMap m_resultView;
    QVariantMap m_resultGraph;
    QList<int> m_hidden;
    // -- WP P5QUI2 -- the core's curves of the result, and which ask is the last
    QJsonObject m_curves;
    quint64 m_curvesAsk = 0;
    int m_resultSmoothing = 6;
    bool m_resultTarget = true;
    // -- WP P5QUI2 -- the passive RTA, and the client read-only mode
    bool m_passiveOn = false;
    bool m_passiveStarting = false;
    QVariantMap m_passiveGraph;
    QVariantMap m_passiveCompareGraph;
    bool m_passiveHeld = false;
    bool m_passiveReady = false;
    int m_passiveFrames = 0;
    QString m_passiveAveraging = QStringLiteral("slow");
    bool m_clientReadOnly = false;
    dspi_capture::SourceFactory m_captureSource;
    // -- WP ATQT --
    bool m_systemRunning = false;
    QString m_systemText;
    std::function<void()> m_systemStop;

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

    friend class TuneSession;
};

#endif // MEASURESESSION_H
