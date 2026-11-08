#include "AutoEqLibrary.h"
#include "DSPiBridge.h"

#include <QCoreApplication>
#include <QDateTime>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocale>
#include <QNetworkAccessManager>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QSettings>
#include <QStandardPaths>
#include <QThread>

extern "C" {
#include "dspi_core.h"
}

// The core's fetch and what stays with it for as long as the core may call it
struct AutoEqFetchContext {
    AutoEqLibrary::Fetcher fetch;
    std::atomic_bool stop{ false };
};

namespace {
constexpr int kBands = 10;
const char *kFavoritesKey = "autoeq/favorites";
constexpr int kPollMs = 100;
constexpr qint64 kFetchTimeoutMs = 30000;       // as the Terminal's: a cancel waits for requests in flight
constexpr int kFetchTickMs = 100;               // how soon a cancel reaches a request
// Far past what the core accepts (64 MiB); it refuses, in its own words, what
// reaches it over its limit
constexpr qint64 kMaxBytes = 128 * 1024 * 1024;

class FunctionThread : public QThread {
public:
    explicit FunctionThread(const std::function<void()> &work) : m_work(work) {}
    void run() override { m_work(); }
private:
    std::function<void()> m_work;
};

qint64 headerValue(const QNetworkReply *reply, const char *name) {
    bool ok = false;
    const qint64 v = reply->rawHeader(name).trimmed().toLongLong(&ok);  // a date is not a number: absent
    return ok && v >= 0 ? v : -1;
}

// The server answered, and the whole answer came. A connection lost part way has
// a status too, but its body is not the server's answer.
bool isAnswer(const QNetworkReply *reply, int status) {
    const QNetworkReply::NetworkError e = reply->error();
    return status > 0 && (e == QNetworkReply::NoError || e >= QNetworkReply::ContentAccessDenied
                          || e == QNetworkReply::ProtocolInvalidOperationError)
        && e != QNetworkReply::ProtocolFailure;
}

void fetchCallback(void *userData, const char *url, const char *accept, const char *userAgent, AutoEqFetchSink *sink) {
    const auto *context = static_cast<const AutoEqFetchContext *>(userData);
    try {
        const AutoEqFetchAnswer a = context->fetch(QByteArray(url), accept ? QByteArray(accept) : QByteArray(),
                                                   QByteArray(userAgent), context->stop);
        if (a.answered && a.body.size() <= kMaxBytes)
            dspi_autoeq_fetch_respond(sink, quint16(qBound(0, a.status, 65535)),
                                      reinterpret_cast<const uint8_t *>(a.body.constData()), uint32_t(a.body.size()),
                                      a.retryAfterS, a.rateLimitRemaining, a.rateLimitReset);
        else
            dspi_autoeq_fetch_fail(sink, a.answered ? QCoreApplication::translate("AutoEqLibrary", "The answer was too large.").toUtf8().constData()
                                                 : a.failure.toUtf8().constData());
    } catch (...) {     // Nothing may unwind into the core
        dspi_autoeq_fetch_fail(sink, QCoreApplication::translate("AutoEqLibrary", "The download failed.").toUtf8().constData());
    }
}

QJsonObject jsonOf(const QByteArray &text) { return QJsonDocument::fromJson(QByteArray(text.constData())).object(); }

QString replyError(const QJsonObject &reply, const QString &otherwise) {
    const QString error = reply.value("error").toString();
    return error.isEmpty() ? otherwise : error;
}

// A rebuild can succeed with parts missing, and this is how the person hears of it
QString rebuiltMessage(const QJsonObject &p) {
    const int profiles = p.value("failedProfiles").toInt();
    const int listings = p.value("failedListings").toInt();
    QString message = QCoreApplication::translate("AutoEqLibrary", "Database rebuilt successfully!\nEntries: %1").arg(p.value("entries").toInt());
    if (profiles > 0)
        message += "\n" + (profiles == 1 ? QCoreApplication::translate("AutoEqLibrary", "%1 profile could not be downloaded.")
                                         : QCoreApplication::translate("AutoEqLibrary", "%1 profiles could not be downloaded.")).arg(profiles);
    if (listings > 0)
        message += "\n" + (listings == 1 ? QCoreApplication::translate("AutoEqLibrary", "%1 folder could not be listed, so whatever it holds is missing.")
                                         : QCoreApplication::translate("AutoEqLibrary", "%1 folders could not be listed, so whatever they hold is missing.")).arg(listings);
    if (profiles > 0 || listings > 0) message += "\n" + QCoreApplication::translate("AutoEqLibrary", "Rebuild again to try for these.");
    return message;
}

QString sourceDisplayName(const QString &s) {
    if (s == "crinacle") return "Crinacle";
    if (s == "rtings") return "Rtings";
    if (s == "innerfidelity") return "InnerFidelity";
    if (s == "headphone.com") return "Headphone.com";
    return s;
}

int filterType(const QString &t) {
    if (t == "peaking") return 1;
    if (t == "lowShelf") return 2;
    if (t == "highShelf") return 3;
    if (t == "lowPass") return 4;
    if (t == "highPass") return 5;
    return 0;
}

}

