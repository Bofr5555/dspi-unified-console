#include "AppUpdater.h"
#include "Registrar.h"

#include <QCoreApplication>
#include <QDesktopServices>
#include <QHostAddress>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocale>
#include <QNetworkAccessManager>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QSettings>
#include <climits>
#include <tuple>

static const Registrar::Controller<AppUpdater> updaterRegistered("appUpdater");
static const Registrar::Banner updaterBanner("qrc:/qml/components/UpdateBanner.qml", 20);

#ifndef DSPI_UPDATE_FEED_URL
#define DSPI_UPDATE_FEED_URL ""
#endif

namespace {
const char kAutomaticKey[] = "updates/automatic";
const char kSkippedKey[] = "updates/skipped";
const char kLastCheckedKey[] = "updates/lastChecked";
const char kFeedKey[] = "updates/feedUrl";
// Let launch settle (onboarding, device connect) before the first check
constexpr int kLaunchDelayMs = 5000;
constexpr int kDailyMs = 24 * 60 * 60 * 1000;
constexpr int kTimeoutMs = 20000;
// A release list is tens of kilobytes; anything near this is not one
constexpr qint64 kMaxBytes = 4 * 1024 * 1024;
// Notes past this are for the release page
constexpr int kMaxNotes = 8000;

bool isWebPage(const QUrl &url) {
    return url.isValid() && (url.scheme() == QLatin1String("https") || url.scheme() == QLatin1String("http"))
        && !url.host().isEmpty();
}
}

// -- WP QD37 -- (review c4q N4) The feed says which page "View Release" opens, so a
// plain-http one on a hostile network could send the user anywhere: https only,
// except an override (settings key or setFeed) on this machine, for a local feed.
bool AppUpdater::feedAllowed(const QUrl &url, bool builtIn) {
    if (!isWebPage(url)) return false;
    if (url.scheme() == QLatin1String("https")) return true;
    if (builtIn) return false;
    const QString host = url.host();
    return host.compare(QLatin1String("localhost"), Qt::CaseInsensitive) == 0 || QHostAddress(host).isLoopback();
}

QUrl AppUpdater::resolveFeed(const QUrl &override, const QUrl &builtIn) {
    if (!override.isEmpty()) return feedAllowed(override, false) ? override : QUrl();
    return feedAllowed(builtIn, true) ? builtIn : QUrl();
}

// ── Versions and the feed ──

AppUpdater::Version AppUpdater::Version::parse(const QString &text) {
    Version v;
    QString s = text.trimmed();
    if (s.startsWith(QLatin1Char('v')) || s.startsWith(QLatin1Char('V'))) s.remove(0, 1);
    int end = 0;
    while (end < s.size() && (s[end].isDigit() || s[end] == QLatin1Char('.'))) end++;
    const QStringList fields = s.left(end).split(QLatin1Char('.'));
    bool ok = false;
    v.major = fields.value(0).toInt(&ok);
    if (!ok) return v;
    v.minor = fields.value(1).toInt();
    v.patch = fields.value(2).toInt();
    // "-beta4", ".beta4", " beta 4"; any other suffix reads as a final release
    QString suffix = s.mid(end).toLower();
    while (!suffix.isEmpty() && (suffix[0] == QLatin1Char('-') || suffix[0] == QLatin1Char('.') || suffix[0] == QLatin1Char(' ')))
        suffix.remove(0, 1);
    if (suffix.startsWith(QLatin1String("beta"))) {
        QString digits;
        for (const QChar c : suffix.mid(4)) {
            if (!c.isDigit()) break;
            digits += c;
        }
        v.beta = digits.left(6).toInt();
    }
    v.valid = true;
    return v;
}

bool AppUpdater::Version::operator<(const Version &o) const {
    const auto key = [](const Version &v) { return std::make_tuple(v.major, v.minor, v.patch, v.beta == 0 ? INT_MAX : v.beta); };
    return key(*this) < key(o);
}

