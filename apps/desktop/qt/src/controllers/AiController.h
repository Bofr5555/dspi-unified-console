#ifndef AICONTROLLER_H
#define AICONTROLLER_H

#include <QDateTime>
#include <QElapsedTimer>   // -- WP INTI --
#include <QJsonArray>
#include <QJsonObject>
#include <QMutex>           // -- WP INTG --
#include <QObject>
#include <QSet>
#include <QString>
#include <QStringList>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>
#include <QWaitCondition>   // -- WP INTG --
#include <QtQml/qqmlregistration.h>
#include <atomic>
#include <functional>

#include "CoreApi.h"
#include "Registrar.h"

class DeviceSession;
class QJSEngine;
class QQmlEngine;

// -- WP MCP4 -- AI agents (Settings > AI Agents, the Classic tool window and
// status icon): the MCP server in this process (dspi_mcp_*, over the app's set of
// devices), which agents on this computer connect to. QML sees it as the
// singleton `AiController`. LAY8 builds the Modern popover on it.
//
// - Off by default (mcp/enabled). The server listens exactly while it is on and
//   the start did not fail; a failed start leaves the switch on with `lastError`.
// - What an agent may do is the core's and dspi-mcp's policy; this class only
//   passes the user's setting (`confirmMode`), shows what waits for the user
//   (`confirmations`, `pairing`) and gives the answers.
// - Every dspi_mcp_* call that may wait for an agent's call in progress runs in
//   order on a thread of this controller's, never on the GUI thread. The
//   server's events come on its own threads and only queue a read here.
// - The server keeps its own clocks: nothing is polled. The timer here counts
//   the shown seconds down and lets `recent` fade.
class AiController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The page's switch, stored as mcp/enabled
    Q_PROPERTY(bool enabled READ enabled NOTIFY stateChanged)
    Q_PROPERTY(bool running READ running NOTIFY stateChanged)
    // Starting or stopping
    Q_PROPERTY(bool busy READ busy NOTIFY stateChanged)
    // Why the last start failed, "" otherwise
    Q_PROPERTY(QString lastError READ lastError NOTIFY stateChanged)
    // The user's pause: agents' calls fail and change nothing
    Q_PROPERTY(bool paused READ paused NOTIFY stateChanged)
    // The port agents are told (mcp/port; the stdio relay reads the same key)
    Q_PROPERTY(int port READ port NOTIFY settingsChanged)
    // While running: the addresses listened on, loopback only
    Q_PROPERTY(QStringList addresses READ addresses NOTIFY stateChanged)
    // "off", "starting", "error", "paused", "on", "active" (an agent called within
    // the last minute), "asking" (something waits for the user)
    Q_PROPERTY(QString state READ state NOTIFY stateChanged)
    // The state in words, for the page, the window and the icon's tooltip
    Q_PROPERTY(QString statusLine READ statusLine NOTIFY stateChanged)
    // "ask_everything", "ask_risky" (the default) or "never_ask" (mcp/confirm)
    Q_PROPERTY(QString confirmMode READ confirmMode WRITE setConfirmMode NOTIFY settingsChanged)
    // [{ id, name, created, lastUsed (seconds since the epoch, 0 never) }]
    Q_PROPERTY(QVariantList clients READ clients NOTIFY clientsChanged)
    // -- WP LAY8 -- The clients that called within the last minute, in the list's
    // order: [{ id, name, since (a time, "14:02") }]
    Q_PROPERTY(QVariantList agents READ agents NOTIFY agentsChanged)
    // The relay that asks to pair: { id, name, code, secondsLeft }, {} for none
    Q_PROPERTY(QVariantMap pairing READ pairing NOTIFY pairingChanged)
    // The changes that wait for the user, oldest first: [{ id, client, title,
    // device, risks: [text], riskLines: [text with its kind's label] }]
    Q_PROPERTY(QVariantList confirmations READ confirmations NOTIFY confirmationsChanged)
    // How long the oldest of them waits for the answer still
    Q_PROPERTY(int confirmationSecondsLeft READ confirmationSecondsLeft NOTIFY secondsChanged)
    // What the agents did, newest first: [{ key, time, client, title, detail,
    // device, write, outcome, outcomeText, undo, undoNote }]; `undo` is "" (nothing
    // to undo), "can", "busy", "undone", "changed" (someone changed it since) or
    // "closed" (the device is not open)
    Q_PROPERTY(QVariantList activity READ activity NOTIFY activityChanged)
    // Reads are listed too (they are left out otherwise)
    Q_PROPERTY(bool showReads READ showReads WRITE setShowReads NOTIFY activityChanged)
    Q_PROPERTY(bool canUndoAny READ canUndoAny NOTIFY activityChanged)
    // A client just added: { id, name, token }, {} once put away. Shown once.
    Q_PROPERTY(QVariantMap newClient READ newClient NOTIFY newClientChanged)
    // What to put into an agent's configuration
    Q_PROPERTY(QString relayCommand READ relayCommand CONSTANT)
    Q_PROPERTY(QString desktopConfig READ desktopConfig CONSTANT)
    Q_PROPERTY(QString codeCommand READ codeCommand CONSTANT)
    // -- WP AIVIS -- An agent's call or job runs now, or one ran a moment ago, or one
    // waits for the person; not while paused
    Q_PROPERTY(bool working READ working NOTIFY workChanged)
    // What to say of it: { shown, agent (its name), text ("Changing the gain, OUT3
    // Centre · Demo"), device (the title), deviceId, elsewhere (that device is open
    // but not the one shown), job (a measurement of an agent's plays) }
    Q_PROPERTY(QVariantMap work READ work NOTIFY workChanged)
    // What agents changed in the last moments: [{ device, path, indices }]
    Q_PROPERTY(QVariantList recentTouches READ recentTouches NOTIFY recentTouchesChanged)
    Q_PROPERTY(QString url READ url NOTIFY settingsChanged)
    Q_PROPERTY(QString httpCommand READ httpCommand NOTIFY newClientChanged)
    Q_PROPERTY(QString otherComputerUrl READ otherComputerUrl CONSTANT)
    // -- WP FEATUI -- The app's SPL cap for agents' measurements, dB SPL (a hard
    // ceiling an agent may only stay under, also under "never ask"); and the
    // USB devices remembered as opened here, across restarts ([{ device, name }])
    Q_PROPERTY(int agentSplCap READ agentSplCap WRITE setAgentSplCap NOTIFY settingsChanged)
    // -- WP INTG -- (RE1) the cap on or off: on, an agent's measurement needs a
    // microphone calibration that reads dB SPL (else it is refused); off, the
    // level ceiling holds
    Q_PROPERTY(bool agentSplCapOn READ agentSplCapOn WRITE setAgentSplCapOn NOTIFY settingsChanged)
    // -- WP ADETQ -- Under "never ask", agents may detect the speakers
    // (dspi_mcp_set_agent_detect; off by default, kept in the settings): the
    // page shows detectWarning once before it turns on (AUTODES 4.3)
    Q_PROPERTY(bool agentDetect READ agentDetect WRITE setAgentDetect NOTIFY settingsChanged)
    // -- WP ROLES -- Settings › AI Agents › Agents may set speaker roles from my
    // words (mcp/agentRoles, off by default): under "never ask" an agent's
    // dspi_set_speaker_roles is refused unless it is on
    Q_PROPERTY(bool agentRoles READ agentRoles WRITE setAgentRoles NOTIFY settingsChanged)
    // -- WP ADETQ -- The speaker detection's warning on the mechanism, as the
    // agent's question has it (dspi-mcp's DETECT_WARNING)
    Q_PROPERTY(QString detectWarning READ detectWarning CONSTANT)
    Q_PROPERTY(QVariantList rememberedDevices READ rememberedDevices NOTIFY rememberedChanged)
    // -- WP ODEC -- What the user allowed for a while (one question for an agent's
    // test signals on a device, ten minutes): [{ id, agent, device (its title),
    // deviceId, secondsLeft, left ("9:41") }], counted down here between reads
    Q_PROPERTY(QVariantList grants READ grants NOTIFY grantsChanged)
    // -- WP ODEC -- The screens follow what an agent changes (the device, and
    // Modern's section), never while the user is at a control or a dialog. On by
    // default; not stored (the smoke driver's older sections turn it off).
    Q_PROPERTY(bool followAgent READ followAgent WRITE setFollowAgent NOTIFY followAgentChanged)
