// -- WP FEAT2 -- An AI agent's requests to open a known device and to close it
// again, with a fake agent over HTTP (the real server, on loopback): a unit the
// hub of an open device lists opens in a tab beside the others and is not made
// active; the agent closes only that one; what the app cannot open without
// dialling is declined, in words. Against `uconsole --demo hub --alink --with-measure-units`
// (DSPI_SMOKE_UCONSOLE), as xdev1-devices.
#include "SmokeDriver.h"

#include "AiController.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "LinkController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SessionModel.h"

#include <QDir>
#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPointer>
#include <QProcess>
#include <QTcpServer>
#include <QTcpSocket>

namespace {

const QString A = QStringLiteral("DEMO-TB-A");
const QString B = QStringLiteral("DEMO-TB-B");

// `uconsole --demo hub --alink --with-measure-units` (SmokeXdev1.cpp's)
class AgentHub {
public:
    bool start(const QString &store) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        QDir(store).removeRecursively();
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList{ "--demo", "hub", "--alink", "--with-measure-units", "--loopback", "--json", "--no-keep-awake",
                                          "--open", "--forget-clients", "--store", store, "--port", "0" });
        if (!m_proc.waitForStarted(5000)) return false;
        QElapsedTimer t;
        t.start();
        while (t.elapsed() < 20000) {
            if (!m_proc.canReadLine() && !m_proc.waitForReadyRead(1000)) {
                if (m_proc.state() != QProcess::Running) return false;
                continue;
            }
            while (m_proc.canReadLine()) {
                const QJsonObject e = QJsonDocument::fromJson(m_proc.readLine()).object();
                if (e.value(QLatin1String("event")).toString() != QLatin1String("status")) continue;
                if (const int port = e.value(QLatin1String("port")).toInt(); port > 0) {
                    url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(port);
                    return true;
                }
            }
        }
        return false;
    }
    ~AgentHub() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    QString url;

private:
    QProcess m_proc;
};

// One tool call of the fake agent's (SmokeMcp5.cpp's)
QJsonObject callTool(SmokeDriver &d, int port, const QByteArray &token, const char *tool, const QJsonObject &args) {
    QTcpSocket socket;
    QByteArray in;
    bool done = false;
    QObject::connect(&socket, &QTcpSocket::readyRead, &socket, [&] { in += socket.readAll(); });
    QObject::connect(&socket, &QTcpSocket::disconnected, &socket, [&] {
        in += socket.readAll();
        done = true;
    });
    QObject::connect(&socket, &QTcpSocket::errorOccurred, &socket, [&] { done = true; });
    const QJsonObject meta{ { QStringLiteral("io.modelcontextprotocol/protocolVersion"), QStringLiteral("2026-07-28") },
                            { QStringLiteral("io.modelcontextprotocol/clientCapabilities"), QJsonObject() } };
    const QJsonObject body{ { QStringLiteral("jsonrpc"), QStringLiteral("2.0") },
                            { QStringLiteral("id"), 1 },
                            { QStringLiteral("method"), QStringLiteral("tools/call") },
                            { QStringLiteral("params"), QJsonObject{ { QStringLiteral("name"), QLatin1String(tool) },
                                                                     { QStringLiteral("arguments"), args },
                                                                     { QStringLiteral("_meta"), meta } } } };
    const QByteArray json = QJsonDocument(body).toJson(QJsonDocument::Compact);
    socket.connectToHost(QStringLiteral("127.0.0.1"), quint16(port));
    socket.write("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAuthorization: Bearer " + token +
                 "\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: tools/call\r\nMcp-Name: " + QByteArray(tool) +
                 "\r\nContent-Length: " + QByteArray::number(json.size()) + "\r\n\r\n" + json);
    d.waitFor(20000, [&] { return done; });
    return QJsonDocument::fromJson(in.mid(in.indexOf("\r\n\r\n") + 4)).object().value(QLatin1String("result")).toObject();
}

struct Cleanup {
    std::function<void()> f;
    ~Cleanup() { f(); }
};

