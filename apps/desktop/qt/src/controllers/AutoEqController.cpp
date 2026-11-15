// -- WP QP15 -- AutoEQ: the database, the browser's list, favourites, applying
// a profile, and updating the database (rebuild, import, reset)
#include "AutoEqController.h"

#include "Core.h"             // -- WP P1QT2 --
#include "CoreWorker.h"       // -- WP P1QT2 --
#include "NoticeCenter.h"     // -- WP P1QT2 --
#include "Snapshot.h"         // -- WP P1QT2 --
#include "ToolsController.h"  // -- WP P1QT2 --

#include "DeviceModel.h"
#include "FilesController.h"
#include "Shell.h"

#include <QCoreApplication>
#include <algorithm>
#include <cmath>
#include <memory>
#include <QDateTime>
#include <QDir>
#include <QEventLoop>
#include <QHostAddress>   // -- WP SECQTA --
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

// -- WP SECQTA -- What the fetch may ask for, the first URL and every redirect:
// https only (PLAN-SEC N16), except plain http to the loopback interface, which
// is the smoke driver's local server (SmokeAutoEq). The core asks only for
// https, so this refuses what it never sends.
bool fetchUrlAllowed(const QUrl &url) {
    if (!url.isValid() || url.host().isEmpty()) return false;
    const QString scheme = url.scheme().toLower();
    if (scheme == QLatin1String("https")) return true;
    if (scheme != QLatin1String("http")) return false;
    // A loopback address literal, not a name a resolver could answer for
    const QHostAddress address(url.host());
    return !address.isNull() && address.isLoopback();
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
    case Favorite: return m_favourites.contains(p.id);
    default: return {};
    }
}

QHash<int, QByteArray> AutoEqProfileModel::roleNames() const {
    return { { ProfileId, "profileId" }, { DisplayName, "displayName" }, { Source, "source" },
             { SourceName, "sourceName" }, { FormFactor, "formFactor" }, { Favorite, "favorite" } };
}

void AutoEqProfileModel::setSearchText(const QString &text) {
    if (text == m_search) return;
    m_search = text;
    emit searchTextChanged();
    refilter();
}

void AutoEqProfileModel::setProfiles(std::shared_ptr<const QVector<AutoEqProfile>> profiles) {
    m_profiles = std::move(profiles);
    refilter();
}

void AutoEqProfileModel::setFavourites(const QStringList &ids) {
    m_favourites = ids;
    if (!m_rows.isEmpty()) emit dataChanged(index(0), index(int(m_rows.size()) - 1), { Favorite });
}

