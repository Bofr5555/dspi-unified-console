// -- WP QP17 -- The update checker (behaviour.md §17), against a release feed
// on the loopback interface: which release is offered, what a request
// carries, opt-in and the launch check, failures, the Updates page wired to
// the real checker, the banner with Details and Later, Skip This Version.
// Nothing reaches a real network, and the release page goes to a handler:
// this section opens nothing.
#include "SmokeDriver.h"

#include "AppIdentity.h"
#include "AppUpdater.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"

#include <QDesktopServices>
#include <QHash>
#include <QtQml/QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTcpServer>
#include <QTcpSocket>
#include <QUrl>
#include <cstdio>
#include <functional>

namespace {

// A release feed on 127.0.0.1, served from this thread's event loop (the
// checker's request is asynchronous on the same thread)
class QP17Feed : public QObject {
    Q_OBJECT
public:
    struct Request {
        QByteArray path;
        QList<QPair<QByteArray, QByteArray>> headers;
    };
    std::function<QByteArray(const QByteArray &path)> answer;
    QVector<Request> requests;

    QP17Feed() {
        m_server.listen(QHostAddress::LocalHost);
        connect(&m_server, &QTcpServer::newConnection, this, [this]() {
            while (QTcpSocket *c = m_server.nextPendingConnection()) {
                c->setParent(this);
                connect(c, &QTcpSocket::readyRead, this, [this, c]() { read(c); });
                connect(c, &QTcpSocket::disconnected, c, &QObject::deleteLater);
            }
        });
    }
    QUrl url(const char *path) const {
        return QUrl(QStringLiteral("http://127.0.0.1:%1%2").arg(m_server.serverPort()).arg(QLatin1String(path)));
    }
    QByteArray header(int request, const char *name) const {
        for (const auto &h : requests.value(request).headers)
            if (h.first == name) return h.second;
        return {};
    }

private:
    void read(QTcpSocket *c) {
        QByteArray &text = m_pending[c];
        text += c->readAll();
        const int end = text.indexOf("\r\n\r\n");
        if (end < 0) return;
        const QList<QByteArray> lines = text.left(end).split('\n');
        Request r;
        r.path = lines.value(0).split(' ').value(1);
        for (int i = 1; i < lines.size(); ++i) {
            const int colon = lines[i].indexOf(':');
            if (colon > 0) r.headers.append({ lines[i].left(colon).trimmed().toLower(), lines[i].mid(colon + 1).trimmed() });
        }
        m_pending.remove(c);
        requests.append(r);
        c->write(answer(r.path));
        c->disconnectFromHost();
    }
    QTcpServer m_server;
    QHash<QTcpSocket *, QByteArray> m_pending;
};

class QP17UrlCatcher : public QObject {
    Q_OBJECT
public:
    QStringList urls;
public slots:
    void open(const QUrl &url) { urls.append(url.toString()); }
};

QByteArray reply(const QByteArray &status, const QByteArray &body, const QByteArray &extra = {}) {
    return "HTTP/1.1 " + status + "\r\nContent-Type: application/json\r\n" + extra
         + "Content-Length: " + QByteArray::number(body.size()) + "\r\nConnection: close\r\n\r\n" + body;
}

// A release carrying this app's Linux package unless `assets` says otherwise
QByteArray release(const char *tag, const QByteArray &more = {}, const QByteArray &assets = "linux") {
    const QByteArray v = QByteArray(tag).mid(1);
    QByteArray list;
    if (assets == "linux") list = R"([{"name":"DSPi-uConsole-)" + v + R"(-linux-x86_64.tar.gz"}])";
    else if (assets == "swift") list = R"([{"name":"DSPi-uConsole-Swift-)" + v + R"(-macos.dmg"}])";
    else list = "[]";
    return R"({"tag_name":")" + QByteArray(tag) + R"(","html_url":"https://example.invalid/releases/tag/)"
         + QByteArray(tag) + R"(","draft":false,"assets":)" + list + more + "}";
}

