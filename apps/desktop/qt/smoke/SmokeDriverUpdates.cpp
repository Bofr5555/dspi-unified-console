// WP QD16: the check for a newer DSPi Console, against a release feed on the
// loopback interface. Nothing here reaches a real network, and the release page
// goes to a handler: this pass opens nothing.
#include "SmokeDriver.h"

#include "AppUpdater.h"

#include <QCoreApplication>
#include <QDesktopServices>
#include <QHash>
#include <QQmlApplicationEngine>
#include <QQmlContext>
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
// updater's request is asynchronous on the same thread)
class FeedServer : public QObject {
    Q_OBJECT
public:
    struct Request { QByteArray path; QList<QPair<QByteArray, QByteArray>> headers; };
    std::function<QByteArray(const QByteArray &path)> handler;
    QVector<Request> requests;

    FeedServer() {
        m_server.listen(QHostAddress::LocalHost);
        connect(&m_server, &QTcpServer::newConnection, this, [this]() {
            while (QTcpSocket *c = m_server.nextPendingConnection()) {
                c->setParent(this);
                connect(c, &QTcpSocket::readyRead, this, [this, c]() { read(c); });
                connect(c, &QTcpSocket::disconnected, c, &QObject::deleteLater);
            }
        });
    }
    QString url(const char *path) const {
        return QStringLiteral("http://127.0.0.1:%1%2").arg(m_server.serverPort()).arg(QLatin1String(path));
    }
    QByteArray header(int i, const char *name) const {
        for (const auto &h : requests.value(i).headers)
            if (h.first == name) return h.second;
        return QByteArray();
    }

private:
    void read(QTcpSocket *c) {
        QByteArray &text = m_buffers[c];
        text += c->readAll();
        if (!text.contains("\r\n\r\n")) return;
        const QList<QByteArray> lines = text.left(text.indexOf("\r\n\r\n")).split('\n');
        Request r;
        r.path = lines.value(0).split(' ').value(1);
        for (int i = 1; i < lines.size(); i++) {
            const int colon = lines[i].indexOf(':');
            if (colon > 0) r.headers.append({ lines[i].left(colon).trimmed().toLower(), lines[i].mid(colon + 1).trimmed() });
        }
        m_buffers.remove(c);
        requests.append(r);
        c->write(handler(r.path));
        c->disconnectFromHost();
    }
    QTcpServer m_server;
    QHash<QTcpSocket *, QByteArray> m_buffers;
};

class UrlCatcher : public QObject {
    Q_OBJECT
public:
    QStringList urls;
public slots:
    void open(const QUrl &url) { urls.append(url.toString()); }
};

QByteArray http(const QByteArray &status, const QByteArray &body, const QByteArray &headers = QByteArray()) {
    return "HTTP/1.1 " + status + "\r\nContent-Type: application/json\r\n" + headers + "Content-Length: "
        + QByteArray::number(body.size()) + "\r\nConnection: close\r\n\r\n" + body;
}

QByteArray release(const char *tag, const char *page = nullptr, bool draft = false, const char *extra = "") {
    const QByteArray url = page ? QByteArray(page) : "https://example.invalid/releases/tag/" + QByteArray(tag);
    return R"({"tag_name":")" + QByteArray(tag) + R"(","html_url":")" + url + R"(","draft":)"
        + (draft ? "true" : "false") + extra + "}";
}

// The newer release's notes look like markup, which the page must show as text
const QByteArray kNewer = "[" + release("v1.1.5") + ","
    + release("v1.2.0", nullptr, false, R"(,"name":"DSPi Console v1.2.0","body":"<b>Not bold</b>\n- one\n- two")") + ","
    + release("v1.3.0", nullptr, true) + "," + release("v1.2.1-beta1") + "," + release("v9.0.0", "javascript:alert(1)") + "]";
const QByteArray kOlder = "[" + release("v1.1.6") + "," + release("v1.1.6-beta4") + "," + release("v1.0.0") + "]";

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

void walk(QQuickItem *from, const std::function<void(QQuickItem *)> &visit) {
    visit(from);
    for (QQuickItem *c : from->childItems()) walk(c, visit);
}

QQuickItem *named(QQuickItem *from, const char *name) {
    QQuickItem *found = nullptr;
    walk(from, [&](QQuickItem *i) { if (!found && i->objectName() == QLatin1String(name)) found = i; });
    return found;
}

QStringList shownTexts(QQuickItem *from) {
    QStringList out;
    walk(from, [&](QQuickItem *i) {
        if (i->inherits("QQuickText") && i->isVisible()) out.append(i->property("text").toString());
    });
    return out;
}
}

