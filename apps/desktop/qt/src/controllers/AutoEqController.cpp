// -- WP QP15 -- AutoEQ: the database, the browser's list, favourites, applying
// a profile, and updating the database (rebuild, import, reset)
#include "AutoEqController.h"

#include "DeviceModel.h"
#include "FilesController.h"
#include "Shell.h"

#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QEventLoop>
#include <QFile>
#include <QFileInfo>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocale>
#include <QNetworkAccessManager>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QSettings>
#include <QStandardPaths>

static const Registrar::Controller<AutoEqController> autoEqController;
static const Registrar::Window autoEqWindow("autoeq.browse", "AutoEqWindow");

namespace {
const char kFavorites[] = "autoeq/favorites";
// The input pair a profile goes to (the Mac's master left and right)
constexpr int kProfileChannels[] = { 0, 1 };
// How long the finished rebuild stays on screen before its alert
constexpr int kDoneShownMs = 1500;
constexpr int kPollMs = 100;
constexpr int kFetchTimeoutMs = 30000;
constexpr qint64 kFetchLimit = 16 << 20;
// A database file picked to import: far above the built-in one's 5 MB
constexpr qint64 kImportLimit = 64 << 20;
// The reply buffer of the calls that change the file (AUTOEQ_REPLY_MIN or more)
constexpr uint32_t kReplyBytes = 16 << 10;

std::mutex s_overrideLock;
AutoEqController::FetchOverride s_override;

uint8_t filterType(const QString &name) {
    if (name == QLatin1String("peaking")) return FILTER_PEAKING;
    if (name == QLatin1String("lowShelf")) return FILTER_LOWSHELF;
    if (name == QLatin1String("highShelf")) return FILTER_HIGHSHELF;
    if (name == QLatin1String("lowPass")) return FILTER_LOWPASS;
    if (name == QLatin1String("highPass")) return FILTER_HIGHPASS;
    return FILTER_FLAT;
}

QString sourceName(const QString &source) {
    if (source == QLatin1String("crinacle")) return QStringLiteral("Crinacle");
    if (source == QLatin1String("rtings")) return QStringLiteral("Rtings");
    if (source == QLatin1String("innerfidelity")) return QStringLiteral("InnerFidelity");
    if (source == QLatin1String("headphone.com")) return QStringLiteral("Headphone.com");
    return source;
}

// "18 Jan 2026" from the database's ISO time; the text itself if it is none
QString shownDate(const QString &iso) {
    QDateTime t = QDateTime::fromString(iso, Qt::ISODateWithMs);
    if (!t.isValid()) {
        // Fractions longer than milliseconds: drop them
        QString trimmed = iso;
        const int dot = trimmed.indexOf(QLatin1Char('.'));
        if (dot > 0) {
            int end = dot + 1;
            while (end < trimmed.size() && trimmed.at(end).isDigit()) ++end;
            trimmed.remove(dot, end - dot);
        }
        t = QDateTime::fromString(trimmed, Qt::ISODate);
    }
    return t.isValid() ? QLocale().toString(t.date(), QStringLiteral("d MMM yyyy")) : iso;
}

// One call of a function that changes the database file: never twice
QJsonObject callOnce(const std::function<uint32_t(char *, uint32_t)> &fill) {
    QByteArray buf(int(kReplyBytes), '\0');
    const uint32_t need = fill(buf.data(), kReplyBytes);
    if (need == 0 || need > kReplyBytes) return {};
    buf.truncate(int(need) - 1);
    return QJsonDocument::fromJson(buf).object();
}

qint64 headerNumber(const QNetworkReply *reply, const char *name) {
    bool ok = false;
    const qint64 v = reply->rawHeader(name).trimmed().toLongLong(&ok);
    return ok ? v : -1;
}
} // namespace

QString AutoEqProfile::displayName() const {
    return model.isEmpty() ? manufacturer : manufacturer + QLatin1Char(' ') + model;
}