// ── Model ──

int AutoEqModel::rowCount(const QModelIndex &parent) const { return parent.isValid() ? 0 : m_rows.size(); }

QVariant AutoEqModel::data(const QModelIndex &index, int role) const {
    if (!m_entries || !index.isValid() || index.row() >= m_rows.size()) return {};
    const AutoEqEntry &e = (*m_entries)[m_rows[index.row()]];
    switch (role) {
    case IdRole: return e.id;
    case NameRole: return e.displayName();
    case SourceRole: return e.source;
    case SourceNameRole: return sourceDisplayName(e.source);
    case FormFactorRole: return e.formFactor;
    case FavoriteRole: return m_favorites && m_favorites->contains(e.id);
    default: return {};
    }
}

QHash<int, QByteArray> AutoEqModel::roleNames() const {
    return { { IdRole, "entryId" }, { NameRole, "name" }, { SourceRole, "source" },
             { SourceNameRole, "sourceName" }, { FormFactorRole, "formFactor" }, { FavoriteRole, "favorite" } };
}

void AutoEqModel::reset(const QVector<AutoEqEntry> *entries, const QVector<int> &rows, const QSet<QString> *favorites) {
    beginResetModel();
    m_entries = entries;
    m_rows = rows;
    m_favorites = favorites;
    endResetModel();
}

void AutoEqModel::favoriteChanged(const QString &id) {
    for (int r = 0; r < m_rows.size(); r++)
        if ((*m_entries)[m_rows[r]].id == id) {
            emit dataChanged(index(r), index(r), { FavoriteRole });
            return;
        }
}

// ── Library ──

AutoEqLibrary::AutoEqLibrary(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge), m_fetcher(&AutoEqLibrary::httpGet)
{
    m_poll.setInterval(kPollMs);
    connect(&m_poll, &QTimer::timeout, this, &AutoEqLibrary::pollRebuild);
    // Favourites keep their names, so the menu needs no database
    const QStringList saved = QSettings().value(kFavoritesKey).toStringList();
    for (const QString &item : saved) {
        const int bar = item.indexOf('\t');
        const QString id = bar < 0 ? item : item.left(bar);
        m_favorites.insert(id);
        m_favoriteList.append(QVariantMap{ { "id", id }, { "name", bar < 0 ? id : item.mid(bar + 1) } });
    }
}

AutoEqLibrary::~AutoEqLibrary() {
    retireRebuild();
    for (QThread *t : m_workers) {
        t->wait();
        delete t;
    }
}

QString AutoEqLibrary::userPath() const {
    return QStandardPaths::writableLocation(QStandardPaths::AppDataLocation) + "/autoeq_database.json";
}

bool AutoEqLibrary::hasUserDatabase() const { return QFile::exists(userPath()); }

bool AutoEqLibrary::readDatabase(const QByteArray &json, QString *error) {
    QJsonParseError pe;
    const QJsonObject root = QJsonDocument::fromJson(json, &pe).object();
    const QJsonArray list = root.value("entries").toArray();
    if (pe.error != QJsonParseError::NoError || list.isEmpty()) {
        if (error) *error = tr("This is not an AutoEQ database file.");
        return false;
    }
    QVector<AutoEqEntry> entries;
    entries.reserve(list.size());
    for (const QJsonValue &v : list) {
        const QJsonObject o = v.toObject();
        AutoEqEntry e;
        e.id = o.value("id").toString();
        e.manufacturer = o.value("manufacturer").toString();
        e.model = o.value("model").toString();
        e.source = o.value("source").toString();
        e.formFactor = o.value("formFactor").toString();
        e.preamp = o.value("preamp").toDouble();
        for (const QJsonValue &f : o.value("filters").toArray()) {
            const QJsonObject fo = f.toObject();
            e.filters.append({ fo.value("type").toString(), fo.value("freq").toDouble(1000),
                               fo.value("q").toDouble(0.707), fo.value("gain").toDouble() });
        }
        if (!e.id.isEmpty()) entries.append(e);
    }
    m_entries = entries;
    m_byId.clear();
    for (int i = 0; i < m_entries.size(); i++) m_byId.insert(m_entries[i].id, i);
    const QDateTime when = QDateTime::fromString(root.value("generatedAt").toString(), Qt::ISODateWithMs);
    m_date = when.isValid() ? QLocale().toString(when.toLocalTime().date(), QLocale::ShortFormat)
                            : root.value("generatedAt").toString();
    return true;
}