// The Mac's search: the manufacturer, the model or the whole name contains the text
void AutoEqProfileModel::refilter() {
    beginResetModel();
    m_rows.clear();
    if (m_profiles) {
        const QString q = m_search.trimmed();
        for (int i = 0; i < m_profiles->size(); ++i) {
            const AutoEqProfile &p = m_profiles->at(i);
            if (q.isEmpty() || p.manufacturer.contains(q, Qt::CaseInsensitive) || p.model.contains(q, Qt::CaseInsensitive) ||
                p.displayName().contains(q, Qt::CaseInsensitive))
                m_rows.append(i);
        }
    }
    endResetModel();
    emit shownChanged();
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
    m_favouriteIds = QSettings().value(QLatin1String(kFavorites)).toStringList();
    m_model->setFavourites(m_favouriteIds);
    // Polling the rebuild is not a device read, so not the Poller's: it runs
    // with no device open
    m_rebuildPoll.setInterval(kPollMs);
    connect(&m_rebuildPoll, &QTimer::timeout, this, &AutoEqController::pollRebuild);

    ctx.shell->addHandler(QStringLiteral("autoeq.clearFavorites"), this, [this] { forgetFavourites(); });
    ctx.shell->addHandler(QStringLiteral("autoeq.updateDatabase"), this, [this] { emit updateRequested(); });
    connect(ctx.device, &DeviceModel::snapshotApplied, this, [this] {
        // -- WP P1QT2 -- the pairs follow the device (channel counts, names)
        const QVariantList shown = pairs();
        if (shown != m_pairsShown) {
            m_pairsShown = shown;
            if (m_pair >= shown.size()) m_pair = 0;
            emit pairsChanged();
        }
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

bool AutoEqController::updatedDatabase() const { return QFileInfo::exists(userDatabasePath()); }

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
        p.entry = QJsonDocument(e).toJson(QJsonDocument::Compact);   // -- WP P1QT2 --
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
    emit libraryChanged();
    const QString path = updatedDatabase() ? userDatabasePath() : builtInDatabasePath();
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
            emit libraryChanged();
            emit favouritesChanged();
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

QVariantList AutoEqController::favouriteList() const {
    QVariantList out;
    if (!m_profiles) return out;
    for (const QString &id : m_favouriteIds) {
        const auto it = m_byId.constFind(id);
        if (it == m_byId.cend()) continue;
        const AutoEqProfile &p = m_profiles->at(*it);
        out.append(QVariantMap{ { QStringLiteral("id"), id },
                                { QStringLiteral("title"), p.manufacturer + QLatin1Char(' ') + p.model } });
    }
    return out;
}

bool AutoEqController::isFavourite(const QString &id) const { return m_favouriteIds.contains(id); }

void AutoEqController::setFavouriteIds(const QStringList &ids) {
    m_favouriteIds = ids;
    QSettings s;
    if (ids.isEmpty()) s.remove(QLatin1String(kFavorites));
    else s.setValue(QLatin1String(kFavorites), ids);
    m_model->setFavourites(ids);
    emit favouritesChanged();
}

void AutoEqController::flipFavourite(const QString &id) {
    if (!m_byId.contains(id)) return;
    QStringList ids = m_favouriteIds;
    if (!ids.removeAll(id)) ids.append(id);
    setFavouriteIds(ids);
}

void AutoEqController::forgetFavourites() { setFavouriteIds({}); }

// ---- Applying ------------------------------------------------------------------

void AutoEqController::apply(const QString &id) {
    const auto it = m_byId.constFind(id);
    if (!m_profiles || it == m_byId.cend() || !canApply()) return;
    // -- WP P1QT2 -- the core's apply (P1AQ) on the chosen pair: the state
    // before kept as a snapshot, one undo step, held to the limits profile.
    // The outputs' gain is left alone by default; the trim is suggested.
    const QVariantMap chosen = pairs().value(m_pair).toMap();
    if (chosen.isEmpty()) return;
    const QByteArray entry = m_profiles->at(*it).entry;
    const auto left = uint8_t(chosen.value(QStringLiteral("left")).toInt());
    const auto right = uint8_t(chosen.value(QStringLiteral("right")).toInt());
    auto reply = std::make_shared<QJsonObject>();
    m_ctx.device->write(
        QStringLiteral("autoeq apply"),
        [entry, left, right, reply](FfiCore *c) {
            QByteArray buf(64 * 1024, '\0');
            const uint32_t need = dspi_autoeq_apply(c, entry.constData(), left, right, -1, buf.data(), uint32_t(buf.size()));
            if (need > 0 && need <= uint32_t(buf.size())) buf.truncate(int(need) - 1);
            else buf.clear();
            *reply = QJsonDocument::fromJson(buf).object();
            return reply->value(QLatin1String("ok")).toBool();
        },
        [this, reply](bool) {
            NoticeCenter *notices = Registrar::instance<NoticeCenter>();
            if (!notices) return;
            if (!reply->value(QLatin1String("ok")).toBool()) {
                const QString key = reply->value(QLatin1String("noticeKey")).toString();
                if (!key.isEmpty()) notices->postKey(key, reply->value(QLatin1String("error")).toString());
                return;
            }
            // What the core held or could not do, and the trim it suggests (its
            // notes say so on outputs): a notice, never a modal
            QStringList lines;
            for (const char *f : { "held", "notes" })
                for (const QJsonValue &v : reply->value(QLatin1String(f)).toArray())
                    if (v.isString()) lines << v.toString();
            lines.removeAll(QString());
            if (!lines.isEmpty()) notices->post(lines.join(QLatin1Char('\n')));
        });
}

// -- WP P1QT2 -- The pairs a profile can go to, by wire channel
QVariantList AutoEqController::pairs() const {
    QVariantList out;
    const DspState *s = m_ctx.device->state();
    if (!s) return out;
    const int inputs = std::clamp(int(s->num_input_channels), 0, MAX_INPUTS);
    const int outputs = m_ctx.device->numOutputs();
    for (int i = 0; i + 1 < inputs; i += 2)
        //: AutoEQ's channel pair: %1 and %2 are input numbers
        out << QVariantMap{ { QStringLiteral("title"), tr("Inputs %1 and %2").arg(i + 1).arg(i + 2) },
                            { QStringLiteral("left"), i }, { QStringLiteral("right"), i + 1 },
                            { QStringLiteral("outputs"), false } };
    const ToolsController *tools = Registrar::instance<ToolsController>();
    for (int o = 0; o + 1 < outputs; o += 2) {
        const QString a = tools ? tools->outputName(o) : QString::number(o + 1);
        const QString b = tools ? tools->outputName(o + 1) : QString::number(o + 2);
        //: AutoEQ's channel pair: %1 and %2 are output names
        out << QVariantMap{ { QStringLiteral("title"), tr("Outputs: %1 and %2").arg(a, b) },
                            { QStringLiteral("left"), inputs + o }, { QStringLiteral("right"), inputs + o + 1 },
                            { QStringLiteral("outputs"), true } };
    }
    return out;
}

void AutoEqController::setPair(int index) {
    if (index == m_pair || index < 0) return;
    m_pair = index;
    emit pairsChanged();
    requestPreview(m_previewId);
}

void AutoEqController::requestPreview(const QString &id) {
    m_previewId = id;
    const quint64 seq = ++m_previewSeq;
    const auto it = m_byId.constFind(id);
    const QVariantMap chosen = pairs().value(m_pair).toMap();
    if (id.isEmpty() || !m_profiles || it == m_byId.cend() || chosen.isEmpty() || !m_ctx.device->connected()) {
        if (m_preview.isEmpty()) return;
        m_preview.clear();
        emit previewChanged();
        return;
    }
    const QByteArray entry = m_profiles->at(*it).entry;
    const auto left = uint8_t(chosen.value(QStringLiteral("left")).toInt());
    const auto right = uint8_t(chosen.value(QStringLiteral("right")).toInt());
    // Sends nothing: from the mirror alone, on the worker
    m_ctx.core->worker()->call(
        this,
        [entry, left, right](FfiCore *c) {
            return readSized([&](char *b, uint32_t n) { return dspi_autoeq_preview(c, entry.constData(), left, right, -1, b, n); }, 64 * 1024);
        },
        [this, seq, id](const QByteArray &bytes) {
            if (seq != m_previewSeq) return;
            m_preview = previewView(QJsonDocument::fromJson(bytes).object());
            if (!m_preview.isEmpty()) m_preview.insert(QStringLiteral("profileId"), id);
            emit previewChanged();
        });
}

// The preview's curves for EffectGraph: x by log frequency from 20 Hz, y in
// a range that holds them; the level line; the core's notes
QVariantMap AutoEqController::previewView(const QJsonObject &o) {
    if (!o.value(QLatin1String("ok")).toBool()) return {};
    const QJsonArray channels = o.value(QLatin1String("channels")).toArray();
    double lo = -12, hi = 12;
    for (const QJsonValue &ch : channels)
        for (const QJsonValue &v : ch.toObject().value(QLatin1String("curve")).toArray()) {
            lo = std::min(lo, v.toDouble() - 3);
            hi = std::max(hi, v.toDouble() + 3);
        }
    lo = std::max(lo, -40.0);
    hi = std::min(hi, 40.0);
    const double span = hi - lo;
    auto yOf = [&](double db) { return std::clamp((db - lo) / span, 0.0, 1.0); };
    auto xOf = [](double hz) { return (std::log10(hz) - std::log10(20.0)) / (std::log10(20000.0) - std::log10(20.0)); };
    QVariantList curves;
    for (int c = 0; c < channels.size() && c < 2; ++c) {
        const QJsonArray curve = channels.at(c).toObject().value(QLatin1String("curve")).toArray();
        // The second channel only when it differs from the first
        if (c == 1 && curve == channels.at(0).toObject().value(QLatin1String("curve")).toArray()) break;
        QVariantList pts;
        const int n = int(curve.size());
        for (int i = 0; i < n; ++i) {
            // The core's points: MAGNITUDE_POINTS from 10 Hz to 20 kHz, log spaced
            const double hz = std::pow(10.0, std::log10(10.0) + double(i) / std::max(1, n - 1) * (std::log10(20000.0) - std::log10(10.0)));
            if (hz < 20.0) continue;
            pts << xOf(hz) << yOf(curve.at(i).toDouble());
        }
        curves << QVariantMap{ { QStringLiteral("points"), pts }, { QStringLiteral("index"), c } };
    }
    QVariantList gridDb, gridY;
    const double step = span > 40 ? 10 : 6;
    for (double v = std::ceil(lo / step) * step; v <= hi; v += step) {
        gridDb << int(v);
        gridY << yOf(v);
    }
    QStringList notes;
    for (const char *f : { "held", "notes" })
        for (const QJsonValue &v : o.value(QLatin1String(f)).toArray())
            if (v.isString()) notes << v.toString();
    const QString target = o.value(QLatin1String("levelTarget")).toString();
    const double level = channels.isEmpty() ? 0 : channels.at(0).toObject().value(QLatin1String("levelDb")).toDouble();
    QString levelText;
    if (target == QLatin1String("input_preamp"))
        //: AutoEQ preview: %1 the inputs' preamp in dB
        levelText = tr("Preamp on both inputs: %1 dB").arg(level, 0, 'f', 1);
    else if (target == QLatin1String("output_gain"))
        levelText = tr("Gain on both outputs: %1 dB").arg(level, 0, 'f', 1);
    // `none` (outputs): the core's notes say the gain is kept and the trim it suggests
    return { { QStringLiteral("curves"), curves }, { QStringLiteral("gridDb"), gridDb }, { QStringLiteral("gridY"), gridY },
             { QStringLiteral("levelText"), levelText }, { QStringLiteral("notes"), notes.join(QLatin1Char('\n')) } };
}

// ---- Updating the database -------------------------------------------------------

// An update's alert: what happened, then how many profiles the database holds now
QString AutoEqController::withCount(const QString &what) const {
    return what + QLatin1Char('\n') + tr("Entries: %1").arg(profileCount());
}

QString AutoEqController::updateQuestionText() const {
    return tr("Current database: %1\nEntries: %2\n\nChoose an update method:")
        .arg(m_date.isEmpty() ? tr("Unknown") : m_date)
        .arg(profileCount());
}

void AutoEqController::setFileBusy(bool busy) {
    if (busy == m_fileBusy) return;
    m_fileBusy = busy;
    emit fileBusyChanged();
}

void AutoEqController::chooseDatabaseFile() {
    FilesController::pickFile(false, tr("Import AutoEQ Database"), tr("AutoEQ database (*.json)"), QString(), this,
                              [this](const QString &path) { importDatabaseFrom(path); });
}

void AutoEqController::importDatabaseFrom(const QString &path) {
    if (m_fileBusy) return;
    setFileBusy(true);
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
                setFileBusy(false);
                emit notice(tr("Error"), tr("Failed to import database: %1").arg(error), true);
                return;
            }
            reload([this] {
                setFileBusy(false);
                emit notice(tr("Success"), withCount(tr("Database updated successfully.")), false);
            });
        }, Qt::QueuedConnection);
    });
}

