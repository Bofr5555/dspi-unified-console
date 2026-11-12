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
    static const QRegularExpression form(
        QStringLiteral("^v?(\\d{1,4})\\.(\\d{1,4})(?:\\.(\\d{1,4}))?(?:-beta(\\d{1,4}))?$"),
        QRegularExpression::CaseInsensitiveOption);
    Version v;
    const QRegularExpressionMatch m = form.match(text.trimmed());
    if (!m.hasMatch()) return v;
    v.major = m.captured(1).toInt();
    v.minor = m.captured(2).toInt();
    v.patch = m.captured(3).toInt();
    v.beta = m.captured(4).toInt();
    v.valid = true;
    return v;
}

QString AppUpdater::Version::text() const {
    if (!valid) return {};
    QString s = QStringLiteral("%1.%2.%3").arg(major).arg(minor).arg(patch);
    if (beta) s += QStringLiteral("-beta%1").arg(beta);
    return s;
}

bool AppUpdater::Version::operator<(const Version &o) const {
    if (major != o.major) return major < o.major;
    if (minor != o.minor) return minor < o.minor;
    if (patch != o.patch) return patch < o.patch;
    // A final (beta 0) comes after every beta of the same number
    if (beta == o.beta) return false;
    if (beta == 0) return false;
    if (o.beta == 0) return true;
    return beta < o.beta;
}

bool AppUpdater::Version::operator==(const Version &o) const {
    return valid == o.valid && major == o.major && minor == o.minor && patch == o.patch && beta == o.beta;
}

// ---- the feed -------------------------------------------------------------

QVector<AppUpdater::Release> AppUpdater::parseFeed(const QByteArray &json) {
    QVector<Release> out;
    const QJsonDocument doc = QJsonDocument::fromJson(json);
    if (!doc.isArray()) return out;
    for (const QJsonValue &v : doc.array()) {
        const QJsonObject o = v.toObject();
        if (o.isEmpty() || o.value(QLatin1String("draft")).toBool()) continue;
        Release r;
        r.tag = o.value(QLatin1String("tag_name")).toString();
        r.version = Version::parse(r.tag);
        const QUrl page(o.value(QLatin1String("html_url")).toString());
        if (!r.version.valid || page.scheme() != QLatin1String("https") || page.host().isEmpty()) continue;
        r.page = page.toString();
        r.name = o.value(QLatin1String("name")).toString().trimmed();
        r.notes = o.value(QLatin1String("body")).toString().left(kMaxNotes);
        for (const QJsonValue &a : o.value(QLatin1String("assets")).toArray())
            r.assets << a.toObject().value(QLatin1String("name")).toString();
        out.append(r);
    }
    return out;
}

bool AppUpdater::carriesApp(const Release &r, const QString &platform) {
    // The release scripts' names: DSPi-uConsole-<v>-windows-x86_64(-setup.exe|.zip),
    // DSPi-uConsole-<v>-linux-x86_64.tar.gz, DSPi-uConsole-<v>-x86_64.AppImage.
    // The tag's own version follows the prefix, so the Swift app's and the
    // other clients' packages in the same repository do not count.
    const QString prefix = QStringLiteral("DSPi-uConsole-%1-").arg(r.version.text());
    for (const QString &a : r.assets) {
        if (!a.startsWith(prefix)) continue;
        const QString rest = a.mid(prefix.size());
        if (platform == QLatin1String("windows") && rest.startsWith(QLatin1String("windows-"))) return true;
        if (platform == QLatin1String("linux")
            && (rest.startsWith(QLatin1String("linux-")) || rest.endsWith(QLatin1String(".AppImage"))))
            return true;
    }
    return false;
}