void AutoEqLibrary::load() {
    if (m_loaded) return;
    m_loaded = true;
    m_error.clear();
    QFile user(userPath());
    QFile bundled(":/autoeq/autoeq_database.json");
    QFile &f = user.exists() ? user : bundled;
    if (!f.open(QIODevice::ReadOnly) || !readDatabase(f.readAll(), nullptr))
        m_error = tr("The AutoEQ database could not be read.");
    filter();
    emit databaseChanged();
}

void AutoEqLibrary::setQuery(const QString &q) {
    if (q == m_query) return;
    m_query = q;
    emit queryChanged();
    filter();
}

void AutoEqLibrary::filter() {
    QVector<int> rows;
    const QString q = m_query.trimmed().toLower();
    for (int i = 0; i < m_entries.size(); i++) {
        const AutoEqEntry &e = m_entries[i];
        if (q.isEmpty() || e.manufacturer.toLower().contains(q) || e.model.toLower().contains(q)
            || e.displayName().toLower().contains(q))
            rows.append(i);
    }
    m_model.reset(&m_entries, rows, &m_favorites);
}

QVariantMap AutoEqLibrary::entry(const QString &id) {
    load();
    const auto it = m_byId.constFind(id);
    if (it == m_byId.constEnd()) return {};
    const AutoEqEntry &e = m_entries[*it];
    return { { "id", e.id }, { "name", e.displayName() }, { "source", sourceDisplayName(e.source) },
             { "formFactor", e.formFactor } };
}

void AutoEqLibrary::toggleFavorite(const QString &id) {
    load();
    if (m_favorites.contains(id)) {
        m_favorites.remove(id);
        for (int i = 0; i < m_favoriteList.size(); i++)
            if (m_favoriteList[i].toMap().value("id") == id) { m_favoriteList.removeAt(i); break; }
    } else {
        const auto it = m_byId.constFind(id);
        if (it == m_byId.constEnd()) return;
        m_favorites.insert(id);
        m_favoriteList.append(QVariantMap{ { "id", id }, { "name", m_entries[*it].displayName() } });
    }
    QStringList saved;
    for (const QVariant &v : m_favoriteList) {
        const QVariantMap m = v.toMap();
        saved << m.value("id").toString() + "\t" + m.value("name").toString();
    }
    QSettings().setValue(kFavoritesKey, saved);
    m_model.favoriteChanged(id);
    emit favoritesChanged();
}

void AutoEqLibrary::clearFavorites() {
    const QSet<QString> was = m_favorites;
    m_favorites.clear();
    m_favoriteList.clear();
    QSettings().remove(kFavoritesKey);
    for (const QString &id : was) m_model.favoriteChanged(id);
    emit favoritesChanged();
}

bool AutoEqLibrary::apply(const QString &id) {
    if (!m_bridge->connected()) return false;
    load();
    const auto it = m_byId.constFind(id);
    if (it == m_byId.constEnd()) return false;
    const AutoEqEntry &e = m_entries[*it];
    // Inputs 1 and 2 (app channels 0 and 1): the preamp, the profile's
    // filters, and every remaining band off
    for (int input = 0; input < 2; input++) m_bridge->setInputPreamp(input, float(e.preamp));
    for (int ch = 0; ch < 2; ch++)
        for (int band = 0; band < kBands; band++) {
            if (band < e.filters.size()) {
                const AutoEqEntry::Filter &f = e.filters[band];
                m_bridge->setFilter(ch, band, filterType(f.type), float(f.freq), float(f.gain), float(f.q));
            } else {
                m_bridge->setFilter(ch, band, 0, 1000.0f, 0.0f, 0.707f);
            }
        }
    return true;
}

// ── Updating the database ──

