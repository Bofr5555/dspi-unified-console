// WP QD8: the Link controller and its token store against the fake hub process
// (crates/dspi-link's dspi-link-fakehub), and the Secret Service backend against a
// fake of the service on a direct D-Bus connection (the build image has no bus).
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "LinkController.h"
#include "TokenStore.h"

#include <QDBusArgument>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusObjectPath>
#include <QDBusServer>
#include <QDBusVariant>
#include <QDBusVirtualObject>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QElapsedTimer>
#include <QProcess>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QImage>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QThreadPool>
#include <atomic>
#include <map>
#include <memory>

namespace {
const char kPin[] = "123456";

// One dspi-link-fakehub process, with its faults on stdin
class FakeHubProcess {
public:
    bool start(const QStringList &args) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_FAKEHUB");
        if (binary.isEmpty()) return false;
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList { "--bind", "127.0.0.1:0" } + args);
        if (!m_proc.waitForStarted(5000) || !m_proc.waitForReadyRead(5000)) return false;
        // fake DSPi Link hub "NAME" (kind) on ADDR id ID auth AUTH
        const QString line = QString::fromUtf8(m_proc.readLine());
        const auto match = QRegularExpression(QStringLiteral(" on ([0-9.:]+) id ([^ ]+) ")).match(line);
        if (!match.hasMatch()) return false;
        address = match.captured(1);
        id = match.captured(2);
        url = QStringLiteral("ws://%1/dspi/v1").arg(address);
        return true;
    }
    void fault(const char *command) {
        m_proc.write(command);
        m_proc.write("\n");
        m_proc.waitForBytesWritten(1000);
    }
    void kill() {
        m_proc.kill();
        m_proc.waitForFinished(2000);
    }
    ~FakeHubProcess() { kill(); }
    QString address, id, url;

private:
    QProcess m_proc;
};

// A core and a controller with its own store: what one run of the app has. Delete
// it and make another on the same file, and that is the next run.
QQuickItem *findItem(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findItem(c, name)) return found;
    return nullptr;
}

// A snapshot that keeps changing: the worker is polling.
bool analyserPolls(FfiCore *core, const std::function<bool(int, const std::function<bool()> &)> &waitFor) {
    RtaSnapshot first;
    dspi_rta_get_snapshot(core, &first);
    return waitFor(5000, [&] {
        RtaSnapshot now;
        dspi_rta_get_snapshot(core, &now);
        return now.version != first.version;
    });
}

struct Run {
    FfiCore *core;
    std::unique_ptr<LinkController> link;
    explicit Run(const QString &file) : core(dspi_core_new()) {
        link = std::make_unique<LinkController>(core, std::make_unique<TokenStore>(nullptr, file), nullptr);
    }
    ~Run() {
        link.reset();
        dspi_core_free(core);
    }
};

QString storedIn(const QString &file, const QString &hub) {
    return TokenStore(nullptr, file).load(hub);
}

bool onlyOwnerCanRead(const QString &file) {
    const QFile::Permissions others = QFile::ReadGroup | QFile::WriteGroup | QFile::ExeGroup | QFile::ReadOther
                                      | QFile::WriteOther | QFile::ExeOther;
    return QFile::exists(file) && (QFileInfo(file).permissions() & others) == 0;
}

// The parts of org.freedesktop.Secret the backend uses
class FakeSecretService : public QDBusVirtualObject {
public:
    struct Item {
        QMap<QString, QString> attributes;
        QByteArray secret;
    };
    std::map<QString, Item> items;
    bool locked = false;
    bool dismissPrompt = false;
    bool silentPrompt = false;
    int prompts = 0;
    int next = 0;

    QString introspect(const QString &) const override { return QString(); }