const AppUpdater::Release *AppUpdater::newest(const QVector<Release> &releases, const Version &current,
                                              const QString &platform) {
    const Release *best = nullptr;
    for (const Release &r : releases) {
        if (!platform.isEmpty() && !carriesApp(r, platform)) continue;
        if (current.beta == 0 && r.version.beta != 0) continue;
        if (!(current < r.version)) continue;
        if (!best || best->version < r.version) best = &r;
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

AppUpdater::AppUpdater(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx), m_net(new QNetworkAccessManager(this)), m_daily(new QTimer(this)) {
    m_net->setCookieJar(new NoCookies(m_net));
    m_net->setRedirectPolicy(QNetworkRequest::NoLessSafeRedirectPolicy);
    m_current = Version::parse(AppIdentity::version());
    QSettings s;
    m_automatic = s.value(QLatin1String(kAutomaticKey), false).toBool();
    m_lastChecked = s.value(QLatin1String(kLastCheckedKey)).toDateTime();
    m_feed = resolveFeed(QUrl(s.value(QLatin1String(kFeedOverrideKey)).toString()),
                         QUrl(QStringLiteral(DSPI_UPDATE_FEED_URL)));

    m_daily->setInterval(kDailyMs);
    connect(m_daily, &QTimer::timeout, this, [this]() { check(true); });
    if (m_automatic && hasFeed()) {
        QTimer::singleShot(kLaunchDelayMs, this, [this]() { check(true); });
        m_daily->start();
    }
    // One row handler: a second checker (the smoke driver's launch test) leaves it
    if (hasFeed() && !m_ctx.shell->handles(QStringLiteral("help.checkForUpdates"))) {
        m_ctx.shell->addHandler(QStringLiteral("help.checkForUpdates"), this, [this]() {
            checkNow();
            details();
        });
    }
    // Controllers are made in no set order: hand this to the Updates page
    // once they all exist
    QMetaObject::invokeMethod(this, &AppUpdater::attach, Qt::QueuedConnection);
}

AppUpdater::~AppUpdater() {
    if (m_reply) m_reply->abort();
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
    return "DSPiConsole/" + (m_current.valid ? m_current.text() : AppIdentity::version()).toLatin1();
}

void AppUpdater::setAutomatic(bool on) {
    if (on == m_automatic) return;
    m_automatic = on;
    QSettings().setValue(QLatin1String(kAutomaticKey), on);
    emit automaticChanged();
    if (on && hasFeed()) {
        m_daily->start();
        check(true);
    } else {
        m_daily->stop();
    }
}

void AppUpdater::checkNow() { check(false); }

void AppUpdater::check(bool automatic) {
    if (m_reply) return;
    if (automatic && !m_automatic) return;
    if (!hasFeed()) {
        fail(tr("This build has no release feed."));
        return;
    }
    QNetworkRequest request(m_feed);
    request.setHeader(QNetworkRequest::UserAgentHeader, userAgent());
    request.setRawHeader("Accept", "application/vnd.github+json");
    // Not the system's languages: they would say more about the user than the
    // privacy line does
    request.setRawHeader("Accept-Language", "en");
    request.setAttribute(QNetworkRequest::CookieLoadControlAttribute, QNetworkRequest::Manual);
    request.setAttribute(QNetworkRequest::CookieSaveControlAttribute, QNetworkRequest::Manual);
    request.setAttribute(QNetworkRequest::CacheSaveControlAttribute, false);
    request.setTransferTimeout(kTimeoutMs);
    QNetworkReply *reply = m_net->get(request);
    m_reply = reply;
    connect(reply, &QNetworkReply::downloadProgress, reply, [reply](qint64 received, qint64) {
        if (received > kMaxReply) reply->abort();
    });
    connect(reply, &QNetworkReply::finished, this, [this, reply, automatic]() { finished(reply, automatic); });
    emit stateChanged();
}

void AppUpdater::finished(QNetworkReply *reply, bool automatic) {
    reply->deleteLater();
    m_reply = nullptr;
    const int status = reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
    if (reply->error() != QNetworkReply::NoError || status != 200) {
        // The Mac's rule: a feed that answers 404 (a private repository, or no
        // releases yet) is no news, not a failure to report
        if (status == 404) {
            m_result = QStringLiteral("current");
            m_error.clear();
            m_lastChecked = QDateTime::currentDateTime();
            QSettings().setValue(QLatin1String(kLastCheckedKey), m_lastChecked);
            emit stateChanged();
            return;
        }
        fail(status ? tr("The release feed answered %1.").arg(status)
                    : tr("The release feed could not be reached: %1").arg(reply->errorString()));
        return;
    }
    const QByteArray body = reply->read(kMaxReply + 1);
    if (body.size() > kMaxReply || !QJsonDocument::fromJson(body).isArray()) {
        fail(tr("The release feed's answer is not a list of releases."));
        return;
    }
    const QVector<Release> releases = parseFeed(body);
    const Release *best = newest(releases, m_current, thisPlatform());
    m_error.clear();
    m_lastChecked = QDateTime::currentDateTime();
    QSettings().setValue(QLatin1String(kLastCheckedKey), m_lastChecked);
    if (best) {
        m_release = *best;
        if (m_release.name.isEmpty()) m_release.name = AppIdentity::displayName();
        m_result = QStringLiteral("available");
        const QString skipped = QSettings().value(QLatin1String(kSkippedKey)).toString();
        // A check the user asked for shows its answer on the page; only an
        // automatic one raises the banner
        if (automatic && m_release.tag != skipped) m_announce = true;
    } else {
        m_release = Release();
        m_result = QStringLiteral("current");
        m_announce = false;
    }
    emit stateChanged();
}

void AppUpdater::fail(const QString &why) {
    m_result = QStringLiteral("failed");
    m_error = why;
    m_release = Release();
    m_announce = false;
    emit stateChanged();
}

void AppUpdater::openRelease() {
    const QUrl page(m_release.page);
    if (available() && page.scheme() == QLatin1String("https")) QDesktopServices::openUrl(page);
}

void AppUpdater::skipVersion() {
    if (!available()) return;
    QSettings().setValue(QLatin1String(kSkippedKey), m_release.tag);
    m_announce = false;
    emit stateChanged();
}

void AppUpdater::later() {
    if (!m_announce) return;
    m_announce = false;
    emit stateChanged();
}

void AppUpdater::details() {
    later();
    if (auto *settings = Registrar::instance<SettingsController>()) settings->showPage(QStringLiteral("updates"));
}

void AppUpdater::setFeed(const QUrl &url) {
    m_feed = url.isEmpty() ? resolveFeed(QUrl(), QUrl(QStringLiteral(DSPI_UPDATE_FEED_URL)))
                           : (feedAllowed(url, false) || feedAllowed(url, true) ? url : QUrl());
    emit feedChanged();
}

void AppUpdater::setCurrentVersionForTest(const QString &version) {
    m_current = Version::parse(version);
    emit feedChanged();
}