void AutoEqLibrary::runOffGui(const std::function<void()> &work, const std::function<void()> &done) {
    auto *thread = new FunctionThread(work);
    m_workers.append(thread);
    connect(thread, &QThread::finished, this, [this, thread, done]() {
        m_workers.removeAll(thread);
        thread->deleteLater();
        if (done) done();
    });
    thread->start();
}

void AutoEqLibrary::importFile(const QUrl &file) {
    if (updateRunning()) {
        QTimer::singleShot(0, this, [this]() { emit updateFinished(false, tr("Import Failed"), tr("Another update of the database is running.")); });
        return;
    }
    m_updating = true;
    const QString source = file.toLocalFile();
    const QByteArray path = userPath().toUtf8();
    auto reply = std::make_shared<QByteArray>();
    auto opened = std::make_shared<bool>(false);
    runOffGui([source, path, reply, opened]() {
        QFile f(source);
        *opened = f.open(QIODevice::ReadOnly);
        if (!*opened) return;
        const QByteArray data = f.read(kMaxBytes);
        reply->fill('\0', AUTOEQ_REPLY_MIN);
        dspi_autoeq_import(path.constData(), reinterpret_cast<const uint8_t *>(data.constData()), uint32_t(data.size()),
                           reply->data(), AUTOEQ_REPLY_MIN);
    }, [this, reply, opened]() {
        m_updating = false;
        if (!*opened) { emit updateFinished(false, tr("Import Failed"), tr("The file could not be opened.")); return; }
        const QJsonObject r = jsonOf(*reply);
        if (!r.value("ok").toBool()) { emit updateFinished(false, tr("Import Failed"), replyError(r, tr("The database could not be imported."))); return; }
        m_loaded = false;
        load();
        emit updateFinished(true, QString(), tr("Database updated successfully.\nEntries: %1").arg(m_entries.size()));
    });
}

void AutoEqLibrary::resetToBuiltIn() {
    if (updateRunning()) {
        QTimer::singleShot(0, this, [this]() { emit updateFinished(false, tr("Reset Failed"), tr("Another update of the database is running.")); });
        return;
    }
    m_updating = true;
    const QByteArray path = userPath().toUtf8();
    auto reply = std::make_shared<QByteArray>(int(AUTOEQ_REPLY_MIN), '\0');
    runOffGui([path, reply]() {
        dspi_autoeq_reset_to_builtin(path.constData(), reply->data(), AUTOEQ_REPLY_MIN);
    }, [this, reply]() {
        m_updating = false;
        const QJsonObject r = jsonOf(*reply);
        if (!r.value("ok").toBool()) { emit updateFinished(false, tr("Reset Failed"), replyError(r, tr("The updated database could not be removed."))); return; }
        m_loaded = false;
        load();
        emit updateFinished(true, QString(), tr("Reset to built-in database.\nEntries: %1").arg(m_entries.size()));
    });
}

QString AutoEqLibrary::rebuildWarning() const {
    const QByteArray path = userPath().toUtf8();
    const uint32_t needed = dspi_autoeq_rebuild_warning(path.constData(), nullptr, 0);
    QByteArray text(int(needed) + 1, '\0');
    dspi_autoeq_rebuild_warning(path.constData(), text.data(), uint32_t(text.size()));
    return QString::fromUtf8(text.constData());
}

void AutoEqLibrary::setRebuild(bool on, double progress, const QString &status) {
    m_rebuilding = on;
    m_progress = progress;
    m_status = status;
    emit rebuildChanged();
}

void AutoEqLibrary::startRebuild() {
    if (m_rebuilding) return;
    const auto refuse = [this](const QString &why) {
        // After the caller has opened the progress window, which this closes
        QTimer::singleShot(0, this, [this, why]() { emit rebuildFinished(false, tr("Rebuild failed: %1").arg(why)); });
    };
    if (m_updating) { refuse(tr("Another update of the database is running.")); return; }
    // Qt 5 sets up its network bearer on the first manager ever made, waiting on
    // the main thread if that is another thread's. Do that here, so a fetch never
    // has to.
    { QNetworkAccessManager firstOnThisThread; }
    auto context = std::make_shared<AutoEqFetchContext>();
    context->fetch = m_fetcher;
    m_handle = dspi_autoeq_rebuild_start(userPath().toUtf8().constData(), &fetchCallback, context.get());
    if (!m_handle) { refuse(tr("The database's location could not be used.")); return; }
    m_fetchContext = context;
    setRebuild(true, 0, tr("Connecting to GitHub..."));
    m_poll.start();
}

