// -- WP P2QT -- Device > Events…: this console's event log and the hub's
// shared history (PLAN-P2 M1, P2CC1)
#include "EventsController.h"

#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceSession.h"   // -- WP MDQT2b --
#include "HubFeed.h"
#include "NoticeCenter.h"
#include "Snapshot.h"

#include <QDateTime>
#include <QPointer>
#include <QDir>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLocale>
#include <QStandardPaths>

static const Registrar::Controller<EventsController> eventsController;
static const Registrar::Window eventsWindow("device.events", "EventsWindow");

namespace {
// The local log's read: whole JSON lines, as many as fit (a record's line is at
// most 4 KiB, so none is ever dropped)
constexpr uint32_t kLocalBytes = 512 << 10;

QJsonObject object(const QByteArray &reply) { return QJsonDocument::fromJson(reply).object(); }
bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }

struct Page {
    bool ok = true;
    QJsonArray records;   // oldest first
    QJsonObject reply;    // the hub's reply (its failure)
};

// The open device's newest records in this console's log. Worker thread.
Page localPage(FfiCore *c) {
    QByteArray buf(int(kLocalBytes), '\0');
    dspi_event_log_read(c, nullptr, 0, uint32_t(EventsController::kShown), buf.data(), kLocalBytes);
    buf.truncate(int(qstrnlen(buf.constData(), kLocalBytes)));
    Page p;
    for (const QByteArray &line : buf.split('\n'))
        if (!line.trimmed().isEmpty()) p.records.append(object(line));
    return p;
}
} // namespace

EventsController::EventsController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx), m_session(ctx.activeSession()) {
    // The core keeps the log in the app's data folder, one file per serial
    const QString dir = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation) + QStringLiteral("/events");
    const QByteArray d = QDir::toNativeSeparators(dir).toUtf8();
    ctx.core->worker()->post([d](FfiCore *c) { dspi_event_log_set_dir(c, d.constData()); });
    m_poll = ctx.poller->add(QStringLiteral("event log"), kLocalPollMs, this, &localPage, [this](const Page &p) {
        if (m_source == QLatin1String("local")) applyLocal(p.records);
    });
    ctx.poller->setEnabled(m_poll, false);
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &EventsController::onSnapshot);
    if (HubFeed *feed = HubFeed::of(ctx.core)) connect(feed, &HubFeed::hubEvent, this, &EventsController::onHubEvent);
}

EventsController *EventsController::create(QQmlEngine *, QJSEngine *) {
    EventsController *self = Registrar::instance<EventsController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

void EventsController::track(int delta) {
    const bool was = busy();
    m_busy += delta;
    if (busy() != was) emit busyChanged();
}

QVariantMap EventsController::row(const QJsonObject &r) {
    // The local log's time is `t`, the hub's `timeMs`
    const qint64 ms = r.contains(QLatin1String("timeMs")) ? qint64(r.value(QLatin1String("timeMs")).toDouble())
                                                         : qint64(r.value(QLatin1String("t")).toDouble());
    const QDateTime at = QDateTime::fromMSecsSinceEpoch(ms).toLocalTime();
    QStringList lines;
    for (const QJsonValue &l : r.value(QLatin1String("lines")).toArray()) lines.append(l.toString());
    const int count = r.value(QLatin1String("count")).toInt(1);
    return {
        { QStringLiteral("id"), r.value(QLatin1String("id")).toDouble() },
        { QStringLiteral("time"), QLocale().toString(at.time(), QStringLiteral("HH:mm:ss")) },
        { QStringLiteral("day"), at.toString(QStringLiteral("yyyy-MM-dd")) },
        { QStringLiteral("text"), r.value(QLatin1String("text")).toString() },
        { QStringLiteral("by"), r.value(QLatin1String("by")).toString() },
        { QStringLiteral("detail"), lines.join(QStringLiteral("; ")) },
        // -- WP SHOWFIX -- (finding 2) the device's own record, for details
        { QStringLiteral("raw"), r.value(QLatin1String("raw")).toString() },
        { QStringLiteral("count"), count < 1 ? 1 : count },
    };
}

void EventsController::setSource(const QString &source) {
    const QString s = source == QLatin1String("hub") && m_hubAvailable ? QStringLiteral("hub") : QStringLiteral("local");
    if (s == m_source) return;
    m_source = s;
    ++m_generation;
    m_entries.clear();
    m_problem.clear();
    emit sourceChanged();
    emit entriesChanged();
    m_ctx.poller->setEnabled(m_poll, m_active && m_source == QLatin1String("local"));
    updateWatch();
    reload();
}

void EventsController::setActive(bool on) {
    if (on == m_active) return;
    m_active = on;
    emit activeChanged();
    m_ctx.poller->setEnabled(m_poll, m_active && m_source == QLatin1String("local"));
    updateWatch();
    if (on) reload();
}

void EventsController::onSnapshot() {
    const bool connected = m_ctx.device->connected();
    const QString serial = m_ctx.device->serial();
    const bool remote = m_ctx.device->remote();
    // -- WP MDQT2b -- another device is the active one: the hub's watch of the
    // one left ends there, and its list is not this one's
    const bool switched = m_ctx.activeSession() != m_session;
    if (switched) {
        if (m_watching && m_session)
            m_session->worker()->post([](FfiCore *c) {
                char buf[1024];
                dspi_hub_events_watch(c, false, nullptr, buf, sizeof buf);
            });
        m_watching = false;
        m_session = m_ctx.activeSession();
        if (!m_entries.isEmpty()) {
            m_entries.clear();
            emit entriesChanged();
        }
    }
    if (!switched && connected == m_connected && serial == m_serial && remote == m_remote) return;
    m_connected = connected;
    m_serial = serial;
    m_remote = remote;
    // The hub's watch was for the session that went (the hub forgets it)
    m_watching = false;
    const int generation = ++m_generation;
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            const QJsonObject caps = object(readSized([c](char *b, uint32_t n) { return dspi_hub_caps(c, b, n); }));
            return caps.value(QLatin1String("onHub")).toBool() && caps.value(QLatin1String("events")).toBool();
        },
        [this, generation](bool hub) {
            if (generation != m_generation) return;
            if (hub != m_hubAvailable) {
                m_hubAvailable = hub;
                emit hubAvailableChanged();
            }
            if (!hub && m_source == QLatin1String("hub")) {
                setSource(QStringLiteral("local"));
                return;
            }
            updateWatch();
            if (m_active) reload();
        });
}