public:
    // -- WP FEATUI -- The SPL cap's range and default (dB SPL): below the
    // core's 95 dB default for a speaker
    static constexpr int kSplCapMin = 60;
    static constexpr int kSplCapMax = 95;
    static constexpr int kSplCapDefault = 85;
    static constexpr int kRememberedMax = 64;   // dspi-mcp's REMEMBERED_MAX
    static constexpr int kDefaultPort = 47860;
    static constexpr int kPortMin = 1024;
    static constexpr int kPairingSeconds = 120;    // dspi-mcp's PAIRING_TTL_MS
    static constexpr int kRecentSeconds = 60;
    static constexpr int kActivityShown = 200;
    // -- WP AIVIS -- How long after its last call an agent counts as working, and how
    // long the strip says what it did last
    static constexpr int kWorkingMs = 8000;
    static constexpr int kLingerMs = 20000;
    static constexpr int kTouchMs = 3000;
    static constexpr const char *kPortKey = "mcp/port";
    // -- WP ODEC -- At most one switch to follow an agent in this many ms, and
    // not again to the same device and section within kFollowSameMs
    static constexpr int kFollowMs = 3000;
    static constexpr int kFollowSameMs = 60000;
    // -- WP INTI -- (review RF7) no follow within this long after the user's last key,
    // click or wheel, nor while a mouse button is held (a click acts on release)
    static constexpr int kInputIdleMs = 1000;

    AiController(const AppContext &ctx, QObject *parent);
    ~AiController() override;
    static AiController *create(QQmlEngine *, QJSEngine *);

    bool enabled() const { return m_enabled; }
    bool running() const { return m_running; }
    bool busy() const { return m_busy; }
    QString lastError() const { return m_lastError; }
    bool paused() const { return m_paused; }
    int port() const { return m_port; }
    QStringList addresses() const { return m_running ? m_addresses : QStringList(); }
    QString state() const;
    QString statusLine() const;
    QString confirmMode() const { return m_confirmMode; }
    void setConfirmMode(const QString &mode);
    // -- WP FEATUI --
    int agentSplCap() const { return m_agentSplCap; }
    void setAgentSplCap(int db);
    bool agentSplCapOn() const { return m_agentSplCapOn; }   // -- WP INTG --
    void setAgentSplCapOn(bool on);
    bool agentDetect() const { return m_agentDetect; }   // -- WP ADETQ --
    bool agentRoles() const { return m_agentRoles; }     // -- WP ROLES --
    void setAgentRoles(bool on);                         // -- WP ROLES --
    void setAgentDetect(bool on);
    static QString detectWarning();
    QVariantList rememberedDevices() const { return m_remembered.toVariantList(); }
    Q_INVOKABLE void forgetRememberedDevices();
    // What the start reads: the remembered devices from the settings, handed to
    // the server (also the smoke driver's "a restart")
    void loadRemembered();
    // The server's handle, for a controller that reads through it on a thread of its own
    FfiMcp *mcpHandle() const { return m_mcp; }
    // -- WP INTG -- (review RE20) The handle for another controller's thread
    // (HostAudioController's read): nullptr once shutting down, else held
    // until releaseHandle(), which that thread calls when done; shutdown()
    // frees the handle only after every hold is released.
    FfiMcp *holdHandle();
    void releaseHandle();
    int handleHolds() {
        QMutexLocker lock(&m_holdMutex);
        return m_holds;
    }
    QVariantList clients() const { return m_clients; }
    QVariantList agents() const;   // -- WP LAY8 --
    QVariantMap pairing() const;
    QVariantList confirmations() const;
    int confirmationSecondsLeft() const;
    QVariantList activity() const;
    bool showReads() const { return m_showReads; }
    void setShowReads(bool on);
    bool canUndoAny() const;
    QVariantMap newClient() const { return m_newClient; }
    static QString relayCommand();
    static QString desktopConfig();
    static QString codeCommand();
    QString url() const;
    QString httpCommand() const;
    static QString otherComputerUrl();

    // On or off, stored. Off is the kill switch: the server stops, what waits for
    // the user ends with nothing changed, the agents' timed previews are put back
    Q_INVOKABLE void setEnabled(bool on);
    Q_INVOKABLE void stop() { setEnabled(false); }
    Q_INVOKABLE void setPaused(bool on);
    // kPortMin..65535, and only while off; false (and lastError) otherwise
    Q_INVOKABLE bool setPort(int port);
    Q_INVOKABLE void dismissError();
    // A client for an agent that takes a token in its configuration: newClient
    Q_INVOKABLE void addClient(const QString &name);
    Q_INVOKABLE void clearNewClient();
    Q_INVOKABLE void revoke(double id);
    // -- WP LAY8 -- revoke(), and it is not shown as connected from now on. The
    // server keeps listening; the agent has to ask to be allowed again.
    Q_INVOKABLE void disconnectAgent(double id);
    // The answer to the pairing request on show
    Q_INVOKABLE void answerPairing(bool allow);
    // -- WP LAY8 -- What kind of risk a question is about, in a word or two; "" for a
    // kind this app does not know
    static QString riskLabel(const QString &kind);
    // The answer to confirmation `id`. Allow runs the agent's call.
    Q_INVOKABLE void answer(double id, bool allow);
    // Undoes what the activity's row `key` changed, as the user; or every row's,
    // newest first
    Q_INVOKABLE void undo(const QString &key);
    Q_INVOKABLE void undoAll();
    // The whole activity as JSON, through a save dialog
    Q_INVOKABLE void exportActivity();
    void exportActivityTo(const QString &path);
    Q_INVOKABLE void copy(const QString &text);
    // Settings > AI Agents; the AI agents panel (its tool window in Classic)
    Q_INVOKABLE void openSettings();
    Q_INVOKABLE void openPanel();
    // Settings > Installer, where a limits profile is made
    Q_INVOKABLE void openLimits();
    Q_INVOKABLE void refresh();

    // -- WP AIVIS --
    bool working() const { return m_working; }
    QVariantMap work() const { return m_workShown; }
    QVariantList recentTouches() const;
    // The device the strip is about becomes the one shown; its agent's measurement
    // stops (as the Measure view's Stop does)
    Q_INVOKABLE void showWork();
    Q_INVOKABLE void stopJob();
    // The section of Modern's rail that shows journal path `path`, "" for none
    Q_INVOKABLE static QString sectionOf(const QString &path);

    // -- WP ODEC --
    QVariantList grants() const;
    // The user's Revoke of grant `id`: the agent is asked again for its next test signal
    Q_INVOKABLE void revokeGrant(double id);
    bool followAgent() const { return m_followAgent; }
    void setFollowAgent(bool on);
    // The user is at something a switch of screens would take away: a drag on a
    // device, the keyboard in a text field, a dialog, menu or popup open, an input
    // in flight (inputBusy)
    bool userInteracting() const;
    // -- WP INTI -- (review RF7) a mouse button held, or a key, click or wheel within
    // kInputIdleMs: a switch now could land the input on the device switched to
    bool inputBusy() const;
    // Whether the screens may follow an agent to `device` (and Modern's `section`)
    // now; true counts as a switch for the rate limit. `measure`: the Measure view
    // for an agent's measurement request, which only the user's interaction holds off.
    bool mayFollow(const QString &device, const QString &section, bool measure = false);
    // For the smoke driver: the follow-up of `device`/`section` as an agent's change
    // would ask for it
    void followChange(const QString &device, const QString &section);

    // For the smoke driver: the clients' own folder; calls not answered yet
    void setStoreDir(const QString &dir) { m_storeDir = dir; }
    int pending() const { return m_pending; }

    // -- WP MCP5 -- The AI request interface of the Measure views. An agent's
    // measurement start is a request: this controller takes it from the server
    // (each once) and hands its ticket to the Measure views (measureRequested:
    // { request_id, device, kind, loop, actor, client_name, what, setup }); the
    // view that runs it starts it with dspi_mcp_measure_begin on the device's
    // core and says how it went here. Every request ends in exactly one report.
    // -- WP RCFIX -- (review RC, RC4) `withdrawn`: the job started, but the request
    // was withdrawn meanwhile (its agent gone, agents turned off): stop it
    void reportMeasure(quint64 request, bool started, const QString &note, std::function<void()> withdrawn = {});

    // -- WP FEAT2 -- The same for an agent's request to open a known device as a
    // tab, or to close one it opened: taken once, handed on (deviceRequested:
    // { request_id, kind, device, transport, hub, serial, client_name, what }),
    // and reported once.
    void reportDevice(quint64 request, bool done, const QString &note);
    // The devices the app remembers the user opened, [{ device, name }] (a set
    // id): known to agents beside those the server sees by itself
    void setKnownDevices(const QJsonArray &devices);

