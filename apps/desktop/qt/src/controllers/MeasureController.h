#ifndef MEASURECONTROLLER_H
#define MEASURECONTROLLER_H

// -- WP MDQT2c -- Tools > Measure… as QML and the other controllers see it: a
// facade over the active device's MeasureSession, where it all happens
// (MeasureSession.h). When another device becomes active, every property is
// that one's; what runs on the one left goes on there (`otherRuns`, with a Stop
// of its own, and a notice with the outcome). One microphone: while a run on one
// device has an input open, none is opened for another.

#include <QHash>
#include <QList>
#include <QObject>
#include <QPointer>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

#include "MeasureSession.h"
#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

class MeasureController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

    // ---- The device and who may play the generator ----
    Q_PROPERTY(bool connected READ connected NOTIFY deviceChanged)
    // Why this connection may not play the generator (read-only, the client
    // read-only mode, a hub's Viewer), or "": the passive microphone RTA, the
    // results and the documents only (PLAN-P5 section 1.5)
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
    // -- WP P5QUI2 -- the curves are the core's (dspi_measure_doc_curves, R1):
    // the smoothing asked for (1/N octave, 0 none; 1/6 by default), the
    // target levelled to each output (on by default), and the smoothing the
    // core's last answer has (-1 before it answered)
    Q_PROPERTY(int resultSmoothing READ resultSmoothing WRITE setResultSmoothing NOTIFY resultChanged)
    Q_PROPERTY(bool resultTarget READ resultTarget WRITE setResultTarget NOTIFY resultChanged)
    Q_PROPERTY(int resultCurvesSmoothing READ resultCurvesSmoothing NOTIFY resultChanged)

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

    // ---- The passive microphone RTA (-- WP P5QUI2 --; PLAN-P5 section 1.5) ----
    // Programme audio through the microphone, nothing played or written: what
    // a connection that may not play gets. It runs, or why it may not start
    // now ("" when it may: never beside a measurement, the live view or the
    // tune's runs)
    Q_PROPERTY(bool passiveActive READ passiveActive NOTIFY passiveChanged)
    Q_PROPERTY(QString passiveBlocker READ passiveBlocker NOTIFY runningChanged)
    Q_PROPERTY(QVariantMap passiveGraph READ passiveGraph NOTIFY passiveChanged)
    Q_PROPERTY(QVariantMap passiveCompareGraph READ passiveCompareGraph NOTIFY passiveChanged)
    Q_PROPERTY(bool passiveHeld READ passiveHeld NOTIFY passiveChanged)
    Q_PROPERTY(bool passiveReady READ passiveReady NOTIFY passiveChanged)
    Q_PROPERTY(int passiveFrames READ passiveFrames NOTIFY passiveChanged)
    Q_PROPERTY(QString passiveAveraging READ passiveAveraging NOTIFY passiveChanged)

    // ---- The documents ----
    // [{ id, kind, name, note, serial, saved, where ("local", "hub", "both") }]
    Q_PROPERTY(QVariantList documents READ documents NOTIFY documentsChanged)
    Q_PROPERTY(bool hubDocuments READ hubDocuments NOTIFY documentsChanged)
    Q_PROPERTY(QString documentsNote READ documentsNote NOTIFY documentsChanged)

    // ---- Words ----
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)
    Q_PROPERTY(QString said READ said NOTIFY errorChanged)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)

    // ---- Several devices (-- WP MDQT2c --) ----
    Q_PROPERTY(QString runningOn READ runningOn NOTIFY runningChanged)
    Q_PROPERTY(QVariantList otherRuns READ otherRuns NOTIFY otherRunsChanged)