// ---- The browser's list --------------------------------------------------------

int AutoEqProfileModel::rowCount(const QModelIndex &parent) const { return parent.isValid() ? 0 : int(m_rows.size()); }

QVariant AutoEqProfileModel::data(const QModelIndex &index, int role) const {
    if (!m_profiles || !index.isValid() || index.row() >= m_rows.size()) return {};
    const AutoEqProfile &p = m_profiles->at(m_rows.at(index.row()));
    switch (role) {
    case ProfileId: return p.id;
    case DisplayName: return p.displayName();
    case Source: return p.source;
    case SourceName: return sourceName(p.source);
    case FormFactor: return p.formFactor;
    case Favorite: return m_favorites.contains(p.id);
    default: return {};
    }
}

QHash<int, QByteArray> AutoEqProfileModel::roleNames() const {
    return { { ProfileId, "profileId" }, { DisplayName, "displayName" }, { Source, "source" },
             { SourceName, "sourceName" }, { FormFactor, "formFactor" }, { Favorite, "favorite" } };
}

void AutoEqProfileModel::setQuery(const QString &query) {
    if (query == m_query) return;
    m_query = query;
    emit queryChanged();
    refilter();
}

void AutoEqProfileModel::setProfiles(std::shared_ptr<const QVector<AutoEqProfile>> profiles) {
    m_profiles = std::move(profiles);
    refilter();
}

void AutoEqProfileModel::setFavorites(const QStringList &ids) {
    m_favorites = ids;
    if (!m_rows.isEmpty()) emit dataChanged(index(0), index(int(m_rows.size()) - 1), { Favorite });
}

// The Mac's search: the manufacturer, the model or the whole name contains the text
void AutoEqProfileModel::refilter() {
    beginResetModel();
    m_rows.clear();
    if (m_profiles) {
        const QString q = m_query.trimmed();
        for (int i = 0; i < m_profiles->size(); ++i) {
            const AutoEqProfile &p = m_profiles->at(i);
            if (q.isEmpty() || p.manufacturer.contains(q, Qt::CaseInsensitive) || p.model.contains(q, Qt::CaseInsensitive) ||
                p.displayName().contains(q, Qt::CaseInsensitive))
                m_rows.append(i);
        }
    }
    endResetModel();
    emit countChanged();
}

int AutoEqProfileModel::rowOf(const QString &id) const {
    if (!m_profiles) return -1;
    for (int r = 0; r < m_rows.size(); ++r)
        if (m_profiles->at(m_rows.at(r)).id == id) return r;
    return -1;
}

QString AutoEqProfileModel::idAt(int row) const {
    return m_profiles && row >= 0 && row < m_rows.size() ? m_profiles->at(m_rows.at(row)).id : QString();
}

// ---- The controller ------------------------------------------------------------

AutoEqController::AutoEqController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx), m_model(new AutoEqProfileModel(this)) {
    m_files.setMaxThreadCount(1);
    m_files.setExpiryTimeout(-1);
    m_favoriteIds = QSettings().value(QLatin1String(kFavorites)).toStringList();
    m_model->setFavorites(m_favoriteIds);
    // Polling the rebuild is not a device read, so not the Poller's: it runs
    // with no device open
    m_rebuildPoll.setInterval(kPollMs);
    connect(&m_rebuildPoll, &QTimer::timeout, this, &AutoEqController::pollRebuild);

    ctx.shell->addHandler(QStringLiteral("autoeq.clearFavorites"), this, [this] { clearFavorites(); });
    ctx.shell->addHandler(QStringLiteral("autoeq.updateDatabase"), this, [this] { emit updateRequested(); });
    connect(ctx.device, &DeviceModel::snapshotApplied, this, [this] {
        if (canApply() == m_lastCanApply) return;
        m_lastCanApply = canApply();
        emit canApplyChanged();
    });
    connect(qApp, &QCoreApplication::aboutToQuit, this, &AutoEqController::freeRebuild);
    reload();
}

