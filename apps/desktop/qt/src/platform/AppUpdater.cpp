// -- WP QP17 -- the check for a newer release of this app (AppUpdater.h)
#include "AppUpdater.h"

#include "AppIdentity.h"
#include "SettingsController.h"
#include "Shell.h"

#include <QDesktopServices>
#include <QHostAddress>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QNetworkAccessManager>
#include <QNetworkCookie>
#include <QNetworkCookieJar>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QQmlEngine>
#include <QRegularExpression>
#include <QSettings>
#include <QTimer>
#include <algorithm>

#ifndef DSPI_UPDATE_FEED_URL
#define DSPI_UPDATE_FEED_URL ""
#endif

namespace {

const Registrar::Controller<AppUpdater> appUpdater;
const Registrar::Banner updateBanner("UpdateBanner", 50);

const char *const kAutomaticKey = "updates/automatic";
const char *const kSkippedKey = "updates/skipped";
const char *const kLastCheckedKey = "updates/lastChecked";
const char *const kFeedOverrideKey = "updates/feedUrl";

// The Mac's timings: let the launch settle first, then once a day
constexpr int kLaunchDelayMs = 5000;
constexpr int kDailyMs = 24 * 60 * 60 * 1000;
// A release list is a few hundred kB at most; anything larger is not one
constexpr qint64 kMaxReply = 4 * 1024 * 1024;
constexpr int kTimeoutMs = 20000;
constexpr int kMaxNotes = 8000;

// A cookie jar that keeps nothing, so a check carries nothing from the last
class NoCookies : public QNetworkCookieJar {
public:
    using QNetworkCookieJar::QNetworkCookieJar;
    bool setCookiesFromUrl(const QList<QNetworkCookie> &, const QUrl &) override { return false; }
    QList<QNetworkCookie> cookiesForUrl(const QUrl &) const override { return {}; }
};

QString thisPlatform() {
#if defined(Q_OS_WIN)
    return QStringLiteral("windows");
#elif defined(Q_OS_MACOS)   // -- WP QTMAC --
    return QStringLiteral("macos");
#else
    return QStringLiteral("linux");
#endif
}

bool onThisMachine(const QString &host) {
    if (host.compare(QLatin1String("localhost"), Qt::CaseInsensitive) == 0) return true;
    QHostAddress address;
    return address.setAddress(host) && address.isLoopback();
}

} // namespace

// ---- versions -------------------------------------------------------------

AppUpdater::Version AppUpdater::Version::parse(QString text) {
    static const QRegularExpression tagForm(
        QStringLiteral("^v?(\\d{1,4})\\.(\\d{1,4})(?:\\.(\\d{1,4}))?(?:-beta(\\d{1,4}))?$"),
        QRegularExpression::CaseInsensitiveOption);
    Version read;
    const QRegularExpressionMatch found = tagForm.match(text.trimmed());
    if (!found.hasMatch()) return read;
    // An absent group (no patch, no beta) reads as 0
    for (size_t part = 0; part < read.parts.size(); ++part) read.parts[part] = found.captured(int(part) + 1).toInt();
    read.valid = true;
    return read;
}

QString AppUpdater::Version::text() const {
    if (!valid) return {};
    const QString release = QStringList{ QString::number(parts[0]), QString::number(parts[1]), QString::number(parts[2]) }
                                .join(QLatin1Char('.'));
    return isBeta() ? release + QStringLiteral("-beta") + QString::number(parts[3]) : release;
}

// ---- the feed -------------------------------------------------------------

QVector<AppUpdater::Release> AppUpdater::parseFeed(const QByteArray &json) {
    QVector<Release> kept;
    const QJsonDocument doc = QJsonDocument::fromJson(json);
    const QJsonArray list = doc.array();
    for (const QJsonValue &entry : list) {
        const QJsonObject rel = entry.toObject();
        if (rel.isEmpty() || rel.value(QLatin1String("draft")).toBool()) continue;
        const QUrl page(rel.value(QLatin1String("html_url")).toString());
        Release r;
        r.tag = rel.value(QLatin1String("tag_name")).toString();
        r.version = Version::parse(r.tag);
        // Only a page the browser may open safely, and a tag that is a version
        if (!r.version.valid || page.scheme() != QLatin1String("https") || page.host().isEmpty()) continue;
        r.page = page.toString();
        r.name = rel.value(QLatin1String("name")).toString().trimmed();
        r.notes = rel.value(QLatin1String("body")).toString().left(kMaxNotes);
        const QJsonArray files = rel.value(QLatin1String("assets")).toArray();
        for (const QJsonValue &file : files) r.assets << file.toObject().value(QLatin1String("name")).toString();
        kept.append(r);
    }
    return kept;
}

