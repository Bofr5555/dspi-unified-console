// -- WP MDQT1 -- The open devices, one active
#include "DeviceRegistry.h"

#include "CoreApi.h"
#include "DeviceSession.h"
#include "SessionModel.h"
#include "Snapshot.h"

#include <QCoreApplication>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QPointer>
#include <algorithm>

namespace {
DeviceRegistry *s_instance = nullptr;

// Opens and closes act once, so their reply is asked for once, with room for
// any (fns_core_set.rs, dspi_core_set_open: 4096 bytes)
constexpr uint32_t kActBuffer = 8 * 1024;
// How long quitting waits for an open in progress (a hub's dial has its own
// budget); one that is still running is left to the process's end
constexpr int kQuitWaitMs = 2000;
// -- WP MDQT2c -- How long closing a device waits for the Stop of what plays on
// it; a call stuck on a hub ends then
constexpr int kStopWaitMs = 2000;

QJsonObject actOnce(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(int(kActBuffer), '\0');
    const uint32_t need = call(buf.data(), kActBuffer);
    if (need == 0 || need > kActBuffer) return QJsonObject();
    buf.truncate(int(need) - 1);
    return QJsonDocument::fromJson(buf).object();
}

bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }

QThreadPool *oneThread(QObject *parent) {
    auto *pool = new QThreadPool(parent);
    pool->setMaxThreadCount(1);
    pool->setExpiryTimeout(-1);
    return pool;
}
} // namespace

DeviceRegistry::DeviceRegistry(FfiCoreSet *set, QObject *parent)
    : QObject(parent), m_set(set), m_main(new DeviceSession(dspi_core_set_main(set), true, this)), m_active(m_main),
      m_scanner(oneThread(nullptr)), m_opener(oneThread(nullptr)) {
    s_instance = this;
    m_sessions.append(m_main);
    track(m_main);
    dspi_core_set_set_listener(m_set, &DeviceRegistry::eventThunk, this);
    m_hotplugTimer.setInterval(500);
    connect(&m_hotplugTimer, &QTimer::timeout, this, &DeviceRegistry::pollHotplug);
}

DeviceRegistry::~DeviceRegistry() {
    shutdown();
    if (s_instance == this) s_instance = nullptr;
}

DeviceRegistry *DeviceRegistry::instance() { return s_instance; }

DeviceRegistry *DeviceRegistry::create(QQmlEngine *, QJSEngine *) {
    QJSEngine::setObjectOwnership(s_instance, QJSEngine::CppOwnership);
    return s_instance;
}

template <class Job, class After>
void DeviceRegistry::run(QThreadPool *pool, Job job, After after) {
    // The application is the context, not this: an open in progress at the quit
    // may end after this is gone
    QPointer<DeviceRegistry> guard(this);
    pool->start([guard, job = std::move(job), after = std::move(after)]() mutable {
        auto result = job();
        QCoreApplication *app = QCoreApplication::instance();
        if (!app) return;
        QMetaObject::invokeMethod(
            app,
            [guard, after = std::move(after), result = std::move(result)]() mutable {
                if (guard && !guard->isShutDown()) after(std::move(result));
            },
            Qt::QueuedConnection);
    });
}

void DeviceRegistry::track(DeviceSession *session) {
    connect(session, &DeviceSession::identityChanged, this, &DeviceRegistry::readRows);
    connect(session, &DeviceSession::clipHoldChanged, this, &DeviceRegistry::devicesChanged);
    // -- WP MDQT2c --
    connect(session, &DeviceSession::jobsChanged, this, &DeviceRegistry::devicesChanged);
    // -- WP UA3 -- the tab's unsaved mark, for the device shown or not
    connect(session->model(), &SessionModel::applied, this, [this, session](uint) { readUnsaved(session); });
    connect(session, &DeviceSession::identityChanged, this, [this, session] { readUnsaved(session); });
    connect(session->model(), &SessionModel::replacing, this, [this, session] { emit sessionStopping(session); });
    // -- WP XDEV1 -- (review RB B4) DeviceModel forwards only the active one's
    connect(session->model(), &SessionModel::writeRefused, this,
            [this, session](const QString &what, const QString &key, const QString &message) {
                if (session != m_active) emit backgroundRefused(session->title(), what, key, message);
            });
}

// ---- What is open --------------------------------------------------------------