signals:
    void rememberedChanged();   // -- WP FEATUI --
    void stateChanged();
    void settingsChanged();
    void clientsChanged();
    void agentsChanged();   // -- WP LAY8 --
    void pairingChanged();
    void confirmationsChanged();
    void secondsChanged();
    void activityChanged();
    void newClientChanged();
    // -- WP MCP5 -- an agent's measurement request for the Measure views; the one
    // that takes it (MeasureController, UnitsMeasureController for `loop`
    // "system") reports with reportMeasure()
    void measureRequested(const QJsonObject &ticket);
    // -- WP FEAT2 -- DevicesController's, which reports with reportDevice()
    void deviceRequested(const QJsonObject &ticket);
    // -- WP AIVIS --
    void workChanged();
    void recentTouchesChanged();
    // An agent changed `path` at `indices` on `device` (a DeviceRegistry id)
    void touched(const QString &device, const QString &path, const QVariantList &indices);
    // An agent's `tool` has run on `device`: what it did that no snapshot of the
    // device shows (a filed snapshot, the compare, a tune) is read again
    void agentCalled(const QString &device, const QString &tool, bool write);
    // An agent's measurement began or ended (its document may be filed)
    void measureLogChanged();
    // -- WP ODEC --
    void grantsChanged();
    void followAgentChanged();
    // Modern shows `section` of the device shown, following an agent's change
    void followSection(const QString &section);