QString offered(const QStringList &tags, const char *current) {
    QVector<AppUpdater::Release> releases;
    for (const QString &t : tags) {
        AppUpdater::Release r;
        r.tag = t;
        r.version = AppUpdater::Version::parse(t);
        releases.append(r);
    }
    const AppUpdater::Release *r = AppUpdater::newest(releases, AppUpdater::Version::parse(QString::fromLatin1(current)));
    return r ? r->tag : QString();
}

QObject *qp17Find(QObject *root, const QString &name) {
    if (!root) return nullptr;
    if (root->objectName() == name) return root;
    if (auto *item = qobject_cast<QQuickItem *>(root))
        for (QQuickItem *child : item->childItems())
            if (QObject *o = qp17Find(child, name)) return o;
    for (QObject *child : root->children())
        if (QObject *o = qp17Find(child, name)) return o;
    return nullptr;
}

bool qp17Shown(QObject *o) {
    auto *item = qobject_cast<QQuickItem *>(o);
    return item && item->isVisible();
}

void updatesSection(SmokeDriver &d) {
    // The rules, case for case with the Mac's AppUpdateCheckerTests
    d.check(QStringLiteral("qp17: a final user is not offered betas"),
            offered({ "v1.1.6-beta4", "v1.1.5" }, "1.1.5").isEmpty()
                && offered({ "v1.1.7-beta1", "v1.1.6", "v1.1.5" }, "1.1.5") == "v1.1.6");
    d.check(QStringLiteral("qp17: a beta user is offered newer betas and the final"),
            offered({ "v1.1.6-beta4", "v1.1.6-beta3" }, "1.1.6-beta3") == "v1.1.6-beta4"
                && offered({ "v1.1.6", "v1.1.6-beta4" }, "1.1.6-beta4") == "v1.1.6");
    d.check(QStringLiteral("qp17: nothing newer offers nothing"),
            offered({ "v1.1.6-beta4", "v1.1.5" }, "1.1.6-beta4").isEmpty() && offered({}, "1.1.6").isEmpty()
                && offered({ "v1.1.6" }, "1.1.6").isEmpty());
    d.check(QStringLiteral("qp17: beta 10 sorts above beta 9"),
            offered({ "v1.1.6-beta9", "v1.1.6-beta10" }, "1.1.6-beta2") == "v1.1.6-beta10");
    d.check(QStringLiteral("qp17: versions read as the tags spell them"),
            AppUpdater::Version::parse(QStringLiteral("v1.2")).text() == "1.2.0"
                && AppUpdater::Version::parse(QStringLiteral("1.1.6-beta3")).text() == "1.1.6-beta3"
                && !AppUpdater::Version::parse(QStringLiteral("1.1.6 Beta 3")).valid
                && !AppUpdater::Version::parse(QStringLiteral("nightly")).valid
                && !AppUpdater::Version::parse(QString()).valid);

    const QByteArray newer = "[" + release("v0.12.0") + ","
                           + release("v0.14.0", R"(,"name":"uConsole 0.14","body":"<b>Not bold</b>\n- one\n- two")") + ","
                           + R"({"tag_name":"v0.20.0","html_url":"https://example.invalid/x","draft":true,"assets":[]},)"
                           + release("v0.14.1-beta1") + ","
                           + R"js({"tag_name":"v9.0.0","html_url":"javascript:alert(1)","assets":[]},)js"
                           + release("v0.15.0", {}, "swift") + "," + release("v0.16.0", {}, "none") + "]";
    const QByteArray older = "[" + release("v0.12.0") + "," + release("v0.13.0-beta4") + "]";
    {
        QStringList tags;
        for (const AppUpdater::Release &r : AppUpdater::parseFeed(newer)) tags << r.tag;
        d.check(QStringLiteral("qp17: drafts and pages that are not https are left out"),
                tags == QStringList({ "v0.12.0", "v0.14.0", "v0.14.1-beta1", "v0.15.0", "v0.16.0" }));
        const QVector<AppUpdater::Release> all = AppUpdater::parseFeed(newer);
        const AppUpdater::Release *best = AppUpdater::newest(all, AppUpdater::Version::parse(QStringLiteral("0.13.0")),
                                                             QStringLiteral("linux"));
        d.check(QStringLiteral("qp17: only a release carrying this app's package counts"), best && best->tag == "v0.14.0");
        d.check(QStringLiteral("qp17: anything but a list is no release"),
                AppUpdater::parseFeed(R"({"tag_name":"v2.0.0"})").isEmpty() && AppUpdater::parseFeed("not json").isEmpty()
                    && AppUpdater::parseFeed(R"([1, "x", null])").isEmpty());
    }
    {
        const QUrl https(QStringLiteral("https://releases.example/feed"));
        const QUrl http(QStringLiteral("http://releases.example/feed"));
        d.check(QStringLiteral("qp17: the built-in feed is https only"),
                AppUpdater::resolveFeed(QUrl(), http).isEmpty() && AppUpdater::resolveFeed(QUrl(), https) == https);
        bool loopback = true;
        for (const char *u : { "http://127.0.0.1:8080/feed", "http://localhost/feed", "http://[::1]:9/feed", "http://127.1.2.3/feed" })
            loopback = loopback && AppUpdater::resolveFeed(QUrl(QString::fromLatin1(u)), https) == QUrl(QString::fromLatin1(u));
        d.check(QStringLiteral("qp17: an override on this machine is allowed"), loopback);
        bool remote = true;
        for (const char *u : { "http://releases.example/feed", "https://releases.example/feed", "http://192.168.1.20/feed",
                               "http://127.0.0.1.example/feed", "file:///etc/passwd" })
            remote = remote && AppUpdater::resolveFeed(QUrl(QString::fromLatin1(u)), https).isEmpty();
        d.check(QStringLiteral("qp17: an override off this machine is refused, not swapped for the built-in"), remote);
    }

    auto *up = Registrar::instance<AppUpdater>();
    auto *settings = Registrar::instance<SettingsController>();
    d.check(QStringLiteral("qp17: the checker is registered"), up && settings);
    if (!up || !settings) return;
    // QP10's section hands the page a stand-in and clears it afterwards
    settings->setUpdater(up);
    d.check(QStringLiteral("qp17: the Updates page has the checker"), settings->updater() == up);
    d.check(QStringLiteral("qp17: automatic checks are off by default, nothing was asked"),
            !up->automatic() && !QSettings().contains(QStringLiteral("updates/automatic")) && up->result().isEmpty());
    d.check(QStringLiteral("qp17: this build has no feed, so the row is hidden"),
            !up->hasFeed() && !d.app().shell->handles(QStringLiteral("help.checkForUpdates")));

    QP17Feed feed;
    QByteArray answer = newer;
    feed.answer = [&answer](const QByteArray &path) -> QByteArray {
        if (path == "/feed") return reply("200 OK", answer, "Set-Cookie: track=me; Path=/\r\n");
        if (path == "/error") return reply("500 Internal Server Error", "boom");
        if (path == "/object") return reply("200 OK", R"({"message":"Not Found"})");
        return reply("404 Not Found", "");
    };
    const AppContext ctx{ d.app().core, d.app().device, d.app().shell, d.app().engine, nullptr };

    // A launch with automatic checks off asks nothing, even with a feed set
    QSettings().setValue(QStringLiteral("updates/feedUrl"), feed.url("/feed").toString());
    {
        AppUpdater launched(ctx, nullptr);
        d.check(QStringLiteral("qp17: the settings key points a launch at a local feed"),
                launched.hasFeed() && launched.feedHost() == "127.0.0.1");
        d.spin(5600);
        d.check(QStringLiteral("qp17: off: a launch asks nothing"), feed.requests.isEmpty() && launched.result().isEmpty());
    }
    // On: the launch checks by itself once it has settled, and announces
    QSettings().setValue(QStringLiteral("updates/automatic"), true);
    {
        AppUpdater launched(ctx, nullptr);
        launched.setCurrentVersionForTest(QStringLiteral("0.13.0"));
        d.spin(1000);
        d.check(QStringLiteral("qp17: on: the launch check waits for the app to settle"), feed.requests.isEmpty());
        d.check(QStringLiteral("qp17: on: a launch checks by itself and raises the banner"),
                d.waitFor(15000, [&]() { return launched.result() == "available"; }) && feed.requests.size() == 1
                    && launched.announce());
    }
    QSettings().remove(QStringLiteral("updates/automatic"));
    QSettings().remove(QStringLiteral("updates/feedUrl"));
    feed.requests.clear();

    // What a check sends is what the privacy line says, nothing else
    up->setFeed(feed.url("/feed"));
    up->setCurrentVersionForTest(QStringLiteral("0.13.0"));
    d.check(QStringLiteral("qp17: the test feed is taken"), up->hasFeed() && up->feedHost() == "127.0.0.1");
    d.check(QStringLiteral("qp17: the User-Agent is DSPiConsole/<version>"), up->userAgent() == "DSPiConsole/0.13.0");
    up->checkNow();
    d.check(QStringLiteral("qp17: Check Now asks the feed"), up->checking() && d.waitFor(5000, [&]() { return !up->checking(); }));
    d.check(QStringLiteral("qp17: a newer release is offered"),
            up->result() == "available" && up->releaseVersion() == "0.14.0" && up->releaseName() == "uConsole 0.14"
                && up->releaseNotes() == "<b>Not bold</b>\n- one\n- two"
                && up->releaseUrl() == "https://example.invalid/releases/tag/v0.14.0" && up->lastChecked().isValid());
    d.check(QStringLiteral("qp17: a check the user asked for raises no banner"), !up->announce());
    up->checkNow();
    d.waitFor(5000, [&]() { return !up->checking(); });
    QStringList names;
    for (const auto &h : feed.requests.value(0).headers) names << QString::fromLatin1(h.first);
    std::printf("smoke: qp17 request headers: %s\n", qPrintable(names.join(QStringLiteral(", "))));
    const QStringList allowed = { "host", "user-agent", "accept", "accept-language", "accept-encoding", "connection" };
    bool onlyThese = !names.isEmpty();
    for (const QString &n : names) onlyThese = onlyThese && allowed.contains(n);
    d.check(QStringLiteral("qp17: the request carries no header past the listed ones"),
            feed.requests.size() == 2 && onlyThese && feed.requests[0].path == "/feed");
    d.check(QStringLiteral("qp17: the User-Agent is the privacy line's"), feed.header(0, "user-agent") == up->userAgent());
    d.check(QStringLiteral("qp17: no languages of the system"), feed.header(0, "accept-language") == "en");
    d.check(QStringLiteral("qp17: a cookie the feed sets is not sent back"),
            feed.header(0, "cookie").isEmpty() && feed.header(1, "cookie").isEmpty());

    // Up to date, and the failures the page says
    answer = older;
    up->checkNow();
    d.waitFor(5000, [&]() { return !up->checking(); });
    d.check(QStringLiteral("qp17: nothing newer: up to date"), up->result() == "current" && up->releaseVersion().isEmpty());
    up->setFeed(feed.url("/error"));
    up->checkNow();
    d.waitFor(5000, [&]() { return !up->checking(); });
    d.check(QStringLiteral("qp17: a 500 is a failure the page names"), up->result() == "failed" && up->error().contains("500"));
    up->setFeed(feed.url("/object"));
    up->checkNow();
    d.waitFor(5000, [&]() { return !up->checking(); });
    d.check(QStringLiteral("qp17: an answer that is no list is a failure"), up->result() == "failed" && !up->error().isEmpty());
    up->setFeed(feed.url("/missing"));
    up->checkNow();
    d.waitFor(5000, [&]() { return !up->checking(); });
    d.check(QStringLiteral("qp17: a 404 is no news, as on the Mac"), up->result() == "current" && up->error().isEmpty());
    up->setFeed(QUrl(QStringLiteral("http://releases.example/feed")));
    d.check(QStringLiteral("qp17: a plain http feed elsewhere is refused"), !up->hasFeed());
    up->checkNow();
    d.check(QStringLiteral("qp17: and is never asked"), up->result() == "failed" && !up->checking());

    // The page with the real checker
    answer = newer;
    up->setFeed(feed.url("/feed"));
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    d.check(QStringLiteral("qp17: the Settings window opens"), w != nullptr);
    if (!w) return;
    settings->open(QStringLiteral("updates"));
    d.spin(300);
    auto get = [&](const char *name) { return qp17Find(w->contentItem(), QString::fromLatin1(name)); };
    auto text = [&](const char *name) {
        QObject *o = get(name);
        return o ? o->property("text").toString() : QString();
    };
    d.check(QStringLiteral("qp17: the privacy line names the feed and the User-Agent"),
            text("Updates.privacy").contains("127.0.0.1") && text("Updates.privacy").contains(up->userAgentText()));
    if (QObject *now = get("Updates.checkNow")) QMetaObject::invokeMethod(now, "click");
    d.check(QStringLiteral("qp17: Check Now on the page checks"), d.waitFor(5000, [&]() { return up->result() == "available"; }));
    d.spin(200);
    d.check(QStringLiteral("qp17: the release shows, its notes as text"),
            qp17Shown(get("Updates.availableCard")) && text("Updates.notes") == up->releaseNotes()
                && text("Updates.status").contains("0.14.0"));
    QP17UrlCatcher catcher;
    QDesktopServices::setUrlHandler(QStringLiteral("https"), &catcher, "open");
    if (QObject *view = get("Updates.viewRelease")) QMetaObject::invokeMethod(view, "click");
    QDesktopServices::unsetUrlHandler(QStringLiteral("https"));
    d.check(QStringLiteral("qp17: View Release opens the release page"), catcher.urls == QStringList({ up->releaseUrl() }));
    d.grab(w, QStringLiteral("settings-updates-qp17.png"));

    // The switch turns automatic checks on; that check raises the banner
    if (QObject *sw = get("Updates.automatic.switch")) QMetaObject::invokeMethod(sw, "click");
    d.check(QStringLiteral("qp17: the switch turns automatic checks on, and they are remembered"),
            up->automatic() && QSettings().value(QStringLiteral("updates/automatic")).toBool());
    d.check(QStringLiteral("qp17: turning them on checks"), d.waitFor(5000, [&]() { return up->announce(); }));
    w->setVisible(false);
    d.spin(300);
    QObject *banner = d.object(QStringLiteral("UpdateBanner"));
    d.check(QStringLiteral("qp17: the banner shows the find"),
            qp17Shown(banner) && banner->property("text").toString().contains("0.14.0")
                && banner->property("text").toString().contains("0.13.0"));
    d.grab(d.mainWindow(), QStringLiteral("update-banner.png"));
    if (QObject *later = qp17Find(banner, QStringLiteral("UpdateBanner.later"))) QMetaObject::invokeMethod(later, "click");
    d.spin(200);
    d.check(QStringLiteral("qp17: Later hides it"), !up->announce() && !qp17Shown(banner));
    up->setAutomatic(false);
    up->setAutomatic(true);
    d.waitFor(5000, [&]() { return !up->checking(); });
    d.check(QStringLiteral("qp17: the next automatic find shows it again"), up->announce());
    if (QObject *details = qp17Find(banner, QStringLiteral("UpdateBanner.details"))) QMetaObject::invokeMethod(details, "click");
    d.spin(300);
    d.check(QStringLiteral("qp17: Details opens the Updates page"),
            w->isVisible() && !up->announce() && settings->currentPage() == "updates");
    w->setVisible(false);
    d.spin(200);
    up->skipVersion();
    up->setAutomatic(false);
    up->setAutomatic(true);
    d.waitFor(5000, [&]() { return !up->checking(); });
    d.check(QStringLiteral("qp17: a skipped version is remembered and not announced"),
            up->result() == "available" && !up->announce()
                && QSettings().value(QStringLiteral("updates/skipped")).toString() == "v0.14.0");

    // As found: off, the build's own feed (none), nothing on offer
    up->setAutomatic(false);
    answer = older;
    up->checkNow();
    d.waitFor(5000, [&]() { return !up->checking(); });
    QSettings().remove(QStringLiteral("updates"));
    up->setFeed(QUrl());
    up->setCurrentVersionForTest(AppIdentity::version());
    d.check(QStringLiteral("qp17: left as found"), !up->automatic() && !up->hasFeed() && !up->announce());
}

} // namespace

const SmokeDriver::Section qp17Updates("qp17-updates", 170, &updatesSection);

#include "SmokeUpdates.moc"