DeviceSession *DeviceRegistry::session(const QString &id) const {
    if (id.isEmpty()) return nullptr;
    for (DeviceSession *s : m_sessions)
        if (s->id() == id) return s;
    return nullptr;
}

DeviceSession *DeviceRegistry::holder(const QString &serial) const {
    if (serial.isEmpty()) return nullptr;
    for (DeviceSession *s : m_sessions) {
        const SessionModel *m = s->model();
        if (!m->demo() && !m->document() && m->serial() == serial) return s;
    }
    return nullptr;
}

bool DeviceRegistry::isEmpty() const {
    return m_sessions.size() == 1 && !m_main->model()->connected() && !m_main->model()->remote() &&
           !m_main->model()->demo() && m_main->model()->serial().isEmpty();
}

QVariantList DeviceRegistry::devices() const {
    QVariantList out;
    for (DeviceSession *s : m_sessions) {
        QVariantMap row = s->row();
        if (row.isEmpty()) continue;
        row.insert(QStringLiteral("title"), s->title());
        row.insert(QStringLiteral("active"), s == m_active);
        row.insert(QStringLiteral("clip"), s->clipHold() != 0);
        row.insert(QStringLiteral("busy"), s->busy());   // -- WP MDQT2c --
        row.insert(QStringLiteral("job"), s->job());     // -- WP MDQT2c --
        // -- WP UA3 --
        const bool unsaved = dirty(s);
        const QStringList changes = unsaved ? unsavedChanges(s) : QStringList();
        row.insert(QStringLiteral("dirty"), unsaved);
        row.insert(QStringLiteral("unsaved"), int(changes.size()));
        row.insert(QStringLiteral("changes"), changes);
        out.append(row);
    }
    return out;
}

QString DeviceRegistry::activeId() const { return m_active->id(); }

void DeviceRegistry::setKeepOpen(bool keep) {
    if (keep == m_keepOpen) return;
    m_keepOpen = keep;
    // -- WP XDEV1 -- the policy point (header): leaving Modern
    if (!keep && !kOthersStayOpenInClassic) {
        const QList<DeviceSession *> all = m_sessions;
        for (DeviceSession *s : all)
            if (s != m_active && !s->id().isEmpty()) close(s->id());
    }
    emit keepOpenChanged();
}

// The thread of the call that caused it, no lock of the set held: schedule only
void DeviceRegistry::eventThunk(const char *, void *self) {
    QMetaObject::invokeMethod(static_cast<DeviceRegistry *>(self), &DeviceRegistry::readRows, Qt::QueuedConnection);
}

void DeviceRegistry::readRows() {
    if (!m_set || m_rowsQueued.exchange(true)) return;
    QList<FfiCore *> cores;
    for (DeviceSession *s : std::as_const(m_sessions)) cores.append(s->handle());
    FfiCoreSet *set = m_set;
    std::atomic<bool> *queued = &m_rowsQueued;
    run(
        m_scanner,
        [set, cores, queued] {
            queued->store(false);   // an event from here on reads again
            // Takes in what the cores have open now, then each core's row
            readSized([set](char *b, uint32_t n) { return dspi_core_set_list(set, b, n); }, 4096);
            QList<QVariantMap> rows;
            for (FfiCore *c : cores)
                rows.append(jsonValue(readSized([set, c](char *b, uint32_t n) { return dspi_core_set_device_of(set, c, b, n); }))
                                .toObject()
                                .toVariantMap());
            return rows;
        },
        [this, cores](const QList<QVariantMap> &rows) {
            bool all = true;
            for (DeviceSession *s : std::as_const(m_sessions)) {
                const qsizetype at = cores.indexOf(s->handle());
                if (at >= 0) s->setRow(rows.at(at));
                else all = false;
            }
            emit devicesChanged();
            // -- WP LAY7 -- one opened while this reading was queued, whose own
            // reading found it queued and left it at that
            if (!all) readRows();
        });
}

// -- WP UA3 -- The core's dirty flag and lines of each open device, read after
// every snapshot applied on it (as SidebarController reads the active one's)
bool DeviceRegistry::dirty(DeviceSession *session) const {
    return session && session->model()->connected() && m_unsaved.value(session).dirty;
}

QStringList DeviceRegistry::unsavedChanges(DeviceSession *session) const {
    return dirty(session) ? m_unsaved.value(session).changes : QStringList();
}