void feat2Section(SmokeDriver &d) {
    d.allowSectionTime(180);
    DeviceRegistry *r = DeviceRegistry::instance();
    auto *ai = Registrar::instance<AiController>();
    auto *link = Registrar::instance<LinkController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    d.check("feat2: the controllers", r && ai && link && notices);
    if (!r || !ai || !link || !notices) return;
    // -- WP ODEC -- these checks were made before the screens followed an agent
    struct FollowOff {
        AiController *ai;
        bool was;
        ~FollowOff() { ai->setFollowAgent(was); }
    } followOff{ ai, ai->followAgent() };
    ai->setFollowAgent(false);
    if (qEnvironmentVariable("DSPI_SMOKE_UCONSOLE").isEmpty()) {
        d.notRun("feat2: an agent opens a hub's unit", QStringLiteral("no DSPI_SMOKE_UCONSOLE"));
        return;
    }
    const QString out = d.outDir() + QStringLiteral("/feat2");
    QDir().mkpath(out);
    AgentHub hub;
    const bool up = hub.start(out + QStringLiteral("/hub"));
    d.check("feat2: uconsole --demo hub --alink --with-measure-units runs (DSPI_SMOKE_UCONSOLE)", up);
    if (!up) return;

    DeviceSession *main = r->mainSession();
    const QString mode = ai->confirmMode();
    const Cleanup cleanup{ [&] {
        ai->setKnownDevices({});
        ai->setEnabled(false);
        d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
        ai->setConfirmMode(mode);
        ai->setPort(AiController::kDefaultPort);
        for (const QVariant &c : ai->clients()) ai->revoke(c.toMap().value(QStringLiteral("id")).toDouble());
        // -- FIX016 -- (R1-QT) the revoke answers on the AI thread: no agent left
        // paired for the next section (mcp4-ai's "no agent yet" after it in a shard)
        d.waitFor(5000, [&] { return ai->clients().isEmpty(); });
        for (DeviceSession *s : r->sessions())
            if (s != main) r->close(s->id());
        d.waitFor(15000, [&] { return r->sessions().size() == 1; });
        r->activate(main);
        r->setKeepOpen(false);
        notices->clear();
        d.waitIdle();
    } };

    // ---- Unit A of the hub in a tab, as the user opens it
    r->setKeepOpen(true);
    link->connectToHub(hub.url);
    d.waitFor(30000, [&] {
        bool a = false;
        for (const QVariant &v : link->devices()) a = a || v.toMap().value(QStringLiteral("serial")).toString() == A;
        return !link->busy() && link->phase() == QLatin1String("choose_device") && a;
    });
    for (const QVariant &v : link->devices())
        if (v.toMap().value(QStringLiteral("serial")).toString() == A) link->bindDevice(v.toMap().value(QStringLiteral("handle")).toInt(), A);
    const bool bound = d.waitFor(20000, [&] {
        DeviceSession *a = r->holder(A);
        return r->opening() == 0 && a && r->active() == a && a->model()->connected();
    }) && d.waitIdle(20000);
    d.check("feat2: DEMO-TB-A is open in a tab", bound);
    if (!bound) return;

    // ---- Agents on, under the default setting (opening is not asked about)
    QTcpServer probe;
    const int port = probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
    probe.close();
    ai->setStoreDir(QDir(out).filePath(QStringLiteral("store")));
    QDir().mkpath(QDir(out).filePath(QStringLiteral("store")));
    ai->setPort(port);
    ai->setConfirmMode(QStringLiteral("ask_risky"));
    ai->setEnabled(true);
    d.check("feat2: AI agents on", d.waitFor(5000, [&] { return ai->running() && !ai->busy(); }));
    ai->addClient(QStringLiteral("Smoke agent"));
    d.waitFor(3000, [&] { return !ai->newClient().isEmpty(); });
    const QByteArray token = ai->newClient().value(QStringLiteral("token")).toString().toUtf8();
    ai->clearNewClient();
    const auto call = [&](const char *tool, const QJsonObject &args = {}) { return callTool(d, port, token, tool, args); };
    const auto data = [](const QJsonObject &result) { return result.value(QLatin1String("structuredContent")).toObject(); };
    const auto known = [&] { return data(call("dspi_list_known_devices")); };
    // Where the agent's newest request stands, once the app has answered it
    const auto settled = [&] {
        QJsonObject last;
        d.waitFor(30000, [&] {
            last = known().value(QLatin1String("requests")).toArray().last().toObject();
            const QString s = last.value(QLatin1String("state")).toString();
            return s != QLatin1String("waiting_for_app") && s != QLatin1String("in_progress");
        });
        return last;
    };
    const auto asked = [&](const char *tool, const QString &id) {
        return data(call(tool, { { QStringLiteral("device"), id } })).value(QLatin1String("status")).toString();
    };

    // ---- A unit the hub lists: opened beside, not made active
    QString idB;
    for (const QJsonValue &v : known().value(QLatin1String("known")).toArray()) {
        const QString id = v.toObject().value(QLatin1String("device")).toString();
        if (id.endsWith(QLatin1Char('/') + B)) idB = id;
    }
    d.check("feat2: the agent's known devices list DEMO-TB-B of the hub", !idB.isEmpty());
    const int before = r->sessions().size();
    d.check("feat2: its open is a request to the app", asked("dspi_open_device", idB) == QLatin1String("waiting_for_app"));
    const QJsonObject opened = settled();
    std::printf("smoke: feat2: open: %s\n", QJsonDocument(opened).toJson(QJsonDocument::Compact).constData());
    QPointer<DeviceSession> sb = r->holder(B);
    d.check("feat2: the app opened it as a tab of its own",
            opened.value(QLatin1String("state")).toString() == QLatin1String("done") && sb && sb->model()->connected()
                && r->sessions().size() == before + 1);
    d.check("feat2: and shows the device it showed", r->active() == r->holder(A));
    if (!sb) return;

    // ---- Only its own tab is the agent's to close
    const QJsonObject refused = call("dspi_close_device", { { QStringLiteral("device"), r->active()->id() } });
    d.check("feat2: a device the user opened is not the agent's to close",
            refused.value(QLatin1String("isError")).toBool() && r->holder(A));
    d.check("feat2: closing its own is a request to the app", asked("dspi_close_device", sb->id()) == QLatin1String("waiting_for_app"));
    d.check("feat2: the app closed it, and nothing else",
            settled().value(QLatin1String("state")).toString() == QLatin1String("done")
                && d.waitFor(10000, [&] { return sb.isNull(); }) && r->sessions().size() == before && r->holder(A));

    // ---- What the app would have to dial, or that is not there, is declined
    ai->setKnownDevices({ QJsonObject{ { QStringLiteral("device"), QStringLiteral("hub:not-connected/UNIT") } },
                          QJsonObject{ { QStringLiteral("device"), QStringLiteral("usb:NOT-PLUGGED-IN") } } });
    // -- INTC -- the list reaches the server on the AI thread (AiController::
    // setKnownDevices): ask once the agent sees it, not before (a loaded run
    // asked first and was refused)
    d.waitFor(15000, [&] {
        int seen = 0;
        for (const QJsonValue &v : known().value(QLatin1String("known")).toArray()) {
            const QString id = v.toObject().value(QLatin1String("device")).toString();
            seen += id == QLatin1String("hub:not-connected/UNIT") || id == QLatin1String("usb:NOT-PLUGGED-IN");
        }
        return seen == 2;
    });
    for (const char *id : { "hub:not-connected/UNIT", "usb:NOT-PLUGGED-IN" }) {
        d.check("feat2: a device opened before is a known one", asked("dspi_open_device", QLatin1String(id)) == QLatin1String("waiting_for_app"));
        const QJsonObject o = settled();
        std::printf("smoke: feat2: %s: %s\n", id, qPrintable(o.value(QLatin1String("note")).toString()));
        d.check("feat2: the app declines it and says why",
                o.value(QLatin1String("state")).toString() == QLatin1String("declined")
                    && !o.value(QLatin1String("note")).toString().isEmpty() && r->sessions().size() == before);
    }
    link->closeSession();
}

const SmokeDriver::Section section("feat2-devices", 976, &feat2Section);

} // namespace