bool AppUpdater::carriesApp(const Release &r, const QString &platform) {
    // The release scripts' names: uConsole-for-DSPi-<v>-windows-x86_64(-setup.exe|.zip),
    // uConsole-for-DSPi-<v>-linux-x86_64.tar.gz, uConsole-for-DSPi-<v>-x86_64.AppImage.
    // The tag's own version follows the prefix, and only these platform parts
    // count, so the other clients' files in the same release (…-macos.dmg,
    // …-terminal-linux-x86_64.tar.gz, …-source.tar.gz) do not.
    // -- WP QTMAC -- The macOS build's ("macos") is uConsole-for-DSPi-<v>-macos.dmg;
    // that branch is compiled on macOS only, so the Linux and Windows builds
    // stay as they were.
    const QString prefix = QStringLiteral("uConsole-for-DSPi-%1-").arg(r.version.text());
    const bool windows = platform == QLatin1String("windows");
    return std::any_of(r.assets.cbegin(), r.assets.cend(), [&](const QString &asset) {
        if (!asset.startsWith(prefix)) return false;
        const QStringView rest = QStringView(asset).mid(prefix.size());
#ifdef Q_OS_MACOS
        if (platform == QLatin1String("macos")) return rest == QLatin1String("macos.dmg");
#endif
        return windows ? rest.startsWith(QLatin1String("windows-"))
                       : rest.startsWith(QLatin1String("linux-")) || rest.endsWith(QLatin1String(".AppImage"));
    });
}

const AppUpdater::Release *AppUpdater::newest(const QVector<Release> &releases, const Version &current,
                                              const QString &platform) {
    const Release *best = nullptr;
    for (const Release &r : releases) {
        const bool ours = platform.isEmpty() || carriesApp(r, platform);
        // Someone on a final release is offered finals only
        const bool fitting = current.isBeta() || !r.version.isBeta();
        if (ours && fitting && current < r.version && (!best || best->version < r.version)) best = &r;
    }
    return best;
}

bool AppUpdater::feedAllowed(const QUrl &url, bool builtIn) {
    if (!url.isValid() || url.host().isEmpty()) return false;
    const QString scheme = url.scheme();
    if (builtIn) return scheme == QLatin1String("https");
    // An override serves tests and local feeds: this machine only, http or https
    return (scheme == QLatin1String("http") || scheme == QLatin1String("https")) && onThisMachine(url.host());
}

QUrl AppUpdater::resolveFeed(const QUrl &override, const QUrl &builtIn) {
    if (!override.isEmpty()) return feedAllowed(override, false) ? override : QUrl();
    return feedAllowed(builtIn, true) ? builtIn : QUrl();
}

// ---- the controller -------------------------------------------------------

AppUpdater::AppUpdater(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    m_net = new QNetworkAccessManager(this);
    m_net->setCookieJar(new NoCookies(m_net));
    // -- WP QPI3 -- every hop is checked as the feed was (check())
    m_net->setRedirectPolicy(QNetworkRequest::UserVerifiedRedirectPolicy);
    m_current = Version::parse(AppIdentity::version());
    const QSettings stored;
    m_automatic = stored.value(QLatin1String(kAutomaticKey), false).toBool();
    m_lastChecked = stored.value(QLatin1String(kLastCheckedKey)).toDateTime();
    m_feed = resolveFeed(QUrl(stored.value(QLatin1String(kFeedOverrideKey)).toString()),
                         QUrl(QStringLiteral(DSPI_UPDATE_FEED_URL)));

    m_daily = new QTimer(this);
    m_daily->setInterval(kDailyMs);
    QObject::connect(m_daily, &QTimer::timeout, this, &AppUpdater::launchCheck);
    if (hasFeed() && m_automatic) {
        m_daily->start();
        QTimer::singleShot(kLaunchDelayMs, this, &AppUpdater::launchCheck);
    }
    // One row handler: a second checker (the smoke driver's launch test) leaves it
    const QString row = QStringLiteral("help.checkForUpdates");
    if (hasFeed() && !ctx.shell->handles(row)) ctx.shell->addHandler(row, this, [this] {
        checkNow();
        details();
    });
    // Controllers are made in no set order: hand this to the Updates page
    // once they all exist
    QMetaObject::invokeMethod(this, &AppUpdater::attach, Qt::QueuedConnection);
}

AppUpdater::~AppUpdater() {
    if (!m_reply.isNull()) m_reply->abort();
}

