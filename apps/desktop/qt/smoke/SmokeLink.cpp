// -- WP QP13 -- The Link controller and its token store, each run on a core of
// its own against the fake hub process (crates/dspi-link's dspi-link-fakehub):
// pairing, a wrong PIN, the token used again after a restart (by the hub's id
// and by address alone), another hub at the address (wrong_hub), the codes that
// keep a token, a busy hub's dial left with Disconnect, a dial given up and one
// replaced, and token_refused, the one thing that deletes a token. Then the
// Secret Service backend against a fake of the service on a direct D-Bus
// connection (the build image has no session bus), and the file on its own.
// Adapted from the previous desktop app's QD8 checks.
#include "SmokeDriver.h"

#include "LinkController.h"
#include "LinkFakeHub.h"
#include "TokenStore.h"

#include <QDBusArgument>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusObjectPath>
#include <QDBusServer>
#include <QDBusVariant>
#include <QDBusVirtualObject>
#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QThreadPool>
#include <atomic>
#include <map>
#include <memory>

namespace {
using namespace linksmoke;
const char kPin[] = "123456";

// A core and a controller with its own store: one run of the app. Delete it
// and make another on the same file, and that is the next run.
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

// The session's phase read straight from the core (the controller's own copy
// arrives later, through its reader thread)
QString corePhase(FfiCore *core) {
    char buf[4096];
    const uint32_t need = dspi_link_status(core, buf, sizeof buf);
    if (need == 0 || need > sizeof buf) return QString();
    return QJsonDocument::fromJson(QByteArray(buf, int(need) - 1)).object().value(QStringLiteral("phase")).toString();
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

void secretService(SmokeDriver &d, const QString &scratch) {
    // The Secret Service backend
    const QString address = QStringLiteral("unix:path=") + scratch + QStringLiteral("/secrets.sock");
    QDBusServer server(address);
    server.setAnonymousAuthenticationAllowed(true);
    d.check("qp13: a D-Bus server for the fake service", server.isConnected());
    auto fake = std::make_shared<FakeSecretService>();
    QObject::connect(&server, &QDBusServer::newConnection, &server, [fake](const QDBusConnection &c) {
        QDBusConnection(c).registerVirtualObject(QStringLiteral("/"), fake.get(), QDBusConnection::SubPath);
    });
    QDBusConnection peer = QDBusConnection::connectToPeer(server.address(), QStringLiteral("qp13-secrets"));
    d.check("qp13: connected to the fake service", d.waitFor(3000, [&] { return peer.isConnected(); }));

    // The backend blocks, and the fake answers on this thread
    const auto offThread = [&d](const std::function<void()> &job) {
        std::atomic<bool> done { false };
        QThreadPool::globalInstance()->start([&] { job(); done = true; });
        return d.waitFor(20000, [&] { return done.load(); });
    };
    std::atomic<bool> cancelled { false };
    const QString keyFile = scratch + QStringLiteral("/key-fallback.json");
    const QString hubId = QStringLiteral("AAAA-hub");
    auto store = std::make_unique<TokenStore>(makeSecretServiceBackend(peer, QString(), &cancelled, 3000), keyFile);

    TokenStore::Saved saved = TokenStore::Saved::Nowhere;
    QString got;
    d.check("qp13: secrets: save", offThread([&] { saved = store->save(hubId, QStringLiteral("tok-1")); }));
    d.check("qp13: secrets: in the store, not the file", saved == TokenStore::Saved::InSecretStore && !QFile::exists(keyFile));
    d.check("qp13: secrets: one item, under the id without case", fake->items.size() == 1
          && fake->items.begin()->second.attributes.value(QStringLiteral("dspi-link-hub-id")) == QLatin1String("aaaa-hub")
          && fake->items.begin()->second.secret == "tok-1");
    offThread([&] { got = store->load(QStringLiteral("aaaa-HUB")); });
    d.check("qp13: secrets: load", got == QLatin1String("tok-1"));
    offThread([&] { saved = store->save(hubId, QStringLiteral("tok-2")); });
    d.check("qp13: secrets: a new token replaces it", fake->items.size() == 1 && fake->items.begin()->second.secret == "tok-2");

    fake->locked = true;
    offThread([&] { got = store->load(hubId); });
    d.check("qp13: secrets: a locked keyring prompts, and then reads", fake->prompts == 1 && got == QLatin1String("tok-2") && !fake->locked);

    fake->locked = true;
    fake->dismissPrompt = true;
    offThread([&] { got = store->load(hubId); });
    d.check("qp13: secrets: a dismissed prompt reads nothing", got.isEmpty() && fake->items.size() == 1);
    offThread([&] { saved = store->save(QStringLiteral("hub-b"), QStringLiteral("tok-b")); });
    d.check("qp13: secrets: a dismissed prompt falls back to the file", saved == TokenStore::Saved::InFile && onlyOwnerCanRead(keyFile));

    fake->dismissPrompt = false;
    fake->locked = false;
    offThread([&] { got = store->load(QStringLiteral("hub-b")); });
    d.check("qp13: secrets: the file's copy moves over when the store answers", got == QLatin1String("tok-b")
          && fake->items.size() == 2 && storedIn(keyFile, QStringLiteral("hub-b")).isEmpty());

    fake->locked = true;
    fake->silentPrompt = true;
    QElapsedTimer waited;
    waited.start();
    cancelled = true;
    offThread([&] { got = store->load(hubId); });
    d.check("qp13: secrets: closing the app ends the wait for a prompt", got.isEmpty() && waited.elapsed() < 2000);
    cancelled = false;
    fake->silentPrompt = false;
    fake->locked = false;

    offThread([&] { store->forget(hubId); });
    d.check("qp13: secrets: forget", fake->items.size() == 1);
    offThread([&] { got = store->load(hubId); });
    d.check("qp13: secrets: forgotten", got.isEmpty());

    // No service at all, as in this container's session bus
    QDBusConnection absent(QStringLiteral("qd8-no-such-connection"));
    const QString absentFile = scratch + QStringLiteral("/absent.json");
    TokenStore noService(makeSecretServiceBackend(absent, QString(), &cancelled, 1000), absentFile);
    d.check("qp13: no service: the file", noService.save(hubId, QStringLiteral("tok-3")) == TokenStore::Saved::InFile
          && noService.load(hubId) == QLatin1String("tok-3") && onlyOwnerCanRead(absentFile));

    // The file on its own
    QFile::setPermissions(absentFile, QFile::ReadOwner | QFile::WriteOwner | QFile::ReadOther | QFile::ReadGroup);
    noService.save(QStringLiteral("hub-c"), QStringLiteral("tok-c"));
    d.check("qp13: file: a file others could read goes back to 0600", onlyOwnerCanRead(absentFile));
    {
        QFile damaged(absentFile);
        if (damaged.open(QIODevice::WriteOnly)) damaged.write("not json");
    }
    d.check("qp13: file: damaged reads as nothing", noService.load(hubId).isEmpty());
    d.check("qp13: file: damaged is replaced", noService.save(hubId, QStringLiteral("tok-4")) == TokenStore::Saved::InFile
          && noService.load(hubId) == QLatin1String("tok-4"));

    store.reset();
    QDBusConnection::disconnectFromPeer(QStringLiteral("qp13-secrets"));
}

void linkSection(SmokeDriver &d) {
    const QString scratch = d.outDir() + QStringLiteral("/link");
    QDir(scratch).removeRecursively();
    QDir().mkpath(scratch);
    const QString file = scratch + QStringLiteral("/tokens.json");
    const auto settled = [&d](LinkController &l) { return d.waitFor(30000, [&] { return !l.busy(); }); };
    const auto phaseIs = [&d](LinkController &l, const char *phase) {
        return d.waitFor(3000, [&] { return l.phase() == QLatin1String(phase); });
    };

    FakeHubProcess hub;
    const bool hubUp = hub.start({ "--rta", "--auth", QStringLiteral("window:%1").arg(kPin) });
    d.check("qp13: the fake hub runs (DSPI_SMOKE_FAKEHUB)", hubUp);
    if (!hubUp) {
        secretService(d, scratch);
        return;
    }

    QString firstToken;
    {
        // Pairing
        Run run(file);
        LinkController &l = *run.link;
        d.check("qp13: idle at first, in the core's words", l.phase() == QLatin1String("idle")
                                                              && l.phaseKey() == QLatin1String("idle") && !l.phaseText().isEmpty());
        d.check("qp13: a bad address is refused", !l.connectToAddress(QStringLiteral("not a[n] address:x")) && !l.errorCode().isEmpty());
        d.check("qp13: dials from the UI thread without waiting", l.connectToAddress(hub.address) && l.busy());
        d.check("qp13: the dial ends", settled(l));
        d.check("qp13: asks for the PIN", phaseIs(l, "needs_pairing"));
        d.check("qp13: wording by PIN source", l.phaseKey() == QLatin1String("needs_pairing.window") && !l.phaseText().isEmpty());
        d.check("qp13: the hub's id and name from the hello", l.hubId() == hub.id && !l.hubName().isEmpty());
        l.pair(QStringLiteral("12345"));
        d.check("qp13: a PIN that is not six digits is not sent", !l.busy() && l.errorCode().isEmpty());
        l.pair(QStringLiteral("000000"));
        d.check("qp13: wrong PIN", settled(l) && l.errorCode() == QLatin1String("wrong_pin") && !l.errorText().isEmpty());
        d.check("qp13: a wrong PIN stores nothing", storedIn(file, hub.id).isEmpty());
        l.pair(QString::fromLatin1(kPin));
        d.check("qp13: right PIN", settled(l) && l.errorCode().isEmpty());
        d.check("qp13: past pairing", phaseIs(l, "choose_device"));
        d.check("qp13: the hub's device is listed", d.waitFor(3000, [&] { return l.devices().size() == 1; }));
        d.check("qp13: stored", d.waitFor(3000, [&] { return !l.busy() && !storedIn(file, hub.id).isEmpty(); }));
        firstToken = storedIn(file, hub.id);
        d.check("qp13: no secret store, so a 0600 file", onlyOwnerCanRead(file));
        d.check("qp13: and the notice says so", d.waitFor(1000, [&] { return l.tokenWarning().contains(file); }));
        l.dismissTokenWarning();
        d.check("qp13: the notice can be dismissed", l.tokenWarning().isEmpty());
        // The PIN window closes with this pairing, so no later run can pair again
    }
    {
        // Used again after a restart, the hub known
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, hub.id);
        d.check("qp13: restart: the token works", settled(l) && l.errorCode().isEmpty());
        d.check("qp13: restart: no PIN asked", phaseIs(l, "choose_device"));
    }
    {
        // ... and by address alone: no token goes out until the hub has named itself
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url);
        d.check("qp13: restart by address: the token works", settled(l) && l.errorCode().isEmpty());
        d.check("qp13: restart by address: no PIN asked", phaseIs(l, "choose_device"));
        d.check("qp13: restart: same token, nothing to save", storedIn(file, hub.id) == firstToken);
        d.check("qp13: restart: no new notice", l.tokenWarning().isEmpty());
    }
    {
        // Another hub at the address: expected_hub_id holds the token back
        const QString other = QStringLiteral("00000000-0000-4000-8000-000000000001");
        TokenStore(nullptr, file).save(other, QStringLiteral("other-hubs-token"));
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, other);
        d.check("qp13: wrong_hub", settled(l) && l.errorCode() == QLatin1String("wrong_hub") && !l.errorText().isEmpty());
        d.check("qp13: wrong_hub keeps its token", storedIn(file, other) == QLatin1String("other-hubs-token"));
        d.check("qp13: wrong_hub keeps the other token", storedIn(file, hub.id) == firstToken);
        d.check("qp13: wrong_hub leaves no session", phaseIs(l, "idle"));
    }
    {
        // rate_limited says nothing about the token, nor does another failure
        d.check("qp13: the hub takes token rate_limited", hub.fault("token rate_limited"));
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, hub.id);
        d.check("qp13: rate_limited", settled(l) && l.errorCode() == QLatin1String("rate_limited"));
        d.check("qp13: rate_limited keeps the token", storedIn(file, hub.id) == firstToken);
        d.check("qp13: the hub takes token bad_request", hub.fault("token bad_request"));
        l.connectToHub(hub.url, hub.id);
        d.check("qp13: another failure", settled(l) && l.errorCode() == QLatin1String("bad_request"));
        d.check("qp13: another failure keeps the token", storedIn(file, hub.id) == firstToken);
        // A busy hub is worth another try: the first dial keeps trying, the app
        // shows it dialling, and Disconnect is the way out
        d.check("qp13: the hub takes token internal", hub.fault("token internal"));
        l.connectToHub(hub.url, hub.id);
        d.spin(1500);
        d.check("qp13: a busy hub: still dialling", l.busy() && l.errorCode().isEmpty() && l.phase() == QLatin1String("idle"));
        QElapsedTimer stop;
        stop.start();
        l.closeSession();
        d.check("qp13: a busy hub: Disconnect stops the dial", settled(l) && stop.elapsed() < 10000 && l.errorCode().isEmpty());
        d.check("qp13: a busy hub: no session left", phaseIs(l, "idle") && !dspi_is_connected(run.core));
        d.check("qp13: a busy hub keeps the token", storedIn(file, hub.id) == firstToken);
        d.check("qp13: the hub takes token ok", hub.fault("token ok"));
        l.connectToHub(hub.url, hub.id);
        d.check("qp13: after the limit the same token works", settled(l) && l.errorCode().isEmpty());
        d.check("qp13: after the limit, no PIN", phaseIs(l, "choose_device"));
    }
    {
        // A bound device: read into the core's mirror, and a drop comes back
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, hub.id);
        d.check("qp13: session for the bind", settled(l) && d.waitFor(3000, [&] { return l.devices().size() == 1; }));
        const QVariantMap device = l.devices().value(0).toMap();
        int bound = 0, back = 0;
        QObject::connect(&l, &LinkController::deviceBound, &l, [&] { bound++; });
        QObject::connect(&l, &LinkController::reconnected, &l, [&] { back++; });
        l.bindDevice(device.value(QStringLiteral("handle")).toInt(), device.value(QStringLiteral("serial")).toString());
        d.check("qp13: bound and read", settled(l) && bound == 1 && phaseIs(l, "bound")
                                            && l.boundSerial() == device.value(QStringLiteral("serial")).toString());
        d.check("qp13: the mirror holds it", dspi_is_connected(run.core));
        d.check("qp13: the hub takes drop", hub.fault("drop"));
        d.check("qp13: a drop reads as reconnecting, then bound again",
                d.waitFor(15000, [&] { return back == 1 && l.phase() == QLatin1String("bound"); }));
        d.check("qp13: the mirror came back", dspi_is_connected(run.core));
        d.check("qp13: the session is still up", settled(l) && l.errorCode().isEmpty());
    }
    {
        // A way out during a first dial: the core throws that dial away, so its
        // session never comes up
        d.check("qp13: the hub takes stall", hub.fault("stall"));
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, hub.id);
        d.spin(200);
        QElapsedTimer t;
        t.start();
        l.closeSession();
        d.check("qp13: closing during a dial does not wait", t.elapsed() < 1000 && l.phase() == QLatin1String("idle"));
        bool staleUp = false;
        l.setStaleDialHook([&] { staleUp = true; });
        d.check("qp13: the hub takes unstall", hub.fault("unstall"));
        d.check("qp13: the dial ends", settled(l));
        // The app asked for it, so it shows no error
        d.check("qp13: a dial Disconnect stopped brings up no session",
                !staleUp && l.phase() == QLatin1String("idle") && !dspi_is_connected(run.core));
        d.spin(300);
        l.refreshStatus();
        d.spin(300);
        d.check("qp13: and none shows later", l.phase() == QLatin1String("idle") && l.errorCode().isEmpty());
    }
    {
        // Replaced during a first dial: that dial's session may come up for a
        // moment; the replacing session is the one that stays
        d.check("qp13: the hub takes stall again", hub.fault("stall"));
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(hub.url, hub.id);
        d.spin(200);
        l.connectToHub(hub.url, hub.id);
        d.check("qp13: the hub takes unstall again", hub.fault("unstall"));
        d.check("qp13: both dials end", settled(l) && l.errorCode().isEmpty());
        d.check("qp13: the replacing session is the one up", phaseIs(l, "choose_device") && corePhase(run.core) == QLatin1String("choose_device"));
        l.closeSession();
        d.check("qp13: its session is gone", phaseIs(l, "idle") && corePhase(run.core) == QLatin1String("idle"));
    }
    {
        // A refused token is the one thing that deletes it
        TokenStore(nullptr, file).save(hub.id, QStringLiteral("a-token-this-hub-never-issued"));
        Run run(file);
        LinkController &l = *run.link;
        QStringList changed;
        QObject::connect(&l, &LinkController::tokensChanged, &l, [&](const QString &id) { changed << id; });
        l.connectToHub(hub.url, hub.id);
        d.check("qp13: token_refused asks for a PIN", settled(l) && phaseIs(l, "needs_pairing"));
        d.check("qp13: token_refused deletes it", d.waitFor(3000, [&] { return !l.busy() && storedIn(file, hub.id).isEmpty(); }));
        d.check("qp13: and says so", changed.contains(hub.id));
        d.check("qp13: and only that one", !storedIn(file, QStringLiteral("00000000-0000-4000-8000-000000000001")).isEmpty());
    }
    {
        // The token store as the app keeps it: written by the previous desktop
        // app (one JSON object of hub id to token), read by this one
        const QString earlier = scratch + QStringLiteral("/earlier.json");
        QFile f(earlier);
        d.check("qp13: a token file as the previous app wrote it",
                f.open(QIODevice::WriteOnly) && f.write("{\"0f0e0d0c-0000-4000-8000-00000000abcd\":\"tok-earlier\"}") > 0);
        f.close();
        d.check("qp13: is read, the hub id without case",
                TokenStore(nullptr, earlier).load(QStringLiteral("0F0E0D0C-0000-4000-8000-00000000ABCD")) == QLatin1String("tok-earlier"));
    }
    secretService(d, scratch);
}

const SmokeDriver::Section link("qp13-link", 130, &linkSection);
} // namespace