void DeviceRegistry::readUnsaved(DeviceSession *session) {
    if (!m_set || !session->handle() || !m_sessions.contains(session)) return;
    Unsaved &u = m_unsaved[session];
    if (u.inFlight) {
        u.again = true;
        return;
    }
    u.inFlight = true;
    session->worker()->call(
        session,
        [](FfiCore *c) {
            QStringList lines;
            const QJsonArray list =
                QJsonDocument::fromJson(readSized([c](char *b, uint32_t n) { return dspi_preset_changes(c, b, n); })).array();
            for (const QJsonValue &v : list)
                lines.append(v.toString());
            return std::make_pair(dspi_preset_dirty(c), lines);
        },
        [this, session](const std::pair<bool, QStringList> &r) {
            if (!m_sessions.contains(session)) {
                m_unsaved.remove(session);
                return;
            }
            Unsaved &u = m_unsaved[session];
            u.inFlight = false;
            const bool moved = u.dirty != r.first || u.changes != r.second;
            u.dirty = r.first;
            u.changes = r.second;
            if (std::exchange(u.again, false)) readUnsaved(session);
            if (moved) emit devicesChanged();
        });
}

// ---- Start and end -------------------------------------------------------------

void DeviceRegistry::configure(const QJsonObject &config) {
    FfiCoreSet *set = m_set;
    const QByteArray json = QJsonDocument(config).toJson(QJsonDocument::Compact);
    // On the main session's worker, in order before the first open there
    m_main->worker()->call(
        this,
        [set, json](FfiCore *) {
            return actOnce([&](char *b, uint32_t n) { return dspi_core_set_config(set, json.constData(), b, n); });
        },
        [this](const QJsonObject &reply) {
            if (isOk(reply)) return;
            const QString error = reply.value(QLatin1String("error")).toString();
            qWarning("DeviceRegistry: a data folder was not set: %s", qPrintable(error));
            emit configFailed(error);
        });
}

void DeviceRegistry::startHotplug() {
    if (!m_set || m_hotplugTimer.isActive()) return;
    FfiCoreSet *set = m_set;
    // The core announces every unit present as arrived otherwise
    m_main->worker()->post([set](FfiCore *c) {
        dspi_scan_devices(c, nullptr, 0);
        dspi_core_set_scan_devices(set, nullptr, 0);
    });
    m_hotplugTimer.start();
}

void DeviceRegistry::pollHotplug() {
    // One poll at a time: a replug's reconnect can take a few hundred ms
    if (m_pollInFlight.exchange(true)) return;
    FfiCoreSet *set = m_set;
    std::atomic<bool> *inFlight = &m_pollInFlight;
    m_scanner->start([set, inFlight] {
        dspi_core_set_poll_hotplug(set);
        inFlight->store(false);
    });
}

void DeviceRegistry::shutdown() {
    if (!m_set) return;
    m_hotplugTimer.stop();
    dspi_core_set_set_listener(m_set, nullptr, nullptr);
    for (DeviceSession *s : std::as_const(m_sessions)) s->shutdown();
    // Those on their way out too: their cores go with the set below
    for (DeviceSession *s : std::as_const(m_closing)) s->shutdown();
    m_scanner->waitForDone();
    delete m_scanner;
    const bool quiet = m_opener->waitForDone(kQuitWaitMs);
    FfiCoreSet *set = m_set;
    m_set = nullptr;
    if (!quiet) {
        // An open is still dialling: it ends with the process
        qInfo("DeviceRegistry: an open in progress was left at the quit");
        return;
    }
    delete m_opener;
    // Clears the callbacks first, waiting for one in progress (header contract)
    dspi_core_set_free(set);
}

// ---- Opening, choosing, closing ------------------------------------------------

QString DeviceRegistry::alreadyOpen(const QString &serial) {
    //: %1 is the device's serial number
    return tr("%1 is already open here, and a device has one session at a time. Choose it under Open devices.").arg(serial);
}

void DeviceRegistry::reportRefused(const QString &serial) {
    emit openFailed(QStringLiteral("already_open"), refusalFor(serial));
}

// -- WP XDEV1 -- held by a session, or by a measurement across units
QString DeviceRegistry::refusalFor(const QString &serial) const {
    if (!holder(serial) && extraSerials().contains(serial))
        //: %1 is the device's serial number
        return tr("%1 is being measured with other units here (Measure, Several units), and a device has one session at a time. "
                  "Take it out of that measurement first.")
            .arg(serial);
    return alreadyOpen(serial);
}