AppUpdater *AppUpdater::create(QQmlEngine *, QJSEngine *) {
    AppUpdater *self = Registrar::instance<AppUpdater>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

void AppUpdater::attach() {
    // Only the app's own checker (the smoke driver makes others)
    if (Registrar::instance<AppUpdater>() != this) return;
    if (auto *settings = Registrar::instance<SettingsController>()) settings->setUpdater(this);
}

QByteArray AppUpdater::userAgent() const {
    const QString version = m_current.valid ? m_current.text() : AppIdentity::version();
    return QByteArrayLiteral("uConsole-for-DSPi/") + version.toLatin1();
}

void AppUpdater::setAutomatic(bool on) {
    if (m_automatic == on) return;
    m_automatic = on;
    QSettings().setValue(QLatin1String(kAutomaticKey), on);
    emit automaticChanged();
    if (!on || !hasFeed()) {
        m_daily->stop();
        return;
    }
    m_daily->start();
    check(true);
}

void AppUpdater::checkNow() { check(false); }

void AppUpdater::check(bool automatic) {
    if (!m_reply.isNull() || (automatic && !m_automatic)) return;
    if (!hasFeed()) return settle(QStringLiteral("failed"), tr("This build has no release feed."));
    // -- WP QPI3 -- every release would be newer than a version that cannot be read
    if (!m_current.valid) return settle(QStringLiteral("failed"), tr("This build's version cannot be compared with the releases."));
    QNetworkRequest ask(m_feed);
    ask.setRawHeader("User-Agent", userAgent());
    ask.setRawHeader("Accept", "application/vnd.github+json");
    // Not the system's languages: they would say more about the user than the
    // privacy line does
    ask.setRawHeader("Accept-Language", "en");
    ask.setAttribute(QNetworkRequest::CookieLoadControlAttribute, QNetworkRequest::Manual);
    ask.setAttribute(QNetworkRequest::CookieSaveControlAttribute, QNetworkRequest::Manual);
    ask.setAttribute(QNetworkRequest::CacheSaveControlAttribute, false);
    ask.setTransferTimeout(kTimeoutMs);
    m_replyAutomatic = automatic;
    m_reply = m_net->get(ask);
    QNetworkReply *pending = m_reply;
    QObject::connect(pending, &QNetworkReply::downloadProgress, pending, [pending](qint64 got, qint64) {
        if (got > kMaxReply) pending->abort();
    });
    // -- WP QPI3 -- a redirect leads where the feed itself may be: this
    // machine for a local feed, https for the built-in one
    const bool local = feedAllowed(m_feed, false);
    QObject::connect(pending, &QNetworkReply::redirected, pending, [pending, local](const QUrl &to) {
        const QUrl hop = pending->url().resolved(to);
        if (feedAllowed(hop, !local)) emit pending->redirectAllowed();
        else pending->abort();
    });
    QObject::connect(pending, &QNetworkReply::finished, this, &AppUpdater::replied);
    emit stateChanged();
}

void AppUpdater::replied() {
    const QScopedPointer<QNetworkReply, QScopedPointerDeleteLater> done(m_reply.data());
    m_reply.clear();
    if (!done) return;
    const int status = done->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
    // The Mac's rule: a feed that answers 404 (a private repository, or no
    // releases yet) is no news, not a failure to report
    if (status == 404) {
        m_release = Release();
        return settle(QStringLiteral("current"), QString());
    }
    if (status != 200 || done->error() != QNetworkReply::NoError)
        return settle(QStringLiteral("failed"),
                      status ? tr("The release feed answered %1.").arg(status)
                             : tr("The release feed could not be reached: %1").arg(done->errorString()));
    const QByteArray body = done->read(kMaxReply + 1);
    if (body.size() > kMaxReply || !QJsonDocument::fromJson(body).isArray())
        return settle(QStringLiteral("failed"), tr("The release feed's answer is not a list of releases."));

    const QVector<Release> releases = parseFeed(body);
    const Release *best = newest(releases, m_current, thisPlatform());
    m_release = best ? *best : Release();
    if (best && m_release.name.isEmpty()) m_release.name = AppIdentity::displayName();
    settle(best ? QStringLiteral("available") : QStringLiteral("current"), QString());
}

void AppUpdater::settle(const QString &result, const QString &why) {
    m_result = result;
    m_error = why;
    if (result == QLatin1String("failed")) m_release = Release();
    if (why.isEmpty()) {
        m_lastChecked = QDateTime::currentDateTime();
        QSettings().setValue(QLatin1String(kLastCheckedKey), m_lastChecked);
    }
    // Only an automatic check raises the banner, and never for a skipped
    // release; a check the user asked for shows its answer on the page
    const QString skipped = QSettings().value(QLatin1String(kSkippedKey)).toString();
    if (!available()) m_announce = false;
    else if (m_replyAutomatic && m_release.tag != skipped) m_announce = true;
    m_replyAutomatic = false;
    emit stateChanged();
}

void AppUpdater::openRelease() {
    const QUrl page(m_release.page);
    if (available() && page.scheme() == QLatin1String("https")) QDesktopServices::openUrl(page);
}

void AppUpdater::skipVersion() {
    if (!available()) return;
    QSettings().setValue(QLatin1String(kSkippedKey), m_release.tag);
    later();
}

void AppUpdater::later() {
    if (!m_announce) return;
    m_announce = false;
    emit stateChanged();
}

void AppUpdater::details() {
    later();
    auto *settings = Registrar::instance<SettingsController>();
    if (settings) settings->showPage(QStringLiteral("updates"));
}

void AppUpdater::setFeed(const QUrl &url) {
    const QUrl builtIn(QStringLiteral(DSPI_UPDATE_FEED_URL));
    if (url.isEmpty()) m_feed = resolveFeed(QUrl(), builtIn);
    else m_feed = feedAllowed(url, false) || feedAllowed(url, true) ? url : QUrl();
    emit feedChanged();
}

void AppUpdater::setCurrentVersionForTest(const QString &version) {
    m_current = Version::parse(version);
    emit feedChanged();
}