void SmokeDriver::testUpdates() {
    section("app updates (QD16)");

    // Which release is offered: Swift's AppUpdateCheckerTests, case for case
    check("QD16: a final user is not offered betas", offered({ "v1.1.6-beta4", "v1.1.5" }, "1.1.5").isEmpty()
          && offered({ "v1.1.7-beta1", "v1.1.6", "v1.1.5" }, "1.1.5") == "v1.1.6");
    check("QD16: a beta user is offered newer betas and the final",
          offered({ "v1.1.6-beta4", "v1.1.6-beta3" }, "1.1.6-beta3") == "v1.1.6-beta4"
          && offered({ "v1.1.6", "v1.1.6-beta4" }, "1.1.6-beta4") == "v1.1.6");
    check("QD16: nothing newer offers nothing", offered({ "v1.1.6-beta4", "v1.1.6-beta3", "v1.1.5" }, "1.1.6-beta4").isEmpty()
          && offered({}, "1.1.6").isEmpty() && offered({ "v1.1.6" }, "1.1.6").isEmpty());
    check("QD16: beta 10 sorts above beta 9", offered({ "v1.1.6-beta9", "v1.1.6-beta10" }, "1.1.6-beta2") == "v1.1.6-beta10");
    check("QD16: versions read as the tags spell them", AppUpdater::Version::parse("v1.2").text() == "1.2.0"
          && AppUpdater::Version::parse("1.1.6 Beta 3").text() == "1.1.6"    // a beta needs its number next to it
          && AppUpdater::Version::parse("1.1.6-beta3").text() == "1.1.6-beta3"
          && !AppUpdater::Version::parse("nightly").valid && !AppUpdater::Version::parse("").valid);
    const QVector<AppUpdater::Release> feed = AppUpdater::parseFeed(kNewer);
    QStringList tags;
    for (const AppUpdater::Release &r : feed) tags << r.tag;
    check("QD16: drafts and pages that are not http(s) are left out", tags == QStringList({ "v1.1.5", "v1.2.0", "v1.2.1-beta1" }));
    check("QD16: anything but a list is no release", AppUpdater::parseFeed("{\"tag_name\":\"v2.0.0\"}").isEmpty()
          && AppUpdater::parseFeed("not json").isEmpty() && AppUpdater::parseFeed("[1, \"x\", null]").isEmpty());

    auto *up = qobject_cast<AppUpdater *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("appUpdater")).value<QObject *>());
    check("QD16: the controller is registered", up != nullptr);
    if (!up) return;

    // Opt-in: off on a new install, and nothing was asked for during this whole run
    check("QD16: automatic checks are off by default", !up->automatic()
          && !QSettings().contains(QStringLiteral("updates/automatic")) && up->result().isEmpty());
    check("QD16: no feed in this build until the owner sets one", QStringLiteral(DSPI_UPDATE_FEED_URL).isEmpty()
          ? !up->hasFeed() && up->privacyLine().contains("never checks") : up->hasFeed());

    FeedServer server;
    QByteArray answer = kNewer;
    server.handler = [&answer](const QByteArray &path) -> QByteArray {
        if (path == "/feed") return http("200 OK", answer, "Set-Cookie: track=me; Path=/\r\n");
        if (path == "/error") return http("500 Internal Server Error", "boom");
        if (path == "/object") return http("200 OK", "{\"message\":\"Not Found\"}");
        return http("404 Not Found", "");
    };

    // A launch with automatic checks off asks nothing, even with a feed set
    QSettings().setValue(QStringLiteral("updates/feedUrl"), server.url("/feed"));
    {
        AppUpdater launched(nullptr, nullptr);
        check("QD16: the settings key points a launch at a local feed", launched.hasFeed() && launched.feedHost() == "127.0.0.1");
        spin(6000);
        check("QD16: off: a launch asks nothing", server.requests.isEmpty() && launched.result().isEmpty());
    }
    // With them on, a launch checks by itself, after the launch has settled
    QSettings().setValue(QStringLiteral("updates/automatic"), true);
    {
        AppUpdater launched(nullptr, nullptr);
        spin(1000);
        check("QD16: on: the launch check waits for the app to settle", server.requests.isEmpty());
        check("QD16: on: a launch checks by itself", waitFor(8000, [&]() { return launched.result() == "available"; })
              && server.requests.size() == 1 && launched.announce());
    }
    QSettings().remove(QStringLiteral("updates/automatic"));
    QSettings().remove(QStringLiteral("updates/feedUrl"));
    server.requests.clear();

    // What a check sends is what the privacy line says, nothing else
    up->setFeed(QUrl(server.url("/feed")));
    up->setCurrentVersionForTest(QStringLiteral("1.1.6"));
    check("QD16: the privacy line names the host and the User-Agent", up->privacyLine().contains("127.0.0.1")
          && up->privacyLine().contains(QString::fromLatin1(up->userAgent())) && up->userAgent() == "DSPiConsole/1.1.6");
    up->checkNow();
    check("QD16: Check Now asks the feed", up->checking() && waitFor(5000, [&]() { return !up->checking(); }));
    check("QD16: a newer release is offered", up->result() == "available" && up->availableVersion() == "1.2.0"
          && up->availableTitle() == "DSPi Console v1.2.0" && up->notes() == "<b>Not bold</b>\n- one\n- two"
          && up->pageUrl() == "https://example.invalid/releases/tag/v1.2.0" && !up->lastChecked().isEmpty());
    check("QD16: a check the user asked for raises no banner", !up->announce());
    up->checkNow();
    waitFor(5000, [&]() { return !up->checking(); });
    QStringList names;
    for (const auto &h : server.requests.value(0).headers) names << QString::fromLatin1(h.first);
    printf("smoke: QD16 request headers: %s\n", qPrintable(names.join(", ")));
    const QStringList allowed = { "host", "user-agent", "accept", "accept-language", "accept-encoding", "connection" };
    bool onlyThese = !names.isEmpty();
    for (const QString &n : names) onlyThese = onlyThese && allowed.contains(n);
    check("QD16: the request carries no header past the listed ones", server.requests.size() == 2 && onlyThese
          && server.requests[0].path == "/feed");
    check("QD16: User-Agent as the privacy line says", server.header(0, "user-agent") == up->userAgent());
    check("QD16: no languages of the system", server.header(0, "accept-language") == "en");
    check("QD16: a cookie the feed sets is not sent back", server.header(1, "cookie").isEmpty()
          && server.header(0, "cookie").isEmpty());

    // Up to date, and failures the page can say
    answer = kOlder;
    up->checkNow();
    waitFor(5000, [&]() { return !up->checking(); });
    check("QD16: nothing newer: up to date", up->result() == "current" && up->availableVersion().isEmpty());
    up->setFeed(QUrl(server.url("/error")));
    up->checkNow();
    waitFor(5000, [&]() { return !up->checking(); });
    check("QD16: a 500 is a failure", up->result() == "failed" && up->failure().contains("500"));
    up->setFeed(QUrl(server.url("/object")));
    up->checkNow();
    waitFor(5000, [&]() { return !up->checking(); });
    check("QD16: an answer that is no list is a failure", up->result() == "failed" && !up->failure().isEmpty());
    up->setFeed(QUrl(QStringLiteral("file:///etc/passwd")));
    check("QD16: a feed that is not http(s) is none", !up->hasFeed());
    // -- WP QD37 -- (review c4q N4) https only for the built-in feed; plain http only
    // for an override on this machine
    {
        const QUrl https(QStringLiteral("https://releases.example/feed"));
        const QUrl http(QStringLiteral("http://releases.example/feed"));
        check("QD37: an http built-in feed is refused", AppUpdater::resolveFeed(QUrl(), http).isEmpty()
              && !AppUpdater::feedAllowed(http, true) && !AppUpdater::feedAllowed(QUrl(QStringLiteral("http://127.0.0.1/feed")), true));
        check("QD37: an https built-in feed is taken", AppUpdater::resolveFeed(QUrl(), https) == https);
        bool loopback = true;
        for (const char *u : { "http://127.0.0.1:8080/feed", "http://localhost/feed", "http://[::1]:9/feed", "http://127.1.2.3/feed" })
            loopback = loopback && AppUpdater::resolveFeed(QUrl(QString::fromLatin1(u)), https) == QUrl(QString::fromLatin1(u));
        check("QD37: a loopback http override is allowed", loopback);
        bool remote = true;
        for (const char *u : { "http://releases.example/feed", "http://192.168.1.20/feed", "http://127.0.0.1.example/feed", "http:///feed" })
            remote = remote && AppUpdater::resolveFeed(QUrl(QString::fromLatin1(u)), https).isEmpty();
        check("QD37: an http override off this machine is refused, not swapped for the built-in", remote);
        up->setFeed(http);
        check("QD37: setFeed refuses it too", !up->hasFeed());
        up->setFeed(QUrl(server.url("/feed")));
        check("QD37: and takes the loopback test feed", up->hasFeed() && up->feedHost() == QLatin1String("127.0.0.1"));
        up->setFeed(QUrl(QStringLiteral("file:///etc/passwd")));
    }
    up->checkNow();
    check("QD16: and is never asked", up->result() == "failed" && !up->checking());

    // The page: privacy line, notes as text, the release page through the handler
    answer = kNewer;
    up->setFeed(QUrl(server.url("/feed")));
    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    QVariant offeredPage;
    if (settings) QMetaObject::invokeMethod(settings, "featureAvailable", Q_RETURN_ARG(QVariant, offeredPage), Q_ARG(QVariant, QStringLiteral("updates")));
    check("QD16: the settings window offers the page", settings && offeredPage.toBool());
    if (!settings) return;
    settings->setVisible(true);
    spin(200);
    QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QStringLiteral("updates")));
    spin(300);
    QQuickItem *page = named(settings->contentItem(), "updatesPage");
    check("QD16: the page opens", page && page->isVisible());
    if (page) {
        check("QD16: the privacy line is on the page before any check", shownTexts(page).contains(up->privacyLine()));
        QQuickItem *automatic = named(page, "updatesAutomatic");
        check("QD16: the switch shows off", automatic && !automatic->property("checked").toBool());
        QQuickItem *now = named(page, "updatesCheckNow");
        if (now) QMetaObject::invokeMethod(now, "clicked");
        check("QD16: Check Now on the page checks", waitFor(5000, [&]() { return up->result() == "available"; }));
        spin(200);
        QQuickItem *available = named(page, "updatesAvailable");
        check("QD16: the release is shown", available && available->isVisible()
              && shownTexts(page).contains("DSPi Console v1.2.0") && shownTexts(page).contains("Version 1.2.0"));
        QQuickItem *notes = named(page, "updatesNotes");
        check("QD16: the notes show, as text", notes && notes->isVisible() && shownTexts(notes).contains(up->notes())
              && notes->property("verbatim").toBool());
        UrlCatcher catcher;
        QDesktopServices::setUrlHandler(QStringLiteral("https"), &catcher, "open");
        if (QQuickItem *open = named(page, "updatesOpenPage")) QMetaObject::invokeMethod(open, "clicked");
        QDesktopServices::unsetUrlHandler(QStringLiteral("https"));
        check("QD16: View Release opens the release page", catcher.urls == QStringList({ up->pageUrl() }));
        settings->grabWindow().save(m_out + QStringLiteral("/settings-updates.png"));

        // Automatic checks: the switch turns them on, a find raises the banner
        if (automatic) QMetaObject::invokeMethod(automatic, "toggled", Q_ARG(bool, true));
        check("QD16: the switch turns automatic checks on, and they are remembered", up->automatic()
              && QSettings().value(QStringLiteral("updates/automatic")).toBool());
        check("QD16: turning them on checks", waitFor(5000, [&]() { return up->announce(); }));
    }
    settings->setVisible(false);
    spin(300);
    QQuickItem *banner = named(qobject_cast<QQuickWindow *>(root())->contentItem(), "updateBanner");
    check("QD16: the banner shows the find", banner && banner->isVisible()
          && shownTexts(banner).contains("DSPi uConsole 1.2.0 is available. You have 1.1.6."));
    qobject_cast<QQuickWindow *>(root())->grabWindow().save(m_out + QStringLiteral("/update-banner.png"));
    if (QQuickItem *later = banner ? named(banner, "updateBannerLater") : nullptr) QMetaObject::invokeMethod(later, "clicked");
    spin(200);
    check("QD16: Later hides it", !up->announce() && banner && !banner->isVisible());
    up->setAutomatic(false);
    up->setAutomatic(true);
    waitFor(5000, [&]() { return !up->checking(); });
    check("QD16: the next automatic find shows it again", up->announce());
    if (QQuickItem *details = banner ? named(banner, "updateBannerDetails") : nullptr) QMetaObject::invokeMethod(details, "clicked");
    spin(300);
    QQuickItem *shownPage = named(settings->contentItem(), "updatesPage");
    check("QD16: Details opens the Updates page", settings->isVisible() && !up->announce() && shownPage && shownPage->isVisible());
    settings->setVisible(false);
    spin(200);
    up->setAutomatic(false);
    up->setAutomatic(true);
    waitFor(5000, [&]() { return !up->checking(); });
    up->skipVersion();
    check("QD16: a skipped version is remembered and not announced", !up->announce()
          && QSettings().value(QStringLiteral("updates/skipped")).toString() == "v1.2.0");
    up->setAutomatic(false);
    up->setAutomatic(true);
    waitFor(5000, [&]() { return !up->checking(); });
    check("QD16: nor by the next check", up->result() == "available" && !up->announce());

    // As found: off, no feed of the test's, nothing on offer
    up->setAutomatic(false);
    answer = kOlder;
    up->checkNow();
    waitFor(5000, [&]() { return !up->checking(); });
    QSettings().remove(QStringLiteral("updates"));
    up->setFeed(QUrl());
    up->setCurrentVersionForTest(QCoreApplication::applicationVersion());
    check("QD16: left as found", !up->automatic() && up->result() == "current" && !up->announce());
}

#include "SmokeDriverUpdates.moc"