    bool handleMessage(const QDBusMessage &m, const QDBusConnection &c) override {
        QDBusConnection connection(c);
        const QString method = m.member();
        const QDBusObjectPath none(QStringLiteral("/"));
        const QDBusObjectPath prompt(QStringLiteral("/prompt/1"));
        QDBusMessage reply = m.createReply();
        if (method == QLatin1String("OpenSession")) {
            reply << QVariant::fromValue(QDBusVariant(QString())) << QVariant::fromValue(QDBusObjectPath("/session/1"));
        } else if (method == QLatin1String("Close")) {
        } else if (method == QLatin1String("SearchItems")) {
            const auto want = qdbus_cast<QMap<QString, QString>>(m.arguments().at(0));
            QList<QDBusObjectPath> found;
            for (const auto &entry : items)
                if (entry.second.attributes == want) found.append(QDBusObjectPath(entry.first));
            QList<QDBusObjectPath> empty;
            reply << QVariant::fromValue(locked ? empty : found) << QVariant::fromValue(locked ? found : empty);
        } else if (method == QLatin1String("Unlock")) {
            const auto objects = qdbus_cast<QList<QDBusObjectPath>>(m.arguments().at(0));
            reply << QVariant::fromValue(locked ? QList<QDBusObjectPath>() : objects)
                  << QVariant::fromValue(locked ? prompt : none);
        } else if (method == QLatin1String("Prompt")) {
            prompts++;
            connection.send(reply);
            if (silentPrompt) return true;
            if (!dismissPrompt) locked = false;
            QDBusMessage done = QDBusMessage::createSignal(m.path(), m.interface(), QStringLiteral("Completed"));
            done << dismissPrompt << QVariant::fromValue(QDBusVariant(QString()));
            connection.send(done);
            return true;
        } else if (method == QLatin1String("CreateItem")) {
            const auto properties = qdbus_cast<QVariantMap>(m.arguments().at(0));
            Item item;
            item.attributes = qdbus_cast<QMap<QString, QString>>(properties.value(QStringLiteral("org.freedesktop.Secret.Item.Attributes")));
            const auto secret = m.arguments().at(1).value<QDBusArgument>();
            QDBusObjectPath session;
            QByteArray parameters;
            QString type;
            secret.beginStructure();
            secret >> session >> parameters >> item.secret >> type;
            secret.endStructure();
            QString path;
            for (const auto &entry : items)
                if (entry.second.attributes == item.attributes) path = entry.first;
            if (path.isEmpty()) path = QStringLiteral("/item/%1").arg(++next);
            items[path] = item;
            reply << QVariant::fromValue(QDBusObjectPath(path)) << QVariant::fromValue(none);
        } else if (method == QLatin1String("GetSecret")) {
            QDBusArgument secret;
            secret.beginStructure();
            secret << QDBusObjectPath("/session/1") << QByteArray() << items[m.path()].secret << QStringLiteral("text/plain");
            secret.endStructure();
            reply << QVariant::fromValue(secret);
        } else if (method == QLatin1String("Delete")) {
            items.erase(m.path());
            reply << QVariant::fromValue(none);
        } else {
            reply = m.createErrorReply(QDBusError::UnknownMethod, method);
        }
        connection.send(reply);
        return true;
    }
};
}

