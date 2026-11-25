#include "AiController.h"

#include "AutoEqController.h"
// -- WP AIVIS --
#include "CsController.h"
#include "DashboardController.h"
#include "MonitorController.h"
#include "OutputLatencyController.h"
#include "ToolsController.h"
#include "Core.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "FilesController.h"
#include "MeasureController.h"   // -- WP AIVIS --
#include "SpeakersController.h"  // -- WP ROLES --
#include "NoticeCenter.h"
#include "SessionModel.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SnapshotsController.h"
#include "ChannelController.h"     // -- WP SHOWFIX --
#include "PeqEditorController.h"   // -- WP SHOWFIX --
#include "SidebarController.h"     // -- WP SHOWFIX --
#include "UnitsMeasureController.h"   // -- WP AIVIS --
#include "Snapshot.h"

#include "AppIdentity.h"

#include <QClipboard>
#include <QCoreApplication>
#include <QDir>
#include <QEvent>   // -- WP INTI --
#include <QFile>
#include <QGuiApplication>
#include <QJSEngine>
#include <QJsonDocument>
#include <QLocale>
#include <QMetaMethod>   // -- WP MCP5 --
#include <QRegularExpression>   // -- WP AIVIS --
#include <QSaveFile>
#include <QSettings>
#include <QStandardPaths>
#include <QTimeZone>
#include <algorithm>
#include <memory>
#include <utility>

static const Registrar::Controller<AiController> aiController;
static const Registrar::Window aiWindow("device.aiAgents", "AiAgentsWindow");
// -- WP AIVIS -- what an agent does now: the first strip in both layouts
static const Registrar::Banner aiWorkBanner("AiWorkBanner", 1, Registrar::Blocking);
// Settings > AI Agents (Application, after Networking)
static const SettingsRegistry::Page aiPage{
    "ai", SettingsRegistry::Application, 47, QT_TRANSLATE_NOOP("SettingsPages", "AI Agents"), "bot", "AiAgentsPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Allow AI agents"), QT_TRANSLATE_NOOP("SettingsPages", "MCP server"),
      QT_TRANSLATE_NOOP("SettingsPages", "Connect an agent"), QT_TRANSLATE_NOOP("SettingsPages", "Claude Desktop, Claude Code, Cursor"),
      QT_TRANSLATE_NOOP("SettingsPages", "Clients and tokens"), QT_TRANSLATE_NOOP("SettingsPages", "Ask before changes"),
      QT_TRANSLATE_NOOP("SettingsPages", "Agent activity"), QT_TRANSLATE_NOOP("SettingsPages", "Port"),
      QT_TRANSLATE_NOOP("SettingsPages", "Use from another computer"),
      // -- WP FEATUI --
      QT_TRANSLATE_NOOP("SettingsPages", "Loudest measurement, SPL cap"), QT_TRANSLATE_NOOP("SettingsPages", "Remembered devices") } };

namespace {

const char *const kEnabledKey = "mcp/enabled";
const char *const kConfirmKey = "mcp/confirm";
// -- WP FEATUI --
const char *const kSplCapKey = "mcp/agentSplCapDb";
const char *const kSplCapOnKey = "mcp/agentSplCapOn";   // -- WP INTG --
const char *const kAgentDetectKey = "mcp/agentDetect";   // -- WP ADETQ --
const char *const kAgentRolesKey = "mcp/agentRoles";     // -- WP ROLES --
const char *const kRememberedKey = "devices/remembered";
const QString kConfirmDefault = QStringLiteral("ask_risky");
const QStringList kConfirmModes{ QStringLiteral("ask_everything"), kConfirmDefault, QStringLiteral("never_ask") };

QJsonObject parse(const QByteArray &reply) { return QJsonDocument::fromJson(reply).object(); }

// For the two calls that act (start, add_client): one buffer of the size they
// promise to fit, never a second call (fns_mcp.rs, "Short buffers")
QJsonObject actOnce(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(int(MCP_REPLY_MIN), '\0');
    const uint32_t needed = call(buf.data(), MCP_REPLY_MIN);
    if (needed == 0 || needed > MCP_REPLY_MIN) return {};
    return parse(QByteArray(buf.constData(), int(needed) - 1));
}

bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }

QString str(const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toString(); }

bool portValid(int port) { return port >= 1 && port <= 65535; }

// What starts this app: the AppImage, when it runs from one (its mount goes away)
QString programPath() {
    const QString image = qEnvironmentVariable("APPIMAGE");
    return QDir::toNativeSeparators(image.isEmpty() ? QCoreApplication::applicationFilePath() : image);
}

QString rowKey(const QJsonObject &e) {
    return QStringLiteral("%1:%2").arg(e.value(QLatin1String("run")).toInteger()).arg(e.value(QLatin1String("id")).toInteger());
}

QStringList journalIds(const QJsonObject &e) {
    QStringList ids;
    for (const QJsonValue &v : e.value(QLatin1String("journal_ids")).toArray()) ids << QString::number(v.toInteger());
    return ids;
}

// One change put back: whether it was, and why not
struct Undone {
    bool ok = true;
    QString note;
};

} // namespace

static QString agentName(const QString &label);   // -- WP ODEC -- below

AiController::AiController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    m_thread.setMaxThreadCount(1);
    m_thread.setExpiryTimeout(-1);
    m_storeDir = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
    QSettings s;
    m_enabled = s.value(QLatin1String(kEnabledKey), false).toBool();
    m_confirmMode = s.value(QLatin1String(kConfirmKey), kConfirmDefault).toString();
    if (!kConfirmModes.contains(m_confirmMode)) m_confirmMode = kConfirmDefault;
    // A port nobody can listen on is put right, for the relay too, which reads the key
    if (s.contains(QLatin1String(kPortKey))) {
        m_port = s.value(QLatin1String(kPortKey)).toInt();
        if (!portValid(m_port)) {
            m_port = kDefaultPort;
            s.setValue(QLatin1String(kPortKey), m_port);
        }
    }

    m_mcp = dspi_mcp_new(ctx.registry->set());
    if (m_mcp) {
        dspi_mcp_set_listener(m_mcp, &AiController::eventThunk, this);
        dspi_mcp_set_confirm(m_mcp, m_confirmMode.toUtf8().constData());
    }
    // -- WP FEATUI -- the agent's work said on a hub device's hub
    m_presenceTimer.setInterval(kPresenceRefreshMs);
    connect(&m_presenceTimer, &QTimer::timeout, this, [this] { announcePresence(true); });
    // -- WP FEATUI -- the agents' SPL cap, and the devices remembered from earlier runs
    m_agentSplCap = qBound(kSplCapMin, s.value(QLatin1String(kSplCapKey), kSplCapDefault).toInt(), kSplCapMax);
    m_agentSplCapOn = s.value(QLatin1String(kSplCapOnKey), true).toBool();   // -- WP INTG --
    applySplCap();
    // -- WP ADETQ -- the never-ask switch for the speaker detection
    m_agentDetect = s.value(QLatin1String(kAgentDetectKey), false).toBool();
    if (m_mcp) dspi_mcp_set_agent_detect(m_mcp, m_agentDetect);
    // -- WP ROLES -- the never-ask switch for speaker roles from the person's words
    m_agentRoles = s.value(QLatin1String(kAgentRolesKey), false).toBool();
    if (m_mcp) dspi_mcp_set_agent_roles(m_mcp, m_agentRoles);
    loadRemembered();
    m_tick.setInterval(1000);
    connect(&m_tick, &QTimer::timeout, this, &AiController::tick);
    m_readSoon.setSingleShot(true);
    m_readSoon.setInterval(100);
    connect(&m_readSoon, &QTimer::timeout, this, &AiController::readActivity);
    connect(&m_readSoon, &QTimer::timeout, this, &AiController::readAgents);   // -- WP LAY8 --
    connect(ctx.registry, &DeviceRegistry::devicesChanged, this, &AiController::onDevices);
    // -- WP UAG -- (UXP A7) a refusal is said for a few seconds
    m_refusalTimer.setSingleShot(true);
    m_refusalTimer.setInterval(kRefusalMs);
    connect(&m_refusalTimer, &QTimer::timeout, this, &AiController::clearRefusal);
    // -- WP AIVIS --
    m_workTimer.setInterval(500);
    connect(&m_workTimer, &QTimer::timeout, this, &AiController::refreshWork);
    connect(this, &AiController::stateChanged, this, &AiController::refreshWork);
    connect(this, &AiController::confirmationsChanged, this, &AiController::refreshWork);
    connect(ctx.registry, &DeviceRegistry::devicesChanged, this, &AiController::refreshWork);
    // -- WP ODEC -- a follow the rate limit held off, tried once it allows
    m_followTimer.setSingleShot(true);
    connect(&m_followTimer, &QTimer::timeout, this, [this] {
        const QString device = std::exchange(m_followPendingDevice, QString());
        const FollowAt at = std::exchange(m_followPendingAt, FollowAt());   // -- WP SHOWFIX --
        if (!device.isEmpty()) followTo(device, at);
    });
    // -- WP SHOWFIX -- (finding 10) a question's device, once an input rests
    m_questionTimer.setSingleShot(true);
    connect(&m_questionTimer, &QTimer::timeout, this, &AiController::followQuestion);
    // -- WP SHOWFIX -- (finding 3) the channel and band of a change followed to
    m_showTimer.setInterval(50);
    connect(&m_showTimer, &QTimer::timeout, this, [this] {
        if (showFollowedStep()) m_showTimer.stop();
    });
    // -- WP INTI -- (review RF7) the user's input, for the follow gate
    if (QCoreApplication::instance()) QCoreApplication::instance()->installEventFilter(this);
    // It must stop before the set is freed, which main() does after the event loop
    connect(QCoreApplication::instance(), &QCoreApplication::aboutToQuit, this, &AiController::shutdown);
    if (m_enabled) QTimer::singleShot(0, this, &AiController::reconcile);
}

AiController::~AiController() { shutdown(); }