void DeviceRegistry::openExtra(const QJsonObject &target, ExtraDone done) {
    const QString serial = target.value(QLatin1String("serial")).toString();
    if (!m_set) {
        if (done) done(nullptr, QString(), QStringLiteral("io"), tr("The app is closing."));
        return;
    }
    m_extraOpening.append(serial);
    FfiCoreSet *set = m_set;
    // -- WP INT5 -- not a device the user has open: AI agents are not offered it
    QJsonObject extra = target;
    extra.insert(QStringLiteral("extra"), true);
    const QByteArray json = QJsonDocument(extra).toJson(QJsonDocument::Compact);
    struct Opened {
        QJsonObject reply;
        FfiCore *core = nullptr;
    };
    run(
        m_opener,
        [set, json] {
            Opened o;
            o.reply = actOnce([&](char *b, uint32_t n) { return dspi_core_set_open(set, json.constData(), b, n); });
            if (!isOk(o.reply)) return o;
            const QByteArray id = o.reply.value(QLatin1String("id")).toString().toUtf8();
            o.core = dspi_core_set_core(set, id.constData());
            return o;
        },
        [this, serial, done](const Opened &o) {
            m_extraOpening.removeOne(serial);
            const QString id = o.reply.value(QLatin1String("id")).toString();
            if (!o.core) {
                QString code = o.reply.value(QLatin1String("code")).toString();
                if (code.isEmpty()) code = QStringLiteral("io");
                QString error = o.reply.value(QLatin1String("error")).toString();
                if (code == QLatin1String("already_open")) error = alreadyOpen(serial);
                if (done) done(nullptr, QString(), code, error);
                emit extrasChanged();   // -- WP LAY7 --
                return;
            }
            m_extras.insert(id, serial);
            if (done) done(o.core, id, QString(), QString());
            emit extrasChanged();   // -- WP LAY7 --
        });
}

void DeviceRegistry::closeExtra(const QString &id) {
    if (!m_set || !m_extras.contains(id)) return;
    FfiCoreSet *set = m_set;
    const QByteArray wire = id.toUtf8();
    // Its serial is held until the set gave it back
    run(
        m_opener, [set, wire] { return isOk(actOnce([&](char *b, uint32_t n) { return dspi_core_set_close(set, wire.constData(), b, n); })); },
        [this, id](bool) {
            m_extras.remove(id);
            emit extrasChanged();   // -- WP LAY7 --
        });
}

void DeviceRegistry::open(const QJsonObject &target, Done done, bool activate) {
    if (!m_set) return;
    struct Opened {
        QJsonObject reply;
        FfiCore *core = nullptr;
        DeviceSnapshot snap;
    };
    ++m_opening;
    FfiCoreSet *set = m_set;
    const QByteArray json = QJsonDocument(target).toJson(QJsonDocument::Compact);
    run(
        m_opener,
        [set, json] {
            Opened o;
            o.reply = actOnce([&](char *b, uint32_t n) { return dspi_core_set_open(set, json.constData(), b, n); });
            if (!isOk(o.reply)) return o;
            const QByteArray id = o.reply.value(QLatin1String("id")).toString().toUtf8();
            o.core = dspi_core_set_core(set, id.constData());
            if (o.core) {
                dspi_fetch_all(o.core);
                o.snap = readSnapshot(o.core);
            }
            return o;
        },
        [this, target, done, activate](const Opened &o) {
            --m_opening;
            if (!o.core) {
                QString code = o.reply.value(QLatin1String("code")).toString();
                if (code.isEmpty()) code = QStringLiteral("io");
                QString error = o.reply.value(QLatin1String("error")).toString();
                qInfo("DeviceRegistry: not opened (%s): %s", qPrintable(code), qPrintable(error));
                if (code == QLatin1String("already_open"))
                    error = refusalFor(target.value(QLatin1String("serial")).toString());   // -- WP XDEV1 --
                emit openFailed(code, error);
                if (done) done(nullptr, code, error);
                return;
            }
            auto *session = new DeviceSession(o.core, false, this);
            const QString kind = target.value(QLatin1String("kind")).toString();
            m_sessions.append(session);
            track(session);
            session->setBackground(true);
            session->model()->adoptOpened(kind, target.value(QLatin1String("serial")).toString(), o.snap);
            // Until the set's own row is read
            session->setRow({ { QStringLiteral("id"), o.reply.value(QLatin1String("id")).toString() },
                              { QStringLiteral("label"), session->model()->deviceTitle() },
                              { QStringLiteral("transport"), kind == QLatin1String("document") ? QStringLiteral("file") : kind },
                              { QStringLiteral("serial"), session->model()->serial() },
                              { QStringLiteral("state"), QStringLiteral("live") } });
            readRows();
            emit sessionOpened(session);
            emit devicesChanged();
            if (activate) this->activate(session);
            if (done) done(session, QString(), QString());
        });
}