QString AppUpdater::Version::text() const {
    if (!valid) return QString();
    QString t = QStringLiteral("%1.%2.%3").arg(major).arg(minor).arg(patch);
    if (beta > 0) t += QStringLiteral("-beta%1").arg(beta);
    return t;
}

QVector<AppUpdater::Release> AppUpdater::parseFeed(const QByteArray &json) {
    QVector<Release> out;
    const QJsonDocument doc = QJsonDocument::fromJson(json);
    for (const QJsonValue &item : doc.array()) {
        const QJsonObject o = item.toObject();
        if (o.value(QLatin1String("draft")).toBool()) continue;
        Release r;
        r.tag = o.value(QLatin1String("tag_name")).toString();
        r.version = Version::parse(r.tag);
        r.page = QUrl(o.value(QLatin1String("html_url")).toString(), QUrl::StrictMode);
        if (!r.version.valid || !isWebPage(r.page)) continue;
        r.title = o.value(QLatin1String("name")).toString().trimmed();
        r.notes = o.value(QLatin1String("body")).toString().trimmed();
        if (r.notes.size() > kMaxNotes) r.notes = r.notes.left(kMaxNotes) + QStringLiteral("\n…");
        out.append(r);
    }
    return out;
}

const AppUpdater::Release *AppUpdater::newest(const QVector<Release> &releases, const Version &current) {
    const Release *best = nullptr;
    for (const Release &r : releases) {
        if (current.beta == 0 && r.version.beta != 0) continue;
        if (!(current < r.version)) continue;
        if (!best || best->version < r.version) best = &r;
    }
    return best;
}

// ── The controller ──

AppUpdater::AppUpdater(DSPiBridge *, QObject *parent)
    : QObject(parent)
    , m_current(Version::parse(QCoreApplication::applicationVersion()))
{
    const QSettings settings;
    m_automatic = settings.value(QLatin1String(kAutomaticKey), false).toBool();
    m_skipped = settings.value(QLatin1String(kSkippedKey)).toString();
    m_lastChecked = settings.value(QLatin1String(kLastCheckedKey)).toDateTime();
    setFeed(QUrl());

    m_daily.setInterval(kDailyMs);
    connect(&m_daily, &QTimer::timeout, this, [this]() { check(true); });
    m_launch.setSingleShot(true);
    m_launch.setInterval(kLaunchDelayMs);
    connect(&m_launch, &QTimer::timeout, this, [this]() { check(true); });
    m_timeout.setSingleShot(true);
    m_timeout.setInterval(kTimeoutMs);
    connect(&m_timeout, &QTimer::timeout, this, [this]() {
        if (m_reply) {
            m_failure = QStringLiteral("The release feed did not answer.");
            m_reply->abort();
        }
    });
    if (m_automatic) {
        m_launch.start();
        m_daily.start();
    }
}

void AppUpdater::setFeed(const QUrl &feed) {
    QUrl url = feed;
    if (url.isEmpty()) url = QUrl(QSettings().value(QLatin1String(kFeedKey)).toString());
    m_feed = resolveFeed(url, QUrl(QStringLiteral(DSPI_UPDATE_FEED_URL)));
    emit feedChanged();
}

QByteArray AppUpdater::userAgent() const {
    return "DSPiConsole/" + m_current.text().toLatin1();
}

QString AppUpdater::privacyLine() const {
    if (!hasFeed()) return QStringLiteral("This build has no release feed, so it never checks.");
    return QStringLiteral("A check asks %1 for its list of releases. It sends what any web request does, your IP address, "
                          "and the app's version (User-Agent %2); nothing about your devices, presets or settings. "
                          "With automatic checks on, it checks at launch and once a day.")
        .arg(feedHost(), QString::fromLatin1(userAgent()));
}

void AppUpdater::setAutomatic(bool on) {
    if (on == m_automatic) return;
    m_automatic = on;
    QSettings().setValue(QLatin1String(kAutomaticKey), on);
    if (on) {
        m_daily.start();
        check(true);
    } else {
        m_daily.stop();
        m_launch.stop();
    }
    emit automaticChanged();
}