AutoEqController::~AutoEqController() {
    freeRebuild();
    m_files.waitForDone();
}

AutoEqController *AutoEqController::create(QQmlEngine *, QJSEngine *) {
    AutoEqController *self = Registrar::instance<AutoEqController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

QString AutoEqController::userDatabasePath() {
    return QStandardPaths::writableLocation(QStandardPaths::AppDataLocation) + QStringLiteral("/autoeq_database.json");
}

QString AutoEqController::builtInDatabasePath() { return QStringLiteral(":/autoeq/autoeq_database.json"); }

bool AutoEqController::hasUserDatabase() const { return QFileInfo::exists(userDatabasePath()); }

bool AutoEqController::canApply() const { return m_ctx.device->connected() && !m_ctx.device->readOnly(); }

// The user's database when there is one, else the built-in one. Any thread.
AutoEqController::Loaded AutoEqController::load(const QString &path) {
    Loaded out;
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) {
        out.error = tr("AutoEQ database not found");
        return out;
    }
    QJsonParseError parse{};
    const QJsonDocument doc = QJsonDocument::fromJson(f.readAll(), &parse);
    const QJsonObject root = doc.object();
    if (parse.error != QJsonParseError::NoError || !root.value(QLatin1String("entries")).isArray()) {
        out.error = tr("Failed to load database: %1")
                        .arg(parse.error != QJsonParseError::NoError ? parse.errorString() : tr("no profiles"));
        return out;
    }
    auto profiles = std::make_shared<QVector<AutoEqProfile>>();
    const QJsonArray entries = root.value(QLatin1String("entries")).toArray();
    profiles->reserve(entries.size());
    for (const QJsonValue &v : entries) {
        const QJsonObject e = v.toObject();
        AutoEqProfile p;
        p.id = e.value(QLatin1String("id")).toString();
        if (p.id.isEmpty()) continue;
        p.manufacturer = e.value(QLatin1String("manufacturer")).toString();
        p.model = e.value(QLatin1String("model")).toString();
        p.source = e.value(QLatin1String("source")).toString();
        p.formFactor = e.value(QLatin1String("formFactor")).toString();
        p.preamp = float(e.value(QLatin1String("preamp")).toDouble());
        for (const QJsonValue &b : e.value(QLatin1String("filters")).toArray()) {
            const QJsonObject o = b.toObject();
            p.bands.append({ filterType(o.value(QLatin1String("type")).toString()),
                             float(o.value(QLatin1String("freq")).toDouble(1000)),
                             float(o.value(QLatin1String("q")).toDouble(0.707)),
                             float(o.value(QLatin1String("gain")).toDouble()) });
        }
        profiles->append(std::move(p));
    }
    out.profiles = profiles;
    out.date = shownDate(root.value(QLatin1String("generatedAt")).toString());
    return out;
}

void AutoEqController::reload(std::function<void()> then) {
    m_loading = true;
    emit databaseChanged();
    const QString path = hasUserDatabase() ? userDatabasePath() : builtInDatabasePath();
    m_files.start([this, path, then] {
        Loaded loaded = load(path);
        QMetaObject::invokeMethod(this, [this, loaded, then] {
            m_loading = false;
            m_error = loaded.error;
            m_date = loaded.date;
            m_profiles = loaded.profiles;
            m_byId.clear();
            if (m_profiles)
                for (int i = 0; i < m_profiles->size(); ++i) m_byId.insert(m_profiles->at(i).id, i);
            m_model->setProfiles(m_profiles);
            emit databaseChanged();
            emit favoritesChanged();
            if (then) then();
        }, Qt::QueuedConnection);
    });
}