void DeviceRegistry::openDemo() {
#ifdef DSPI_SMOKE
    // -- WP UB12 -- as SessionModel::openDemo: the smoke build's is the blank one
    const QString kind = SessionModel::smokeDemoExample ? QStringLiteral("demo") : QStringLiteral("demoBlank");
#else
    const QString kind = QStringLiteral("demo");
#endif
    if (isEmpty()) m_main->model()->openDemo();
    else open({ { QStringLiteral("kind"), kind } });
}

void DeviceRegistry::openUsb(const QString &serial) {
    if (serial.isEmpty()) return;
    if (isEmpty()) m_main->model()->selectDevice(serial);
    else open({ { QStringLiteral("kind"), QStringLiteral("usb") }, { QStringLiteral("serial"), serial } });
}

bool DeviceRegistry::activate(const QString &id) {
    DeviceSession *s = session(id);
    if (!s) return false;
    activate(s);
    return true;
}

void DeviceRegistry::activate(DeviceSession *session) {
    if (!session || session == m_active || !m_sessions.contains(session)) return;
    if (m_stopping.contains(session)) return;   // -- WP MDQT2c -- it is being closed
    emit activeLeaving();   // -- WP MDQT2a --
    if (session == m_active || !m_sessions.contains(session)) return;
    m_active->setBackground(true);
    m_active = session;
    m_active->setBackground(false);
    emit activeMoved();
    emit activeChanged();
    emit devicesChanged();
}

void DeviceRegistry::close(const QString &id) {
    DeviceSession *s = session(id);
    if (!s || !m_set || m_stopping.contains(s)) return;
    if (s == m_active) {
        // The neighbour before it, else the one after; the main session is
        // there with no device too
        const qsizetype at = m_sessions.indexOf(s);
        DeviceSession *next = nullptr;
        for (qsizetype i = at - 1; i >= 0 && !next; --i)
            if (!m_sessions.at(i)->id().isEmpty() && !m_stopping.contains(m_sessions.at(i))) next = m_sessions.at(i);
        for (qsizetype i = at + 1; i < m_sessions.size() && !next; ++i)
            if (!m_sessions.at(i)->id().isEmpty() && !m_stopping.contains(m_sessions.at(i))) next = m_sessions.at(i);
        activate(next ? next : m_main);
    }
    const QByteArray wire = id.toUtf8();
    // -- WP MDQT2c -- What plays on it stops first, while it is there
    emit sessionStopping(s);
    const bool drain = s->takeDrain();
    if (s->isMain() || !drain) {
        endSession(s, wire);
        return;
    }
    m_stopping.append(s);
    const auto end = [this, s, wire] {
        if (!m_stopping.removeOne(s) || !m_set) return;
        endSession(s, wire);
    };
    QTimer::singleShot(kStopWaitMs, s, end);
    s->worker()->call(s, [](FfiCore *) { return true; }, [end](bool) { end(); });
}

void DeviceRegistry::endSession(DeviceSession *s, const QByteArray &wire) {
    FfiCoreSet *set = m_set;
    const auto closeIt = [set, wire] {
        return isOk(actOnce([&](char *b, uint32_t n) { return dspi_core_set_close(set, wire.constData(), b, n); }));
    };
    if (s->isMain()) {
        // Its core and callbacks stay: closed in order with its other calls
        s->model()->closed([closeIt](FfiCore *) { closeIt(); });
        return;
    }
    // Let go of it first (LinkController ends its session, so a call stuck on
    // a hub ends); then its worker runs dry, and only then its core closes
    emit sessionClosing(s);
    m_sessions.removeOne(s);
    m_unsaved.remove(s);   // -- WP UA3 --
    m_closing.append(s);
    s->setBackground(false);
    emit devicesChanged();
    if (FfiCore *c = s->handle()) dspi_link_close(c);
    s->worker()->call(
        s, [closeIt](FfiCore *) { return closeIt(); },
        [this, s](bool) {
            m_closing.removeOne(s);
            s->shutdown();
            s->deleteLater();
            readRows();
        });
}
