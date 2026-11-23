// -- WP ODEC -- The owner's answers of 2026-11-23 on screen, with a fake agent over
// HTTP against the real server: one question lets an agent play test signals on
// a device for ten minutes (the sheet says so; the grant shows with its time left
// and a Revoke, and the agent hears of the Revoke); the screens follow what an
// agent changes (the device, Modern's section), not while the user drags or a
// dialog is open, and at most once in a few seconds; the window title says
// "(simulated)" in the function test only.
#include "SmokeDriver.h"

#include "AiController.h"
#include "AppIdentity.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "LayoutController.h"
#include "MeasureController.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"

extern "C" {
#include "dspi_core.h"
}

#include <QDir>
#include <QJsonArray>
#include <QKeyEvent>     // -- WP INTI --
#include <QMouseEvent>   // -- WP INTI --
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTcpServer>
#include <QTcpSocket>
#include <memory>

namespace {

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// One tool call of the fake agent's, read on this thread's event loop (as SmokeAivis's)
struct Call {
    QTcpSocket socket;
    QByteArray in;
    bool done = false;

    Call(int port, const QByteArray &token, const char *tool, const QJsonObject &args) {
        QObject::connect(&socket, &QTcpSocket::readyRead, &socket, [this] { in += socket.readAll(); });
        QObject::connect(&socket, &QTcpSocket::disconnected, &socket, [this] {
            in += socket.readAll();
            done = true;
        });
        QObject::connect(&socket, &QTcpSocket::errorOccurred, &socket, [this] { done = true; });
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
    }
    QJsonObject result() const {
        return QJsonDocument::fromJson(in.mid(in.indexOf("\r\n\r\n") + 4)).object().value(QLatin1String("result")).toObject();
    }
};

bool click(QObject *item) {
    auto *i = qobject_cast<QQuickItem *>(item);
    return i && i->isVisible() && i->isEnabled() && QMetaObject::invokeMethod(i, "clicked");
}

QString textOf(QObject *item) { return item ? item->property("text").toString() : QString(); }

struct Scene {
    SmokeDriver &d;
    int port;
    QByteArray token;