void AutoEqController::resetToBuiltIn() {
    if (m_fileBusy) return;
    setFileBusy(true);
    const QByteArray db = userDatabasePath().toUtf8();
    m_files.start([this, db] {
        const QJsonObject reply =
            callOnce([&](char *b, uint32_t n) { return dspi_autoeq_reset_to_builtin(db.constData(), b, n); });
        const QString error =
            reply.value(QLatin1String("ok")).toBool() ? QString() : reply.value(QLatin1String("error")).toString();
        QMetaObject::invokeMethod(this, [this, error] {
            if (!error.isEmpty()) {
                setFileBusy(false);
                emit notice(tr("Error"), tr("Failed to reset: %1").arg(error), true);
                return;
            }
            reload([this] {
                setFileBusy(false);
                emit notice(tr("Success"), withCount(tr("Reset to built-in database.")), false);
            });
        }, Qt::QueuedConnection);
    });
}

// ---- Rebuilding from GitHub --------------------------------------------------------

void AutoEqController::rebuildFromGitHub() {
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
    m_rebuildFinished = false;
    m_rebuildFraction = 0;
    m_rebuildLine = tr("Connecting to GitHub…");
    emit rebuildMoved();
    m_rebuildPoll.start();
}

void AutoEqController::stopRebuild() {
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
    m_rebuildFraction = qBound(0.0, o.value(QLatin1String("fraction")).toDouble(), 1.0);
    if (!status.isEmpty()) m_rebuildLine = status;
    emit rebuildMoved();
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
        emit rebuildMoved();
        if (!status.isEmpty()) emit notice(tr("Error"), tr("Rebuild failed: %1").arg(status), true);
        return;
    }
    // Keep the sheet up briefly to show it finished, as the Mac does
    m_rebuildFraction = 1.0;
    m_rebuildFinished = true;
    emit rebuildMoved();
    reload([this] {
        QTimer::singleShot(kDoneShownMs, this, [this] {
            m_rebuildShown = false;
            m_rebuildFinished = false;
            emit rebuildMoved();
            emit notice(tr("Success"), withCount(tr("Database rebuilt successfully!")), false);
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
    // -- WP SECQTA -- refused before any request goes out
    const QUrl target = QUrl::fromEncoded(url);
    if (!fetchUrlAllowed(target)) {
        out.error = QStringLiteral("refused: AutoEQ downloads must use https");
        return out;
    }
    QNetworkAccessManager manager;
    QNetworkRequest request(target);
    request.setRawHeader("User-Agent", userAgent);
    if (!accept.isEmpty()) request.setRawHeader("Accept", accept);
    request.setTransferTimeout(kFetchTimeoutMs);
    // -- WP SECQTA -- every redirect is checked like the first URL: an http
    // redirect is refused, from https and from the loopback alike
    request.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::UserVerifiedRedirectPolicy);
    QNetworkReply *reply = manager.get(request);
    bool redirectRefused = false;
    QObject::connect(reply, &QNetworkReply::redirected, reply, [reply, &redirectRefused](const QUrl &next) {
        if (fetchUrlAllowed(reply->url().resolved(next))) {
            emit reply->redirectAllowed();
        } else {
            redirectRefused = true;
            reply->abort();
        }
    });
    QEventLoop loop;
    QObject::connect(reply, &QNetworkReply::finished, &loop, &QEventLoop::quit);
    if (!reply->isFinished()) loop.exec();
    out.status = redirectRefused ? 0 : reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
    if (redirectRefused) {
        out.error = QStringLiteral("refused: a redirect left https");
    } else if (out.status == 0) {
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