private:
    static void eventThunk(const char *json, void *self);
    void onEvent(const QJsonObject &event);
    void reconcile();
    void startServer();
    void stopServer();
    void readClients();
    void readAgents();   // -- WP LAY8 --
    void setAgents(const QJsonArray &clients);   // -- WP LAY8 --
    void readConfirmations();
    void readActivity();
    void sendActive();
    void onDevices();
    void setPairing(const QJsonObject &pairing);
    void tick();
    void shutdown();
    void undoNext();
    QString deviceTitle(const QString &id) const;
    void setError(const QString &error);
    // Runs `job` on the server's thread, then `done` on the GUI thread
    template <class Job, class Done>
    void onThread(Job job, Done done);

    AppContext m_ctx;
    FfiMcp *m_mcp = nullptr;
    QString m_storeDir;
    bool m_enabled = false;
    bool m_running = false;
    bool m_busy = false;
    bool m_paused = false;
    bool m_shutDown = false;
    QString m_lastError;
    int m_port = kDefaultPort;
    QStringList m_addresses;
    QString m_confirmMode;
    // -- WP FEATUI --
    int m_agentSplCap = kSplCapDefault;
    QMutex m_holdMutex;             // -- WP INTG -- (RE20) holds of the handle
    QWaitCondition m_holdsDone;
    int m_holds = 0;
    bool m_agentSplCapOn = true;   // -- WP INTG --
    bool m_agentDetect = false;    // -- WP ADETQ --
    bool m_agentRoles = false;     // -- WP ROLES --
    void applySplCap();
    QJsonArray m_remembered;
    void remember();
    // The agent's work said on the hub of the device it works on (dspi-hub's
    // `presence`), so every console of that hub shows it: again every
    // kPresenceRefreshMs while it works, cleared when it stops
    static constexpr int kPresenceTtlS = 20;
    static constexpr int kPresenceRefreshMs = 10000;
    QTimer m_presenceTimer;
    QString m_presenceDevice, m_presenceSent;
    void announcePresence(bool again);
    QVariantList m_clients;
    // -- WP LAY8 --
    struct Agent {
        quint64 id;
        QString name;
        QDateTime since, last;
    };
    QList<Agent> m_agents;
    QJsonObject m_pairing;
    QDateTime m_pairingEnds;
    struct Waiting {
        quint64 id;
        QString client, title, device;
        QStringList risks;
        QStringList riskLines;   // -- WP LAY8 --
        bool sound = false;      // -- WP FEATUI -- it plays a test signal: nothing to undo
        bool wholesale = false;  // -- WP FUNC2c -- a preset load, a recall, an import: ends the undo history
        QJsonObject grant;       // -- WP ODEC -- what Allow grants beyond this call ({} for nothing)
        bool detect = false;     // -- WP ADETQ -- it asks to detect the speakers
        QString warning;         // -- WP ADETQ -- the server's warning on the mechanism, "" for none
        QVariantList roleCells;  // -- WP ROLES -- speaker roles from the person's words: output, speaker, role per row
        QString rolesAfter;      // -- WP ROLES -- the question's words after the table
        QDateTime ends;
        bool operator==(const Waiting &o) const { return id == o.id; }   // a question never changes
    };
    QList<Waiting> m_waiting;
    QString grantText(const Waiting &w) const;   // -- WP ODEC --, ADETQ
    QSet<quint64> m_answered;   // answered here, not read as ended yet
    QJsonArray m_activity;      // oldest first, as the server gives it
    QHash<QString, QPair<QString, QString>> m_undo;   // row key: its undo state and note
    QStringList m_undoQueue;
    bool m_undoing = false;
    QString m_lastWrite;        // the newest write's key, to see a new one
    bool m_showReads = false;
    QString m_activeSent;       // the last sendActive()'s
    QStringList m_open;         // the open devices' ids and titles, for the rows
    QVariantMap m_newClient;
    QDateTime m_lastCall;
    QTimer m_tick;
    QTimer m_readSoon;          // coalesces the activity events
    std::atomic<bool> m_callQueued{ false };
    int m_pending = 0;
    void takeMeasureRequests();   // -- WP MCP5 --
    void takeDeviceRequests();   // -- WP FEAT2 --
    QSet<quint64> m_measureTaken;   // -- WP MCP5 -- asked to take, once each
    // -- WP AIVIS --
    void readWork();
    void refreshWork();
    void setWork(bool working, const QVariantMap &shown);
    void readMeasureLog();
    void tell(const QList<QJsonObject> &rows);
    void undoNoticed();
    void readBehind(const QString &device, const QString &tool, bool write);
    QString doing(const QJsonObject &row) const;
    QString targetOf(const QJsonObject &row, DeviceSession *session) const;
    std::atomic<bool> m_workQueued{ false };
    QJsonObject m_work;         // dspi_mcp_work's last
    bool m_working = false;
    QVariantMap m_workShown;
    QTimer m_workTimer;         // lets `work` fade
    qint64 m_activityRun = 0;   // the newest activity row seen: its run and id
    qint64 m_activityId = 0;
    bool m_activityRead = false;
    struct Touch {
        QString device, path;
        QVariantList indices;
        QDateTime at;
    };
    QList<Touch> m_touches;
    QStringList m_noticeKeys;   // the rows the notice on show is about, oldest first
    QString m_noticeAgent;
    QSet<quint64> m_endedJobs;  // the ended measurement requests already said
    QThreadPool m_thread;       // one thread: the server's calls, in order
    // -- WP ODEC --
    struct Grant {
        quint64 id;
        QString agent, deviceId;
        QDateTime until;
        bool detect = false;   // -- WP ADETQ -- it covers detecting the speakers too
        bool operator==(const Grant &o) const { return id == o.id && until == o.until && detect == o.detect; }
    };
    QList<Grant> m_grants;
    void readGrants();
    QString grantSuffix(const QString &device) const;
    bool m_followAgent = true;
    QDateTime m_lastFollow;
    QString m_lastFollowKey;
    QDateTime m_lastFollowKeyAt;
    QString m_followPendingDevice, m_followPendingSection;
    QTimer m_followTimer;       // a follow held off by the rate limit, tried after it
    // -- WP INTI -- (review RF7) the user's input, stamped by the application's event filter
    QElapsedTimer m_lastInput;  // the last key, click, wheel or touch
    QElapsedTimer m_pressedAt;  // the newest press
    int m_buttonsHeld = 0;      // 1 from a press to the next release
    int inputIdleLeftMs() const;

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;   // -- WP INTI --
};

#endif // AICONTROLLER_H
