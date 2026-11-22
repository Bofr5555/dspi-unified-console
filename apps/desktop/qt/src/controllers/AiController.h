#ifndef AICONTROLLER_H
#define AICONTROLLER_H

#include <QDateTime>
#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QSet>
#include <QString>
#include <QStringList>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>
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
    Q_PROPERTY(QString url READ url NOTIFY settingsChanged)
    Q_PROPERTY(QString httpCommand READ httpCommand NOTIFY newClientChanged)
    Q_PROPERTY(QString otherComputerUrl READ otherComputerUrl CONSTANT)
public:
    static constexpr int kDefaultPort = 47860;
    static constexpr int kPortMin = 1024;
    static constexpr int kPairingSeconds = 120;    // dspi-mcp's PAIRING_TTL_MS
    static constexpr int kRecentSeconds = 60;
    static constexpr int kActivityShown = 200;
    static constexpr const char *kPortKey = "mcp/port";

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
        QDateTime ends;
        bool operator==(const Waiting &o) const { return id == o.id; }   // a question never changes
    };
    QList<Waiting> m_waiting;
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
    QThreadPool m_thread;       // one thread: the server's calls, in order
};

#endif // AICONTROLLER_H