void AutoEqLibrary::cancelRebuild() {
    if (!m_rebuilding) return;
    retireRebuild();
    setRebuild(false, 0, QString());
}

void AutoEqLibrary::retireRebuild() {
    m_poll.stop();
    if (!m_handle) return;
    AutoEqRebuildHandle *handle = m_handle;
    const std::shared_ptr<AutoEqFetchContext> context = m_fetchContext;     // valid until the free has returned
    m_handle = nullptr;
    m_fetchContext.reset();
    context->stop = true;
    dspi_autoeq_rebuild_cancel(handle);
    runOffGui([handle, context]() { dspi_autoeq_rebuild_free(handle); }, nullptr);
}

void AutoEqLibrary::pollRebuild() {
    if (!m_handle) return;
    QByteArray text(1024, '\0');
    uint32_t needed = dspi_autoeq_rebuild_poll(m_handle, text.data(), uint32_t(text.size()));
    if (needed > uint32_t(text.size())) {
        text.fill('\0', int(needed));
        needed = dspi_autoeq_rebuild_poll(m_handle, text.data(), uint32_t(text.size()));
        if (needed > uint32_t(text.size())) return;     // Grew again: the next tick reads it
    }
    const QJsonObject p = jsonOf(text);
    const QString status = p.value("status").toString();
    const double fraction = p.value("fraction").toDouble();
    if (!p.value("finished").toBool()) {
        if (status != m_status || fraction != m_progress) setRebuild(true, fraction, status);
        return;
    }
    retireRebuild();    // Over: nothing left to wait for
    if (p.value("succeeded").toBool()) {
        m_loaded = false;
        load();
        setRebuild(false, 1.0, status);
        emit rebuildFinished(true, rebuiltMessage(p));
    } else {
        setRebuild(false, 0, QString());
        if (p.value("code").toString() != "cancelled") emit rebuildFinished(false, tr("Rebuild failed: %1").arg(status));
    }
}

// ── The core's HTTP GET ──

AutoEqFetchAnswer AutoEqLibrary::httpGet(const QByteArray &url, const QByteArray &accept,
                                         const QByteArray &userAgent, const std::atomic_bool &stop) {
    AutoEqFetchAnswer answer;
    {
        QEventLoop loop;    // First: this gives a thread the core started an event dispatcher
        QNetworkAccessManager net;
        QNetworkRequest request(QUrl::fromEncoded(url));
        // Follows redirects, and never from https to http. TLS verification is
        // Qt's default, on: no certificate error is ever ignored here.
        request.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::NoLessSafeRedirectPolicy);
        request.setRawHeader("User-Agent", userAgent);
        if (!accept.isEmpty()) request.setRawHeader("Accept", accept);

        QNetworkReply *reply = net.get(request);
        QString stopped;
        QObject::connect(reply, &QNetworkReply::finished, &loop, &QEventLoop::quit);
        QObject::connect(reply, &QNetworkReply::downloadProgress, &loop, [&](qint64 received, qint64) {
            if (received > kMaxBytes && stopped.isEmpty()) { stopped = tr("The answer was too large."); reply->abort(); }
        });
        QElapsedTimer elapsed;
        elapsed.start();
        QTimer tick;
        tick.setInterval(kFetchTickMs);
        QObject::connect(&tick, &QTimer::timeout, &loop, [&]() {
            if (reply->isFinished() || !stopped.isEmpty()) return;
            if (stop) stopped = tr("Cancelled.");
            else if (elapsed.elapsed() > kFetchTimeoutMs) stopped = tr("The request timed out.");
            else return;
            reply->abort();
        });
        tick.start();
        if (!reply->isFinished()) loop.exec();
        tick.stop();

        const int status = reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
        if (stopped.isEmpty() && isAnswer(reply, status)) {
            answer.answered = true;
            answer.status = status;
            answer.body = reply->readAll();
            answer.retryAfterS = headerValue(reply, "Retry-After");
            answer.rateLimitRemaining = headerValue(reply, "X-RateLimit-Remaining");
            answer.rateLimitReset = headerValue(reply, "X-RateLimit-Reset");
        } else {
            answer.failure = stopped.isEmpty() ? reply->errorString() : stopped;
        }
        delete reply;
    }
    // The manager leaves deleteLater()s behind, and this thread has no loop to come
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    return answer;
}