QVariantMap AutoEqController::profile(const QString &id) const {
    const auto it = m_byId.constFind(id);
    if (!m_profiles || it == m_byId.cend()) return {};
    const AutoEqProfile &p = m_profiles->at(*it);
    const bool inEar = p.formFactor == QLatin1String("in-ear") || p.formFactor == QLatin1String("earbud");
    return { { QStringLiteral("name"), p.displayName() },
             { QStringLiteral("sourceName"), sourceName(p.source) },
             { QStringLiteral("formFactor"), p.formFactor },
             { QStringLiteral("formFactorIcon"), inEar ? QStringLiteral("ear") : QStringLiteral("headphones") } };
}

// ---- Favourites ----------------------------------------------------------------

QVariantList AutoEqController::favorites() const {
    QVariantList out;
    if (!m_profiles) return out;
    for (const QString &id : m_favoriteIds) {
        const auto it = m_byId.constFind(id);
        if (it == m_byId.cend()) continue;
        const AutoEqProfile &p = m_profiles->at(*it);
        out.append(QVariantMap{ { QStringLiteral("id"), id },
                                { QStringLiteral("title"), p.manufacturer + QLatin1Char(' ') + p.model } });
    }
    return out;
}

bool AutoEqController::isFavorite(const QString &id) const { return m_favoriteIds.contains(id); }

void AutoEqController::setFavoriteIds(const QStringList &ids) {
    m_favoriteIds = ids;
    QSettings s;
    if (ids.isEmpty()) s.remove(QLatin1String(kFavorites));
    else s.setValue(QLatin1String(kFavorites), ids);
    m_model->setFavorites(ids);
    emit favoritesChanged();
}

void AutoEqController::toggleFavorite(const QString &id) {
    if (!m_byId.contains(id)) return;
    QStringList ids = m_favoriteIds;
    if (!ids.removeAll(id)) ids.append(id);
    setFavoriteIds(ids);
}

void AutoEqController::clearFavorites() { setFavoriteIds({}); }

// ---- Applying ------------------------------------------------------------------

void AutoEqController::apply(const QString &id) {
    const auto it = m_byId.constFind(id);
    if (!m_profiles || it == m_byId.cend() || !canApply()) return;
    const AutoEqProfile p = m_profiles->at(*it);
    m_ctx.device->write(QStringLiteral("autoeq apply"), [p](FfiCore *c) {
        bool ok = true;
        for (const int ch : kProfileChannels) {
            // Per input, not the global preamp, which trims every input
            ok = dspi_set_input_preamp(c, uint8_t(ch), p.preamp) && ok;
            for (int band = 0; band < BANDS_PER_CHANNEL; ++band) {
                FilterParams f{};
                if (band < p.bands.size()) {
                    const AutoEqProfile::Band &b = p.bands.at(band);
                    f = { b.type, false, b.freq, b.q, b.gain, 0.0f };
                } else {
                    // The bands past the profile go flat, so nothing stale stays
                    f = { FILTER_FLAT, false, 1000.0f, 0.707f, 0.0f, 0.0f };
                }
                ok = dspi_set_filter(c, uint8_t(ch), uint8_t(band), f) && ok;
            }
        }
        return ok;
    });
}

// ---- Updating the database -------------------------------------------------------

QString AutoEqController::updateText() const {
    return tr("Current database: %1\nEntries: %2\n\nChoose an update method:")
        .arg(m_date.isEmpty() ? tr("Unknown") : m_date)
        .arg(entryCount());
}

void AutoEqController::setUpdating(bool updating) {
    if (updating == m_updating) return;
    m_updating = updating;
    emit updatingChanged();
}

void AutoEqController::chooseDatabaseFile() {
    FilesController::pickFile(false, tr("Import AutoEQ Database"), tr("AutoEQ database (*.json)"), QString(), this,
                              [this](const QString &path) { importDatabaseFrom(path); });
}

