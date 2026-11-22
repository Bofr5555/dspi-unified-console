#include "AiController.h"

#include "AutoEqController.h"
#include "Core.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "FilesController.h"
#include "NoticeCenter.h"
#include "SessionModel.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SnapshotsController.h"
#include "Snapshot.h"

#include "AppIdentity.h"

#include <QClipboard>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QJSEngine>
#include <QJsonDocument>
#include <QLocale>
#include <QSaveFile>
#include <QSettings>
#include <QStandardPaths>
#include <memory>
#include <utility>

static const Registrar::Controller<AiController> aiController;
static const Registrar::Window aiWindow("device.aiAgents", "AiAgentsWindow");
// Settings > AI Agents (Application, after Networking)
static const SettingsRegistry::Page aiPage{
    "ai", SettingsRegistry::Application, 47, QT_TRANSLATE_NOOP("SettingsPages", "AI Agents"), "bot", "AiAgentsPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Allow AI agents"), QT_TRANSLATE_NOOP("SettingsPages", "MCP server"),
      QT_TRANSLATE_NOOP("SettingsPages", "Connect an agent"), QT_TRANSLATE_NOOP("SettingsPages", "Claude Desktop, Claude Code, Cursor"),
      QT_TRANSLATE_NOOP("SettingsPages", "Clients and tokens"), QT_TRANSLATE_NOOP("SettingsPages", "Ask before changes"),
      QT_TRANSLATE_NOOP("SettingsPages", "Agent activity"), QT_TRANSLATE_NOOP("SettingsPages", "Port"),
      QT_TRANSLATE_NOOP("SettingsPages", "Use from another computer") } };

namespace {

const char *const kEnabledKey = "mcp/enabled";
const char *const kConfirmKey = "mcp/confirm";
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
    m_tick.setInterval(1000);
    connect(&m_tick, &QTimer::timeout, this, &AiController::tick);
    m_readSoon.setSingleShot(true);
    m_readSoon.setInterval(100);
    connect(&m_readSoon, &QTimer::timeout, this, &AiController::readActivity);
    connect(ctx.registry, &DeviceRegistry::devicesChanged, this, &AiController::onDevices);
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
    m_thread.waitForDone();
    if (!m_mcp) return;
    // Waits for a call in progress; puts the agents' timed previews back
    dspi_mcp_stop(m_mcp);
    dspi_mcp_set_listener(m_mcp, nullptr, nullptr);
    dspi_mcp_free(m_mcp);
    m_mcp = nullptr;
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
    }
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
    const bool recent = m_lastCall.isValid() && m_lastCall.secsTo(now) < kRecentSeconds;
    if (!recent) m_lastCall = QDateTime();
    emit stateChanged();
    if (m_pairing.isEmpty() && m_waiting.isEmpty() && !recent) m_tick.stop();
}

// ---- Starting and stopping ---------------------------------------------------------

void AiController::setEnabled(bool on) {
    if (on == m_enabled) return;
    m_enabled = on;
    QSettings().setValue(QLatin1String(kEnabledKey), on);
    if (!on) m_lastError.clear();   // a failed start is tried again at the next switch-on
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
                     for (const QJsonValue &r : c.value(QLatin1String("risks")).toArray()) w.risks << str(r.toObject(), "what");
                     w.ends = QDateTime::currentDateTimeUtc().addMSecs(c.value(QLatin1String("remaining_ms")).toInteger());
                     there.insert(w.id);
                     list.append(w);
                 }
                 m_answered.intersect(there);
                 if (!list.isEmpty()) m_tick.start();
                 // Only a change: the rows' buttons are armed anew with each
                 if (list == m_waiting) return;
                 m_waiting = list;
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
            { QStringLiteral("risks"), w.risks },
        });
    }
    return out;
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
                 // A write may have begun, kept or reverted a timed preview: the
                 // safe apply's banner reads it again
                 QString newest;
                 for (qsizetype i = m_activity.size(); i-- > 0 && newest.isEmpty();) {
                     const QJsonObject e = m_activity.at(i).toObject();
                     if (e.value(QLatin1String("write")).toBool()) newest = rowKey(e);
                 }
                 if (newest == m_lastWrite) return;
                 m_lastWrite = newest;
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
        else if (write && outcome == QLatin1String("ok") && !journalIds(e).isEmpty())
            undo = m_ctx.registry->session(device) ? QStringLiteral("can") : QStringLiteral("closed");
        const QString title = str(e, "title");
        rows.append(QVariantMap{
            { QStringLiteral("key"), key },
            { QStringLiteral("time"),
              QLocale().toString(QDateTime::fromMSecsSinceEpoch(e.value(QLatin1String("at_unix_ms")).toInteger()).time(), QLocale::ShortFormat) },
            { QStringLiteral("client"), str(e, "client_name") },
            { QStringLiteral("title"), title.isEmpty() ? str(e, "tool") : title },
            { QStringLiteral("detail"), str(e, "detail") },
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
