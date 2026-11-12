// -- WP QP14 -- Settings > Networking on a hub of the app's own, through the real
// page and HubController, on loopback: sharing from the page's switch, the
// address and status line, the hub's own demo, a client (a second core in this
// process, as a Link client) pairing with the code the page shows, roles, the
// access question, the port refused while running, a restart on rename,
// revoking, a port in use with the switch left on, the Windows Firewall and
// Public-network rows (simulated off Windows), and browsing only while the page
// shows. Adapted from the previous desktop app's QD14 checks.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "DevicesController.h"
#include "FirewallController.h"
#include "HubController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTcpServer>
#include <atomic>
#include <cstdio>
#include <memory>
#include <thread>

namespace {

QJsonObject hubJson(FfiHub *hub, uint32_t (*call)(FfiHub *, char *, uint32_t)) {
    return QJsonDocument::fromJson(readSized([&](char *b, uint32_t n) { return call(hub, b, n); })).object();
}

// A free port on loopback, for a hub of the test's own
int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// A Link client of the hub: a core of its own, dialling and pairing on a thread
// of its own as an app would (both calls block)
struct Client {
    FfiCore *core = dspi_core_new();
    std::thread thread;
    std::atomic<bool> done{ false };
    QJsonObject hello, paired;

    ~Client() {
        if (thread.joinable()) thread.join();
        if (core) {
            dspi_link_close(core);
            dspi_core_free(core);
        }
    }
    void connectAndPair(const QString &address, const QString &pin) {
        done = false;
        const QByteArray url = QStringLiteral("ws://%1/dspi/v1").arg(address).toUtf8();
        const QByteArray code = pin.toUtf8();
        thread = std::thread([this, url, code]() {
            QByteArray buf(int(LINK_REPLY_MIN), '\0');
            uint32_t n = dspi_link_connect(core, url.constData(), "Smoke client", "0", "", "", buf.data(), LINK_REPLY_MIN);
            hello = n > 0 && n <= LINK_REPLY_MIN ? QJsonDocument::fromJson(QByteArray(buf.constData(), int(n) - 1)).object()
                                                 : QJsonObject();
            if (hello.value(QLatin1String("needsPairing")).toBool() && !code.isEmpty()) {
                n = dspi_link_pair(core, code.constData(), buf.data(), LINK_REPLY_MIN);
                paired = n > 0 && n <= LINK_REPLY_MIN ? QJsonDocument::fromJson(QByteArray(buf.constData(), int(n) - 1)).object()
                                                      : QJsonObject();
            }
            done = true;
        });
    }
    bool wait(SmokeDriver &d) {
        const bool finished = d.waitFor(30000, [this]() { return done.load(); });
        if (thread.joinable()) thread.join();
        return finished;
    }
};

bool click(QQuickItem *item) {
    return item && item->isVisible() && item->isEnabled() && QMetaObject::invokeMethod(item, "clicked");
}

// A switch as a person flips it: the new state, then its toggled signal
void flip(QQuickItem *item) {
    if (!item) return;
    item->setProperty("checked", !item->property("checked").toBool());
    QMetaObject::invokeMethod(item, "toggled");
}

QVariantMap firstClient(HubController *hub) {
    return hub->clients().isEmpty() ? QVariantMap() : hub->clients().first().toMap();
}

void networkingSection(SmokeDriver &d) {
    auto *hub = Registrar::instance<HubController>();
    auto *settings = Registrar::instance<SettingsController>();
    auto *devices = Registrar::instance<DevicesController>();
    auto *firewall = Registrar::instance<FirewallController>();
    DeviceModel *device = d.app().device;
    d.check(QStringLiteral("qp14: the controllers exist"), hub && settings && devices && firewall);
    if (!hub || !settings || !devices || !firewall) return;

    hub->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("qp14-networking-store")));
    hub->setExtraOptions(HUB_OPT_LOOPBACK);
    d.check(QStringLiteral("qp14: sharing is off by default, with Require Pairing"),
            !hub->sharing() && !hub->running() && hub->access() == QLatin1String("pin"));
    d.check(QStringLiteral("qp14: the default port is the Mac's"), hub->port() == 11915);
    d.check(QStringLiteral("qp14: a port below 1024 is refused"), !hub->setPort(80) && hub->port() == 11915);
    const int port = freePort();
    d.check(QStringLiteral("qp14: a free port is taken while stopped"), port > 0 && hub->setPort(port) && hub->port() == port);
    d.check(QStringLiteral("qp14: and stored as hub/port"), QSettings().value(QStringLiteral("hub/port")).toInt() == port);

    // ---- The page ----------------------------------------------------------------------
    d.app().shell->trigger(QStringLiteral("edit.settings"));
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    d.check(QStringLiteral("qp14: the Settings window opens"), w && d.waitFor(3000, [&]() { return w->isVisible(); }));
    if (!w) return;
    d.check(QStringLiteral("qp14: Networking is listed after Advanced"),
            [&]() {
                QStringList ids;
                for (const QVariant &row : settings->sidebarRows(QString()))
                    if (row.toMap().contains(QStringLiteral("id"))) ids << row.toMap().value(QStringLiteral("id")).toString();
                return ids.indexOf(QStringLiteral("networking")) == ids.indexOf(QStringLiteral("advanced")) + 1;
            }());
    const auto searched = [&](const char *query) {
        for (const QVariant &row : settings->sidebarRows(QString::fromLatin1(query)))
            if (row.toMap().value(QStringLiteral("id")) == QLatin1String("networking")) return true;
        return false;
    };
    d.check(QStringLiteral("qp14: \"pin\" and \"tray\" find Networking (behaviour.md §9)"), searched("pin") && searched("tray"));
    settings->open(QStringLiteral("networking"));
    d.spin(400);
    QQuickItem *scope = w->contentItem();
    const auto item = [&](const char *name) { return d.item(QString::fromLatin1(name), scope); };
    QQuickItem *page = item("NetworkingPage");
    d.check(QStringLiteral("qp14: the Networking page loads"), page != nullptr);
    if (!page) return;
    d.check(QStringLiteral("qp14: the hub browser runs while the page shows"),
            devices->lookForHubs() && (devices->browseRunning() || !devices->browseError().isEmpty()));
    d.check(QStringLiteral("qp14: no firewall rows off Windows"), !firewall->supported() && !item("Networking.firewall")->isVisible());
    d.check(QStringLiteral("qp14: Paired Clients shows with Require Pairing"), item("Networking.clients")->isVisible());
    d.check(QStringLiteral("qp14: Pair is off while not sharing"), !item("Networking.pair")->isEnabled());
    d.grab(w, QStringLiteral("settings-networking.png"));

    // ---- Sharing on, from the switch -------------------------------------------------------
    flip(item("Networking.share"));
    d.check(QStringLiteral("qp14: the switch starts a hub"), d.waitFor(15000, [&]() { return hub->running() && !hub->busy(); }));
    d.check(QStringLiteral("qp14: the switch is kept as hub/sharing"), QSettings().value(QStringLiteral("hub/sharing")).toBool());
    d.check(QStringLiteral("qp14: no error"), hub->lastError().isEmpty());
    if (!hub->lastError().isEmpty()) std::fprintf(stderr, "hub: %s\n", qPrintable(hub->lastError()));
    const QString address = hub->address();
    d.check(QStringLiteral("qp14: the address to dial"), address == QStringLiteral("127.0.0.1:%1").arg(port));
    d.check(QStringLiteral("qp14: the status line names it, with no clients"),
            hub->statusLine() == address + QStringLiteral(" · No clients connected") && item("Networking.status")->isVisible()
                && item("Networking.status")->property("text").toString() == hub->statusLine());
    const QJsonObject status = hubJson(hub->handle(), dspi_hub_status);
    d.check(QStringLiteral("qp14: the hub runs with the page's port and access"),
            status.value(QStringLiteral("port")).toInt() == port && status.value(QStringLiteral("access")) == QLatin1String("pin"));
    d.check(QStringLiteral("qp14: the hub has an id and a name"), !hub->hubId().isEmpty() && !hub->hubName().isEmpty()
                                                                     && status.value(QStringLiteral("name")).toString() == hub->hubName());
    d.check(QStringLiteral("qp14: the port is read-only while sharing"), !item("Networking.port")->isEnabled());
    // The app shows the demo, so the hub serves one of its own (behaviour.md §10)
    const QJsonArray served = hubJson(hub->handle(), dspi_hub_devices).value(QStringLiteral("value")).toArray();
    d.check(QStringLiteral("qp14: the hub shares a demo device of its own"),
            device->demo() && hub->sharesOwnDemo() && served.size() == 1);
    d.check(QStringLiteral("qp14: the page says the hub's demo is its own"),
            d.waitFor(1000, [&]() { return item("Networking.demoNote")->isVisible(); }));
    d.check(QStringLiteral("qp14: the app's device is still open"), d.waitIdle() && device->connected() && device->demo());

    // A client with no code gets no way in
    {
        Client client;
        client.connectAndPair(address, QString());
        d.check(QStringLiteral("qp14: a client must pair (Require Pairing)"),
                client.wait(d) && client.hello.value(QStringLiteral("needsPairing")).toBool());
    }

    // ---- Pairing: the page's button opens a window, the code is what the client types ----
    d.check(QStringLiteral("qp14: Pair is on while sharing"), click(item("Networking.pair")));
    d.check(QStringLiteral("qp14: a six-digit code shows"),
            d.waitFor(5000, [&]() { return hub->pairingCode().size() == 6; }) && hub->pairingSecondsLeft() > 100);
    d.spin(200);
    d.check(QStringLiteral("qp14: the code is on the page, and Pair hides"),
            item("Networking.pairingCode")->isVisible() && item("Networking.pairingCode")->property("text") == hub->pairingCode()
                && !item("Networking.pair")->isVisible());
    d.check(QStringLiteral("qp14: the time left counts down from two minutes"),
            item("Networking.pairingExpiry")->property("text").toString().startsWith(QStringLiteral("Expires in 1:5"))
                || item("Networking.pairingExpiry")->property("text").toString() == QLatin1String("Expires in 2:00"));
    d.grab(w, QStringLiteral("settings-networking-pairing.png"));

    auto client = std::make_unique<Client>();
    client->connectAndPair(address, hub->pairingCode());
    d.check(QStringLiteral("qp14: a client pairs with the page's code"),
            client->wait(d) && client->paired.value(QStringLiteral("ok")).toBool()
                && !client->paired.value(QStringLiteral("token")).toString().isEmpty());
    d.check(QStringLiteral("qp14: the client is listed, and the code is gone"),
            d.waitFor(5000, [&]() { return hub->clients().size() == 1 && hub->pairingCode().isEmpty(); }));
    d.check(QStringLiteral("qp14: its name is the one it gave"), firstClient(hub).value(QStringLiteral("name")) == QLatin1String("Smoke client"));
    d.check(QStringLiteral("qp14: the status line counts it"),
            d.waitFor(3000, [&]() { return hub->statusLine().endsWith(QStringLiteral("1 client connected")); }));
    d.spin(200);
    d.check(QStringLiteral("qp14: the page lists it"), item("Networking.client.0") && item("Networking.client.0")->isVisible());
    d.grab(w, QStringLiteral("settings-networking-client.png"));
    // The lower half: Other Hubs and Running in the Background
    page->setProperty("contentY", qMax(0.0, page->property("contentHeight").toReal() - page->height()));
    d.spin(100);
    d.grab(w, QStringLiteral("settings-networking-background.png"));
    page->setProperty("contentY", 0);
    const int cid = firstClient(hub).value(QStringLiteral("cid")).toInt();
    hub->setRole(cid, QStringLiteral("viewer"));
    d.check(QStringLiteral("qp14: a role changes"),
            d.waitFor(5000, [&]() { return firstClient(hub).value(QStringLiteral("role")) == QLatin1String("viewer"); }));
    d.check(QStringLiteral("qp14: the page's role picker follows"),
            d.waitFor(1000, [&]() { return item("Networking.client.0.role")->property("currentIndex").toInt() == 0; }));

    // ---- Access: going Open would cost this client, so the page asks first ----
    hub->refresh();
    d.check(QStringLiteral("qp14: what going Open would cost"),
            d.waitFor(5000, [&]() { return hub->revokeCost(QStringLiteral("none")) == 1 && hub->revokeCost(QStringLiteral("pin")) == 0; }));
    QQuickItem *access = item("Networking.access");
    if (access) QMetaObject::invokeMethod(access, "chosen", Q_ARG(QVariant, QStringLiteral("none")));
    QObject *question = page->findChild<QObject *>(QStringLiteral("Networking.accessQuestion"));
    d.check(QStringLiteral("qp14: going Open asks first, with the cost"),
            question && d.waitFor(3000, [&]() { return question->property("opened").toBool(); })
                && question->property("text").toString().startsWith(QStringLiteral("1 paired client")));
    d.grab(w, QStringLiteral("settings-networking-access.png"));
    if (question) QMetaObject::invokeMethod(question, "close");
    d.check(QStringLiteral("qp14: refused, the access is unchanged"),
            d.waitFor(2000, [&]() { return question && !question->property("visible").toBool(); }) && hub->access() == QLatin1String("pin")
                && hub->running());
    d.check(QStringLiteral("qp14: and the picker shows Require Pairing again"),
            d.item(QStringLiteral("Networking.access.combo"), scope)->property("currentIndex").toInt() == 1);

    // ---- Port refused while running; a rename restarts the hub ----
    d.check(QStringLiteral("qp14: the port is refused while sharing"),
            !hub->setPort(port + 1) && hub->lastError() == QLatin1String("Turn sharing off to change the port."));
    hub->dismissError();
    QQuickItem *name = item("Networking.name");
    if (name) {
        name->setProperty("text", QStringLiteral("Smoke studio"));
        QMetaObject::invokeMethod(name, "committed", Q_ARG(QString, QStringLiteral("Smoke studio")));
    }
    d.check(QStringLiteral("qp14: a rename from the page restarts the hub with it"), d.waitFor(15000, [&]() {
        return hub->running() && !hub->busy() && hubJson(hub->handle(), dspi_hub_status).value(QStringLiteral("name")) == QLatin1String("Smoke studio");
    }));
    d.check(QStringLiteral("qp14: the name is stored as hub/name"), QSettings().value(QStringLiteral("hub/name")) == QLatin1String("Smoke studio"));
    d.check(QStringLiteral("qp14: the client is still paired after the restart"),
            d.waitFor(5000, [&]() { return hub->clients().size() == 1; }));

    hub->revoke(firstClient(hub).value(QStringLiteral("cid")).toInt());
    d.check(QStringLiteral("qp14: revoking empties the list"), d.waitFor(5000, [&]() { return hub->clients().isEmpty(); }));
    client.reset();

    // A pairing window cancelled from the page
    hub->allowNewClient();
    d.check(QStringLiteral("qp14: another window opens"), d.waitFor(5000, [&]() { return !hub->pairingCode().isEmpty(); }));
    d.spin(100);
    d.check(QStringLiteral("qp14: Cancel closes it on the hub"),
            click(item("Networking.pairingCancel")) && hub->pairingCode().isEmpty() && d.waitFor(5000, [&]() {
                return !hubJson(hub->handle(), dspi_hub_status).value(QStringLiteral("pairingOpen")).toBool();
            }));

    flip(item("Networking.share"));
    d.check(QStringLiteral("qp14: sharing off stops the hub"), d.waitFor(15000, [&]() { return !hub->running() && !hub->busy(); }));
    d.check(QStringLiteral("qp14: and the app's device is back on its own path"), d.waitIdle() && device->connected());
    d.check(QStringLiteral("qp14: no demo note once stopped"), !item("Networking.demoNote")->isVisible());

    // ---- A port in use: the switch stays on and the page says why ----
    QTcpServer squatter;
    if (squatter.listen(QHostAddress::LocalHost, 0)) {
        d.check(QStringLiteral("qp14: a busy port is taken while stopped"), hub->setPort(int(squatter.serverPort())));
        hub->setSharing(true);
        d.check(QStringLiteral("qp14: a port in use fails the start"),
                d.waitFor(15000, [&]() { return !hub->busy() && !hub->lastError().isEmpty(); }) && !hub->running() && hub->sharing());
        d.spin(200);
        d.check(QStringLiteral("qp14: the page shows it in a line, the switch still on"),
                item("Networking.error")->isVisible() && item("Networking.share")->property("checked").toBool()
                    && item("Networking.errorText")->property("text").toString().contains(QString::number(squatter.serverPort())));
        d.grab(w, QStringLiteral("settings-networking-error.png"));

        // The Windows rows, as Windows would report them, while sharing is on
        firewall->simulate(QStringLiteral("none"), true);
        d.spin(100);
        d.check(QStringLiteral("qp14: no rule: the firewall row offers the button"),
                item("Networking.firewall")->isVisible() && item("Networking.firewallAllow")->isVisible()
                    && item("Networking.firewallText")->property("text").toString().startsWith(QStringLiteral("Windows Firewall has no rule")));
        d.check(QStringLiteral("qp14: a Public network shows its row"), item("Networking.publicNetwork")->isVisible());
        d.grab(w, QStringLiteral("settings-networking-firewall.png"));
        firewall->simulate(QStringLiteral("blocked"), false);
        d.check(QStringLiteral("qp14: blocked says so; no Public row on a Private network"),
                item("Networking.firewallText")->property("text").toString().startsWith(QStringLiteral("Windows Firewall blocks"))
                    && !item("Networking.publicNetwork")->isVisible());
        firewall->simulate(QStringLiteral("allowed"), false);
        d.check(QStringLiteral("qp14: allowed shows no row"), !item("Networking.firewall")->isVisible());
        firewall->simulate(QStringLiteral("none"), true);

        hub->setSharing(false);
        d.check(QStringLiteral("qp14: switched off after the failure, the line goes"),
                !hub->sharing() && hub->lastError().isEmpty() && !item("Networking.error")->isVisible());
        d.check(QStringLiteral("qp14: and the Windows rows go with sharing"),
                !item("Networking.firewall")->isVisible() && !item("Networking.publicNetwork")->isVisible());
        firewall->simulate(QString(), false);
        squatter.close();
    }

    // Leaving the page stops browsing
    settings->open(QStringLiteral("about"));
    d.spin(300);
    d.check(QStringLiteral("qp14: no browsing once the page is gone"), !devices->browseRunning());
    w->close();
    hub->setPort(11915);
    QSettings().remove(QStringLiteral("hub"));
}

} // namespace

static const SmokeDriver::Section networking("qp14-networking", 140, &networkingSection);