void SmokeDriver::testLink() {
    section("QD8: Link controller and token store");
    const QString scratch = qEnvironmentVariable("DSPI_SMOKE_OUT") + QStringLiteral("/link");
    QDir(scratch).removeRecursively();
    QDir().mkpath(scratch);
    const QString file = scratch + QStringLiteral("/tokens.json");
    const auto settled = [this](LinkController &l) { return waitFor(30000, [&] { return !l.busy(); }); };

    FakeHubProcess hub;
    const bool hubUp = hub.start({ "--rta", "--auth", QStringLiteral("window:%1").arg(kPin) });
    check("QD8: the fake hub runs (DSPI_SMOKE_FAKEHUB)", hubUp);
    if (!hubUp) return;

    QString firstToken;
    {
        // Pairing
        Run run(file);
        LinkController &l = *run.link;
        check("QD8: idle at first, with CG10's text", l.phase() == QLatin1String("idle") && l.phaseKey() == QLatin1String("idle"));
        check("QD8: a bad address is refused", !l.connectToAddress(QStringLiteral("not a[n] address:x")) && !l.errorCode().isEmpty());
        check("QD8: dials from the UI thread without waiting", l.connectToAddress(hub.address) && l.busy());
        check("QD8: the dial ends", settled(l));
        check("QD8: asks for the PIN", waitFor(2000, [&] { return l.phase() == QLatin1String("needs_pairing"); }));
        check("QD8: wording by pin source", l.phaseKey() == QLatin1String("needs_pairing.window") && !l.phaseText().isEmpty());
        check("QD8: the hub's id and name from the hello", l.hubId() == hub.id && !l.hubName().isEmpty());
        l.pair(QStringLiteral("000000"));
        check("QD8: wrong PIN", settled(l) && l.errorCode() == QLatin1String("wrong_pin"));
        check("QD8: a wrong PIN stores nothing", storedIn(file, hub.id).isEmpty());
        l.pair(QString::fromLatin1(kPin));
        check("QD8: right PIN", settled(l) && l.errorCode().isEmpty());
        check("QD8: past pairing", waitFor(3000, [&] { return l.phase() == QLatin1String("choose_device"); }));
        check("QD8: the hub's device is listed", waitFor(3000, [&] { return l.devices().size() == 1; }));
        check("QD8: stored", waitFor(3000, [&] { return !l.busy() && !storedIn(file, hub.id).isEmpty(); }));
        firstToken = storedIn(file, hub.id);
        check("QD8: no secret store, so a 0600 file", onlyOwnerCanRead(file));
        check("QD8: and the warning says so", waitFor(1000, [&] { return !l.tokenWarning().isEmpty(); }));
        l.dismissTokenWarning();
        check("QD8: the warning can be dismissed", l.tokenWarning().isEmpty());
        // The PIN window closes with this pairing, so no later run can pair again
    }
    {
        // Reuse after a restart, the hub known
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, hub.id);
        check("QD8: restart: the token works", settled(l) && l.errorCode().isEmpty());
        check("QD8: restart: no PIN asked", waitFor(3000, [&] { return l.phase() == QLatin1String("choose_device"); }));
    }
    {
        // ... and by address alone: no token until the hub has named itself
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url);
        check("QD8: restart by address: the token works", settled(l) && l.errorCode().isEmpty());
        check("QD8: restart by address: no PIN asked", waitFor(3000, [&] { return l.phase() == QLatin1String("choose_device"); }));
        check("QD8: restart: same token, nothing to save", storedIn(file, hub.id) == firstToken);
        check("QD8: restart: no new warning", l.tokenWarning().isEmpty());
    }
    {
        // Another hub at the address
        const QString other = QStringLiteral("00000000-0000-4000-8000-000000000001");
        TokenStore(nullptr, file).save(other, QStringLiteral("other-hubs-token"));
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, other);
        check("QD8: wrong_hub", settled(l) && l.errorCode() == QLatin1String("wrong_hub") && !l.errorText().isEmpty());
        check("QD8: wrong_hub keeps its token", storedIn(file, other) == QLatin1String("other-hubs-token"));
        check("QD8: wrong_hub keeps the other token", storedIn(file, hub.id) == firstToken);
        check("QD8: wrong_hub leaves no session", l.phase() == QLatin1String("idle"));
    }
    {
        // rate_limited says nothing about the token
        hub.fault("token rate_limited");
        spin(300);
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, hub.id);
        check("QD8: rate_limited", settled(l) && l.errorCode() == QLatin1String("rate_limited"));
        check("QD8: rate_limited keeps the token", storedIn(file, hub.id) == firstToken);
        hub.fault("token internal");
        spin(300);
        l.connectToHub(hub.url, hub.id);
        check("QD8: another failure", settled(l) && !l.errorCode().isEmpty());
        check("QD8: another failure keeps the token", storedIn(file, hub.id) == firstToken);
        hub.fault("token ok");
        spin(300);
        l.connectToHub(hub.url, hub.id);
        check("QD8: after the limit the same token works", settled(l) && l.errorCode().isEmpty());
        check("QD8: after the limit, no PIN", waitFor(3000, [&] { return l.phase() == QLatin1String("choose_device"); }));
    }
    {
        // The lost-connection rule, and the analyser, on a bound device
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, hub.id);
        check("QD8: session for the bind", settled(l) && waitFor(3000, [&] { return l.devices().size() == 1; }));
        const QVariantMap device = l.devices().value(0).toMap();
        int bound = 0, back = 0;
        QObject::connect(&l, &LinkController::deviceBound, &l, [&] { bound++; });
        QObject::connect(&l, &LinkController::reconnected, &l, [&] { back++; });
        l.bindDevice(device.value(QStringLiteral("handle")).toInt(), device.value(QStringLiteral("serial")).toString());
        check("QD8: bound and read", settled(l) && bound == 1 && waitFor(3000, [&] { return l.phase() == QLatin1String("bound"); }));
        check("QD8: the mirror holds it", dspi_is_connected(run.core));
        hub.fault("drop");
        check("QD8: a drop reads as reconnecting, then bound again",
              waitFor(15000, [&] { return back == 1 && l.phase() == QLatin1String("bound"); }));
        check("QD8: the mirror came back", dspi_is_connected(run.core));
        check("QD8: the session is still up", settled(l) && l.errorCode().isEmpty());
    }
    {
        // A way out during a first dial: the session it makes is thrown away
        hub.fault("stall");
        spin(300);
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, hub.id);
        spin(200);
        QElapsedTimer t;
        t.start();
        l.closeSession();
        check("QD8: closing during a dial does not wait", t.elapsed() < 1000 && l.phase() == QLatin1String("idle"));
        hub.fault("unstall");
        check("QD8: the dial ends", settled(l));
        check("QD8: its session is gone", waitFor(3000, [&] { return l.phase() == QLatin1String("idle"); }));
    }
    {
        // A refused token is the one thing that deletes it
        TokenStore(nullptr, file).save(hub.id, QStringLiteral("a-token-this-hub-never-issued"));
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, hub.id);
        check("QD8: token_refused asks for a PIN", settled(l) && waitFor(3000, [&] { return l.phase() == QLatin1String("needs_pairing"); }));
        check("QD8: token_refused deletes it", waitFor(3000, [&] { return !l.busy() && storedIn(file, hub.id).isEmpty(); }));
        check("QD8: and only that one", !storedIn(file, QStringLiteral("00000000-0000-4000-8000-000000000001")).isEmpty());
    }

    // The app's own controller and banner, with the demo device open behind them
    auto *app = qobject_cast<LinkController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("linkSession")).value<QObject *>());
    check("QD8: registered as linkSession", app != nullptr);
    FakeHubProcess open;
    if (app && open.start({ "--rta", "--auth", "none" })) {
        // A Repeater's items are found through the visual tree
        auto *host = root()->findChild<QQuickItem *>(QStringLiteral("bannerHost"));
        auto *abort = host ? findItem(host, QStringLiteral("linkLostAbort")) : nullptr;
        auto *row = host ? findItem(host, QStringLiteral("linkLostRow")) : nullptr;
        auto *label = host ? findItem(host, QStringLiteral("linkLostText")) : nullptr;
        check("QD8: the banner is loaded", abort && row && label);
        const bool demoBefore = m_.bridge->connected();
        app->connectToHub(open.url);
        check("QD8: app session", settled(*app) && waitFor(3000, [&] { return app->phase() == QLatin1String("choose_device"); }));
        check("QD8: no banner while all is well", row && !row->isVisible());
        // -- WP QD6 -- The bridge beside a hub session
        auto *b = m_.bridge;
        b->disconnectDevice();
        spin(800);   // a hotplug poll or two
        check("QD6: while choosing on a hub, a scan opens nothing", !b->connected());
        const QVariantMap hubDevice = app->devices().value(0).toMap();
        const QString hubSerial = hubDevice.value(QStringLiteral("serial")).toString();
        int changing = 0;
        const QMetaObject::Connection counting = QObject::connect(b, &DSPiBridge::deviceChanging, b, [&] { changing++; });
        app->bindDevice(hubDevice.value(QStringLiteral("handle")).toInt(), hubSerial);
        check("QD6: the bridge adopts what the session binds", settled(*app) && waitFor(3000, [&] { return app->phase() == QLatin1String("bound"); })
                                                                  && b->connected() && b->selectedSerial() == hubSerial);
        check("QD6: and tells the controllers first", changing >= 2);
        QObject::disconnect(counting);
        check("QD6: it is writable", b->writable());
        // -- WP QD18 --
        testHostAudioOnHub();
        // Nothing here starts the analyser: binding does, and a drop starts it again
        const auto wait = [this](int ms, const std::function<bool()> &done) { return waitFor(ms, done); };
        RtaRequest request = {};
        request.tap = RTA_TAP_INPUT;
        request.mask = 1;
        dspi_rta_set_request(b->core(), &request);
        RtaCapsInfo caps;
        check("QD6: binding probes the analyser", waitFor(5000, [&] { dspi_rta_get_caps(b->core(), &caps); return caps.supported; }));
        check("QD8: the analyser runs", analyserPolls(b->core(), wait));
        int states = 0, back = 0;
        const QMetaObject::Connection stateCount = QObject::connect(b, &DSPiBridge::stateChanged, b, [&] { states++; });
        const QMetaObject::Connection backCount = QObject::connect(app, &LinkController::reconnected, b, [&] { back++; });
        open.fault("drop");
        check("QD6: a drop comes back bound", waitFor(15000, [&] { return back == 1 && app->phase() == QLatin1String("bound"); }));
        check("QD6: a reconnect redraws", states > 0);
        QObject::disconnect(stateCount);
        QObject::disconnect(backCount);
        check("QD8: the analyser's probe came back", settled(*app) && waitFor(5000, [&] { dspi_rta_get_caps(b->core(), &caps); return caps.supported; }));
        check("QD8: and it polls again", analyserPolls(b->core(), wait));
        check("QD6: still the hub's device", b->selectedSerial() == hubSerial);
        open.kill();   // gone, not back in a moment
        check("QD8: banner at once", waitFor(2000, [&] { return app->phase() == QLatin1String("reconnecting") && row && row->isVisible(); }));
        spin(800);
        check("QD6: while reconnecting, a scan leaves the device alone", b->selectedSerial() == hubSerial);
        if (auto *main = qobject_cast<QQuickWindow *>(root())) main->grabWindow().save(m_out + QStringLiteral("/window-link-banner.png"));
        if (label && abort) {
            check("QD8: banner in CG10's words", label->property("text").toString() == QLatin1String("Connection lost, reconnecting"));
            check("QD8: its button in CG10's words", abort->property("text").toString() == QLatin1String("Disconnect") && app->actionKey() == QLatin1String("reconnecting.disconnect"));
            QMetaObject::invokeMethod(abort, "clicked");
        }
        check("QD8: the button leaves the hub", waitFor(2000, [&] { return app->phase() == QLatin1String("idle") && row && !row->isVisible(); }));
        // -- WP QD6 -- The device goes with the session, and the scan takes over again
        check("QD6: back to what a scan finds", waitFor(3000, [&] { return b->connected() == demoBefore && b->selectedSerial() != hubSerial; }));
    } else {
        check("QD8: a second fake hub runs", false);
    }

    // The Secret Service backend
    const QString address = QStringLiteral("unix:path=") + scratch + QStringLiteral("/secrets.sock");
    QDBusServer server(address);
    server.setAnonymousAuthenticationAllowed(true);
    check("QD8: a D-Bus server for the fake service", server.isConnected());
    auto fake = std::make_shared<FakeSecretService>();
    QObject::connect(&server, &QDBusServer::newConnection, &server, [fake](const QDBusConnection &c) {
        QDBusConnection(c).registerVirtualObject(QStringLiteral("/"), fake.get(), QDBusConnection::SubPath);
    });
    QDBusConnection peer = QDBusConnection::connectToPeer(server.address(), QStringLiteral("qd8-secrets"));
    check("QD8: connected to the fake service", waitFor(3000, [&] { return peer.isConnected(); }));

    // The backend blocks, and the fake answers on this thread
    const auto offThread = [this](const std::function<void()> &job) {
        std::atomic<bool> done { false };
        QThreadPool::globalInstance()->start([&] { job(); done = true; });
        return waitFor(20000, [&] { return done.load(); });
    };
    std::atomic<bool> cancelled { false };
    const QString keyFile = scratch + QStringLiteral("/key-fallback.json");
    const QString hubId = QStringLiteral("AAAA-hub");
    auto store = std::make_unique<TokenStore>(makeSecretServiceBackend(peer, QString(), &cancelled, 3000), keyFile);

    TokenStore::Saved saved = TokenStore::Saved::Nowhere;
    QString got;
    check("QD8: secrets: save", offThread([&] { saved = store->save(hubId, QStringLiteral("tok-1")); }));
    check("QD8: secrets: in the store, not the file", saved == TokenStore::Saved::InSecretStore && !QFile::exists(keyFile));
    check("QD8: secrets: one item, under the id without case", fake->items.size() == 1
          && fake->items.begin()->second.attributes.value(QStringLiteral("dspi-link-hub-id")) == QLatin1String("aaaa-hub")
          && fake->items.begin()->second.secret == "tok-1");
    offThread([&] { got = store->load(QStringLiteral("aaaa-HUB")); });
    check("QD8: secrets: load", got == QLatin1String("tok-1"));
    offThread([&] { saved = store->save(hubId, QStringLiteral("tok-2")); });
    check("QD8: secrets: a new token replaces it", fake->items.size() == 1 && fake->items.begin()->second.secret == "tok-2");

    fake->locked = true;
    offThread([&] { got = store->load(hubId); });
    check("QD8: secrets: a locked keyring prompts, and then reads", fake->prompts == 1 && got == QLatin1String("tok-2") && !fake->locked);

    fake->locked = true;
    fake->dismissPrompt = true;
    offThread([&] { got = store->load(hubId); });
    check("QD8: secrets: a dismissed prompt reads nothing", got.isEmpty() && fake->items.size() == 1);
    offThread([&] { saved = store->save(QStringLiteral("hub-b"), QStringLiteral("tok-b")); });
    check("QD8: secrets: a dismissed prompt falls back to the file", saved == TokenStore::Saved::InFile && onlyOwnerCanRead(keyFile));

    fake->dismissPrompt = false;
    fake->locked = false;
    offThread([&] { got = store->load(QStringLiteral("hub-b")); });
    check("QD8: secrets: the file's copy moves over when the store answers", got == QLatin1String("tok-b")
          && fake->items.size() == 2 && storedIn(keyFile, QStringLiteral("hub-b")).isEmpty());

    fake->locked = true;
    fake->silentPrompt = true;
    QElapsedTimer waited;
    waited.start();
    cancelled = true;
    offThread([&] { got = store->load(hubId); });
    check("QD8: secrets: closing the app ends the wait for a prompt", got.isEmpty() && waited.elapsed() < 2000);
    cancelled = false;
    fake->silentPrompt = false;
    fake->locked = false;

    offThread([&] { store->forget(hubId); });
    check("QD8: secrets: forget", fake->items.size() == 1);
    offThread([&] { got = store->load(hubId); });
    check("QD8: secrets: forgotten", got.isEmpty());

    // No service at all, as in this container's session bus
    QDBusConnection absent(QStringLiteral("qd8-no-such-connection"));
    const QString absentFile = scratch + QStringLiteral("/absent.json");
    TokenStore noService(makeSecretServiceBackend(absent, QString(), &cancelled, 1000), absentFile);
    check("QD8: no service: the file", noService.save(hubId, QStringLiteral("tok-3")) == TokenStore::Saved::InFile
          && noService.load(hubId) == QLatin1String("tok-3") && onlyOwnerCanRead(absentFile));

    // The file on its own
    QFile::setPermissions(absentFile, QFile::ReadOwner | QFile::WriteOwner | QFile::ReadOther | QFile::ReadGroup);
    noService.save(QStringLiteral("hub-c"), QStringLiteral("tok-c"));
    check("QD8: file: a file others could read goes back to 0600", onlyOwnerCanRead(absentFile));
    {
        QFile damaged(absentFile);
        damaged.open(QIODevice::WriteOnly);
        damaged.write("not json");
    }
    check("QD8: file: damaged reads as nothing", noService.load(hubId).isEmpty());
    check("QD8: file: damaged is replaced", noService.save(hubId, QStringLiteral("tok-4")) == TokenStore::Saved::InFile
          && noService.load(hubId) == QLatin1String("tok-4"));

    store.reset();
    QDBusConnection::disconnectFromPeer(QStringLiteral("qd8-secrets"));
}
