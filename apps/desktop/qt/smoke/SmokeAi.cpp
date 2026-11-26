// -- WP MCP4 -- AI agents with a fake agent over HTTP (the real server, on
// loopback): off by default, the question before it goes on, the Classic icon,
// pairing allowed and denied from its sheet, a change made and undone from the
// activity (and tagged in the History), risky changes held for the sheet
// (denied, allowed, answered from the window), the three settings, a timed
// preview in the safe apply's banner, Pause, and Stop, which puts the preview
// back and turns the setting off.
#include "SmokeDriver.h"

#include "AiController.h"
#include "DeviceModel.h"
#include "HistoryController.h"
#include "LayoutController.h"
#include "LimitsController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SnapshotsController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QClipboard>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTcpServer>
#include <QTcpSocket>
#include <cmath>
#include <cstdio>
#include <memory>

namespace {

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// One request of the fake agent's, read on this thread's event loop
struct Exchange {
    QTcpSocket socket;
    QByteArray in;
    bool done = false;

    Exchange(int port, const QByteArray &path, const QByteArray &headers, const QJsonObject &body) {
        QObject::connect(&socket, &QTcpSocket::readyRead, &socket, [this] { in += socket.readAll(); });
        QObject::connect(&socket, &QTcpSocket::disconnected, &socket, [this] {
            in += socket.readAll();
            done = true;
        });
        QObject::connect(&socket, &QTcpSocket::errorOccurred, &socket, [this] { done = true; });
        const QByteArray json = QJsonDocument(body).toJson(QJsonDocument::Compact);
        socket.connectToHost(QStringLiteral("127.0.0.1"), quint16(port));
        socket.write("POST " + path + " HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n" + headers +
                     "Content-Length: " + QByteArray::number(json.size()) + "\r\n\r\n" + json);
    }
    int status() const { return in.split(' ').value(1).toInt(); }
    QJsonObject body() const { return QJsonDocument::fromJson(in.mid(in.indexOf("\r\n\r\n") + 4)).object(); }
};

struct Agent {
    SmokeDriver &d;
    int port;
    QByteArray token;
    // -- WP AIVIS -- the output it sets: what the user took back from it is theirs, so
    // each part of the section has one of its own
    const int *out = nullptr;