void EventsController::reload() {
    if (m_source == QLatin1String("hub")) readHub();
    else readLocal();
}

void EventsController::readLocal() {
    if (m_source != QLatin1String("local")) return;
    if (!m_ctx.device->connected()) {
        if (!m_entries.isEmpty()) {
            m_entries.clear();
            emit entriesChanged();
        }
        return;
    }
    const int generation = m_generation;
    track(1);
    m_ctx.core->worker()->call(this, &localPage, [this, generation](const Page &p) {
        track(-1);
        if (generation == m_generation && m_source == QLatin1String("local")) applyLocal(p.records);
    });
}

void EventsController::applyLocal(const QJsonArray &records) {
    QVariantList rows;
    for (qsizetype i = records.size() - 1; i >= 0; --i) rows.append(row(records.at(i).toObject()));
    const bool changed = rows != m_entries || !m_problem.isEmpty();
    m_entries = rows;
    m_problem.clear();
    if (changed) emit entriesChanged();
    emit loaded(QStringLiteral("local"), true);
}

void EventsController::readHub() {
    if (!m_hubAvailable || !m_ctx.device->connected()) return;
    const int generation = m_generation;
    track(1);
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            Page p;
            p.reply = object(readSized(
                [c](char *b, uint32_t n) { return dspi_hub_events_read(c, nullptr, 0, uint32_t(kShown), b, n); }, 64 << 10));
            p.ok = isOk(p.reply);
            p.records = p.reply.value(QLatin1String("records")).toArray();
            return p;
        },
        [this, generation](const Page &p) {
            track(-1);
            if (generation != m_generation || m_source != QLatin1String("hub")) return;
            QVariantList rows;
            m_newestHubId = 0;
            for (qsizetype i = p.records.size() - 1; i >= 0; --i) {
                const QJsonObject r = p.records.at(i).toObject();
                m_newestHubId = std::max(m_newestHubId, quint64(r.value(QLatin1String("id")).toDouble()));
                rows.append(row(r));
            }
            m_entries = rows;
            m_problem.clear();
            if (!p.ok) {
                // The hub's words, or its notice row's
                const QString key = p.reply.value(QLatin1String("noticeKey")).toString();
                const QString error = p.reply.value(QLatin1String("error")).toString();
                auto *notices = Registrar::instance<NoticeCenter>();
                m_problem = notices && !key.isEmpty() ? notices->line(key, error) : error;
                if (m_problem.isEmpty()) m_problem = tr("The hub's history could not be read.");
            }
            emit entriesChanged();
            emit loaded(QStringLiteral("hub"), p.ok);
        });
}

void EventsController::updateWatch() {
    const bool want = m_active && m_source == QLatin1String("hub") && m_hubAvailable && m_connected;
    if (want == m_watching) return;
    m_watching = want;
    m_ctx.core->worker()->post([want](FfiCore *c) {
        char buf[1024];
        dspi_hub_events_watch(c, want, nullptr, buf, sizeof buf);
    });
}

// A new record on the hub: at the top, when it is the open device's
void EventsController::onHubEvent(const QString &kind, const QJsonObject &event) {
    if (kind != QLatin1String("event.added") || m_source != QLatin1String("hub")) return;
    const QString serial = event.value(QLatin1String("serial")).toString();
    if (!serial.isEmpty() && serial != m_serial) return;
    const QJsonObject record = event.value(QLatin1String("record")).toObject();
    const quint64 id = quint64(record.value(QLatin1String("id")).toDouble());
    // One already listed (the read and the event crossed)
    if (record.isEmpty() || id <= m_newestHubId) return;
    m_newestHubId = id;
    m_entries.prepend(row(record));
    while (m_entries.size() > kShown) m_entries.removeLast();
    emit entriesChanged();
}