    QJsonObject result(const std::unique_ptr<Call> &c) const {
        d.waitFor(50000, [&] { return c->done; });
        return c->result();
    }
    std::unique_ptr<Call> call(const char *tool, const QJsonObject &args = {}) const {
        return std::make_unique<Call>(port, token, tool, args);
    }
    bool ok(const char *tool, const QJsonObject &args = {}) const {
        return !result(call(tool, args)).value(QLatin1String("isError")).toBool(true);
    }
};

QJsonObject gain(int output, double db, const QString &device = QString()) {
    QJsonObject o{ { QStringLiteral("output"), output }, { QStringLiteral("gain_db"), db } };
    if (!device.isEmpty()) o.insert(QStringLiteral("device"), device);
    return o;
}

void odecSection(SmokeDriver &d) {
    d.allowSectionTime(180);
    auto *ai = Registrar::instance<AiController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *layout = Registrar::instance<LayoutController>();
    auto *registry = DeviceRegistry::instance();
    Shell *shell = d.app().shell;
    d.check("odec: the controllers exist", ai && mc && layout && registry);
    if (!ai || !mc || !layout || !registry) return;

    // ---- The window title says "(simulated)" in the function test only
    const QByteArray simWas = qgetenv("DSPI_SIM_SCENARIO");
    qunsetenv("DSPI_SIM_SCENARIO");
    d.check("odec: the window title is the app's name", AppIdentity::windowTitle() == AppIdentity::displayName()
            && !AppIdentity::simulated());
    qputenv("DSPI_SIM_SCENARIO", "/work/scenario.json");
    d.check("odec: the function test's title says simulated", AppIdentity::simulated()
            && AppIdentity::windowTitle() == AppIdentity::displayName() + QStringLiteral(" (simulated)"));
    if (simWas.isEmpty()) qunsetenv("DSPI_SIM_SCENARIO");
    else qputenv("DSPI_SIM_SCENARIO", simWas);

    // ---- On, asking about risky changes
    const QString mode = ai->confirmMode();
    const bool capOn = ai->agentSplCapOn();
    const bool followWas = ai->followAgent();
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("odec-store")));
    QDir().mkpath(QDir(d.outDir()).filePath(QStringLiteral("odec-store")));
    Scene s{ d, freePort(), {} };
    ai->setPort(s.port);
    ai->setConfirmMode(QStringLiteral("ask_risky"));
    ai->setAgentSplCapOn(false);   // the simulated microphone reads no dB SPL
    ai->setFollowAgent(false);
    ai->setEnabled(true);
    d.check("odec: the server runs", d.waitFor(5000, [&] { return ai->running() && !ai->busy(); }));
    ai->addClient(QStringLiteral("Odec agent"));
    d.waitFor(3000, [&] { return !ai->newClient().isEmpty(); });
    s.token = ai->newClient().value(QStringLiteral("token")).toString().toUtf8();
    ai->clearNewClient();

    // A confirmed speaker setup, so that the agent may measure
    DeviceModel *device = d.app().device;
    device->write(QLatin1String("odec volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
    d.waitIdle();
    const QByteArray layoutJson = QJsonDocument(QJsonObject{
        { QStringLiteral("outputs"), QJsonArray{ QJsonObject{ { QStringLiteral("output"), 0 }, { QStringLiteral("role"), QStringLiteral("low") },
                                                             { QStringLiteral("speaker"), QStringLiteral("L") }, { QStringLiteral("group"), 0 } } } },
        { QStringLiteral("confirmedByUser"), true } }).toJson(QJsonDocument::Compact);
    bool set = false;
    d.app().core->worker()->call(&d, [layoutJson](FfiCore *c) {
        QByteArray buf(1 << 16, '\0');
        return dspi_speaker_layout_set(c, layoutJson.constData(), buf.data(), uint32_t(buf.size())) > 0;
    }, [&](bool) { set = true; });
    d.waitFor(5000, [&] { return set; });

    // ---- One question for test signals: the sheet says what Allow grants
    const QJsonObject args{ { QStringLiteral("outputs"), QJsonArray{ 0 } }, { QStringLiteral("sweep_ms"), 5000 }, { QStringLiteral("passes"), 8 } };
    auto held = s.call("dspi_measure_start", args);
    d.check("odec: a measurement start is asked", d.waitFor(5000, [&] { return !ai->confirmations().isEmpty(); }));
    const QVariantMap q = ai->confirmations().value(0).toMap();
    const QString grantText = q.value(QStringLiteral("grantText")).toString();
    d.check("odec: the question carries the grant", q.value(QStringLiteral("grant")).toMap().value(QStringLiteral("kind")).toString()
                                                         == QLatin1String("test_signals"));
    d.check("odec: and says it in words, ten minutes, the rest still asked",
            grantText.contains(QLatin1String("10 minutes")) && grantText.contains(QLatin1String("Odec agent"))
                && grantText.contains(QLatin1String("still asked one at a time")));
    d.check("odec: the risk line stays a test signal",
            q.value(QStringLiteral("riskLines")).toStringList().value(0).startsWith(QLatin1String("Test signal")));
    QObject *sheetLine = d.object(QStringLiteral("AiConfirmSheet.grant"));
    d.check("odec: the sheet shows the grant's line", d.waitFor(2000, [&] { return sheetLine && textOf(sheetLine) == grantText; }));
    ai->answer(q.value(QStringLiteral("id")).toDouble(), true);
    QJsonObject r = s.result(held);
    d.check("odec: Allow starts it", !r.value(QLatin1String("isError")).toBool(true));
    d.check("odec: and the grant shows, with the time left", d.waitFor(3000, [&] { return !ai->grants().isEmpty(); })
            && ai->grants().value(0).toMap().value(QStringLiteral("secondsLeft")).toInt() > 590
            && ai->grants().value(0).toMap().value(QStringLiteral("agent")).toString() == QLatin1String("Odec agent"));
    d.check("odec: the strip shows it with Revoke", d.waitFor(2000, [&] {
        QQuickItem *g = d.item(QStringLiteral("AiWorkBanner.grant"));
        return g && g->isVisible() && textOf(g).contains(QLatin1String("left")) && d.item(QStringLiteral("AiWorkBanner.revokeGrant"))->isVisible();
    }));
    const int left = ai->grants().value(0).toMap().value(QStringLiteral("secondsLeft")).toInt();
    d.check("odec: it counts down", d.waitFor(3000, [&] {
        return ai->grants().value(0).toMap().value(QStringLiteral("secondsLeft")).toInt() < left;
    }));
    d.waitFor(15000, [&] { return mc->measuring(); });
    s.ok("dspi_measure_stop");
    d.waitFor(10000, [&] { return !mc->measuring(); });
    d.check("odec: Revoke ends it", click(d.item(QStringLiteral("AiWorkBanner.revokeGrant")))
            && d.waitFor(3000, [&] { return ai->grants().isEmpty(); }));
    d.check("odec: and the agent is told", d.waitFor(3000, [&] {
        return QJsonDocument(s.result(s.call("dspi_get_user_activity", { { QStringLiteral("since"), 0 } }))).toJson().contains("revoked your permission");
    }));

    // ---- The screens follow the agent: the device, Modern's section
    ai->setConfirmMode(QStringLiteral("never_ask"));
    d.check("odec: in Modern", d.switchLayout("modern") && shell->pageHost() != nullptr);
    DeviceSession *a = registry->active();
    registry->openDemo();
    d.check("odec: a second demo", d.waitFor(10000, [&] { return registry->opening() == 0 && registry->sessions().size() == 2; }));
    DeviceSession *b = registry->sessions().value(1);
    if (a && b && shell->pageHost()) {
        const auto go = [&](const char *section) {
            d.goTo(QString::fromLatin1(section));
            d.waitIdle();
        };
        const auto section = [&] { return shell->pageHost()->property("section").toString(); };
        registry->activate(a);
        go("overview");
        d.spin(300);
        ai->setFollowAgent(true);
        d.check("odec: an agent's gain on the device not shown brings it to the front", s.ok("dspi_set_gain", gain(0, -24, b->id()))
                && d.waitFor(3000, [&] { return registry->active() == b; }));
        d.check("odec: on the section of the change", d.waitFor(2000, [&] { return section() == QLatin1String("equaliser"); }));

        // While the user drags on the device shown: only the strip
        registry->activate(a);
        go("overview");
        d.spin(AiController::kFollowMs + 300);
        a->model()->beginDrag();
        const bool changed = s.ok("dspi_set_route", { { QStringLiteral("device"), b->id() }, { QStringLiteral("input"), 0 },
                                                       { QStringLiteral("output"), 1 }, { QStringLiteral("on"), true },
                                                       { QStringLiteral("gain_db"), -20 } });
        d.spin(800);
        d.check("odec: not while the user drags", changed && ai->userInteracting() && registry->active() == a
                && section() == QLatin1String("overview"));
        a->model()->endDrag();
        d.check("odec: the strip still says it", d.waitFor(2000, [&] {
            QQuickItem *t = d.item(QStringLiteral("AiWorkBanner.text"));
            return t && t->isVisible() && textOf(t).length() > 0;
        }));

        // Nor under an open dialog
        QQmlComponent dialog(d.app().engine);
        dialog.setData("import QtQuick.Controls.Basic\nDialog { modal: true; title: \"Odec\"; width: 200; height: 100 }", QUrl());
        QObject *open = dialog.create();
        if (open && d.mainWindow()) {
            open->setParent(d.mainWindow()->contentItem());
            open->setProperty("parent", QVariant::fromValue<QObject *>(d.mainWindow()->contentItem()));
            QMetaObject::invokeMethod(open, "open");
        }
        d.check("odec: a dialog is open", d.waitFor(2000, [&] { return open && open->property("opened").toBool() && ai->userInteracting(); }));
        s.ok("dspi_set_gain", gain(2, -26, b->id()));
        d.spin(800);
        d.check("odec: not under an open dialog", registry->active() == a && section() == QLatin1String("overview"));
        if (open) QMetaObject::invokeMethod(open, "close");
        d.waitFor(2000, [&] { return !ai->userInteracting(); });
        delete open;

        // -- WP INTI -- (review RF7) Nor while an input is in flight: a mouse button held
        // (a click acts on release), a key just pressed. The events go to an object of
        // their own: the application's filter sees them, no control does.
        // A press whose release another object took, more than 10 s ago (a popup's grab in
        // an earlier section did it once): it must not change what follows
        QObject sink;
        QMouseEvent stray(QEvent::MouseButtonPress, QPointF(1, 1), QPointF(1, 1), Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(&sink, &stray);
        d.spin(10500);
        QMouseEvent press(QEvent::MouseButtonPress, QPointF(1, 1), QPointF(1, 1), Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(&sink, &press);
        const bool loud = s.ok("dspi_set_loudness", { { QStringLiteral("device"), b->id() }, { QStringLiteral("enabled"), true } });
        d.spin(AiController::kInputIdleMs + 500);
        d.check("odec: not while a mouse button is held", loud && ai->inputBusy() && registry->active() == a
                && section() == QLatin1String("overview"));
        QMouseEvent release(QEvent::MouseButtonRelease, QPointF(1, 1), QPointF(1, 1), Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
        QCoreApplication::sendEvent(&sink, &release);
        d.check("odec: right after the click, still not", ai->inputBusy() && registry->active() == a);
        d.check("odec: once the click is over, followed", d.waitFor(AiController::kInputIdleMs + 2000, [&] { return registry->active() == b; })
                && d.waitFor(1000, [&] { return section() == QLatin1String("effects"); }));
        registry->activate(a);
        go("overview");
        d.spin(AiController::kFollowMs + 300);
        QKeyEvent key(QEvent::KeyPress, Qt::Key_Right, Qt::NoModifier);
        QCoreApplication::sendEvent(&sink, &key);
        // (the overview: a place not followed to in the last minute)
        s.ok("dspi_set_device_name", { { QStringLiteral("device"), b->id() }, { QStringLiteral("name"), QStringLiteral("Odec B") } });
        d.spin(300);
        d.check("odec: not right after a key", ai->inputBusy() && registry->active() == a);
        d.check("odec: once the keyboard rests, followed", d.waitFor(AiController::kInputIdleMs + 2000, [&] { return registry->active() == b; }));
        registry->activate(a);
        go("overview");

        // At most one switch in a few seconds: the next change waits for the limit
        d.spin(AiController::kFollowMs + 300);
        s.ok("dspi_set_route", { { QStringLiteral("device"), b->id() }, { QStringLiteral("input"), 1 }, { QStringLiteral("output"), 1 },
                                 { QStringLiteral("on"), true }, { QStringLiteral("gain_db"), -20 } });
        d.check("odec: a change is followed", d.waitFor(3000, [&] { return registry->active() == b; })
                && d.waitFor(1000, [&] { return section() == QLatin1String("routing"); }));
        registry->activate(a);
        d.waitIdle();
        s.ok("dspi_set_volume", { { QStringLiteral("device"), b->id() }, { QStringLiteral("volume"), QStringLiteral("user") },
                                  { QStringLiteral("db"), -30 } });
        d.spin(500);
        d.check("odec: the next one within the limit is not at once", registry->active() == a);
        d.check("odec: but once the limit allows", d.waitFor(AiController::kFollowMs + 1500, [&] { return registry->active() == b; }));
        // The same device and section again: left where the user put it
        registry->activate(a);
        go("overview");
        d.spin(AiController::kFollowMs + 300);
        s.ok("dspi_set_volume", { { QStringLiteral("device"), b->id() }, { QStringLiteral("volume"), QStringLiteral("user") },
                                  { QStringLiteral("db"), -32 } });
        d.spin(800);
        d.check("odec: not again to the same place soon after", registry->active() == a);
        registry->close(b->id());
        d.waitFor(5000, [&] { return registry->sessions().size() == 1; });
    }

    // As it was
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setFollowAgent(followWas);
    ai->setConfirmMode(mode);
    ai->setAgentSplCapOn(capOn);
    ai->setPort(AiController::kDefaultPort);
    ai->revoke(ai->clients().value(0).toMap().value(QStringLiteral("id")).toDouble());
    d.switchLayout("classic");
    d.waitIdle();
    d.spin(200);
}

const SmokeDriver::Section odec("odec-agent", 985, &odecSection);

} // namespace