    std::unique_ptr<Exchange> call(const char *tool, const QJsonObject &args) const {
        const QJsonObject meta{ { QStringLiteral("io.modelcontextprotocol/protocolVersion"), QStringLiteral("2026-07-28") },
                                { QStringLiteral("io.modelcontextprotocol/clientCapabilities"), QJsonObject() } };
        const QJsonObject params{ { QStringLiteral("name"), QLatin1String(tool) },
                                  { QStringLiteral("arguments"), args },
                                  { QStringLiteral("_meta"), meta } };
        const QJsonObject body{ { QStringLiteral("jsonrpc"), QStringLiteral("2.0") },
                                { QStringLiteral("id"), 1 },
                                { QStringLiteral("method"), QStringLiteral("tools/call") },
                                { QStringLiteral("params"), params } };
        return std::make_unique<Exchange>(
            port, "/mcp",
            "Authorization: Bearer " + token + "\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: tools/call\r\nMcp-Name: " +
                QByteArray(tool) + "\r\n",
            body);
    }
    std::unique_ptr<Exchange> gain(double db) const {
        // the tools count from 1 (SHOWFIX 9); `out` from 0, as the device does
        return call("dspi_set_gain", { { QStringLiteral("output"), (out ? *out : 0) + 1 }, { QStringLiteral("gain_db"), db } });
    }
    // The result, once it has come
    QJsonObject result(Exchange &e) const {
        d.waitFor(10000, [&] { return e.done; });
        return e.body().value(QLatin1String("result")).toObject();
    }
};

bool failed(const QJsonObject &result) { return result.value(QLatin1String("isError")).toBool(true); }

bool click(QObject *item) {
    auto *i = qobject_cast<QQuickItem *>(item);
    return i && i->isVisible() && i->isEnabled() && QMetaObject::invokeMethod(i, "clicked");
}

bool opened(QObject *dialog) { return dialog && dialog->property("opened").toBool(); }

void aiSection(SmokeDriver &d) {
    auto *ai = Registrar::instance<AiController>();
    auto *settings = Registrar::instance<SettingsController>();
    auto *history = Registrar::instance<HistoryController>();
    auto *snapshots = Registrar::instance<SnapshotsController>();
    auto *limits = Registrar::instance<LimitsController>();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    QQuickWindow *main = d.mainWindow();
    d.check("mcp4: the controllers exist", ai && settings && history && snapshots && limits && main);
    if (!ai || !settings || !history || !snapshots || !limits || !main) return;
    // -- WP ODEC -- these checks were made before the screens followed an agent
    struct FollowOff {
        AiController *ai;
        bool was;
        ~FollowOff() { ai->setFollowAgent(was); }
    } followOff{ ai, ai->followAgent() };
    ai->setFollowAgent(false);

    int out = 0;   // -- WP AIVIS --
    const auto gainIs = [&](double db) { return device->state() && std::abs(double(device->state()->output_gain_db[out]) - db) < 0.01; };
    const auto setGain = [&](float db) {
        device->write(QStringLiteral("mcp4 gain"), [db, out](FfiCore *c) { return dspi_set_output_gain(c, uint8_t(out), db); });
        d.waitIdle();
    };
    const auto settled = [&] { return d.waitFor(5000, [&] { return ai->pending() == 0; }); };

    // ---- Off by default -----------------------------------------------------------------
    d.check("mcp4: AI agents are off by default, asking about risky changes", !ai->enabled() && !ai->running()
            && ai->state() == QLatin1String("off") && ai->confirmMode() == QLatin1String("ask_risky"));
    d.check("mcp4: the port is the server's own", ai->port() == AiController::kDefaultPort);
    QQuickItem *icon = d.item(QStringLiteral("AiStatusIcon"));
    d.check("mcp4: no status icon in Classic while they are off", icon && !icon->isVisible());
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("mcp4-store")));
    QDir().mkpath(QDir(d.outDir()).filePath(QStringLiteral("mcp4-store")));
    d.check("mcp4: a port below 1024 is refused, with why", !ai->setPort(80) && !ai->lastError().isEmpty()
            && ai->port() == AiController::kDefaultPort);
    const int port = freePort();
    d.check("mcp4: a free port is kept as mcp/port, which the relay reads", port > 0 && ai->setPort(port) && ai->lastError().isEmpty()
            && QSettings().value(QLatin1String(AiController::kPortKey)).toInt() == port);

    // ---- The page -----------------------------------------------------------------------
    ai->openSettings();
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check("mcp4: Settings opens on AI Agents", w && d.waitFor(3000, [&] { return w->isVisible() && d.item(QStringLiteral("AiAgentsPage"), w->contentItem()); })
            && settings->currentPage() == QLatin1String("ai"));
    if (!w) return;
    const auto item = [&](const char *name) { return d.item(QString::fromLatin1(name), w->contentItem()); };
    const auto found = [&](const char *query) {
        for (const QVariant &row : settings->sidebarRows(QString::fromLatin1(query)))
            if (row.toMap().value(QStringLiteral("id")) == QLatin1String("ai")) return true;
        return false;
    };
    d.check("mcp4: \"mcp\" and \"cursor\" find it", found("mcp") && found("cursor"));
    const QJsonObject config = QJsonDocument::fromJson(ai->desktopConfig().toUtf8()).object();
    const QJsonObject dspi = config.value(QLatin1String("mcpServers")).toObject().value(QLatin1String("dspi")).toObject();
    d.check("mcp4: Claude Desktop's configuration starts this program with --mcp",
            QFile::exists(dspi.value(QLatin1String("command")).toString())
                && dspi.value(QLatin1String("args")).toArray() == QJsonArray{ QStringLiteral("--mcp") });
    click(item("AiAgents.config.copy"));
    d.check("mcp4: Copy copies it", QGuiApplication::clipboard()->text() == ai->desktopConfig());
    d.check("mcp4: another computer's way is only a link", ai->otherComputerUrl().endsWith(QLatin1String("/docs/mcp-ssh-forwarding.md")));
    d.grab(w, QStringLiteral("settings-ai.png"));
    // -- WP UAG -- (UXP A9) with no agent paired, Connect an Agent is right under the switch
    {
        QQuickItem *connect = item("AiAgents.connect");
        QQuickItem *permissions = item("AiAgents.permissions");
        d.check("uag: no agent yet: Connect an Agent comes before the settings", ai->clients().isEmpty() && connect && permissions
                && connect->mapToScene(QPointF()).y() < permissions->mapToScene(QPointF()).y());
        QQuickItem *hostAsks = item("AiAgents.hostAsks");
        d.check("uag: it says the agent app may ask too", hostAsks && hostAsks->property("text").toString().contains(QLatin1String("this app still asks you")));
    }

    QObject *question = item("AiAgentsPage")->property("enableDialog").value<QObject *>();
    QQuickItem *allow = item("AiAgents.allow");
    const auto flip = [&] {
        allow->setProperty("checked", !allow->property("checked").toBool());
        QMetaObject::invokeMethod(allow, "toggled");
    };
    flip();
    d.check("mcp4: turning them on asks first", d.waitFor(2000, [&] { return opened(question); }) && !ai->enabled());
    d.grab(w, QStringLiteral("settings-ai-question.png"));
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.spin(200);
    d.check("mcp4: Cancel leaves them off", !ai->enabled() && !allow->property("checked").toBool());

    // A port another program has
    {
        QTcpServer other;
        other.listen(QHostAddress::LocalHost, quint16(port));
        flip();
        d.waitFor(2000, [&] { return opened(question); });
        QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
        d.check("mcp4: a port in use is said, the switch left on",
                d.waitFor(5000, [&] { return !ai->lastError().isEmpty() && !ai->busy(); }) && ai->enabled() && !ai->running()
                    && ai->state() == QLatin1String("error"));
    }
    click(item("AiAgents.retry"));
    d.check("mcp4: Try Again starts them", d.waitFor(5000, [&] { return ai->running() && !ai->busy(); }) && ai->lastError().isEmpty()
            && QSettings().value(QStringLiteral("mcp/enabled")).toBool());
    bool loopback = !ai->addresses().isEmpty();
    for (const QString &a : ai->addresses()) loopback = loopback && (a.startsWith(QLatin1String("127.0.0.1:")) || a.startsWith(QLatin1String("[::1]:")));
    d.check("mcp4: the page names the addresses, loopback only", loopback && item("AiAgents.addresses")->isVisible()
            && item("AiAgents.addresses")->property("text").toString().contains(ai->addresses().first()));
    d.check("mcp4: the port is fixed while they are on", !item("AiAgents.port")->isEnabled() && !ai->setPort(freePort()));
    ai->dismissError();
    d.check("mcp4: a limits profile is offered while the device has none", item("AiAgents.limitsOffer")->isVisible() == !limits->active());
    d.check("mcp4: the status icon appears in Classic", d.waitFor(1000, [&] { return icon->isVisible(); }));

    // ---- Pairing ------------------------------------------------------------------------
    QObject *pairing = d.object(QStringLiteral("AiPairingSheet"));
    const auto overlay = [&](const char *name) { return d.item(QString::fromLatin1(name), main->contentItem()->parentItem()); };
    const auto pair = [&](const char *name) {
        auto e = std::make_unique<Exchange>(port, "/pair", QByteArray(), QJsonObject{ { QStringLiteral("name"), QLatin1String(name) } });
        d.waitFor(5000, [&] { return e->done && opened(pairing); });
        return e;
    };
    Agent agent{ d, port, {}, &out };
    auto asked = pair("Denied agent");
    d.check("mcp4: an agent that asks to pair is shown with its name and code", opened(pairing)
            && ai->pairing().value(QStringLiteral("name")) == QLatin1String("Denied agent")
            && ai->pairing().value(QStringLiteral("code")) == asked->body().value(QLatin1String("code")).toString());
    d.check("mcp4: the state says somebody waits", ai->state() == QLatin1String("asking"));
    const auto answerAtOnce = click(overlay("AiPairingSheet.allow"));
    d.check("mcp4: a click in the first moment answers nothing", !answerAtOnce && opened(pairing));
    d.waitFor(3000, [&] { return pairing->property("armed").toBool(); });
    d.grab(main, QStringLiteral("ai-pairing.png"));
    // -- WP UAG -- (UXP A9) where the person checks the code
    d.check("uag: the pairing sheet says where the code shows", [&] {
        QObject *where = d.object(QStringLiteral("AiPairingSheet.where"));
        return where && where->property("text").toString().contains(QLatin1String("The agent's chat or its log shows the same code"));
    }());
    d.check("mcp4: Deny", click(overlay("AiPairingSheet.deny")) && d.waitFor(2000, [&] { return !opened(pairing); }));
    const auto waitFor = [&](const std::unique_ptr<Exchange> &request) {
        auto e = std::make_unique<Exchange>(port, "/pair/wait", QByteArray(),
                                            QJsonObject{ { QStringLiteral("ticket"), request->body().value(QLatin1String("ticket")) } });
        d.waitFor(10000, [&] { return e->done; });
        return e;
    };
    d.check("mcp4: the denied agent gets no token", waitFor(asked)->status() == 403 && ai->clients().isEmpty());
    asked = pair("Fake agent");
    d.waitFor(3000, [&] { return pairing->property("armed").toBool(); });
    d.check("mcp4: Allow", click(overlay("AiPairingSheet.allow")) && d.waitFor(2000, [&] { return !opened(pairing); }));
    agent.token = waitFor(asked)->body().value(QLatin1String("token")).toString().toUtf8();
    d.check("mcp4: the allowed agent gets its token, and is listed", agent.token.startsWith("dspi-mcp-")
            && d.waitFor(3000, [&] { return ai->clients().size() == 1; })
            && ai->clients().first().toMap().value(QStringLiteral("name")) == QLatin1String("Fake agent"));
    // -- WP UAG -- (UXP A9) and Connect an Agent moves down, after the settings
    d.check("uag: once an agent is paired, Connect an Agent comes after the settings", d.waitFor(3000, [&] {
        QQuickItem *connect = item("AiAgents.connect");
        QQuickItem *permissions = item("AiAgents.permissions");
        return connect && permissions && connect->mapToScene(QPointF()).y() > permissions->mapToScene(QPointF()).y();
    }));

    // ---- A change made at once, and undone -----------------------------------------------
    setGain(-10);
    auto e = agent.gain(-15);
    d.check("mcp4: a quieter change is made at once", !failed(agent.result(*e)) && d.waitFor(3000, [&] { return gainIs(-15); }));
    const auto row = [&](int n) { return ai->activity().value(n).toMap(); };
    d.check("mcp4: the activity says what, who and on which device", d.waitFor(3000, [&] { return row(0).value(QStringLiteral("undo")) == QLatin1String("can"); })
            // -- WP SHOWFIX -- (finding 4) the core's words, with a typographic minus
            && row(0).value(QStringLiteral("detail")).toString().contains(QStringLiteral("\u221215 dB"))
            && row(0).value(QStringLiteral("client")).toString().contains(QLatin1String("Fake agent"))
            && !row(0).value(QStringLiteral("device")).toString().isEmpty());
    d.check("mcp4: the state says an agent is active", ai->state() == QLatin1String("active"));
    d.check("mcp4: the History tags the agent's change", d.waitFor(3000, [&] {
        return !history->entries().isEmpty()
            && history->entries().first().toMap().value(QStringLiteral("agent")).toString().contains(QLatin1String("Fake agent"));
    }));
    d.check("mcp4: the user's own has no tag", history->entries().size() > 1
            && history->entries().at(1).toMap().value(QStringLiteral("agent")).toString().isEmpty());
    QQuickItem *log = item("AiAgents.log.list");
    d.check("mcp4: the page lists it, with Undo", log && d.waitFor(2000, [&] { return item("AiAgents.log.row.0.undo") != nullptr; }));
    d.check("mcp4: Undo of the row", click(item("AiAgents.log.row.0.undo")) && d.waitFor(3000, [&] { return gainIs(-10); })
            && d.waitFor(2000, [&] { return row(0).value(QStringLiteral("undo")) == QLatin1String("undone"); }));
    // Someone changed it since: theirs stays
    out = 1;
    setGain(-10);
    e = agent.gain(-16);
    agent.result(*e);
    d.waitFor(3000, [&] { return gainIs(-16); });
    setGain(-12);
    settled();
    d.waitFor(3000, [&] { return row(0).value(QStringLiteral("undo")) == QLatin1String("can"); });
    ai->undo(row(0).value(QStringLiteral("key")).toString());
    d.check("mcp4: an Undo leaves what someone has changed since, and says so",
            d.waitFor(3000, [&] { return row(0).value(QStringLiteral("undo")) == QLatin1String("changed"); })
                && !row(0).value(QStringLiteral("undoNote")).toString().isEmpty() && gainIs(-12));
    // Undo All, newest first
    out = 2;
    setGain(-12);
    e = agent.gain(-20);
    agent.result(*e);
    e = agent.gain(-25);
    agent.result(*e);
    d.waitFor(3000, [&] { return gainIs(-25); });
    settled();
    d.waitFor(3000, [&] { return row(1).value(QStringLiteral("undo")) == QLatin1String("can"); });
    ai->undoAll();
    d.check("mcp4: Undo All undoes them, newest first", d.waitFor(5000, [&] { return gainIs(-12) && !ai->canUndoAny(); }));

    // ---- Held for the user ----------------------------------------------------------------
    QObject *sheet = d.object(QStringLiteral("AiConfirmSheet"));
    const auto sheetOpen = [&] { return d.waitFor(5000, [&] { return opened(sheet); }); };
    const auto armed = [&] { return d.waitFor(3000, [&] { return sheet->property("armed").toBool(); }); };
    e = agent.gain(-3);
    d.check("mcp4: a louder change is held, with a sheet that says it in words", sheetOpen() && ai->confirmations().size() == 1
            && ai->confirmations().first().toMap().value(QStringLiteral("risks")).toStringList().join(QString()).contains(QStringLiteral("Out %1").arg(out + 1)));
    const auto early = click(overlay("AiConfirmSheet.allow"));
    d.spin(100);
    d.check("mcp4: nothing is made meanwhile, nor by a click in the first moment", !early && gainIs(-12) && !e->done);
    armed();
    d.grab(main, QStringLiteral("ai-confirm.png"));
    d.check("mcp4: Deny changes nothing, and the agent is told", click(overlay("AiConfirmSheet.deny")) && failed(agent.result(*e)) && gainIs(-12)
            && d.waitFor(2000, [&] { return !opened(sheet); }));
    e = agent.gain(-3);
    sheetOpen();
    armed();
    d.check("mcp4: Allow makes it, and the agent gets its result", click(overlay("AiConfirmSheet.allow")) && !failed(agent.result(*e))
            && d.waitFor(3000, [&] { return gainIs(-3); }));
    // Two waiting: one answer does not answer the next
    // -- WP UAG -- (UXP D2) asked apart; two asked together are one sheet (below, "uag: two raises …")
    setGain(-12);
    auto first = agent.gain(-9);
    sheetOpen();
    d.spin(1600);
    auto second = agent.gain(-8);
    sheetOpen();
    d.waitFor(3000, [&] { return ai->confirmations().size() == 2; });
    armed();
    click(overlay("AiConfirmSheet.allow"));
    const bool again = click(overlay("AiConfirmSheet.allow"));
    d.check("mcp4: a second click does not answer the next question", !again && d.waitFor(3000, [&] { return ai->confirmations().size() == 1; })
            && opened(sheet));
    armed();
    click(overlay("AiConfirmSheet.deny"));
    d.check("mcp4: each gets its own answer", !failed(agent.result(*first)) != !failed(agent.result(*second)));
    // The agent goes away: its question goes
    setGain(-12);
    e = agent.gain(-2);
    sheetOpen();
    e.reset();
    d.check("mcp4: the sheet of an agent that went away goes", d.waitFor(6000, [&] { return !opened(sheet); }) && gainIs(-12));

    // ---- The window -------------------------------------------------------------------------
    click(icon);
    QWindow *window = nullptr;
    d.check("mcp4: the icon opens the AI agents window", d.waitFor(3000, [&] {
        for (QWindow *t : QGuiApplication::topLevelWindows())
            if (t->objectName() == QLatin1String("AiAgentsWindow") && t->isVisible()) window = t;
        return window != nullptr;
    }));
    auto *panelWindow = qobject_cast<QQuickWindow *>(window);
    const auto inPanel = [&](const char *name) { return panelWindow ? d.item(QString::fromLatin1(name), panelWindow->contentItem()) : nullptr; };
    e = agent.gain(-4);
    d.check("mcp4: it shows what waits too", d.waitFor(5000, [&] { return inPanel("AiAgents.ask.0") != nullptr; }));
    d.check("mcp4: and answers it", d.waitFor(3000, [&] { return click(inPanel("AiAgents.ask.0.allow")); }) && !failed(agent.result(*e))
            && d.waitFor(3000, [&] { return gainIs(-4) && !opened(sheet); }));
    d.grab(panelWindow, QStringLiteral("window-ai-agents.png"));
    click(inPanel("AiAgents.pause"));
    e = agent.gain(-30);
    // -- WP AIVIS -- said as paused, not as an error
    d.check("mcp4: paused, agents change nothing", agent.result(*e).value(QLatin1String("structuredContent")).toObject().value(QLatin1String("status")).toString()
                    == QLatin1String("paused") && gainIs(-4) && ai->state() == QLatin1String("paused"));
    click(inPanel("AiAgents.pause"));
    d.check("mcp4: and Resume lets them again", !ai->paused());

    // ---- The three settings -------------------------------------------------------------------
    ai->setConfirmMode(QStringLiteral("never_ask"));
    e = agent.gain(-1);
    d.check("mcp4: never ask makes a louder change without asking", !failed(agent.result(*e)) && d.waitFor(3000, [&] { return gainIs(-1); })
            && ai->confirmations().isEmpty() && QSettings().value(QStringLiteral("mcp/confirm")) == QLatin1String("never_ask"));
    ai->setConfirmMode(QStringLiteral("ask_everything"));
    e = agent.gain(-20);
    d.check("mcp4: ask about every change holds a quieter one", sheetOpen() && gainIs(-1));
    armed();
    click(overlay("AiConfirmSheet.deny"));
    agent.result(*e);
    ai->setConfirmMode(QStringLiteral("ask_risky"));

    // ---- Both layouts ---------------------------------------------------------------------------
    auto *layout = Registrar::instance<LayoutController>();
    if (layout) {
        layout->setLayout(QStringLiteral("modern"));
        d.spin(400);
        e = agent.gain(0);
        d.check("mcp4: Modern shows the sheet too", sheetOpen());
        armed();
        d.grab(main, QStringLiteral("modern-ai-confirm.png"));
        // -- WP LAY8 -- each risk with its kind
        // -- WP UAG -- (UXP A6) the change heads the sheet; its row in the screen's words
        const auto textOf = [](QObject *o) { return o ? o->property("text").toString() : QString(); };
        QQuickItem *risk = overlay("AiConfirmSheet.risk.0");
        const QVariantMap asked = ai->confirmations().value(0).toMap();
        const QVariantMap row = asked.value(QStringLiteral("batch")).toList().value(0).toMap();
        d.check("uag: the sheet is headed by the change, without the AI mark",
                d.object(QStringLiteral("AiConfirmSheet"))->property("heading").toString().startsWith(QLatin1String("Fake agent wants to make "))
                    && d.object(QStringLiteral("AiConfirmSheet"))->property("heading").toString().contains(QLatin1String(" louder")));
        d.check("uag: its row is the asked change in the screen's words, from → to",
                risk && row.value(QStringLiteral("risky")).toBool() && risk->property("text").toString() == row.value(QStringLiteral("said")).toString()
                    && risk->property("text").toString().contains(QStringLiteral("→")) && !risk->property("text").toString().contains(QLatin1String("Louder:")));
        d.check("uag: it promises the Undo the activity has",
                textOf(d.object(QStringLiteral("AiConfirmSheet.after"))).contains(QLatin1String("undo it afterwards")));
        click(overlay("AiConfirmSheet.deny"));
        agent.result(*e);
        // -- WP UAG -- (UXP A6, C11) a flash write: no "(flash)", and no Undo promised
        e = agent.call("dspi_preset_set_startup", { { QStringLiteral("mode"), QStringLiteral("slot") }, { QStringLiteral("slot"), 2 } });
        d.check("uag: a flash write is asked", sheetOpen());
        armed();
        d.check("uag: its row names the preset without (flash)", d.waitFor(2000, [&] {
            QQuickItem *r = overlay("AiConfirmSheet.risk.0");
            return r && r->property("text").toString().startsWith(QLatin1String("Start with preset 2")) && !r->property("text").toString().contains(QLatin1String("flash"));
        }));
        d.check("uag: and says Undo cannot take it back",
                textOf(d.object(QStringLiteral("AiConfirmSheet.after"))).contains(QLatin1String("Undo cannot take it back"))
                    && !textOf(d.object(QStringLiteral("AiConfirmSheet.after"))).contains(QLatin1String("undo it afterwards")));
        d.check("uag: and what happens if nobody answers", textOf(d.object(QStringLiteral("AiConfirmSheet.left"))).contains(QLatin1String("nothing is changed and the agent is told so")));
        d.grab(main, QStringLiteral("uag-flash-question.png"));
        click(overlay("AiConfirmSheet.deny"));
        agent.result(*e);
        // -- WP UAG -- (UXP D2) two questions one agent raises together are one sheet; one Allow answers
        // exactly those; Answer One by One splits them
        {
            const auto userGain = [&](int o, float db) {
                device->write(QStringLiteral("uag gain"), [o, db](FfiCore *c) { return dspi_set_output_gain(c, uint8_t(o), db); });
                d.waitIdle();
            };
            const auto gainOf = [&](int o) { return device->state() ? double(device->state()->output_gain_db[o]) : 99.0; };
            const auto raise = [&](int o, double db) {
                return agent.call("dspi_set_gain", { { QStringLiteral("output"), o + 1 }, { QStringLiteral("gain_db"), db } });
            };
            userGain(6, -20);
            userGain(7, -20);
            auto first = raise(6, -10);
            auto second = raise(7, -10);
            QObject *confirm = d.object(QStringLiteral("AiConfirmSheet"));
            d.check("uag: two raises sent together are one sheet", d.waitFor(5000, [&] {
                return ai->confirmations().size() == 2 && confirm->property("heading").toString().contains(QLatin1String("wants to make 2 changes"));
            }));
            armed();
            d.grab(main, QStringLiteral("uag-grouped-question.png"));
            click(overlay("AiConfirmSheet.allow"));
            d.check("uag: one Allow answers both", !failed(agent.result(*first)) && !failed(agent.result(*second))
                    && d.waitFor(3000, [&] { return std::abs(gainOf(6) + 10) < 0.05 && std::abs(gainOf(7) + 10) < 0.05; }));
            // (other outputs: those the agent set and the person then set are the person's)
            userGain(4, -20);
            userGain(5, -20);
            first = raise(4, -12);
            second = raise(5, -12);
            d.waitFor(5000, [&] { return ai->confirmations().size() == 2; });
            armed();
            click(overlay("AiConfirmSheet.oneByOne"));
            d.check("uag: Answer One by One shows one question", d.waitFor(2000, [&] {
                return !confirm->property("heading").toString().contains(QLatin1String("changes"));
            }));
            armed();
            click(overlay("AiConfirmSheet.deny"));
            d.check("uag: Deny answers that one only", failed(agent.result(*first)) && ai->confirmations().size() == 1 && opened(confirm));
            armed();
            click(overlay("AiConfirmSheet.deny"));
            agent.result(*second);
        }
        // -- WP LAY8 -- The AI button and its popover, on the real server
        {
            QQuickItem *button = d.item(QStringLiteral("TopBar.ai"));
            QObject *popover = d.object(QStringLiteral("AiPopover"));
            const auto face = [&] { return button ? button->property("face").toString() : QString(); };
            d.check("lay8: an agent that called shows as connected, by its paired name",
                    popover && d.waitFor(3000, [&] { return face() == QLatin1String("connected") && ai->agents().size() == 1; })
                        && ai->agents().first().toMap().value(QStringLiteral("name")) == QLatin1String("Fake agent"));
            ai->addClient(QStringLiteral("<b>Second</b>"));
            d.waitFor(3000, [&] { return !ai->newClient().isEmpty(); });
            Agent other{ d, port, ai->newClient().value(QStringLiteral("token")).toString().toUtf8() };
            ai->clearNewClient();
            out = 3;
            setGain(-10);
            e = other.gain(-22);
            const bool made = !failed(other.result(*e));
            d.check("lay8: a second one beside it, in the order of the clients", made && d.waitFor(3000, [&] {
                return ai->agents().size() == 2 && ai->agents().last().toMap().value(QStringLiteral("name")) == QLatin1String("<b>Second</b>");
            }));
            click(button);
            d.check("lay8: the popover shows both, and the last change with Undo",
                    d.waitFor(2000, [&] { return opened(popover) && overlay("AiPopover.agent.1") && overlay("AiPopover.change.0.undo"); }));
            d.spin(300);
            d.grab(main, QStringLiteral("lay8-real.png"));
            d.check("lay8: its Undo", click(overlay("AiPopover.change.0.undo")) && d.waitFor(3000, [&] { return gainIs(-10); }));
            d.check("lay8: Disconnect stops that agent only, at once", click(overlay("AiPopover.agent.1.disconnect"))
                    && d.waitFor(3000, [&] { return ai->agents().size() == 1 && ai->clients().size() == 1; })
                    && ai->agents().first().toMap().value(QStringLiteral("name")) == QLatin1String("Fake agent"));
            e = other.gain(-23);
            d.waitFor(5000, [&] { return e->done; });
            d.check("lay8: its token is refused, and the server still listens for others", e->status() == 401 && ai->running() && gainIs(-10));
            e = agent.gain(-11);
            d.check("lay8: the other agent goes on", !failed(agent.result(*e)) && d.waitFor(3000, [&] { return gainIs(-11); }));
            e = agent.gain(0);
            d.check("lay8: a question closes the popover, and its button says somebody waits",
                    sheetOpen() && d.waitFor(1000, [&] { return !opened(popover); }) && face() == QLatin1String("attention"));
            armed();
            click(overlay("AiConfirmSheet.deny"));
            agent.result(*e);
        }
        layout->setLayout(QStringLiteral("classic"));
        d.spin(400);
        icon = d.item(QStringLiteral("AiStatusIcon"));
    }

    // ---- A client made by hand ------------------------------------------------------------------
    ai->addClient(QStringLiteral("<b>By hand</b>"));
    // -- INTC -- the token comes first and the list of clients is read after it
    // (AiController::addClient): wait for both, not for the token alone; the
    // bound only stops a hang, the wait ends when both are there
    d.check("mcp4: a client made by hand shows its token once, with the command",
            d.waitFor(15000, [&] { return !ai->newClient().isEmpty() && ai->clients().size() == 2; })
            && ai->httpCommand().contains(ai->newClient().value(QStringLiteral("token")).toString()));
    const QByteArray handToken = ai->newClient().value(QStringLiteral("token")).toString().toUtf8();
    ai->clearNewClient();
    d.check("mcp4: and not again", ai->newClient().isEmpty() && ai->httpCommand().isEmpty());
    settled();
    d.check("mcp4: Remove stops its token", [&] {
        ai->revoke(ai->clients().last().toMap().value(QStringLiteral("id")).toDouble());
        if (!d.waitFor(3000, [&] { return ai->clients().size() == 1; })) return false;
        auto x = Agent{ d, port, handToken }.gain(-5);
        d.waitFor(5000, [&] { return x->done; });
        return x->status() == 401;
    }());

    // ---- -- WP UAG -- (UXP A7) A refusal the person can see, for a few seconds; one meaning per Stop
    {
        out = 5;
        ai->setConfirmMode(QStringLiteral("never_ask"));
        const QJsonObject hp{ { QStringLiteral("family"), QStringLiteral("linkwitz_riley") }, { QStringLiteral("order"), 4 },
                              { QStringLiteral("freq_hz"), 2000 } };
        e = agent.call("dspi_set_crossover", { { QStringLiteral("output"), out + 1 }, { QStringLiteral("high_pass"), hp } });
        const bool set = !failed(agent.result(*e));
        e = agent.call("dspi_set_crossover", { { QStringLiteral("output"), out + 1 }, { QStringLiteral("high_pass"), QJsonValue(QJsonValue::Null) } });
        d.check("uag: the console refuses taking a tweeter's high-pass away", set && failed(agent.result(*e)));
        QObject *line = d.object(QStringLiteral("AiWorkBanner.text"));
        d.check("uag: the strip says why, in the person's words", d.waitFor(3000, [&] {
            const QString t = line ? line->property("text").toString() : QString();
            return t.startsWith(QLatin1String("Refused: Kept ")) && t.contains(QLatin1String("high-pass at 1 kHz"));
        }));
        d.grab(main, QStringLiteral("uag-refusal-strip.png"));
        d.check("uag: the activity says it under Not done", d.waitFor(3000, [&] {
            return row(0).value(QStringLiteral("refused")).toBool() && row(0).value(QStringLiteral("outcomeText")) == QLatin1String("Not done")
                   && row(0).value(QStringLiteral("detail")).toString().startsWith(QLatin1String("Kept "));
        }));
        d.check("uag: and the strip lets it go after a few seconds",
                d.waitFor(AiController::kRefusalMs + 2000, [&] { return ai->refusal().isEmpty(); }));
        QQuickItem *stop = inPanel("AiAgents.stop");
        d.check("uag: the panel's Stop says what it stops", stop && stop->property("text").toString() == QLatin1String("Stop Agents"));
        ai->setConfirmMode(QStringLiteral("ask_risky"));
    }

    // ---- The timed preview, and Stop ----------------------------------------------------------------
    out = 4;
    setGain(-10);
    e = agent.call("dspi_apply_changes", { { QStringLiteral("guard_s"), 300 },
                                           { QStringLiteral("changes"), QJsonArray{ QJsonObject{ { QStringLiteral("op"), QStringLiteral("set_gain") },
                                                                                                { QStringLiteral("output"), out + 1 },
                                                                                                { QStringLiteral("gain_db"), -18 } } } } });
    d.check("mcp4: a timed preview is made", !failed(agent.result(*e)) && d.waitFor(3000, [&] { return gainIs(-18); }));
    d.check("mcp4: the safe apply's banner shows it, with Keep and Revert", d.waitFor(4000, [&] { return snapshots->guardWaiting(); })
            && d.waitFor(2000, [&] { QQuickItem *b = d.item(QStringLiteral("SafeApplyBanner")); return b && b->isVisible(); }));
    d.grab(main, QStringLiteral("ai-preview.png"));
    e = agent.gain(5);
    sheetOpen();
    d.check("mcp4: Stop", click(inPanel("AiAgents.stop")) && d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); }));
    d.check("mcp4: Stop puts the preview back", d.waitFor(4000, [&] { return gainIs(-10); })
            && d.waitFor(4000, [&] { return !snapshots->guardWaiting(); }));
    d.check("mcp4: it ends the question, with nothing changed", d.waitFor(3000, [&] { return !opened(sheet); }) && failed(agent.result(*e)) && gainIs(-10));
    d.check("mcp4: it turns the setting off, and nothing listens any more", !ai->enabled() && !QSettings().value(QStringLiteral("mcp/enabled")).toBool()
            && [&] { auto x = agent.gain(-11); d.waitFor(3000, [&] { return x->done; }); return x->status() == 0; }());
    d.check("mcp4: the status icon goes", d.waitFor(1000, [&] { return icon && !icon->isVisible(); }));
    settled();
    d.check("mcp4: the activity stays, for undoing", !ai->activity().isEmpty() && d.waitFor(3000, [&] {
        return row(0).value(QStringLiteral("detail")).toString().contains(QLatin1String("put back"));
    }));
    const QString file = QDir(d.outDir()).filePath(QStringLiteral("ai-activity.json"));
    ai->exportActivityTo(file);
    settled();
    QFile f(file);
    d.check("mcp4: Export writes it as JSON", f.open(QIODevice::ReadOnly) && QJsonDocument::fromJson(f.readAll()).array().size() >= ai->activity().size());

    // As it was
    if (window) window->close();
    w->close();
    ai->setPort(AiController::kDefaultPort);
    ai->revoke(ai->clients().value(0).toMap().value(QStringLiteral("id")).toDouble());
    d.waitIdle();
    d.spin(200);
}

const SmokeDriver::Section ai("mcp4-ai", 982, &aiSection);

} // namespace