void AutoEqController::importDatabaseFrom(const QString &path) {
    if (m_updating) return;
    setUpdating(true);
    const QString target = userDatabasePath();
    m_files.start([this, path, target] {
        QString error;
        QJsonObject reply;
        QFile f(path);
        if (!f.open(QIODevice::ReadOnly)) {
            error = f.errorString();
        } else {
            const QByteArray data = f.read(kImportLimit);
            QDir().mkpath(QFileInfo(target).absolutePath());
            const QByteArray db = target.toUtf8();
            reply = callOnce([&](char *b, uint32_t n) {
                return dspi_autoeq_import(db.constData(), reinterpret_cast<const uint8_t *>(data.constData()),
                                          uint32_t(data.size()), b, n);
            });
            if (!reply.value(QLatin1String("ok")).toBool()) error = reply.value(QLatin1String("error")).toString();
        }
        QMetaObject::invokeMethod(this, [this, error] {
            if (!error.isEmpty()) {
                setUpdating(false);
                emit notice(tr("Error"), tr("Failed to import database: %1").arg(error), true);
                return;
            }
            reload([this] {
                setUpdating(false);
                emit notice(tr("Success"), tr("Database updated successfully.\nEntries: %1").arg(entryCount()), false);
            });
        }, Qt::QueuedConnection);
    });
}

void AutoEqController::resetToBuiltIn() {
    if (m_updating) return;
    setUpdating(true);
    const QByteArray db = userDatabasePath().toUtf8();
    m_files.start([this, db] {
        const QJsonObject reply =
            callOnce([&](char *b, uint32_t n) { return dspi_autoeq_reset_to_builtin(db.constData(), b, n); });
        const QString error =
            reply.value(QLatin1String("ok")).toBool() ? QString() : reply.value(QLatin1String("error")).toString();
        QMetaObject::invokeMethod(this, [this, error] {
            if (!error.isEmpty()) {
                setUpdating(false);
                emit notice(tr("Error"), tr("Failed to reset: %1").arg(error), true);
                return;
            }
            reload([this] {
                setUpdating(false);
                emit notice(tr("Success"), tr("Reset to built-in database.\nEntries: %1").arg(entryCount()), false);
            });
        }, Qt::QueuedConnection);
    });
}

// ---- Rebuilding from GitHub --------------------------------------------------------

void AutoEqController::startRebuild() {
    if (m_rebuild || m_rebuildShown) return;
    const QString target = userDatabasePath();
    QDir().mkpath(QFileInfo(target).absolutePath());
    m_rebuildPath = target.toUtf8();
    m_rebuild = dspi_autoeq_rebuild_start(m_rebuildPath.constData(), &AutoEqController::fetchThunk, this);
    if (!m_rebuild) {
        emit notice(tr("Error"), tr("Rebuild failed: %1").arg(tr("the database file cannot be written.")), true);
        return;
    }
    m_rebuildShown = true;
    m_rebuildDone = false;
    m_rebuildProgress = 0;
    m_rebuildStatus = tr("Connecting to GitHub...");
    emit rebuildChanged();
    m_rebuildPoll.start();
}

void AutoEqController::cancelRebuild() {
    if (m_rebuild) dspi_autoeq_rebuild_cancel(m_rebuild);
}

// The core's progress; never blocks
void AutoEqController::pollRebuild() {
    if (!m_rebuild) return;
    AutoEqRebuildHandle *h = m_rebuild;
    QByteArray buf(1024, '\0');
    uint32_t need = dspi_autoeq_rebuild_poll(h, buf.data(), uint32_t(buf.size()));
    if (need > uint32_t(buf.size())) {
        buf = QByteArray(int(need), '\0');
        need = dspi_autoeq_rebuild_poll(h, buf.data(), uint32_t(buf.size()));
    }
    if (need == 0 || need > uint32_t(buf.size())) return;
    buf.truncate(int(need) - 1);
    const QJsonObject o = QJsonDocument::fromJson(buf).object();
    const QString status = o.value(QLatin1String("status")).toString();
    m_rebuildProgress = qBound(0.0, o.value(QLatin1String("fraction")).toDouble(), 1.0);
    if (!status.isEmpty()) m_rebuildStatus = status;
    emit rebuildChanged();
    if (!o.value(QLatin1String("finished")).toBool()) return;
    m_rebuildPoll.stop();
    const bool succeeded = o.value(QLatin1String("succeeded")).toBool();
    const bool cancelled = o.value(QLatin1String("code")).toString() == QLatin1String("cancelled");
    freeRebuild();
    if (cancelled) endRebuild(false, QString());
    else endRebuild(succeeded, status);
}