QString AppUpdater::lastChecked() const {
    return m_lastChecked.isValid() ? QLocale().toString(m_lastChecked, QLocale::ShortFormat) : QString();
}

bool AppUpdater::announce() const {
    return m_result == QLatin1String("available") && m_fromAutomatic && !m_dismissed && m_offer.tag != m_skipped;
}

void AppUpdater::checkNow() { check(false); }

void AppUpdater::check(bool automatic) {
    if (m_reply) return;
    if (automatic && !m_automatic) return;
    if (!hasFeed()) {
        setFailure(QStringLiteral("This build has no release feed."));
        return;
    }
    if (!m_net) m_net = new QNetworkAccessManager(this);
    QNetworkRequest request(m_feed);
    // privacyLine lists what goes out: these are set here, not left to Qt, which
    // would otherwise send its own User-Agent and the system's languages. The
    // manager has no cache, and no answer is kept in one.
    request.setRawHeader("User-Agent", userAgent());
    request.setRawHeader("Accept", "application/vnd.github+json");
    request.setRawHeader("Accept-Language", "en");
    request.setAttribute(QNetworkRequest::CookieLoadControlAttribute, QNetworkRequest::Manual);
    request.setAttribute(QNetworkRequest::CookieSaveControlAttribute, QNetworkRequest::Manual);
    request.setAttribute(QNetworkRequest::CacheSaveControlAttribute, false);
    request.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::NoLessSafeRedirectPolicy);
    m_fromAutomatic = automatic;
    m_failure.clear();
    m_reply = m_net->get(request);
    connect(m_reply, &QNetworkReply::downloadProgress, this, [this](qint64 received, qint64) {
        if (m_reply && received > kMaxBytes) {
            m_failure = QStringLiteral("The release feed's answer was too large.");
            m_reply->abort();
        }
    });
    connect(m_reply, &QNetworkReply::finished, this, &AppUpdater::finished);
    m_timeout.start();
    emit stateChanged();
}

void AppUpdater::finished() {
    QNetworkReply *reply = m_reply;
    m_reply = nullptr;
    m_timeout.stop();
    reply->deleteLater();
    const int status = reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
    if (!m_failure.isEmpty()) { setFailure(m_failure); return; }
    if (reply->error() != QNetworkReply::NoError && status == 0) { setFailure(reply->errorString()); return; }
    if (status != 200) { setFailure(QStringLiteral("The release feed answered %1.").arg(status)); return; }
    const QByteArray body = reply->readAll();
    if (!QJsonDocument::fromJson(body).isArray()) {
        setFailure(QStringLiteral("The release feed's answer is not a list of releases."));
        return;
    }
    const QVector<Release> releases = parseFeed(body);
    const Release *best = newest(releases, m_current);
    m_offer = best ? *best : Release();
    if (best && m_offer.title.isEmpty()) m_offer.title = QStringLiteral("DSPi Console ") + m_offer.version.text();
    m_result = best ? QStringLiteral("available") : QStringLiteral("current");
    m_dismissed = false;
    m_lastChecked = QDateTime::currentDateTime();
    QSettings().setValue(QLatin1String(kLastCheckedKey), m_lastChecked);
    emit stateChanged();
}

void AppUpdater::setFailure(const QString &why) {
    m_result = QStringLiteral("failed");
    m_failure = why;
    m_offer = Release();
    emit stateChanged();
}

void AppUpdater::openPage() {
    if (isWebPage(m_offer.page)) QDesktopServices::openUrl(m_offer.page);
}

void AppUpdater::skipVersion() {
    if (m_offer.tag.isEmpty()) return;
    m_skipped = m_offer.tag;
    QSettings().setValue(QLatin1String(kSkippedKey), m_skipped);
    emit stateChanged();
}

void AppUpdater::dismiss() {
    m_dismissed = true;
    emit stateChanged();
}