public:
    MeasureController(const AppContext &ctx, QObject *parent = nullptr);
    ~MeasureController() override;
    static MeasureController *create(QQmlEngine *, QJSEngine *);

    // The active device's, made when it is first shown
    MeasureSession *session() const;
    QList<MeasureSession *> sessions() const { return m_sessions.values(); }
    // The one, but for `but`, whose input is open or being opened; null for none
    MeasureSession *capturing(const MeasureSession *but) const;
    // Something on a device that is not shown has an outcome: a notice names it
    void report(MeasureSession *on, const QString &text, bool warning);

    // Which device this window's runs are on, "" with one device open
    QString runningOn() const;
    // What runs on the other open devices: [{ id, title, text }]
    QVariantList otherRuns() const { return m_others; }
    Q_INVOKABLE void stopOther(const QString &id);
    void readTune(const QString &id);   // -- WP AIVIS --
    Q_INVOKABLE void showOther(const QString &id);
    // What was started stops, on every device
    Q_INVOKABLE void stopAll();
    // -- WP LAY5b -- A host shows the panel, or no longer does (a count: its
    // page and its window are two). While one does, the shown device's
    // measurement thread runs (MeasureSession::wake)
    Q_INVOKABLE void panelShown(bool shown);
    // For `panel`, whose host closes it. A window beside a page of the panel
    // (Modern) stops what runs on the device shown: the others' go on, with
    // their Stop on the page. Anywhere else nothing would show those any
    // more: every device's stops.
    Q_INVOKABLE void panelClosing(bool pageRemains);
    // -- WP RBFIX -- The devices stopAll() would stop something on ("" for none)
    QString runsOn() const;

    bool connected() const { return session()->connected(); }
    QString playBlocker() const { return session()->playBlocker(); }
    QVariantList outputs() const { return session()->outputs(); }

    bool simAvailable() const { return session()->simAvailable(); }
    QString simLabel() const { return session()->simLabel(); }
    QString micChoice() const { return session()->micChoice(); }
    void setMicChoice(const QString &choice) { session()->setMicChoice(choice); }
    QVariantList inputs() const { return session()->inputs(); }
    bool micOpen() const { return session()->micOpen(); }
    QString micSummary() const { return session()->micSummary(); }
    bool micCalibrated() const { return session()->micCalibrated(); }
    QByteArray micDocument() const { return session()->micDocument(); }

    QVariantList setupOutputs() const { return session()->setupOutputs(); }
    QString placement() const { return session()->placement(); }
    void setPlacement(const QString &p) { session()->setPlacement(p); }
    int count() const { return session()->count(); }
    void setCount(int n) { session()->setCount(n); }
    QString coupler() const { return session()->coupler(); }
    void setCoupler(const QString &c) { session()->setCoupler(c); }
    int sweepMs() const { return session()->sweepMs(); }
    void setSweepMs(int ms) { session()->setSweepMs(ms); }
    int passes() const { return session()->passes(); }
    void setPasses(int n) { session()->setPasses(n); }
    QString runName() const { return session()->runName(); }
    void setRunName(const QString &n) { session()->setRunName(n); }
    QString setupProblem() const { return session()->setupProblem(); }
    QVariantList preflightLines() const { return session()->preflightLines(); }
    bool preflightDone() const { return session()->preflightDone(); }
    bool preflightClear() const { return session()->preflightClear(); }

    QString phase() const { return session()->phase(); }
    QString phaseText() const { return session()->phaseText(); }
    int progress() const { return session()->progress(); }
    bool measuring() const { return session()->measuring(); }
    bool waitingPosition() const { return session()->waitingPosition(); }
    QString issueText() const { return session()->issueText(); }
    QString timingText() const { return session()->timingText(); }
    bool timingReliable() const { return session()->timingReliable(); }
    bool running() const { return session()->running(); }
    QString runningText() const { return session()->runningText(); }

    bool hasResult() const { return session()->hasResult(); }
    QByteArray resultDocument() const { return session()->resultDocument(); }
    QString resultName() const { return session()->resultName(); }
    QString resultSummary() const { return session()->resultSummary(); }
    QString resultTiming() const { return session()->resultTiming(); }
    QVariant resultTimingReliable() const { return session()->resultTimingReliable(); }
    QVariantList resultOutputs() const { return session()->resultOutputs(); }
    QVariantMap resultGraph() const { return session()->resultGraph(); }
    QString resultFrom() const { return session()->resultFrom(); }
    int resultSmoothing() const { return session()->resultSmoothing(); }
    void setResultSmoothing(int n) { session()->setResultSmoothing(n); }
    bool resultTarget() const { return session()->resultTarget(); }
    void setResultTarget(bool on) { session()->setResultTarget(on); }
    int resultCurvesSmoothing() const { return session()->resultCurvesSmoothing(); }

    bool passiveActive() const { return session()->passiveActive(); }
    QString passiveBlocker() const { return session()->passiveBlocker(); }
    QVariantMap passiveGraph() const { return session()->passiveGraph(); }
    QVariantMap passiveCompareGraph() const { return session()->passiveCompareGraph(); }
    bool passiveHeld() const { return session()->passiveHeld(); }
    bool passiveReady() const { return session()->passiveReady(); }
    int passiveFrames() const { return session()->passiveFrames(); }
    QString passiveAveraging() const { return session()->passiveAveraging(); }

    bool liveActive() const { return session()->liveActive(); }
    QString liveText() const { return session()->liveText(); }
    int liveSeq() const { return session()->liveSeq(); }
    QString liveMode() const { return session()->liveMode(); }
    int liveOutput() const { return session()->liveOutput(); }
    QString liveIssueText() const { return session()->liveIssueText(); }
    QVariantMap liveGraph() const { return session()->liveGraph(); }
    QVariantMap compareGraph() const { return session()->compareGraph(); }
    bool rtaHeld() const { return session()->rtaHeld(); }
    bool rtaReady() const { return session()->rtaReady(); }
    int rtaFrames() const { return session()->rtaFrames(); }
    bool liveMeasured() const { return session()->liveMeasured(); }
    double livePeakHz() const { return session()->livePeakHz(); }
    QVariantMap proposal() const { return session()->proposal(); }
    QString liveSaid() const { return session()->liveSaid(); }
    QString targetName() const { return session()->targetName(); }
    QByteArray targetDocument() const { return session()->targetDocument(); }

    QVariantList documents() const { return session()->documents(); }
    bool hubDocuments() const { return session()->hubDocuments(); }
    QString documentsNote() const { return session()->documentsNote(); }

    QString error() const { return session()->error(); }
    QString said() const { return session()->said(); }
    bool busy() const { return session()->busy(); }

    // ---- The microphone ----
    Q_INVOKABLE void refreshInputs() { session()->refreshInputs(); }
    // An open dialog, then importMicFile(); clearMic() goes back to relative dB
    Q_INVOKABLE void chooseMicFile() { session()->chooseMicFile(); }
    Q_INVOKABLE void importMicFile(const QString &path) { session()->importMicFile(path); }
    Q_INVOKABLE void clearMic() { session()->clearMic(); }

    // ---- The setup and the run ----
    Q_INVOKABLE void setOutputOn(int output, bool on) { session()->setOutputOn(output, on); }
    Q_INVOKABLE void setOutputRole(int output, const QString &role) { session()->setOutputRole(output, role); }
    Q_INVOKABLE void setOutputSpeaker(int output, const QString &speaker) { session()->setOutputSpeaker(output, speaker); }
    Q_INVOKABLE void setOutputGroup(int output, int group) { session()->setOutputGroup(output, group); }
    // The layout the device's state suggests (roles, speakers, groups)
    Q_INVOKABLE void proposeLayout() { session()->proposeLayout(); }
    // -- WP ROLEQ -- the rows from the stored speaker layout (its outputs
    // chosen), else the proposal; done when a device opens
    Q_INVOKABLE void fillFromLayout() { session()->fillFromLayout(); }
    Q_INVOKABLE void preflight() { session()->preflight(); }
    // Preflight, the microphone, then the run
    Q_INVOKABLE void start() { session()->start(); }
    Q_INVOKABLE void nextPosition() { session()->nextPosition(); }
    // Stop: the run, the live view and the tune's runs, the microphone
    Q_INVOKABLE void stop() { session()->stop(); }

    // ---- The result ----
    Q_INVOKABLE void setOutputShown(int output, bool shown) { session()->setOutputShown(output, shown); }
    // An open dialog, then openResultFile()
    Q_INVOKABLE void chooseResultFile() { session()->chooseResultFile(); }
    Q_INVOKABLE void openResultFile(const QString &path) { session()->openResultFile(path); }
    // Files the result in this console's store, or on the hub
    Q_INVOKABLE void saveResult() { session()->saveResult(); }
    Q_INVOKABLE void saveResultToHub() { session()->saveResultToHub(); }
    // A save dialog, then saveResultTo() (the document's text)
    Q_INVOKABLE void exportResult() { session()->exportResult(); }
    Q_INVOKABLE void saveResultTo(const QString &path) { session()->saveResultTo(path); }
    // Output `output`'s response as REW text: a save dialog, then exportRewTo()
    Q_INVOKABLE void exportRew(int output) { session()->exportRew(output); }
    Q_INVOKABLE void exportRewTo(int output, const QString &path) { session()->exportRewTo(output, path); }
    Q_INVOKABLE void clearResult() { session()->clearResult(); }

    // ---- The live view ----
    // { output, role, mode ("sweep", "pinkRta"), placement, coupler, smoothing,
    // averaging }
    Q_INVOKABLE void liveStart(const QVariantMap &setup) { session()->liveStart(setup); }
    Q_INVOKABLE void liveStop() { session()->liveStop(); }
    Q_INVOKABLE void cutPeak(double freqHz) { session()->cutPeak(freqHz); }
    Q_INVOKABLE void fitRange(double f1Hz, double f2Hz) { session()->fitRange(f1Hz, f2Hz); }
    // Writes the pending change as one undo step, or drops it
    Q_INVOKABLE void liveApply() { session()->liveApply(); }
    Q_INVOKABLE void liveDiscard() { session()->liveDiscard(); }
    Q_INVOKABLE void liveHold(bool on) { session()->liveHold(on); }
    Q_INVOKABLE void liveAveraging(const QString &averaging) { session()->liveAveraging(averaging); }
    // Edit > Undo (HistoryController): takes the last step back
    Q_INVOKABLE void undo() { session()->undo(); }
    // A target file (REW or CSV): an open dialog, then importTargetFile()
    Q_INVOKABLE void chooseTargetFile(const QString &use, const QString &coupler) { session()->chooseTargetFile(use, coupler); }
    Q_INVOKABLE void importTargetFile(const QString &path, const QString &use, const QString &coupler) { session()->importTargetFile(path, use, coupler); }
    Q_INVOKABLE void clearTarget() { session()->clearTarget(); }

    // ---- The passive microphone RTA (-- WP P5QUI2 --) ----
    // The chosen input (never the simulated microphone: it hears only the
    // generator), then the core's passive RTA; Stop (or stop()) ends it
    Q_INVOKABLE void passiveStart(const QString &averaging) { session()->passiveStart(averaging); }
    Q_INVOKABLE void passiveStop() { session()->passiveStop(); }
    Q_INVOKABLE void passiveHold(bool on) { session()->passiveHold(on); }
    Q_INVOKABLE void setPassiveAveraging(const QString &averaging) { session()->setPassiveAveraging(averaging); }
    // Reads the client read-only mode again (playBlocker); also on every snapshot
    Q_INVOKABLE void refreshAccess() { session()->refreshAccess(); }

    // ---- The documents ----
    Q_INVOKABLE void reloadDocuments() { session()->reloadDocuments(); }
    // Opens document `id` (a measurement as the result, a mic as the
    // calibration, a target as the target); `where` "hub" reads the hub's
    Q_INVOKABLE void openDocument(const QString &id, const QString &where) { session()->openDocument(id, where); }
    Q_INVOKABLE void renameDocument(const QString &id, const QString &where, const QString &name) { session()->renameDocument(id, where, name); }
    Q_INVOKABLE void noteDocument(const QString &id, const QString &where, const QString &note) { session()->noteDocument(id, where, note); }
    Q_INVOKABLE void deleteDocument(const QString &id, const QString &where) { session()->deleteDocument(id, where); }

    Q_INVOKABLE void clearError() { session()->clearError(); }

    // A JSON reply's failure in words (the notice's title when the table has
    // one, else the core's message)
    static QString failureText(const QJsonObject &reply) { return MeasureSession::failureText(reply); }
    // An issue {key, output, message} in words
    QString issueWords(const QJsonObject &issue) const { return session()->issueWords(issue); }
    QString outputName(int output) const { return session()->outputName(output); }
    void fail(const QString &text) { session()->fail(text); }
    void say(const QString &text) { session()->say(text); }
    void setResultDocument(const QByteArray &text, const QString &from) { session()->setResultDocument(text, from); }

    // A call's reply as text, the buffer grown as the call asks
    static QByteArray reply(const std::function<uint32_t(char *, uint32_t)> &call, int first = 1 << 16) { return MeasureSession::reply(call, first); }
    // A reply's JSON object (empty for one that is not)
    static QJsonObject object(const QByteArray &text) { return MeasureSession::object(text); }
    // A curve record ({freqsHz, magDb, valid, spl}) as a graph series
    static QVariantMap series(const QJsonObject &curve, const QString &kind, const QString &label, int colour = 0) {
        return MeasureSession::series(curve, kind, label, colour);
    }
    // The graph of `list`: { series, yMin, yMax, unit }
    static QVariantMap graph(const QVariantList &list, const QString &unit) { return MeasureSession::graph(list, unit); }

    // The smoke driver's view of the capture
    AudioCapture *capture() const { return session()->capture(); }
    // -- WP P5QUI2 -- the smoke driver's seam: the capture reads from this
    // source instead of an input (null: the inputs again)
    void setCaptureSource(dspi_capture::SourceFactory factory) { session()->setCaptureSource(std::move(factory)); }

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
    // -- WP MDQT2c --
    void otherRunsChanged();
    void runsChanged();   // -- WP RBFIX -- runsOn()
    // Another device is shown: a tab's own drafts start over (a placement chosen
    // for the last one is not assumed for it)
    void deviceSwitched();
    void sessionMade(MeasureSession *session);

private:
    MeasureSession *make(DeviceSession *device) const;
    template <class Signal>
    void relay(MeasureSession *from, Signal signal, void (MeasureController::*to)());
    void onSwitched();
    void checkOthers();
    QVariantList others() const;
    void drop(DeviceSession *device);
    void shutdown();
    MeasureSession *byId(const QString &id) const;
    // -- WP MCP5 -- an AI agent's request (AiController::measureRequested): run
    // on its device's session, shown as "Requested by AI · name"
    void agentRequest(const QJsonObject &ticket);

    AppContext m_ctx;
    mutable QHash<DeviceSession *, MeasureSession *> m_sessions;
    QPointer<DeviceSession> m_announced;   // the one the last change notices were for
    // The console's, for a device opened later: the input and the calibration
    QString m_input;
    QByteArray m_calibration;
    QString m_calibrationSummary;
    QVariantList m_others;
    bool m_shutDown = false;
    int m_panels = 0;   // -- WP LAY5b -- panelShown()
};

#endif // MEASURECONTROLLER_H