void AutoEqController::endRebuild(bool succeeded, const QString &status) {
    if (!succeeded) {
        m_rebuildShown = false;
        emit rebuildChanged();
        if (!status.isEmpty()) emit notice(tr("Error"), tr("Rebuild failed: %1").arg(status), true);
        return;
    }
    // Keep the sheet up briefly to show it finished, as the Mac does
    m_rebuildProgress = 1.0;
    m_rebuildDone = true;
    emit rebuildChanged();
    reload([this] {
        QTimer::singleShot(kDoneShownMs, this, [this] {
            m_rebuildShown = false;
            m_rebuildDone = false;
            emit rebuildChanged();
            emit notice(tr("Success"), tr("Database rebuilt successfully!\nEntries: %1").arg(entryCount()), false);
        });
    });
}

// Waits out the requests in flight (none once the rebuild has finished)
void AutoEqController::freeRebuild() {
    if (!m_rebuild) return;
    m_rebuildPoll.stop();
    AutoEqRebuildHandle *h = std::exchange(m_rebuild, nullptr);
    dspi_autoeq_rebuild_free(h);
}

void AutoEqController::setFetchOverride(FetchOverride fetch) {
    std::lock_guard<std::mutex> hold(s_overrideLock);
    s_override = std::move(fetch);
}

// The rebuild's threads, up to 20 at once
void AutoEqController::fetchThunk(void *, const char *url, const char *accept, const char *agent, AutoEqFetchSink *sink) {
    FetchOverride scripted;
    {
        std::lock_guard<std::mutex> hold(s_overrideLock);
        scripted = s_override;
    }
    const QByteArray u(url), a(accept ? accept : ""), ua(agent);
    const Answer answer = scripted ? scripted(u, a, ua) : fetch(u, a, ua);
    if (!answer.answered) {
        dspi_autoeq_fetch_fail(sink, answer.error.toUtf8().constData());
        return;
    }
    dspi_autoeq_fetch_respond(sink, uint16_t(answer.status), reinterpret_cast<const uint8_t *>(answer.body.constData()),
                              uint32_t(answer.body.size()), answer.retryAfter, answer.remaining, answer.reset);
}

// Its own manager and event loop on the calling thread, so it never waits on
// the GUI thread
AutoEqController::Answer AutoEqController::fetch(const QByteArray &url, const QByteArray &accept, const QByteArray &userAgent) {
    Answer out;
    QNetworkAccessManager manager;
    QNetworkRequest request(QUrl::fromEncoded(url));
    request.setRawHeader("User-Agent", userAgent);
    if (!accept.isEmpty()) request.setRawHeader("Accept", accept);
    request.setTransferTimeout(kFetchTimeoutMs);
    request.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::NoLessSafeRedirectPolicy);
    QNetworkReply *reply = manager.get(request);
    QEventLoop loop;
    QObject::connect(reply, &QNetworkReply::finished, &loop, &QEventLoop::quit);
    if (!reply->isFinished()) loop.exec();
    out.status = reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
    if (out.status == 0) {
        out.error = reply->errorString();
    } else {
        out.answered = true;
        out.body = reply->read(kFetchLimit);
        out.retryAfter = headerNumber(reply, "Retry-After");
        out.remaining = headerNumber(reply, "X-RateLimit-Remaining");
        out.reset = headerNumber(reply, "X-RateLimit-Reset");
    }
    delete reply;
    return out;
}