AiController *AiController::create(QQmlEngine *, QJSEngine *) {
    AiController *self = Registrar::instance<AiController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

void AiController::shutdown() {
    if (m_shutDown) return;
    m_shutDown = true;
    m_tick.stop();
    m_readSoon.stop();
    m_workTimer.stop();   // -- WP AIVIS --
    m_presenceTimer.stop();   // -- WP FEATUI --
    m_thread.waitForDone();
    {
        // -- WP INTG -- (review RE20) other controllers' threads done with it
        QMutexLocker lock(&m_holdMutex);
        while (m_holds > 0) m_holdsDone.wait(&m_holdMutex);
    }
    if (!m_mcp) return;
    // Waits for a call in progress; puts the agents' timed previews back
    dspi_mcp_stop(m_mcp);
    dspi_mcp_set_listener(m_mcp, nullptr, nullptr);
    dspi_mcp_free(m_mcp);
    m_mcp = nullptr;
}

// -- WP INTG -- (review RE20)
FfiMcp *AiController::holdHandle() {
    QMutexLocker lock(&m_holdMutex);
    if (m_shutDown || !m_mcp) return nullptr;
    ++m_holds;
    return m_mcp;
}

void AiController::releaseHandle() {
    QMutexLocker lock(&m_holdMutex);
    if (m_holds > 0 && --m_holds == 0) m_holdsDone.wakeAll();
}

template <class Job, class Done>
void AiController::onThread(Job job, Done done) {
    if (m_shutDown || !m_mcp) return;
    ++m_pending;
    m_thread.start([this, job = std::move(job), done = std::move(done)]() mutable {
        auto result = job();
        QMetaObject::invokeMethod(
            this,
            [this, done = std::move(done), result = std::move(result)]() mutable {
                --m_pending;
                done(std::move(result));
            },
            Qt::QueuedConnection);
    });
}

// ---- Events -------------------------------------------------------------------

// The server's threads: queue and return. The activity's come at an agent's
// pace: one waits at a time.
void AiController::eventThunk(const char *json, void *self) {
    auto *ai = static_cast<AiController *>(self);
    const QJsonObject event = parse(QByteArray(json));
    if (str(event, "event") == QLatin1String("activity") && ai->m_callQueued.exchange(true)) return;
    // -- WP AIVIS --
    if (str(event, "event") == QLatin1String("work") && ai->m_workQueued.exchange(true)) return;
    QMetaObject::invokeMethod(ai, [ai, event] { ai->onEvent(event); }, Qt::QueuedConnection);
}

void AiController::onEvent(const QJsonObject &event) {
    const QString what = str(event, "event");
    if (what == QLatin1String("activity")) {
        m_callQueued = false;
        m_lastCall = QDateTime::currentDateTimeUtc();
        m_tick.start();
        if (!m_readSoon.isActive()) m_readSoon.start();
        emit stateChanged();
    } else if (what == QLatin1String("confirmationsChanged")) {
        readConfirmations();
    } else if (what == QLatin1String("clientsChanged")) {
        readClients();
    } else if (what == QLatin1String("pairingRequested")) {
        setPairing(event);
    } else if (what == QLatin1String("pairingEnded")) {
        if (event.value(QLatin1String("id")) == m_pairing.value(QLatin1String("id"))) setPairing({});
    } else if (what == QLatin1String("measureRequests")) {
        takeMeasureRequests();   // -- WP MCP5 --
        readMeasureLog();        // -- WP AIVIS --
    } else if (what == QLatin1String("deviceRequests")) {
        takeDeviceRequests();   // -- WP FEAT2 --
    } else if (what == QLatin1String("work")) {   // -- WP AIVIS --
        m_workQueued = false;
        readWork();
    }
}

// -- WP MCP5 -- An agent's measurement requests: each taken once, its ticket to
// the Measure views. A request no view takes is reported at once, so the agent
// hears why instead of waiting for the server's lapse.
void AiController::takeMeasureRequests() {
    FfiMcp *mcp = m_mcp;
    onThread([mcp] { return parse(readSized([mcp](char *b, uint32_t n) { return dspi_mcp_measure_requests(mcp, b, n); }, 4096)); },
             [this, mcp](const QJsonObject &reply) {
                 for (const QJsonValue &v : reply.value(QLatin1String("value")).toArray()) {
                     const quint64 id = quint64(v.toObject().value(QLatin1String("request_id")).toDouble());
                     if (id == 0 || m_measureTaken.contains(id)) continue;
                     m_measureTaken.insert(id);
                     // Taking acts: one buffer that holds any ticket, never a second call
                     onThread(
                         [mcp, id] {
                             QByteArray buf(1 << 22, '\0');
                             const uint32_t needed = dspi_mcp_measure_take(mcp, id, buf.data(), uint32_t(buf.size()));
                             if (needed == 0 || needed > uint32_t(buf.size())) return QJsonObject();
                             return parse(QByteArray(buf.constData(), int(needed) - 1));
                         },
                              [this, id](const QJsonObject &taken) {
                                  if (!isOk(taken)) return;   // withdrawn or lapsed meanwhile
                                  const QJsonObject ticket = taken.value(QLatin1String("ticket")).toObject();
                                  if (isSignalConnected(QMetaMethod::fromSignal(&AiController::measureRequested)))
                                      emit measureRequested(ticket);
                                  else
                                      reportMeasure(id, false, tr("The app's Measure view is not available."));
                              });
                 }
             });
}

void AiController::reportMeasure(quint64 request, bool started, const QString &note, std::function<void()> withdrawn) {
    FfiMcp *mcp = m_mcp;
    const QByteArray text = note.toUtf8();
    onThread([mcp, request, started, text] { return dspi_mcp_measure_report(mcp, request, started, text.constData()); },
             [started, withdrawn = std::move(withdrawn)](bool taken) {
                 if (started && !taken && withdrawn) withdrawn();   // -- WP RCFIX --
             });
}

// -- WP FEAT2 -- An agent's requests to open or close a device, as above.
void AiController::takeDeviceRequests() {
    FfiMcp *mcp = m_mcp;
    onThread([mcp] { return parse(readSized([mcp](char *b, uint32_t n) { return dspi_mcp_device_requests(mcp, b, n); }, 4096)); },
             [this, mcp](const QJsonObject &reply) {
                 for (const QJsonValue &v : reply.value(QLatin1String("value")).toArray()) {
                     const quint64 id = quint64(v.toObject().value(QLatin1String("request_id")).toDouble());
                     if (id == 0) continue;
                     // The server gives each out once
                     onThread(
                         [mcp, id] {
                             QByteArray buf(1 << 16, '\0');
                             const uint32_t needed = dspi_mcp_device_take(mcp, id, buf.data(), uint32_t(buf.size()));
                             if (needed == 0 || needed > uint32_t(buf.size())) return QJsonObject();
                             return parse(QByteArray(buf.constData(), int(needed) - 1));
                         },
                         [this, id](const QJsonObject &taken) {
                             if (!isOk(taken)) return;   // withdrawn, lapsed or taken already
                             const QJsonObject ticket = taken.value(QLatin1String("ticket")).toObject();
                             if (isSignalConnected(QMetaMethod::fromSignal(&AiController::deviceRequested)))
                                 emit deviceRequested(ticket);
                             else
                                 reportDevice(id, false, tr("The app cannot open devices now."));
                         });
                 }
             });
}

void AiController::reportDevice(quint64 request, bool done, const QString &note) {
    FfiMcp *mcp = m_mcp;
    const QByteArray text = note.toUtf8();
    onThread([mcp, request, done, text] { return dspi_mcp_device_report(mcp, request, done, text.constData()); },
             [](bool) {});
}

void AiController::setKnownDevices(const QJsonArray &devices) {
    FfiMcp *mcp = m_mcp;
    const QByteArray json = QJsonDocument(devices).toJson(QJsonDocument::Compact);
    onThread([mcp, json] { return dspi_mcp_set_known_devices(mcp, json.constData()); }, [](bool) {});
}

void AiController::setPairing(const QJsonObject &pairing) {
    m_pairing = pairing;
    // The server notices an unanswered request only when asked about it
    if (!pairing.isEmpty()) {
        m_pairingEnds = QDateTime::currentDateTimeUtc().addSecs(kPairingSeconds);
        m_tick.start();
    }
    emit pairingChanged();
    emit stateChanged();
}

void AiController::tick() {
    const QDateTime now = QDateTime::currentDateTimeUtc();
    if (!m_pairing.isEmpty()) {
        if (now >= m_pairingEnds) setPairing({});
        else emit pairingChanged();
    }
    if (!m_waiting.isEmpty()) emit secondsChanged();
    // -- WP ODEC -- the grants' time left
    if (!m_grants.isEmpty()) {
        m_grants.removeIf([&](const Grant &g) { return g.until <= now; });
        emit grantsChanged();
    }
    const bool recent = m_lastCall.isValid() && m_lastCall.secsTo(now) < kRecentSeconds;
    if (!recent) m_lastCall = QDateTime();
    // -- WP LAY8 --
    if (m_agents.removeIf([&](const Agent &a) { return a.last.secsTo(now) >= kRecentSeconds; })) emit agentsChanged();
    emit stateChanged();
    if (m_pairing.isEmpty() && m_waiting.isEmpty() && !recent && m_grants.isEmpty()) m_tick.stop();
}

// ---- Starting and stopping ---------------------------------------------------------

void AiController::setEnabled(bool on) {
    if (on == m_enabled) return;
    m_enabled = on;
    QSettings().setValue(QLatin1String(kEnabledKey), on);
    if (!on) m_lastError.clear();   // a failed start is tried again at the next switch-on
    if (!on) clearRefusal();        // -- WP UAG -- and so is Stop Agents
    emit stateChanged();
    reconcile();
}

void AiController::reconcile() {
    if (m_shutDown || m_busy || !m_mcp) return;
    if (m_running && !m_enabled) stopServer();
    else if (!m_running && m_enabled && m_lastError.isEmpty()) startServer();
}

void AiController::startServer() {
    m_busy = true;
    emit stateChanged();
    FfiMcp *mcp = m_mcp;
    const QJsonObject config{
        { QStringLiteral("port"), m_port },
        { QStringLiteral("storeDir"), QDir::toNativeSeparators(m_storeDir) },
        { QStringLiteral("appVersion"), AppIdentity::version() },
        { QStringLiteral("templateDir"), QDir::toNativeSeparators(FilesController::templatesDir()) },
        { QStringLiteral("measuring"), true },   // -- WP MCP5 -- the Measure views take agents' requests
        { QStringLiteral("opening"), true },     // -- WP FEAT2 -- DevicesController opens and closes devices for them
    };
    const QByteArray json = QJsonDocument(config).toJson(QJsonDocument::Compact);
    onThread(
        [mcp, json] {
            // The AutoEQ database the app uses, as AutoEqController picks it
            QFile db(QFile::exists(AutoEqController::userDatabasePath()) ? AutoEqController::userDatabasePath()
                                                                         : AutoEqController::builtInDatabasePath());
            if (db.open(QIODevice::ReadOnly)) {
                const QByteArray bytes = db.readAll();
                dspi_mcp_set_autoeq(mcp, reinterpret_cast<const uint8_t *>(bytes.constData()), uint32_t(bytes.size()));
            }
            return actOnce([&](char *b, uint32_t n) { return dspi_mcp_start(mcp, json.constData(), b, n); });
        },
        [this](const QJsonObject &reply) {
            m_busy = false;
            if (!isOk(reply)) {
                const QString why = str(reply, "error");
                m_lastError = why.isEmpty() ? tr("Could not start on port %1.").arg(m_port) : why;
                emit stateChanged();
                reconcile();   // switched off meanwhile
                return;
            }
            m_running = true;
            m_endedJobs.clear();   // -- WP AIVIS -- a run's requests count from 1
            m_measureTaken.clear();
            m_lastError.clear();
            m_addresses.clear();
            for (const QJsonValue &a : reply.value(QLatin1String("addresses")).toArray()) m_addresses << a.toString();
            emit stateChanged();
            readClients();
            reconcile();
        });
}

void AiController::stopServer() {
    m_busy = true;
    emit stateChanged();
    FfiMcp *mcp = m_mcp;
    onThread([mcp] { return dspi_mcp_stop(mcp); },
             [this](bool) {
                 m_busy = false;
                 m_running = false;
                 m_addresses.clear();
                 m_waiting.clear();
                 m_answered.clear();
                 m_lastCall = QDateTime();
                 m_work = QJsonObject();   // -- WP AIVIS --
                 setAgents({});   // -- WP LAY8 --
                 setPairing({});
                 emit confirmationsChanged();
                 emit stateChanged();
                 readClients();
                 // Its timed previews were put back
                 readActivity();
                 reconcile();
             });
}

void AiController::setPaused(bool on) {
    if (on == m_paused || !m_mcp) return;
    m_paused = on;
    dspi_mcp_pause(m_mcp, on);   // never waits for a call in progress
    clearRefusal();              // -- WP UAG -- the person's Pause is the newer news
    emit stateChanged();
}

bool AiController::setPort(int port) {
    if (m_running || m_busy) {
        setError(tr("Turn AI agents off to change the port."));
        return false;
    }
    if (port < kPortMin || port > 65535) {
        setError(tr("The port must be between %1 and 65535.").arg(kPortMin));
        return false;
    }
    m_lastError.clear();
    m_port = port;
    QSettings().setValue(QLatin1String(kPortKey), port);
    emit settingsChanged();
    emit stateChanged();
    emit newClientChanged();
    reconcile();   // a start that failed on the last port
    return true;
}

void AiController::setError(const QString &error) {
    m_lastError = error;
    emit stateChanged();
}

void AiController::dismissError() {
    if (m_lastError.isEmpty()) return;
    m_lastError.clear();
    emit stateChanged();
    reconcile();
}

void AiController::setConfirmMode(const QString &mode) {
    if (mode == m_confirmMode || !kConfirmModes.contains(mode) || !m_mcp) return;
    m_confirmMode = mode;
    QSettings().setValue(QLatin1String(kConfirmKey), mode);
    dspi_mcp_set_confirm(m_mcp, mode.toUtf8().constData());
    emit settingsChanged();
}

QString AiController::state() const {
    if (m_busy) return QStringLiteral("starting");
    if (!m_running) return m_enabled && !m_lastError.isEmpty() ? QStringLiteral("error") : QStringLiteral("off");
    if (!m_pairing.isEmpty() || !confirmations().isEmpty()) return QStringLiteral("asking");
    if (m_paused) return QStringLiteral("paused");
    return m_lastCall.isValid() ? QStringLiteral("active") : QStringLiteral("on");
}

QString AiController::statusLine() const {
    const QString s = state();
    if (m_busy) return m_enabled ? tr("Starting…") : tr("Stopping…");
    if (s == QLatin1String("off")) return tr("AI agents are off.");
    if (s == QLatin1String("error")) return tr("AI agents could not be turned on.");
    if (s == QLatin1String("asking")) return tr("An AI agent is waiting for your answer.");
    if (s == QLatin1String("paused")) return tr("AI agents are paused.");
    if (s == QLatin1String("active")) return tr("An AI agent is active.");
    return tr("Waiting for AI agents.");
}

// ---- The device agents get when they name none ----------------------------------------

// The registry's rows change with every clip mark: only what matters here goes on
void AiController::onDevices() {
    if (m_ctx.registry->activeId() != m_activeSent) sendActive();
    remember();   // -- WP FEATUI --
    QStringList open;
    for (const DeviceSession *s : m_ctx.registry->sessions()) open << s->id() + QLatin1Char(' ') + s->title();
    if (open == m_open) return;
    m_open = open;
    // The rows name their devices, and Undo needs them open
    emit activityChanged();
    emit confirmationsChanged();
}

void AiController::sendActive() {
    FfiMcp *mcp = m_mcp;
    m_activeSent = m_ctx.registry->activeId();
    const QByteArray id = m_activeSent.toUtf8();
    onThread([mcp, id] { return dspi_mcp_set_active_device(mcp, id.isEmpty() ? nullptr : id.constData()); }, [](bool) {});
}

QString AiController::deviceTitle(const QString &id) const {
    if (id.isEmpty()) return {};
    const DeviceSession *session = m_ctx.registry->session(id);
    return session ? session->title() : id;
}

// ---- Clients and pairing --------------------------------------------------------------

void AiController::readClients() {
    FfiMcp *mcp = m_mcp;
    onThread([mcp] { return parse(readSized([mcp](char *b, uint32_t n) { return dspi_mcp_clients(mcp, b, n); })); },
             [this](const QJsonObject &reply) {
                 if (!isOk(reply)) return;   // never started in this run: no folder yet
                 QVariantList list;
                 for (const QJsonValue &v : reply.value(QLatin1String("value")).toArray()) {
                     const QJsonObject c = v.toObject();
                     list.append(QVariantMap{
                         { QStringLiteral("id"), c.value(QLatin1String("id")).toDouble() },
                         { QStringLiteral("name"), str(c, "name") },
                         { QStringLiteral("created"), c.value(QLatin1String("created")).toDouble() },
                         { QStringLiteral("lastUsed"), c.value(QLatin1String("lastUsed")).toDouble() },
                     });
                 }
                 if (list == m_clients) return;
                 m_clients = list;
                 emit clientsChanged();
             });
}

void AiController::addClient(const QString &name) {
    const QString trimmed = name.trimmed();
    if (trimmed.isEmpty()) return;
    FfiMcp *mcp = m_mcp;
    const QByteArray utf8 = trimmed.toUtf8();
    onThread([mcp, utf8] { return actOnce([&](char *b, uint32_t n) { return dspi_mcp_add_client(mcp, utf8.constData(), b, n); }); },
             [this, trimmed](const QJsonObject &reply) {
                 if (!isOk(reply)) {
                     setError(str(reply, "error"));
                     return;
                 }
                 m_newClient = { { QStringLiteral("id"), reply.value(QLatin1String("id")).toDouble() },
                                 { QStringLiteral("name"), trimmed },
                                 { QStringLiteral("token"), str(reply, "token") } };
                 emit newClientChanged();
                 readClients();
             });
}

void AiController::clearNewClient() {
    if (m_newClient.isEmpty()) return;
    m_newClient.clear();
    emit newClientChanged();
}

void AiController::revoke(double id) {
    FfiMcp *mcp = m_mcp;
    if (m_newClient.value(QStringLiteral("id")).toDouble() == id) clearNewClient();
    onThread([mcp, id] { return dspi_mcp_revoke(mcp, uint64_t(id)); }, [this](bool) { readClients(); });
}

// -- WP LAY8 -- Who is connected: MCP over HTTP keeps no connection, so it is who
// called within the last minute, as for `active`
void AiController::readAgents() {
    FfiMcp *mcp = m_mcp;
    onThread([mcp] { return parse(readSized([mcp](char *b, uint32_t n) { return dspi_mcp_clients(mcp, b, n); })); },
             [this](const QJsonObject &reply) {
                 if (isOk(reply) && m_running) setAgents(reply.value(QLatin1String("value")).toArray());
             });
}

void AiController::setAgents(const QJsonArray &clients) {
    const QDateTime now = QDateTime::currentDateTimeUtc();
    QList<Agent> agents;
    for (const QJsonValue &v : clients) {
        const QJsonObject c = v.toObject();
        const qint64 used = c.value(QLatin1String("lastUsed")).toInteger();
        const QDateTime last = QDateTime::fromSecsSinceEpoch(used, QTimeZone::UTC);
        if (used == 0 || last.secsTo(now) >= kRecentSeconds) continue;
        Agent a{ quint64(c.value(QLatin1String("id")).toInteger()), str(c, "name"), now, last };
        for (const Agent &known : std::as_const(m_agents))
            if (known.id == a.id) a.since = known.since;
        agents.append(a);
    }
    const bool changed = !std::equal(agents.cbegin(), agents.cend(), m_agents.cbegin(), m_agents.cend(),
                                     [](const Agent &a, const Agent &b) { return a.id == b.id && a.name == b.name; });
    m_agents = agents;
    if (changed) emit agentsChanged();
    // The one that called is gone (disconnected, removed)
    if (m_agents.isEmpty() && m_lastCall.isValid()) {
        m_lastCall = QDateTime();
        emit stateChanged();
    }
}

QVariantList AiController::agents() const {
    QVariantList out;
    for (const Agent &a : m_agents)
        out.append(QVariantMap{ { QStringLiteral("id"), double(a.id) },
                                { QStringLiteral("name"), a.name },
                                { QStringLiteral("since"), QLocale().toString(a.since.toLocalTime().time(), QLocale::ShortFormat) } });
    return out;
}

void AiController::disconnectAgent(double id) {
    revoke(id);
    readAgents();   // after it, in order
}

QVariantMap AiController::pairing() const {
    if (m_pairing.isEmpty()) return {};
    return { { QStringLiteral("id"), m_pairing.value(QLatin1String("id")).toDouble() },
             { QStringLiteral("name"), str(m_pairing, "name") },
             { QStringLiteral("code"), str(m_pairing, "code") },
             { QStringLiteral("secondsLeft"), int(qMax<qint64>(0, QDateTime::currentDateTimeUtc().secsTo(m_pairingEnds))) } };
}

void AiController::answerPairing(bool allow) {
    if (m_pairing.isEmpty()) return;
    const quint64 id = quint64(m_pairing.value(QLatin1String("id")).toInteger());
    setPairing({});
    FfiMcp *mcp = m_mcp;
    onThread([mcp, id, allow] { return dspi_mcp_pairing_answer(mcp, id, allow); },
             [this, allow](bool taken) {
                 if (allow && !taken) {
                     if (auto *notices = Registrar::instance<NoticeCenter>())
                         notices->post(tr("The agent was not added: it had stopped waiting, or its token could not be kept."),
                                       QStringLiteral("warning"));
                 }
                 readClients();
             });
}

// ---- What waits for the user ----------------------------------------------------------

// -- WP LAY8 -- dspi-mcp's RiskKind
QString AiController::riskLabel(const QString &kind) {
    static const QHash<QString, const char *> labels{
        { QStringLiteral("louder"), QT_TRANSLATE_NOOP("AiController", "Louder") },
        { QStringLiteral("unmute"), QT_TRANSLATE_NOOP("AiController", "Unmute") },
        { QStringLiteral("enable"), QT_TRANSLATE_NOOP("AiController", "Output on") },
        { QStringLiteral("new_route"), QT_TRANSLATE_NOOP("AiController", "New route") },
        { QStringLiteral("boost"), QT_TRANSLATE_NOOP("AiController", "EQ boost") },
        { QStringLiteral("delay_step"), QT_TRANSLATE_NOOP("AiController", "Delay") },
        { QStringLiteral("tweeter_crossing"), QT_TRANSLATE_NOOP("AiController", "Tweeter protection") },
        { QStringLiteral("change"), QT_TRANSLATE_NOOP("AiController", "Change") },
        { QStringLiteral("test_signal"), QT_TRANSLATE_NOOP("AiController", "Test signal") },
        { QStringLiteral("detect"), QT_TRANSLATE_NOOP("AiController", "Speaker detection") },   // -- WP ADETQ --
        { QStringLiteral("speaker_roles"), QT_TRANSLATE_NOOP("AiController", "Speaker roles") },   // -- WP ROLES --
        { QStringLiteral("recall"), QT_TRANSLATE_NOOP("AiController", "Replaces settings") },
        { QStringLiteral("flash_write"), QT_TRANSLATE_NOOP("AiController", "Saved on the device") },
        { QStringLiteral("installation_save"), QT_TRANSLATE_NOOP("AiController", "Installation") },
        { QStringLiteral("source"), QT_TRANSLATE_NOOP("AiController", "Input source") },
        { QStringLiteral("system"), QT_TRANSLATE_NOOP("AiController", "Every unit") },
    };
    const char *label = labels.value(kind);
    return label ? tr(label) : QString();
}

void AiController::readConfirmations() {
    FfiMcp *mcp = m_mcp;
    onThread([mcp] { return parse(readSized([mcp](char *b, uint32_t n) { return dspi_mcp_confirmations(mcp, b, n); }, 4096)); },
             [this](const QJsonObject &reply) {
                 if (!isOk(reply)) return;
                 QList<Waiting> list;
                 QSet<quint64> there;
                 for (const QJsonValue &v : reply.value(QLatin1String("value")).toArray()) {
                     const QJsonObject c = v.toObject();
                     Waiting w;
                     w.id = quint64(c.value(QLatin1String("confirmation_id")).toInteger());
                     w.client = str(c, "client_name");
                     w.title = str(c, "title");
                     w.device = str(c, "device");
                     for (const QJsonValue &r : c.value(QLatin1String("risks")).toArray()) {
                         // -- WP UAG -- (UXP A6) the change in the screen's words: the first asked one heads the sheet
                         if (w.heading.isEmpty())
                             w.heading = questionHeading(w.client, str(r.toObject(), "kind"), str(r.toObject(), "channel"),
                                                         deviceTitle(w.device));
                         w.flash = w.flash || str(r.toObject(), "kind") == QLatin1String("flash_write")
                                   || str(r.toObject(), "kind") == QLatin1String("installation_save");
                         w.risks << str(r.toObject(), "what");
                         w.sound = w.sound || str(r.toObject(), "kind") == QLatin1String("test_signal");   // -- WP FEATUI --
                         w.wholesale = w.wholesale || str(r.toObject(), "kind") == QLatin1String("recall");   // -- WP FUNC2c --
                         w.detect = w.detect || str(r.toObject(), "kind") == QLatin1String("detect");   // -- WP ADETQ --
                         // -- WP LAY8 --
                         const QString label = riskLabel(str(r.toObject(), "kind"));
                         // -- WP ADETQ -- a risk whose words already start with its kind ("Speaker detection: ...") keeps them
                         const bool named = (!label.isEmpty() && w.risks.constLast().startsWith(label + QLatin1Char(':')))
                                            // -- WP ROLES -- "Speaker roles from your words: ..." names itself too
                                            || str(r.toObject(), "kind") == QLatin1String("speaker_roles");
                         //: %1 is a kind of change ("Louder"), %2 the change ("Out 1 gain up from -12 dB to -3 dB")
                         w.riskLines << (label.isEmpty() || named ? w.risks.constLast() : tr("%1: %2").arg(label, w.risks.constLast()));
                     }
                     // -- WP UAG -- the whole call, the asked rows first (said: the app's words, never an agent's)
                     for (const QJsonValue &b : c.value(QLatin1String("batch")).toArray())
                         w.batch << QVariantMap{ { QStringLiteral("said"), str(b.toObject(), "said") },
                                                 { QStringLiteral("risky"), b.toObject().value(QLatin1String("risky")).toBool() } };
                     w.grant = c.value(QLatin1String("grant")).toObject();   // -- WP ODEC --
                     w.warning = str(c, "warning");   // -- WP ADETQ --
                     // -- WP ROLES -- the whole table of speaker roles from the person's words
                     const QJsonObject roles = c.value(QLatin1String("roles")).toObject();
                     static const QHash<QString, const char *> roleWords{
                         { QStringLiteral("tweeter"), QT_TRANSLATE_NOOP("AiController", "Tweeter") },
                         { QStringLiteral("midrange"), QT_TRANSLATE_NOOP("AiController", "Midrange") },
                         { QStringLiteral("woofer"), QT_TRANSLATE_NOOP("AiController", "Woofer") },
                         { QStringLiteral("sub"), QT_TRANSLATE_NOOP("AiController", "Subwoofer") },
                         { QStringLiteral("full_range"), QT_TRANSLATE_NOOP("AiController", "Full range") },
                     };
                     for (const QJsonValue &r : roles.value(QLatin1String("rows")).toArray()) {
                         const QJsonObject row = r.toObject();
                         const char *word = roleWords.value(str(row, "role"));
                         //: An output in the speaker roles' table: %1 its number from 1
                         w.roleCells << tr("Out %1").arg(row.value(QLatin1String("output")).toInt() + 1) << str(row, "speaker")
                                     << (word ? tr(word) : str(row, "role"));
                     }
                     w.rolesAfter = str(roles, "after");
                     w.ends = QDateTime::currentDateTimeUtc().addMSecs(c.value(QLatin1String("remaining_ms")).toInteger());
                     there.insert(w.id);
                     list.append(w);
                 }
                 m_answered.intersect(there);
                 if (!list.isEmpty()) m_tick.start();
                 // Only a change: the rows' buttons are armed anew with each
                 if (list == m_waiting) return;
                 m_waiting = list;
                 followQuestion();   // -- WP SHOWFIX -- before its sheet opens
                 emit confirmationsChanged();
                 emit secondsChanged();
                 emit stateChanged();
             });
}

QVariantList AiController::confirmations() const {
    QVariantList out;
    for (const Waiting &w : m_waiting) {
        if (m_answered.contains(w.id)) continue;
        out.append(QVariantMap{
            { QStringLiteral("id"), double(w.id) },
            { QStringLiteral("client"), w.client },
            { QStringLiteral("title"), w.title },
            { QStringLiteral("device"), deviceTitle(w.device) },
            { QStringLiteral("deviceId"), w.device },   // -- WP SHOWFIX --
            { QStringLiteral("risks"), w.risks },
            { QStringLiteral("riskLines"), w.riskLines },   // -- WP LAY8 --
            { QStringLiteral("sound"), w.sound },           // -- WP FEATUI --
            { QStringLiteral("wholesale"), w.wholesale },   // -- WP FUNC2c --
            // -- WP ODEC -- the grouped question: the server's grant and the sheet's sentence
            { QStringLiteral("grant"), w.grant.isEmpty() ? QVariant() : QVariant(w.grant.toVariantMap()) },
            { QStringLiteral("grantText"), grantText(w) },
            // -- WP ADETQ -- the speaker detection's question (AUTODES 4.1): its heading and the warning
            { QStringLiteral("detect"), w.detect },
            { QStringLiteral("warning"), w.warning },
            // -- WP ROLES -- speaker roles from the person's words: the table, then the rest
            { QStringLiteral("roleCells"), w.roleCells },
            { QStringLiteral("rolesAfter"), w.rolesAfter },
            // -- WP UAG -- (UXP A6)
            { QStringLiteral("heading"), w.heading },
            { QStringLiteral("batch"), w.batch },
            { QStringLiteral("flash"), w.flash },
            { QStringLiteral("agent"), agentName(w.client) },
        });
    }
    return out;
}

// -- WP UAG -- (UXP A7) The strip stops saying the refusal
void AiController::clearRefusal() {
    m_refusalTimer.stop();
    if (m_refusal.isEmpty()) return;
    m_refusal.clear();
    emit refusalChanged();
}

// -- WP UAG -- (UXP A6) The question's heading: the change in one line, as the first asked
// row is ("Claude Code wants to make Right tweeter louder (Fronts)")
QString AiController::questionHeading(const QString &client, const QString &kind, const QString &channel,
                                      const QString &device) {
    const QString agent = agentName(client);
    const bool named = !channel.isEmpty();
    //: In an AI agent's question, an output it does not name
    const QString output = named ? channel : tr("an output");
    QString line;
    // %1 is the agent's name, %2 an output's name ("Right tweeter")
    if (kind == QLatin1String("louder"))
        line = named ? tr("%1 wants to make %2 louder").arg(agent, channel) : tr("%1 wants to make the sound louder").arg(agent);
    else if (kind == QLatin1String("unmute"))
        line = named ? tr("%1 wants to unmute %2").arg(agent, channel) : tr("%1 wants to unmute the volume").arg(agent);
    else if (kind == QLatin1String("enable"))
        line = tr("%1 wants to turn on %2").arg(agent, output);
    else if (kind == QLatin1String("new_route"))
        line = tr("%1 wants to send a new input to %2").arg(agent, output);
    else if (kind == QLatin1String("boost"))
        line = named ? tr("%1 wants to boost the EQ of %2").arg(agent, channel) : tr("%1 wants to boost an EQ band").arg(agent);
    else if (kind == QLatin1String("delay_step"))
        line = named ? tr("%1 wants to move the delay of %2 a long way").arg(agent, channel)
                     : tr("%1 wants to move a delay a long way").arg(agent);
    else if (kind == QLatin1String("tweeter_crossing"))
        line = tr("%1 wants to lower the tweeter protection of %2").arg(agent, output);
    else if (kind == QLatin1String("test_signal"))
        line = tr("%1 wants to play test signals").arg(agent);
    else if (kind == QLatin1String("recall"))
        line = tr("%1 wants to replace the settings").arg(agent);
    else if (kind == QLatin1String("flash_write"))
        line = tr("%1 wants to save on the device").arg(agent);
    else if (kind == QLatin1String("installation_save"))
        line = tr("%1 wants to save the installation").arg(agent);
    else if (kind == QLatin1String("source"))
        line = tr("%1 wants to change the input source").arg(agent);
    else if (kind == QLatin1String("system"))
        line = tr("%1 wants to change every unit").arg(agent);
    else
        line = tr("%1 wants to change the settings").arg(agent);
    //: An AI agent's question's heading: %1 the change ("Claude Code wants to make Sub louder"), %2 a device's name
    return device.isEmpty() ? line : tr("%1 (%2)").arg(line, device);
}

// -- WP ODEC -- The grouped question's sentence: what Allow grants beyond this call
QString AiController::grantText(const Waiting &w) const {
    if (w.grant.isEmpty()) return {};
    const QString device = deviceTitle(w.grant.value(QLatin1String("device")).toString().isEmpty() ? w.device
                                                                                                 : w.grant.value(QLatin1String("device")).toString());
    const int minutes = w.grant.value(QLatin1String("minutes")).toInt(10);
    // -- WP ADETQ -- a detection's grant covers detecting again (Grant.detect)
    if (w.grant.value(QLatin1String("detect")).toBool())
        //: The question for an AI agent's speaker detection: %1 is the agent's name, %2 a device's name, %3 minutes
        return tr("Allow also lets %1 play test signals and detect the speakers on %2 for the next %3 minutes, for this task. "
                  "Louder changes and preset saves are still asked one at a time.")
            .arg(agentName(w.client), device)
            .arg(minutes);
    //: The question for an AI agent's test signals: %1 is the agent's name, %2 a device's name, %3 minutes
    return tr("Allow also lets %1 play test signals on %2 for the next %3 minutes, for this task. "
              "Louder changes and preset saves are still asked one at a time.")
        .arg(agentName(w.client), device)
        .arg(minutes);
}

int AiController::confirmationSecondsLeft() const {
    for (const Waiting &w : m_waiting)
        if (!m_answered.contains(w.id)) return int(qMax<qint64>(0, QDateTime::currentDateTimeUtc().secsTo(w.ends)));
    return 0;
}

void AiController::answer(double id, bool allow) {
    const quint64 n = quint64(id);
    if (m_answered.contains(n)) return;
    m_answered.insert(n);
    followQuestion();   // -- WP SHOWFIX -- the next question's device
    emit confirmationsChanged();
    emit stateChanged();
    FfiMcp *mcp = m_mcp;
    onThread([mcp, n, allow] { return dspi_mcp_confirm_answer(mcp, n, allow); },
             [this, allow](bool taken) {
                 if (allow && !taken) {
                     if (auto *notices = Registrar::instance<NoticeCenter>())
                         notices->post(tr("The agent's change was not made: it was no longer waiting for your answer."),
                                       QStringLiteral("warning"));
                 }
                 readConfirmations();
             });
}

// ---- The activity ---------------------------------------------------------------------

void AiController::readActivity() {
    FfiMcp *mcp = m_mcp;
    onThread([mcp] {
        return parse(readSized([mcp](char *b, uint32_t n) { return dspi_mcp_activity(mcp, kActivityShown, b, n); }, 16 * 1024));
    },
             [this](const QJsonObject &reply) {
                 if (!isOk(reply)) return;
                 m_activity = reply.value(QLatin1String("value")).toArray();
                 emit activityChanged();
                 // -- WP AIVIS -- what is new in it
                 QList<QJsonObject> fresh;
                 for (const QJsonValue &v : std::as_const(m_activity)) {
                     const QJsonObject e = v.toObject();
                     const qint64 run = e.value(QLatin1String("run")).toInteger();
                     const qint64 id = e.value(QLatin1String("id")).toInteger();
                     if (run < m_activityRun || (run == m_activityRun && id <= m_activityId)) continue;
                     m_activityRun = run;
                     m_activityId = id;
                     fresh.append(e);
                 }
                 if (!fresh.isEmpty()) tell(fresh);
                 // A write may have begun, kept or reverted a timed preview: the
                 // safe apply's banner reads it again
                 QString newest;
                 for (qsizetype i = m_activity.size(); i-- > 0 && newest.isEmpty();) {
                     const QJsonObject e = m_activity.at(i).toObject();
                     if (e.value(QLatin1String("write")).toBool()) newest = rowKey(e);
                 }
                 if (newest == m_lastWrite) return;
                 m_lastWrite = newest;
                 // -- WP AIVIS -- (agentCalled reads more; this one covers what ended by itself)
                 if (auto *snapshots = Registrar::instance<SnapshotsController>()) snapshots->readGuard();
             });
}

QVariantList AiController::activity() const {
    QVariantList rows;
    for (qsizetype i = m_activity.size(); i-- > 0;) {
        const QJsonObject e = m_activity.at(i).toObject();
        const bool write = e.value(QLatin1String("write")).toBool();
        const QString outcome = str(e, "outcome");
        if (!write && !m_showReads && outcome == QLatin1String("ok")) continue;
        const QString key = rowKey(e);
        const QString device = str(e, "device");
        const auto done = m_undo.constFind(key);
        QString undo;
        if (done != m_undo.cend()) undo = done->first;
        // -- WP INTG -- (review RE2) a failed write that left changes may be undone too
        else if (write && (outcome == QLatin1String("ok") || outcome == QLatin1String("error")) && !journalIds(e).isEmpty())
            undo = m_ctx.registry->session(device) ? QStringLiteral("can") : QStringLiteral("closed");
        const QString title = str(e, "title");
        rows.append(QVariantMap{
            { QStringLiteral("key"), key },
            { QStringLiteral("time"),
              QLocale().toString(QDateTime::fromMSecsSinceEpoch(e.value(QLatin1String("at_unix_ms")).toInteger()).time(), QLocale::ShortFormat) },
            { QStringLiteral("client"), str(e, "client_name") },
            { QStringLiteral("title"), title.isEmpty() ? str(e, "tool") : title },
            // -- WP SHOWFIX -- (finding 4) the core's words where the server has them
            { QStringLiteral("detail"), str(e, "said").isEmpty() ? str(e, "detail") : str(e, "said") },
            { QStringLiteral("device"), deviceTitle(device) },
            { QStringLiteral("write"), write },
            { QStringLiteral("outcome"), outcome },
            { QStringLiteral("outcomeText"),
              outcome == QLatin1String("ok")      ? (write ? tr("Done") : tr("Read"))
              : outcome == QLatin1String("held")  ? tr("Asked you")
              : outcome == QLatin1String("error") ? tr("Not done")
                                                  : tr("Held back") },
            { QStringLiteral("undo"), undo },
            { QStringLiteral("undoNote"), done != m_undo.cend() ? done->second : QString() },
            // -- WP UAG -- (UXP A7) the console refused it: `detail` is why, in the person's words
            { QStringLiteral("refused"), outcome == QLatin1String("error") && !str(e, "said").isEmpty() && journalIds(e).isEmpty() },
        });
    }
    return rows;
}

void AiController::setShowReads(bool on) {
    if (on == m_showReads) return;
    m_showReads = on;
    emit activityChanged();
}

bool AiController::canUndoAny() const {
    for (const QVariant &v : activity())
        if (v.toMap().value(QStringLiteral("undo")) == QLatin1String("can")) return true;
    return false;
}

void AiController::undo(const QString &key) {
    if (m_undoQueue.contains(key)) return;
    m_undoQueue << key;
    m_undo.insert(key, { QStringLiteral("busy"), QString() });
    emit activityChanged();
    undoNext();
}

void AiController::undoAll() {
    for (const QVariant &v : activity()) {   // newest first
        const QVariantMap row = v.toMap();
        if (row.value(QStringLiteral("undo")) == QLatin1String("can")) undo(row.value(QStringLiteral("key")).toString());
    }
}

// One row at a time, its changes newest first, as the user's own (the core's
// revert_entry): a change someone else has overwritten since stays as it is.
void AiController::undoNext() {
    if (m_undoing || m_undoQueue.isEmpty()) return;
    const QString key = m_undoQueue.takeFirst();
    QJsonObject entry;
    for (const QJsonValue &v : std::as_const(m_activity))
        if (rowKey(v.toObject()) == key) entry = v.toObject();
    DeviceSession *session = m_ctx.registry->session(str(entry, "device"));
    const QStringList ids = journalIds(entry);
    if (!session || ids.isEmpty()) {
        m_undo.insert(key, { QStringLiteral("closed"), tr("The device is not open.") });
        emit activityChanged();
        undoNext();
        return;
    }
    m_undoing = true;
    auto out = std::make_shared<Undone>();
    session->model()->write(
        QStringLiteral("undo AI %1").arg(key),
        [ids, out](FfiCore *c) {
            for (qsizetype i = ids.size(); i-- > 0;) {
                const uint64_t id = ids.at(i).toULongLong();
                const QJsonObject r = parse(readSized([&](char *b, uint32_t n) { return dspi_revert_entry(c, id, b, n); }));
                if (isOk(r) && r.value(QLatin1String("entry")).isObject()) continue;
                out->ok = false;
                out->note = isOk(r) ? str(r, "note") : str(r, "error");
            }
            return out->ok;
        },
        [this, key, out](bool ok) {
            m_undoing = false;
            m_undo.insert(key, ok && out->ok ? QPair<QString, QString>{ QStringLiteral("undone"), QString() }
                                             : QPair<QString, QString>{ QStringLiteral("changed"),
                                                                        out->note.isEmpty() ? tr("It could not be undone.") : out->note });
            emit activityChanged();
            undoNext();
        });
}

void AiController::exportActivity() {
    FilesController::pickFile(true, tr("Export AI Activity"), tr("JSON (*.json)"), QStringLiteral("ai-activity.json"), this,
                              [this](const QString &path) { exportActivityTo(path); });
}

void AiController::exportActivityTo(const QString &path) {
    FfiMcp *mcp = m_mcp;
    onThread(
        [mcp, path] {
            const QJsonObject reply =
                parse(readSized([mcp](char *b, uint32_t n) { return dspi_mcp_activity(mcp, UINT32_MAX, b, n); }, 64 * 1024));
            QSaveFile f(path);
            if (!isOk(reply) || !f.open(QIODevice::WriteOnly)) return f.errorString();
            f.write(QJsonDocument(reply.value(QLatin1String("value")).toArray()).toJson());
            return f.commit() ? QString() : f.errorString();
        },
        [this](const QString &error) {
            if (auto *notices = Registrar::instance<NoticeCenter>())
                notices->post(error.isEmpty() ? tr("AI activity exported") : tr("The AI activity was not exported: %1").arg(error),
                              error.isEmpty() ? QStringLiteral("info") : QStringLiteral("error"));
        });
}

// ---- -- WP AIVIS -- What an agent does now, and what it just did -----------------------

// The server's own lock is a leaf, so this never waits for a call: on the GUI thread
void AiController::readWork() {
    if (m_shutDown || !m_mcp) return;
    FfiMcp *mcp = m_mcp;
    m_work = parse(readSized([mcp](char *b, uint32_t n) { return dspi_mcp_work(mcp, b, n); }, 8192));
    readGrants();   // -- WP ODEC --
    refreshWork();
}

// What `row` (a call) does, in words
QString AiController::doing(const QJsonObject &row) const {
    static const QHash<QString, const char *> words{
        { QStringLiteral("dspi_set_peq_band"), QT_TRANSLATE_NOOP("AiController", "Changing the EQ") },
        { QStringLiteral("dspi_clear_peq_band"), QT_TRANSLATE_NOOP("AiController", "Clearing an EQ band") },
        { QStringLiteral("dspi_set_peq_bypass"), QT_TRANSLATE_NOOP("AiController", "Changing the EQ") },
        { QStringLiteral("dspi_set_eq_bypass"), QT_TRANSLATE_NOOP("AiController", "Switching the EQ") },
        { QStringLiteral("dspi_set_crossover"), QT_TRANSLATE_NOOP("AiController", "Changing the crossover") },
        { QStringLiteral("dspi_set_gain"), QT_TRANSLATE_NOOP("AiController", "Changing the gain") },
        { QStringLiteral("dspi_set_delay"), QT_TRANSLATE_NOOP("AiController", "Changing the delay") },
        { QStringLiteral("dspi_set_polarity"), QT_TRANSLATE_NOOP("AiController", "Changing the polarity") },
        { QStringLiteral("dspi_set_mute"), QT_TRANSLATE_NOOP("AiController", "Changing a mute") },
        { QStringLiteral("dspi_set_output_enabled"), QT_TRANSLATE_NOOP("AiController", "Switching an output") },
        { QStringLiteral("dspi_set_route"), QT_TRANSLATE_NOOP("AiController", "Changing the routing") },
        { QStringLiteral("dspi_set_volume"), QT_TRANSLATE_NOOP("AiController", "Changing the volume") },
        { QStringLiteral("dspi_set_user_mute"), QT_TRANSLATE_NOOP("AiController", "Changing the mute") },
        { QStringLiteral("dspi_apply_changes"), QT_TRANSLATE_NOOP("AiController", "Making several changes") },
        { QStringLiteral("dspi_keep_changes"), QT_TRANSLATE_NOOP("AiController", "Keeping its changes") },
        { QStringLiteral("dspi_revert_changes"), QT_TRANSLATE_NOOP("AiController", "Putting its changes back") },
        { QStringLiteral("dspi_undo"), QT_TRANSLATE_NOOP("AiController", "Undoing its change") },
        { QStringLiteral("dspi_preset_load"), QT_TRANSLATE_NOOP("AiController", "Loading a preset") },
        { QStringLiteral("dspi_preset_save"), QT_TRANSLATE_NOOP("AiController", "Saving a preset") },
        { QStringLiteral("dspi_snapshot_recall"), QT_TRANSLATE_NOOP("AiController", "Recalling a snapshot") },
        { QStringLiteral("dspi_config_import"), QT_TRANSLATE_NOOP("AiController", "Importing a configuration") },
        { QStringLiteral("dspi_filters_import"), QT_TRANSLATE_NOOP("AiController", "Importing filters") },
        { QStringLiteral("dspi_autoeq_apply"), QT_TRANSLATE_NOOP("AiController", "Applying an AutoEQ profile") },
        { QStringLiteral("dspi_set_source"), QT_TRANSLATE_NOOP("AiController", "Changing the input source") },
        { QStringLiteral("dspi_measure_start"), QT_TRANSLATE_NOOP("AiController", "Starting a measurement") },
        { QStringLiteral("dspi_measure_rolloff_start"), QT_TRANSLATE_NOOP("AiController", "Starting a low-end search") },
        { QStringLiteral("dspi_detect_speakers"), QT_TRANSLATE_NOOP("AiController", "Asking to detect the speakers") },   // -- WP ADETQ --
        { QStringLiteral("dspi_set_speaker_roles"), QT_TRANSLATE_NOOP("AiController", "Setting the speaker roles from your words") },   // -- WP ROLES --
        { QStringLiteral("dspi_measure_stop"), QT_TRANSLATE_NOOP("AiController", "Stopping a measurement") },
        { QStringLiteral("dspi_tune_propose"), QT_TRANSLATE_NOOP("AiController", "Working out an auto-tune") },
        { QStringLiteral("dspi_tune_apply"), QT_TRANSLATE_NOOP("AiController", "Applying an auto-tune") },
        { QStringLiteral("dspi_tune_verify"), QT_TRANSLATE_NOOP("AiController", "Verifying an auto-tune") },
        { QStringLiteral("dspi_installation_draft"), QT_TRANSLATE_NOOP("AiController", "Editing its installation draft") },
        { QStringLiteral("dspi_installation_save"), QT_TRANSLATE_NOOP("AiController", "Saving the installation") },
        { QStringLiteral("dspi_open_device"), QT_TRANSLATE_NOOP("AiController", "Opening a device") },
        { QStringLiteral("dspi_close_device"), QT_TRANSLATE_NOOP("AiController", "Closing a device") },
    };
    if (const char *w = words.value(str(row, "tool"))) return tr(w);
    // A tool without words of its own: its title, in the server's
    const QString title = str(row, "title");
    //: %1 is what an AI agent reads or does, in English ("Read a channel")
    return (row.value(QLatin1String("write")).toBool() ? tr("Changing: %1") : tr("Reading: %1")).arg(title);
}

// Where: "Centre woofer, band 2", as the person knows the channel
// -- WP SHOWFIX -- (finding 5) the core's channel words (dspi_channel_words):
// the speaker layout's "Centre woofer", the input's "LFE", else "Out 3"
QString AiController::targetOf(const QJsonObject &row, DeviceSession *session) const {
    QStringList parts;
    const DspState *st = session ? session->model()->state() : nullptr;
    const QJsonObject words = session ? session->model()->snapshot().channelWords : QJsonObject();
    const auto word = [&](const char *kind, int i, const QString &number) {
        const QString w = words.value(QLatin1String(kind)).toArray().at(i).toString();
        return w.isEmpty() ? number : w;
    };
    const int inputs = st ? st->num_input_channels : 0;
    int output = row.value(QLatin1String("output")).toInt(-1);
    int input = row.value(QLatin1String("input")).toInt(-1);
    const int channel = row.value(QLatin1String("channel")).toInt(-1);
    if (channel >= 0 && st) (channel < inputs ? input : output) = channel < inputs ? channel : channel - inputs;
    //: An input's short name: %1 is its number
    if (input >= 0) parts << word("inputs", input, tr("In %1").arg(input + 1));
    //: An output's short name: %1 is its number
    if (output >= 0) parts << word("outputs", output, tr("Out %1").arg(output + 1));
    const int band = row.value(QLatin1String("band")).toInt(-1);
    if (band >= 0) parts << tr("band %1").arg(band + 1);
    return parts.join(QStringLiteral(", "));
}

// "AI · Claude Code" is "Claude Code" beside the robot
static QString agentName(const QString &label) {
    const QString prefix = QStringLiteral("AI · ");
    return label.startsWith(prefix) ? label.mid(prefix.size()) : label;
}

void AiController::refreshWork() {
    if (m_shutDown) return;
    const QJsonArray calls = m_work.value(QLatin1String("calls")).toArray();
    const QJsonArray jobs = m_work.value(QLatin1String("jobs")).toArray();
    const QJsonArray guards = m_work.value(QLatin1String("guards")).toArray();
    const QJsonObject last = m_work.value(QLatin1String("last")).toObject();
    const qint64 ago = last.isEmpty() ? kLingerMs
                                      : qMax<qint64>(0, QDateTime::currentMSecsSinceEpoch() - last.value(QLatin1String("at_unix_ms")).toInteger());
    QJsonObject call;   // the newest, a write before a read
    for (const QJsonValue &v : calls) {
        const QJsonObject c = v.toObject();
        if (call.isEmpty() || c.value(QLatin1String("write")).toBool() || !call.value(QLatin1String("write")).toBool()) call = c;
    }
    const QJsonObject job = jobs.isEmpty() ? QJsonObject() : jobs.last().toObject();
    const QVariantList asked = confirmations();
    const bool writes = call.value(QLatin1String("write")).toBool();

    QString agent, text, device;
    bool plays = false;
    const auto device_of = [this](const QString &id) { return m_ctx.registry->session(id.isEmpty() ? m_ctx.registry->activeId() : id); };
    const auto fromCall = [&](const QJsonObject &row) {
        agent = str(row, "client_name");
        device = str(row, "device");
        DeviceSession *s = device_of(device);
        const QString target = targetOf(row, s);
        text = doing(row);
        //: What an AI agent does, and to what: "Changing the gain: OUT3 Centre"
        if (!target.isEmpty()) text = tr("%1: %2").arg(text, target);
    };
    if (writes) {
        fromCall(call);
    } else if (!job.isEmpty()) {
        agent = str(job, "client_name");
        device = str(job, "device");
        const bool running = str(job, "state") == QLatin1String("running");
        plays = running;
        // The Measure view's own words for where it is ("Measuring: position 1 of 3")
        const DeviceSession *s = device_of(device);
        const auto *measure = Registrar::instance<MeasureController>();
        const MeasureSession *m = measure && s ? measure->sessionOf(const_cast<DeviceSession *>(s)) : nullptr;
        const bool measureJob = running && s && s->hasJob(QStringLiteral("MeasureController"));
        // -- WP REVDFIX -- (REVD D3) the agent's run as the session says it, not
        // its job line cut at the first "." ("5.5 s sweeps" lost the sentence)
        text = measureJob && m ? m->agentDoing() : QString();
        if (text.isEmpty() && measureJob) {
            text = s->job();
            text.remove(QRegularExpression(QStringLiteral("^Requested by [^:]*: ?")));
        }
        // -- WP ADETQ -- the speaker detection says which output, of how many (AUTODES 5.1)
        if (running && str(job, "kind") == QLatin1String("detect"))
            text = m && m->detecting() ? m->detectStripText() : tr("Detecting speakers");
        if (text.isEmpty()) text = running ? tr("Measuring: %1").arg(str(job, "what")) : tr("Starting: %1").arg(str(job, "what"));
    } else if (!asked.isEmpty()) {
        const QVariantMap q = asked.first().toMap();
        agent = q.value(QStringLiteral("client")).toString();
        text = tr("Waiting for your answer: %1").arg(q.value(QStringLiteral("title")).toString());
        for (const Waiting &w : m_waiting)
            if (!m_answered.contains(w.id)) {
                device = w.device;
                break;
            }
    } else if (!call.isEmpty()) {
        fromCall(call);
    } else if (ago < kLingerMs) {
        fromCall(last);
        //: %1 is what an AI agent did a moment ago ("Changing the gain: OUT3")
        if (ago >= kWorkingMs) text = tr("Last: %1").arg(text);
    } else if (!guards.isEmpty()) {
        agent = str(guards.last().toObject(), "client_name");
        device = str(guards.last().toObject(), "device");
        text = tr("Its timed preview waits: it is put back unless it is kept");
    }
    const bool busy = !calls.isEmpty() || !jobs.isEmpty() || !asked.isEmpty() || !guards.isEmpty();
    const bool working = m_running && !m_paused && (busy || ago < kWorkingMs);
    const bool paused = m_running && m_paused;
    if (paused && !plays) text = tr("AI agents are paused. Nothing is changed until Resume.");
    DeviceSession *on = device.isEmpty() ? nullptr : m_ctx.registry->session(device);
    const QString title = on ? on->title() : QString();
    const bool elsewhere = on && on != m_ctx.activeSession();
    if (!(paused && !plays) && !title.isEmpty() && (elsewhere || m_ctx.registry->sessions().size() > 1))
        text += QStringLiteral(" · ") + title;
    const bool shown = m_running && (busy || paused || ago < kLingerMs);
    if (shown && (ago < kLingerMs || busy)) {
        if (!m_workTimer.isActive()) m_workTimer.start();
    } else {
        m_workTimer.stop();
    }
    setWork(working, shown ? QVariantMap{ { QStringLiteral("shown"), true },
                                          { QStringLiteral("agent"), paused && !plays ? QString() : agentName(agent) },
                                          { QStringLiteral("text"), text },
                                          { QStringLiteral("device"), title },
                                          { QStringLiteral("deviceId"), device },
                                          { QStringLiteral("elsewhere"), elsewhere && !paused },
                                          { QStringLiteral("job"), plays } }
                           : QVariantMap());
}

void AiController::setWork(bool working, const QVariantMap &shown) {
    if (working == m_working && shown == m_workShown) return;
    m_working = working;
    m_workShown = shown;
    emit workChanged();
    announcePresence(false);   // -- WP FEATUI --
}

// ---- -- WP ODEC -- Grants, and the screens following an agent ---------------------------

void AiController::readGrants() {
    const QDateTime now = QDateTime::currentDateTimeUtc();
    QList<Grant> list;
    for (const QJsonValue &v : m_work.value(QLatin1String("grants")).toArray()) {
        const QJsonObject g = v.toObject();
        const qint64 left = g.value(QLatin1String("left_s")).toInteger();
        if (left <= 0) continue;
        Grant row{ quint64(g.value(QLatin1String("id")).toInteger()), agentName(str(g, "client_name")), str(g, "device"), now.addSecs(left),
                   g.value(QLatin1String("detect")).toBool() };   // -- WP ADETQ --
        // The same grant read again keeps its countdown (the seconds round)
        for (const Grant &was : std::as_const(m_grants))
            if (was.id == row.id && qAbs(was.until.secsTo(row.until)) <= 2) row.until = was.until;
        list.append(row);
    }
    if (list == m_grants) return;
    m_grants = list;
    if (!m_grants.isEmpty()) m_tick.start();
    emit grantsChanged();
    announcePresence(false);
}

QVariantList AiController::grants() const {
    QVariantList out;
    const QDateTime now = QDateTime::currentDateTimeUtc();
    for (const Grant &g : m_grants) {
        const qint64 left = qMax<qint64>(0, now.secsTo(g.until));
        out.append(QVariantMap{
            { QStringLiteral("id"), double(g.id) },
            { QStringLiteral("agent"), g.agent },
            { QStringLiteral("device"), deviceTitle(g.deviceId) },
            { QStringLiteral("deviceId"), g.deviceId },
            { QStringLiteral("secondsLeft"), int(left) },
            { QStringLiteral("left"), QStringLiteral("%1:%2").arg(left / 60).arg(left % 60, 2, 10, QLatin1Char('0')) },
            { QStringLiteral("detect"), g.detect },   // -- WP ADETQ --
        });
    }
    return out;
}

void AiController::revokeGrant(double id) {
    const quint64 n = quint64(id);
    if (m_grants.removeIf([n](const Grant &g) { return g.id == n; })) {
        emit grantsChanged();
        announcePresence(false);
    }
    FfiMcp *mcp = m_mcp;
    onThread([mcp, n] { return dspi_mcp_grant_revoke(mcp, n); }, [this](bool) { readWork(); });
}

// " · test signals allowed, 9 min left" for a grant on `device`, "" for none
QString AiController::grantSuffix(const QString &device) const {
    const QDateTime now = QDateTime::currentDateTimeUtc();
    for (const Grant &g : m_grants)
        if (g.deviceId == device && !device.isEmpty()) {
            const qint64 minutes = (qMax<qint64>(0, now.secsTo(g.until)) + 59) / 60;
            // -- WP ADETQ --
            if (g.detect)
                //: Said on the hub beside what an AI agent does: %n minutes left of the user's allowing its test signals and speaker detection
                return QStringLiteral(" · ") + tr("test signals and speaker detection allowed, %n min left", "", int(minutes));
            //: Said on the hub beside what an AI agent does: %n minutes left of the user's allowing its test signals
            return QStringLiteral(" · ") + tr("test signals allowed, %n min left", "", int(minutes));
        }
    return {};
}

void AiController::setFollowAgent(bool on) {
    if (on == m_followAgent) return;
    m_followAgent = on;
    emit followAgentChanged();
}

// A drag, the keyboard in a text field, a dialog, a menu or a popup of a window
// -- WP INTI -- (review RF7) every key, click, wheel and touch is stamped, wherever it goes;
// the filter only reads the event's type and never takes it
bool AiController::eventFilter(QObject *watched, QEvent *event) {
    switch (event->type()) {
    case QEvent::MouseButtonPress:
    case QEvent::MouseButtonDblClick:
    case QEvent::TouchBegin:
        // Every press restarts the clock: a release another object took (a popup's
        // grab) must not leave an old press counted, nor make a new one look stale
        m_buttonsHeld = 1;
        m_pressedAt.start();
        m_lastInput.start();
        break;
    case QEvent::MouseButtonRelease:
    case QEvent::TouchEnd:
    case QEvent::TouchCancel:
        m_buttonsHeld = 0;
        m_lastInput.start();
        break;
    case QEvent::KeyPress:
    case QEvent::KeyRelease:
    case QEvent::Shortcut:
    case QEvent::Wheel:
        m_lastInput.start();
        break;
    default:
        break;
    }
    return QObject::eventFilter(watched, event);
}

int AiController::inputIdleLeftMs() const {
    // A press whose release never came (a popup took it) holds for 10 s at most
    if (m_buttonsHeld > 0 && m_pressedAt.isValid() && m_pressedAt.elapsed() < 10000) return kInputIdleMs;
    if (!m_lastInput.isValid()) return 0;
    return int(qMax<qint64>(0, kInputIdleMs - m_lastInput.elapsed()));
}

bool AiController::inputBusy() const { return inputIdleLeftMs() > 0; }

bool AiController::userInteracting() const { return inputBusy() || userAt(true); }

// -- WP SHOWFIX -- (finding 10) `questionSheets`: an agent's question open counts
bool AiController::userAt(bool questionSheets) const {
    for (DeviceSession *s : m_ctx.registry->sessions())
        if (s->model() && s->model()->dragging()) return true;
    if (m_ctx.device && m_ctx.device->dragging()) return true;
    QObject *focus = QGuiApplication::focusObject();
    if (focus && (focus->inherits("QQuickTextInput") || focus->inherits("QQuickTextEdit"))) return true;
    if (QGuiApplication::modalWindow()) return true;
    for (QWindow *window : QGuiApplication::topLevelWindows()) {
        if (!window->isVisible()) continue;
        const QList<QObject *> all = window->findChildren<QObject *>();
        for (const QObject *o : all)
            if (o->inherits("QQuickPopup") && o->property("opened").toBool() && !o->property("aiPassive").toBool()
                && (questionSheets || !o->property("aiQuestion").toBool()))
                return true;
    }
    return false;
}

bool AiController::mayFollow(const QString &device, const QString &section, bool measure) {
    if (userInteracting()) return false;
    const QDateTime now = QDateTime::currentDateTimeUtc();
    if (!measure) {
        if (!m_followAgent) return false;
        if (m_lastFollow.isValid() && m_lastFollow.msecsTo(now) < kFollowMs) return false;
        const QString key = device + QLatin1Char('\n') + section;
        if (key == m_lastFollowKey && m_lastFollowKeyAt.isValid() && m_lastFollowKeyAt.msecsTo(now) < kFollowSameMs) return false;
        m_lastFollowKey = key;
        m_lastFollowKeyAt = now;
    } else {
        m_lastFollowKey = device + QStringLiteral("\nmeasure");
        m_lastFollowKeyAt = now;
    }
    m_lastFollow = now;
    return true;
}

// The device an agent changed comes to the front (both layouts), and Modern shows
// the section of the change; held off while the user is at something, and at most
// one switch in kFollowMs (the newest change is followed after it)
void AiController::followChange(const QString &device, const QString &section) {
    FollowAt at;   // -- WP SHOWFIX --
    at.section = section;
    followTo(device, at);
}

// -- WP SHOWFIX -- (finding 3) and the tab, channel and band of the change
void AiController::followTo(const QString &device, const FollowAt &at) {
    if (!m_followAgent || device.isEmpty() || !m_ctx.registry->session(device)) return;
    const QDateTime now = QDateTime::currentDateTimeUtc();
    const QString key = device + QLatin1Char('\n') + at.section;
    // The same device and section again soon after: left alone where the user went
    // from it; where the user still is, only the tab, channel and band of the change
    const bool again = key == m_lastFollowKey && m_lastFollowKeyAt.isValid() && m_lastFollowKeyAt.msecsTo(now) < kFollowSameMs;
    if (again && !stillAt(device, at.section)) return;
    if (m_lastFollow.isValid() && m_lastFollow.msecsTo(now) < kFollowMs) {
        m_followPendingDevice = device;
        m_followPendingAt = at;
        m_followTimer.start(int(kFollowMs - m_lastFollow.msecsTo(now)) + 50);
        return;
    }
    // -- WP INTI -- (review RF7) an input in flight: tried again once it is over
    if (inputBusy()) {
        m_followPendingDevice = device;
        m_followPendingAt = at;
        m_followTimer.start(inputIdleLeftMs() + 50);
        return;
    }
    if (again) {
        if (userInteracting()) return;
        m_lastFollow = now;
    } else {
        if (!mayFollow(device, at.section)) return;
        if (device != m_ctx.registry->activeId()) m_ctx.registry->activate(device);
    }
    if (!at.section.isEmpty()) emit followSection(at.section, at.tab);
    showFollowed(device, at);
}

// -- WP SHOWFIX -- (finding 10) The question in front (the sheet shows the first one
// not answered): its device's tab comes to the front as it opens, so that it is not
// asked over another device's page. Not the rate limit of changes: the question
// takes the user's eyes anyway. Not while the user is at a control, a text field
// or another dialog (the sheet names the device then); after an input once it rests.
void AiController::followQuestion() {
    m_questionTimer.stop();
    const Waiting *front = nullptr;
    for (const Waiting &w : std::as_const(m_waiting))
        if (!m_answered.contains(w.id)) {
            front = &w;
            break;
        }
    if (!front || front->id == m_questionFollowed) return;
    if (!m_followAgent || front->device.isEmpty() || front->device == m_ctx.registry->activeId()
        || !m_ctx.registry->session(front->device) || !m_pairing.isEmpty()) {
        m_questionFollowed = front->id;
        return;
    }
    if (userAt(false)) {
        m_questionFollowed = front->id;
        return;
    }
    if (inputBusy()) {
        m_questionTimer.start(inputIdleLeftMs() + 50);
        return;
    }
    m_questionFollowed = front->id;
    m_lastFollow = QDateTime::currentDateTimeUtc();
    m_ctx.registry->activate(front->device);
}

// -- WP SHOWFIX -- The device shown, and Modern's section (none in Classic)
bool AiController::stillAt(const QString &device, const QString &section) const {
    if (device != m_ctx.registry->activeId()) return false;
    const QObject *pages = m_ctx.shell ? m_ctx.shell->pageHost() : nullptr;
    return section.isEmpty() || !pages || pages->property("section").toString() == section;
}

// -- WP SHOWFIX -- (finding 3) The channel of a change selected (the Equaliser's, and
// Classic's sidebar), and its PEQ band in the graph and the table. Each waits for
// what it needs: the device's channels, the graph editing that channel.
void AiController::showFollowed(const QString &device, const FollowAt &at) {
    m_showTimer.stop();
    if (at.channel < 0 && at.output < 0) return;
    m_showDevice = device;
    m_showAt = at;
    m_showFor.start();
    if (!showFollowedStep()) m_showTimer.start();
}

bool AiController::showFollowedStep() {
    // The user went elsewhere or took over meanwhile: left as it is
    if (m_showDevice != m_ctx.registry->activeId() || m_showFor.elapsed() > kShowMs || inputBusy()) return true;
    auto *sidebar = Registrar::instance<SidebarController>();
    if (!sidebar || !m_ctx.device) return true;
    int channel = m_showAt.channel;
    if (channel < 0) {
        auto *channels = Registrar::instance<ChannelController>();
        if (!channels) return true;
        if (!m_ctx.device->state()) return false;   // the device's state is not read yet
        channel = channels->outputChannel(m_showAt.output);
    }
    if (sidebar->selectedChannel() != channel) {
        sidebar->select(channel);
        if (sidebar->selectedChannel() != channel) return false;   // its channels are not read yet
    }
    if (m_showAt.band < 0) return true;
    auto *peq = Registrar::instance<PeqEditorController>();
    if (!peq) return true;
    // The graph takes the channel after the selection (PeqOverlay), which drops
    // the selection of another channel's bands: the band once it has
    if (peq->editedChannel() != channel) return false;
    if (peq->selected() != QList<int>{ m_showAt.band }) peq->rowClicked(m_showAt.band, 0);
    peq->reveal(m_showAt.band);
    return true;
}

void AiController::showWork() { m_ctx.registry->activate(m_workShown.value(QStringLiteral("deviceId")).toString()); }

void AiController::stopJob() {
    const QString device = m_workShown.value(QStringLiteral("deviceId")).toString();
    if (auto *units = Registrar::instance<UnitsMeasureController>(); units && units->running()) units->stop();
    if (auto *measure = Registrar::instance<MeasureController>()) measure->stopOther(device);
}

QString AiController::sectionOf(const QString &path) {
    const QString group = path.section(QLatin1Char('.'), 0, 0);
    if (group == QLatin1String("eq") || group == QLatin1String("ch") || group == QLatin1String("pre") || path == QLatin1String("out.gain")
        || path == QLatin1String("out.delay") || path == QLatin1String("out.mute") || group == QLatin1String("limit"))
        return QStringLiteral("equaliser");
    if (group == QLatin1String("mix") || group == QLatin1String("out")) return QStringLiteral("routing");
    static const QStringList effects{ QStringLiteral("loud"), QStringLiteral("lev"), QStringLiteral("bass"), QStringLiteral("cf"),
                                      QStringLiteral("tube"), QStringLiteral("up"),  QStringLiteral("sub"),  QStringLiteral("ext") };
    if (effects.contains(group)) return QStringLiteral("effects");
    if (group == QLatin1String("preset")) return QStringLiteral("library");
    if (group == QLatin1String("bulk") || group == QLatin1String("dev")) return QStringLiteral("overview");
    return {};
}

// -- WP SHOWFIX -- (finding 3) The journal's indices: a band's [wire channel, band]
// (a crossover band from 20, as BandRow's), a channel's or an input's preamp
// [wire channel], an output's [output from 0] (255: every output)
AiController::FollowAt AiController::followAtOf(const QString &path, const QVariantList &indices) {
    FollowAt at;
    at.section = sectionOf(path);
    if (at.section != QLatin1String("equaliser")) return at;
    const QString group = path.section(QLatin1Char('.'), 0, 0);
    const int i0 = indices.isEmpty() ? -1 : indices.value(0).toInt();
    constexpr int kCrossoverBase = 20;
    if (group == QLatin1String("eq") && indices.size() >= 2) {
        const int band = indices.value(1).toInt();
        at.channel = i0;
        if (band >= kCrossoverBase) {
            at.tab = QStringLiteral("crossover");
        } else {
            at.tab = QStringLiteral("peq");
            at.band = band;
        }
    } else if (group == QLatin1String("ch") || path == QLatin1String("pre")) {
        at.channel = i0;
        if (path == QLatin1String("ch.delay")) at.tab = QStringLiteral("delay");
    } else if (group == QLatin1String("out") || group == QLatin1String("limit")) {
        if (i0 >= 0 && i0 < MAX_OUTPUTS) at.output = i0;
        if (group == QLatin1String("out")) at.tab = QStringLiteral("delay");   // gain, delay, mute: Delay & Gain
    }
    return at;
}

QVariantList AiController::recentTouches() const {
    QVariantList out;
    const QDateTime now = QDateTime::currentDateTimeUtc();
    for (const Touch &t : m_touches)
        if (t.at.msecsTo(now) < kTouchMs)
            out.append(QVariantMap{ { QStringLiteral("device"), t.device }, { QStringLiteral("path"), t.path }, { QStringLiteral("indices"), t.indices } });
    return out;
}

// The activity's new rows: the controls light up, the views read again what no
// snapshot shows, and a notice says what was changed, with Undo
void AiController::tell(const QList<QJsonObject> &rows) {
    // The first read of a run of the app: nothing is news
    const bool first = !m_activityRead;
    m_activityRead = true;
    if (first && rows.size() > 8) return;
    const QDateTime now = QDateTime::currentDateTimeUtc();
    m_touches.removeIf([&](const Touch &t) { return t.at.msecsTo(now) >= kTouchMs; });
    QString followDevice;   // -- WP ODEC --
    FollowAt followAt;      // -- WP SHOWFIX --
    for (const QJsonObject &e : rows) {
        const QString device = str(e, "device");
        const bool write = e.value(QLatin1String("write")).toBool();
        // -- WP UAG -- (UXP A7) the console refused it: the server's words for the person
        // (`said` on a call that changed nothing), in the strip for a few seconds; no toast
        if (str(e, "outcome") == QLatin1String("error") && !str(e, "said").isEmpty() && journalIds(e).isEmpty()) {
            const DeviceSession *on = m_ctx.registry->session(device);
            m_refusal = QVariantMap{ { QStringLiteral("agent"), agentName(str(e, "client_name")) },
                                     { QStringLiteral("text"), str(e, "said") },
                                     { QStringLiteral("device"), on ? on->title() : QString() } };
            m_refusalTimer.start();
            emit refusalChanged();
            continue;
        }
        // -- WP INTG -- (review RE2) a write that failed but left changes (a
        // preset load that left the device silent): said, with Undo
        if (write && str(e, "outcome") == QLatin1String("error") && !journalIds(e).isEmpty()) {
            if (auto *notices = Registrar::instance<NoticeCenter>()) {
                const QString agent = agentName(str(e, "client_name"));
                m_noticeAgent = agent;
                m_noticeKeys = { rowKey(e) };
                bool canUndo = false;
                for (const QVariant &v : activity())
                    canUndo = canUndo || (v.toMap().value(QStringLiteral("key")).toString() == rowKey(e)
                                          && v.toMap().value(QStringLiteral("undo")) == QLatin1String("can"));
                //: An AI agent's change that failed but changed something: %1 is its name, %2 the app's words for what happened
                // -- WP UAG -- (UXP A7) the person's words where the server has them
                notices->postTagged(QStringLiteral("ai"), tr("%1's change did not go through: %2")
                                                              .arg(agent, str(e, "said").isEmpty() ? str(e, "detail") : str(e, "said")),
                                    canUndo ? tr("Undo") : QString(), [this] { undoNoticed(); });
            }
        }
        if (str(e, "outcome") != QLatin1String("ok")) continue;
        emit agentCalled(device, str(e, "tool"), write);
        readBehind(device, str(e, "tool"), write);
        if (!write) continue;
        // -- WP ODEC -- the screens follow it (the newest change of this read)
        // -- WP SHOWFIX -- (finding 14) the first read's too: the server's activity
        // starts empty with the app, and a first read of more is no news (above)
        {
            const QJsonArray touched = e.value(QLatin1String("touched")).toArray();
            FollowAt at;   // -- WP SHOWFIX -- where it is: the section, tab, channel and band
            for (const QJsonValue &v : touched)
                if (at.section.isEmpty())
                    at = followAtOf(str(v.toObject(), "path"), v.toObject().value(QLatin1String("indices")).toArray().toVariantList());
            if (!touched.isEmpty() || !device.isEmpty()) {
                followDevice = device;
                followAt = at;
            }
        }
        for (const QJsonValue &v : e.value(QLatin1String("touched")).toArray()) {
            const QJsonObject t = v.toObject();
            const QVariantList indices = t.value(QLatin1String("indices")).toArray().toVariantList();
            m_touches.append({ device, str(t, "path"), indices, now });
            emit touched(device, str(t, "path"), indices);
        }
        // -- WP UAG -- (UXP A7) a preset load is one sentence of its own (the preset, what the limits
        // held), not "the last: Master volume −127.0 dB" of its quiet steps, and has no Undo
        if (str(e, "tool") == QLatin1String("dspi_preset_load") || str(e, "tool") == QLatin1String("dspi_revert_to_saved")) {
            auto *notices = Registrar::instance<NoticeCenter>();
            if (!notices || str(e, "said").isEmpty()) continue;
            const DeviceSession *on = m_ctx.registry->session(device);
            m_noticeKeys.clear();
            //: An AI agent loaded a preset: %1 is its name, %2 what happened ("Loaded preset 2 “Lunch”: your limits kept …")
            notices->postTagged(QStringLiteral("ai"), tr("%1: %2").arg(agentName(str(e, "client_name")), str(e, "said"))
                                                          + (on && m_ctx.registry->sessions().size() > 1 ? QStringLiteral(" · ") + on->title() : QString()),
                                QString(), {});
            continue;
        }
        if (journalIds(e).isEmpty()) continue;
        auto *notices = Registrar::instance<NoticeCenter>();
        if (!notices) continue;
        const QString agent = agentName(str(e, "client_name"));
        // A run of changes by one agent is one notice, while it shows
        if (notices->tag() != QLatin1String("ai") || agent != m_noticeAgent) m_noticeKeys.clear();
        m_noticeAgent = agent;
        m_noticeKeys << rowKey(e);
        // -- WP SHOWFIX -- (finding 4) the core's words, the channels as the
        // person knows them ("LFE → Left woofer off"), where the server has them
        const QString detail = !str(e, "said").isEmpty() ? str(e, "said")
                               : str(e, "detail").isEmpty() ? str(e, "title") : str(e, "detail");
        const DeviceSession *on = m_ctx.registry->session(device);
        const QString where = on && m_ctx.registry->sessions().size() > 1 ? QStringLiteral(" · ") + on->title() : QString();
        //: An AI agent's change: %1 is its name, %2 the change ("Out 3 output gain -6 dB to -8 dB")
        const QString text = m_noticeKeys.size() == 1 ? tr("%1 changed: %2").arg(agent, detail) + where
                                                      //: %1 is an AI agent's name, %2 its newest change
                                                      : tr("%1 made %n changes, the last: %2", "", int(m_noticeKeys.size())).arg(agent, detail) + where;
        bool canUndo = false;
        for (const QVariant &v : activity())
            canUndo = canUndo || (m_noticeKeys.contains(v.toMap().value(QStringLiteral("key")).toString())
                                  && v.toMap().value(QStringLiteral("undo")) == QLatin1String("can"));
        notices->postTagged(QStringLiteral("ai"), text, canUndo ? tr("Undo") : QString(), [this] { undoNoticed(); });
    }
    emit recentTouchesChanged();
    if (!followDevice.isEmpty()) followTo(followDevice, followAt);   // -- WP ODEC --, SHOWFIX
}

// What `tool` changed on `device` that its snapshot does not hold
void AiController::readBehind(const QString &device, const QString &tool, bool write) {
    if (auto *snapshots = Registrar::instance<SnapshotsController>()) snapshots->agentCalled(device, tool, write);
    if (tool.startsWith(QLatin1String("dspi_tune_")))
        if (auto *measure = Registrar::instance<MeasureController>()) measure->readTune(device);
    if (tool.startsWith(QLatin1String("dspi_system_measure_")))
        if (auto *units = Registrar::instance<UnitsMeasureController>()) units->readAgain();
    if (device != m_ctx.registry->activeId()) return;   // read when it comes to the front
    // -- WP ROLES -- the speaker roles an agent set from the person's words
    if (tool == QLatin1String("dspi_set_speaker_roles"))
        if (auto *speakers = Registrar::instance<SpeakersController>()) speakers->reload();
    if (tool == QLatin1String("dspi_set_subharmonic")) {
        if (auto *tools = Registrar::instance<ToolsController>()) tools->readHeadroom();
    } else if (tool == QLatin1String("dspi_monitor_clear")) {
        if (auto *monitor = Registrar::instance<MonitorController>()) monitor->cleared();
    } else if (tool == QLatin1String("dspi_clear_clips")) {
        if (auto *dashboard = Registrar::instance<DashboardController>()) dashboard->clipsCleared();
    }
    // A recall also writes what is kept outside the settings' image
    static const QStringList recalls{ QStringLiteral("dspi_snapshot_recall"), QStringLiteral("dspi_ab_toggle"), QStringLiteral("dspi_ab_keep"),
                                      QStringLiteral("dspi_revert_changes"), QStringLiteral("dspi_tune_revert"), QStringLiteral("dspi_config_import"),
                                      QStringLiteral("dspi_preset_load"), QStringLiteral("dspi_revert_to_saved") };
    if (!recalls.contains(tool)) return;
    if (auto *cs = Registrar::instance<CsController>(); cs && !cs->dirty()) cs->fetch();
    if (auto *latency = Registrar::instance<OutputLatencyController>(); latency && !latency->hasPending()) latency->refresh();
}

// What the notice is about, newest first
void AiController::undoNoticed() {
    const QStringList keys = m_noticeKeys;
    m_noticeKeys.clear();
    for (qsizetype i = keys.size(); i-- > 0;) undo(keys.at(i));
}

// How the agents' measurements ended: one the user disturbed is said
void AiController::readMeasureLog() {
    FfiMcp *mcp = m_mcp;
    onThread([mcp] { return parse(readSized([mcp](char *b, uint32_t n) { return dspi_mcp_measure_log(mcp, b, n); }, 8192)); },
             [this](const QJsonObject &reply) {
                 for (const QJsonValue &v : reply.value(QLatin1String("value")).toArray()) {
                     const QJsonObject r = v.toObject();
                     const quint64 id = quint64(r.value(QLatin1String("request_id")).toInteger());
                     if (!r.value(QLatin1String("disturbed")).toBool() || m_endedJobs.contains(id)) continue;
                     m_endedJobs.insert(id);
                     if (auto *notices = Registrar::instance<NoticeCenter>())
                         //: %1 is an AI agent's name
                         notices->post(tr("The measurement %1 asked for was stopped: a setting was changed while it ran, so its result would not be valid.")
                                           .arg(agentName(str(r, "client_name"))),
                                       QStringLiteral("warning"));
                 }
                 emit measureLogChanged();
             });
}

// ---- What to put into an agent's configuration ----------------------------------------

QString AiController::relayCommand() {
    const QString path = programPath();
    return (path.contains(QLatin1Char(' ')) ? QLatin1Char('"') + path + QLatin1Char('"') : path) + QStringLiteral(" --mcp");
}

QString AiController::desktopConfig() {
    const QJsonObject server{ { QStringLiteral("command"), programPath() }, { QStringLiteral("args"), QJsonArray{ QStringLiteral("--mcp") } } };
    const QJsonObject root{ { QStringLiteral("mcpServers"), QJsonObject{ { QStringLiteral("dspi"), server } } } };
    return QString::fromUtf8(QJsonDocument(root).toJson()).trimmed();
}

QString AiController::codeCommand() { return QStringLiteral("claude mcp add dspi -- ") + relayCommand(); }

QString AiController::url() const { return QStringLiteral("http://127.0.0.1:%1/mcp").arg(m_port); }

QString AiController::httpCommand() const {
    const QString token = m_newClient.value(QStringLiteral("token")).toString();
    if (token.isEmpty()) return {};
    return QStringLiteral("claude mcp add --transport http dspi %1 --header \"Authorization: Bearer %2\"").arg(url(), token);
}

QString AiController::otherComputerUrl() { return AppIdentity::projectUrl() + QStringLiteral("/blob/main/docs/mcp-ssh-forwarding.md"); }

void AiController::copy(const QString &text) {
    if (QClipboard *clip = QGuiApplication::clipboard()) clip->setText(text);
}

// ---- Going places ---------------------------------------------------------------------

void AiController::openSettings() {
    if (auto *settings = Registrar::instance<SettingsController>()) settings->showPage(QStringLiteral("ai"));
}

// Past the gate: the kill switch is there on an alink sender too
void AiController::openPanel() { m_ctx.shell->showTool(QStringLiteral("device.aiAgents")); }

void AiController::openLimits() {
    if (auto *settings = Registrar::instance<SettingsController>()) settings->showPage(QStringLiteral("installer"));
}

void AiController::refresh() {
    readClients();
    readConfirmations();
    readActivity();
}

// ---- -- WP FEATUI -- The agents' SPL cap; remembered devices -------------------------

void AiController::setAgentSplCap(int db) {
    db = qBound(kSplCapMin, db, kSplCapMax);
    if (db == m_agentSplCap || !m_mcp) return;
    m_agentSplCap = db;
    QSettings().setValue(QLatin1String(kSplCapKey), db);
    applySplCap();
    emit settingsChanged();
}

// -- WP INTG -- (RE1) Off: no SPL cap (0 for none), the level ceiling holds
void AiController::setAgentSplCapOn(bool on) {
    if (on == m_agentSplCapOn || !m_mcp) return;
    m_agentSplCapOn = on;
    QSettings().setValue(QLatin1String(kSplCapOnKey), on);
    applySplCap();
    emit settingsChanged();
}

// -- WP ADETQ --
// -- WP ROLES --
void AiController::setAgentRoles(bool on) {
    if (on == m_agentRoles || !m_mcp) return;
    m_agentRoles = on;
    QSettings().setValue(QLatin1String(kAgentRolesKey), on);
    dspi_mcp_set_agent_roles(m_mcp, on);
    emit settingsChanged();
}

void AiController::setAgentDetect(bool on) {
    if (on == m_agentDetect || !m_mcp) return;
    m_agentDetect = on;
    QSettings().setValue(QLatin1String(kAgentDetectKey), on);
    dspi_mcp_set_agent_detect(m_mcp, on);
    emit settingsChanged();
}

// -- WP ADETQ -- dspi-mcp's DETECT_WARNING, word for word (AUTODES 4.1): the
// agent's question carries the server's copy; the app's own Detect… and the
// never-ask switch show this one
QString AiController::detectWarning() {
    // -- WP FIN015 -- with the free descent's worst case (the core's absolute gate)
    return tr("Nothing is known about these outputs yet. A tweeter wired where the probe expects a woofer could receive "
              "a few seconds of sweep below its floor at −50 dBFS (more with a raised floor): below 1 kHz less than a "
              "hundredth of what a 1 kHz crossover lets through from music, but not nothing. Only a driver still playing "
              "flat at 177 Hz under that quiet sweep, as no tweeter does, is then swept down to 20 Hz at up to −24 dBFS. "
              "Keep the volume where it is and stay by Stop. Afterwards check the result in Settings › Speakers and "
              "confirm or correct it: until you do, the detected roles guard your speakers in place of a confirmed "
              "setup.");
}

void AiController::applySplCap() {
    if (m_mcp) dspi_mcp_set_agent_spl_cap(m_mcp, m_agentSplCapOn ? float(m_agentSplCap) : 0.0f);
}

// The devices open now go first in the list kept across restarts: those on
// USB. A hub's unit is known to agents while its hub is open (the server lists
// it); remembered without its hub it could only be refused, since the app does
// not dial a hub for an agent.
void AiController::remember() {
    QJsonArray list;
    QStringList ids;
    for (const DeviceSession *s : m_ctx.registry->sessions()) {
        const QString id = s->id();
        if (!id.startsWith(QLatin1String("usb:"))) continue;
        if (ids.contains(id)) continue;
        ids << id;
        list.append(QJsonObject{ { QStringLiteral("device"), id }, { QStringLiteral("name"), s->title() } });
    }
    for (const QJsonValue &v : m_remembered) {
        const QString id = v.toObject().value(QLatin1String("device")).toString();
        if (!ids.contains(id) && list.size() < kRememberedMax) {
            ids << id;
            list.append(v);
        }
    }
    if (list == m_remembered) return;
    m_remembered = list;
    QSettings().setValue(QLatin1String(kRememberedKey), QJsonDocument(list).toJson(QJsonDocument::Compact));
    setKnownDevices(list);
    emit rememberedChanged();
}

void AiController::loadRemembered() {
    m_remembered = QJsonArray();
    for (const QJsonValue &v : QJsonDocument::fromJson(QSettings().value(QLatin1String(kRememberedKey)).toByteArray()).array())
        if (v.toObject().value(QLatin1String("device")).toString().startsWith(QLatin1String("usb:")) && m_remembered.size() < kRememberedMax)
            m_remembered.append(v);
    setKnownDevices(m_remembered);
    emit rememberedChanged();
    remember();
}

void AiController::forgetRememberedDevices() {
    m_remembered = QJsonArray();
    QSettings().remove(QLatin1String(kRememberedKey));
    // The ones open now stay known by being open
    setKnownDevices(m_remembered);
    emit rememberedChanged();
    remember();
}

// -- WP FEATUI -- What an agent does on a device reached through a hub is said
// on that hub (`presence.set`), so the web console and other consoles of that
// hub show it; refreshed while it works, cleared when it stops or moves on. A
// hub without the cap, or a Viewer session, refuses: nothing else happens.
void AiController::announcePresence(bool again) {
    const QString id = m_working ? m_workShown.value(QStringLiteral("deviceId")).toString() : QString();
    DeviceSession *on = id.isEmpty() ? nullptr : m_ctx.registry->session(id);
    const QString transport = on ? on->row().value(QStringLiteral("transport")).toString() : QString();
    const bool viaHub = transport == QLatin1String("hub") || transport == QLatin1String("link");
    const QString agent = m_workShown.value(QStringLiteral("agent")).toString();
    // -- WP ODEC -- what the user allowed there for a while, for the hub's other consoles
    const QString what = m_workShown.value(QStringLiteral("text")).toString() + grantSuffix(id);
    const QString key = viaHub ? id + QLatin1Char('\n') + agent + QLatin1Char('\n') + what : QString();
    // Moved on, or stopped: the last one cleared
    if (!m_presenceDevice.isEmpty() && m_presenceDevice != (viaHub ? id : QString())) {
        if (DeviceSession *was = m_ctx.registry->session(m_presenceDevice))
            was->worker()->post([](FfiCore *c) {
                char buf[256];
                dspi_hub_presence_set(c, "", "", "", 0, buf, sizeof buf);
            });
        m_presenceDevice.clear();
        m_presenceSent.clear();
    }
    if (!viaHub) {
        m_presenceTimer.stop();
        return;
    }
    if (key == m_presenceSent && !again) return;
    m_presenceDevice = id;
    m_presenceSent = key;
    if (!m_presenceTimer.isActive()) m_presenceTimer.start();
    const QByteArray label = (agent.isEmpty() ? tr("AI agent") : agent).toUtf8();
    const QByteArray serial = on->row().value(QStringLiteral("serial")).toString().toUtf8();
    const QByteArray words = what.toUtf8();
    on->worker()->post([label, serial, words](FfiCore *c) {
        char buf[256];
        dspi_hub_presence_set(c, label.constData(), serial.constData(), words.constData(), uint32_t(kPresenceTtlS), buf, sizeof buf);
    });
}
