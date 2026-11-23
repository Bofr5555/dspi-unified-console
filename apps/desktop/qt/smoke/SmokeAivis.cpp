// -- WP AIVIS -- The screen shows what an AI agent does, with a fake agent over
// HTTP (the real server, on loopback), in both layouts: the values and views an
// agent changes follow (the snapshots' list and the compare too), on the device
// shown and on another one; the working strip says which agent, what and where
// while a call runs (one held up by a busy server), with Pause and Stop; the AI
// button and Classic's icon turn; the changed control, its section and the other
// device's tab light up; a notice names the change, with Undo; the tray's tooltip
// and menu say it; a measurement the user disturbs stops and says so. And the
// agent is told what the user did, and refused a control the user took back.
#include "SmokeDriver.h"

#include "AiController.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "LayoutController.h"
#include "MeasureController.h"
#include "MeasureSession.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"
#include "SnapshotsController.h"
#include "TrayController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QAction>
#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMenu>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSystemTrayIcon>
#include <QTcpServer>
#include <QTcpSocket>
#include <atomic>
#include <cmath>
#include <functional>
#include <memory>

namespace {

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// One tool call of the fake agent's, read on this thread's event loop
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

QString textOf(QQuickItem *item) { return item ? item->property("text").toString() : QString(); }

struct Scene {
    SmokeDriver &d;
    AiController *ai;
    NoticeCenter *notices;
    int port;
    QByteArray token;

    QJsonObject result(const std::unique_ptr<Call> &c) const {
        d.waitFor(20000, [&] { return c->done; });
        return c->result();
    }
    std::unique_ptr<Call> call(const char *tool, const QJsonObject &args = {}) const {
        return std::make_unique<Call>(port, token, tool, args);
    }
    bool ok(const char *tool, const QJsonObject &args = {}) const {
        return !result(call(tool, args)).value(QLatin1String("isError")).toBool(true);
    }
    QQuickItem *item(const QString &name) const { return d.item(name); }
    int lit(const QString &name) const {
        QQuickItem *i = d.item(name);
        return i ? i->property("times").toInt() : -1;
    }
    QString strip() const {
        QQuickItem *b = d.item(QStringLiteral("AiWorkBanner"));
        return b && b->isVisible() ? textOf(d.item(QStringLiteral("AiWorkBanner.text"))) : QString();
    }
};

QJsonObject gain(int output, double db, const QString &device = QString()) {
    QJsonObject o{ { QStringLiteral("output"), output }, { QStringLiteral("gain_db"), db } };
    if (!device.isEmpty()) o.insert(QStringLiteral("device"), device);
    return o;
}

// What both layouts show of a change on the device shown
void changes(Scene &s, const QString &layout) {
    SmokeDriver &d = s.d;
    const auto say = [&](const char *what) { return QStringLiteral("aivis: ") + layout + QStringLiteral(": ") + QLatin1String(what); };
    DeviceModel *device = d.app().device;
    const bool modern = layout == QLatin1String("modern");
    // Outputs of its own: what the user undid is theirs
    const int one = modern ? 3 : 0, two = modern ? 5 : 2;
    const auto gainIs = [&](double db) { return device->state() && std::abs(double(device->state()->output_gain_db[one]) - db) < 0.01; };
    const QString volumeMark = modern ? QStringLiteral("Dock.volume.touched") : QStringLiteral("GlobalControls.volume.touched");

    s.notices->clear();
    const int before = s.lit(volumeMark);
    const int section = s.lit(QStringLiteral("NavRail.equaliser.touched"));
    d.check(say("the agent sets a gain"), s.ok("dspi_set_gain", gain(one, modern ? -21 : -23)));
    d.check(say("the value on the screen follows"), d.waitFor(1000, [&] { return gainIs(modern ? -21 : -23); }));
    d.check(say("the strip is there, with the agent, what it does and to what"), d.waitFor(2000, [&] {
        return s.strip().contains(QLatin1String("gain"), Qt::CaseInsensitive) && s.strip().contains(QStringLiteral("Out %1").arg(one + 1));
    }) && textOf(s.item(QStringLiteral("AiWorkBanner.agent"))) == QLatin1String("Smoke agent"));
    d.check(say("it counts as working a moment after the call"), s.ai->working());
    QQuickItem *mark = s.item(QStringLiteral("AiWorkBanner.mark"));
    d.check(say("its mark turns"), mark && mark->property("working").toBool());
    QQuickItem *sign = s.item(modern ? QStringLiteral("AiButton.working") : QStringLiteral("AiStatusIcon.working"));
    d.check(say(modern ? "the AI button turns" : "the AI icon turns"), sign && sign->isVisible() && sign->property("working").toBool());
    if (modern)
        d.check(say("and says working"), textOf(s.item(QStringLiteral("AiButton.workingText"))) == QLatin1String("working")
                && s.item(QStringLiteral("AiButton.workingText"))->isVisible());
    d.check(say("a notice names the agent and the change"), d.waitFor(2000, [&] {
        const QString t = s.notices->current().value(QStringLiteral("text")).toString();
        return t.contains(QLatin1String("Smoke agent")) && t.contains(QLatin1String("gain")) && t.contains(QLatin1String("dB"));
    }));
    if (modern) d.check(say("the section the change is in lights up"), d.waitFor(1000, [&] { return s.lit(QStringLiteral("NavRail.equaliser.touched")) > section; }));
    else d.check(say("the channel's row lights up"), d.waitFor(1000, [&] { return s.ai->recentTouches().size() > 0; }));
    d.grab(d.mainWindow(), QStringLiteral("aivis-%1.png").arg(layout));
    QQuickItem *undo = s.item(QStringLiteral("NoticeBanner.action"));
    d.check(say("with Undo"), undo && undo->isVisible() && textOf(undo) == QLatin1String("Undo"));
    d.check(say("Undo puts it back"), click(undo) && d.waitFor(3000, [&] { return !gainIs(modern ? -21 : -23); }));

    d.check(say("the agent sets the volume"), s.ok("dspi_set_volume", { { QStringLiteral("volume"), QStringLiteral("user") }, { QStringLiteral("db"), modern ? -41 : -43 } }));
    d.check(say("the volume lights up"), d.waitFor(1500, [&] { return s.lit(volumeMark) > before; }));

    // Several changes by one agent are one notice
    s.notices->clear();
    for (int i = 0; i < 3; ++i) s.ok("dspi_set_gain", gain(two, (modern ? -30 : -33) - i));
    d.check(say("a run of changes is one notice"), d.waitFor(2000, [&] {
        return s.notices->current().value(QStringLiteral("text")).toString().contains(QLatin1String("3 changes")) && s.notices->waiting() == 0;
    }));
    d.check(say("whose Undo takes them all back"), [&] {
        const QString was = s.notices->current().value(QStringLiteral("text")).toString();
        s.notices->act();
        return !was.isEmpty() && d.waitFor(4000, [&] { return device->state() && device->state()->output_gain_db[two] > -29.5f; });
    }());
    s.notices->clear();
}

void aivisSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    auto *ai = Registrar::instance<AiController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    auto *snapshots = Registrar::instance<SnapshotsController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *tray = Registrar::instance<TrayController>();
    auto *layout = Registrar::instance<LayoutController>();
    auto *registry = DeviceRegistry::instance();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    d.check("aivis: the controllers exist", ai && notices && snapshots && mc && tray && layout && registry);
    if (!ai || !notices || !snapshots || !mc || !tray || !layout || !registry) return;

    const auto user = [&](const char *what, std::function<bool(FfiCore *)> job) {
        device->write(QLatin1String(what), std::move(job));
        d.waitIdle();
    };
    const auto userGain = [&](int output, float db) {
        user("aivis gain", [output, db](FfiCore *c) { return dspi_set_output_gain(c, uint8_t(output), db); });
    };

    // ---- On, with one agent, never asked
    const QString mode = ai->confirmMode();
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("aivis-store")));
    QDir().mkpath(QDir(d.outDir()).filePath(QStringLiteral("aivis-store")));
    Scene s{ d, ai, notices, freePort(), {} };
    ai->setPort(s.port);
    ai->setConfirmMode(QStringLiteral("never_ask"));
    ai->setEnabled(true);
    d.check("aivis: the server runs", d.waitFor(5000, [&] { return ai->running() && !ai->busy(); }));
    ai->addClient(QStringLiteral("Smoke agent"));
    d.waitFor(3000, [&] { return !ai->newClient().isEmpty(); });
    s.token = ai->newClient().value(QStringLiteral("token")).toString().toUtf8();
    ai->clearNewClient();
    d.check("aivis: nothing is said while no agent works", !ai->working() && ai->work().isEmpty() && s.strip().isEmpty());
    for (int o = 0; o < 6; ++o) userGain(o, -20);

    // ---- The tray
    tray->setAvailabilityProbe([] { return true; });
    const bool trayWas = tray->showInTray();
    tray->setShowInTray(true);
    d.check("aivis: the tray has an icon", tray->icon() != nullptr);
    const QString tipBefore = tray->icon() ? tray->icon()->toolTip() : QString();

    // ---- Classic, then Modern
    changes(s, QStringLiteral("classic"));
    d.check("aivis: the tray's tooltip says what the agent does", tray->icon() && tray->icon()->toolTip() != tipBefore
            && tray->icon()->toolTip().contains(QLatin1String("Smoke agent")));
    QAction *row = tray->contextMenu() ? tray->contextMenu()->findChild<QAction *>(QStringLiteral("Tray.ai")) : nullptr;
    QAction *trayPause = tray->contextMenu() ? tray->contextMenu()->findChild<QAction *>(QStringLiteral("Tray.aiPause")) : nullptr;
    d.check("aivis: and its menu, with Pause", row && row->isVisible() && row->text().contains(QLatin1String("Smoke agent"))
            && trayPause && trayPause->isVisible());
    d.check("aivis: hidden in the tray the icon still says it", [&] {
        tray->minimise();
        d.spin(200);
        const bool hidden = tray->windowHidden();
        s.ok("dspi_set_gain", gain(1, -25));
        const bool said = d.waitFor(2000, [&] { return tray->icon() && tray->icon()->toolTip().contains(QLatin1String("Out 2")); });
        tray->restore();
        d.spin(200);
        return hidden && said;
    }());
    layout->setLayout(QStringLiteral("modern"));
    d.check("aivis: in Modern", d.waitFor(5000, [&] { return shell->pageHost() != nullptr; }));
    d.spin(300);
    changes(s, QStringLiteral("modern"));
    const auto go = [&](const char *section, const char *tab) {
        QVariant ok;
        QMetaObject::invokeMethod(shell->pageHost(), "go", Q_RETURN_ARG(QVariant, ok), Q_ARG(QVariant, QString::fromLatin1(section)),
                                  Q_ARG(QVariant, QString::fromLatin1(tab)));
        d.waitIdle();
        d.spin(200);
    };

    // A control on the page shown, and one on a page opened just after the change
    go("routing", "");
    d.check("aivis: a route an agent changes lights up in the matrix", [&] {
        const QString cell = QStringLiteral("Matrix.cell.0.1.touched");
        const int was = s.lit(cell);
        s.ok("dspi_set_route", { { QStringLiteral("input"), 0 }, { QStringLiteral("output"), 1 }, { QStringLiteral("on"), true },
                                 { QStringLiteral("gain_db"), -31 } });
        return was >= 0 && d.waitFor(2000, [&] { return s.lit(cell) > was; });
    }());
    go("overview", "");

    // ---- What no snapshot of the device holds
    const auto listed = [&](const QString &name) {
        for (const QVariant &v : snapshots->snapshots())
            if (v.toMap().value(QStringLiteral("name")) == name) return true;
        return false;
    };
    d.check("aivis: a snapshot the agent takes is in the list at once", s.ok("dspi_snapshot_capture", { { QStringLiteral("name"), QStringLiteral("Aivis A") } })
            && d.waitFor(3000, [&] { return listed(QStringLiteral("Aivis A")); }));
    d.check("aivis: a compare the agent starts is on the screen", [&] {
        const QJsonObject r = s.result(s.call("dspi_list_snapshots"));
        QString id;
        for (const QJsonValue &v : r.value(QLatin1String("structuredContent")).toObject().value(QLatin1String("snapshots")).toArray())
            id = v.toObject().value(QLatin1String("id")).toString();
        s.ok("dspi_ab_start", { { QStringLiteral("a_id"), id }, { QStringLiteral("b_id"), id } });
        return d.waitFor(3000, [&] { return snapshots->abActive(); });
    }());
    d.check("aivis: and ends on it", s.ok("dspi_ab_keep", { { QStringLiteral("side"), QStringLiteral("a") } })
            && d.waitFor(3000, [&] { return !snapshots->abActive(); }));

    // ---- A read that goes on, and one held up: the strip says so while it runs
    d.check("aivis: reading keeps the strip there", s.ok("dspi_get_overview") && d.waitFor(2000, [&] { return s.strip().contains(QLatin1String("Reading")); }));

    // ---- Another device
    DeviceSession *a = registry->active();
    registry->openDemo();
    d.check("aivis: a second demo", d.waitFor(10000, [&] { return registry->opening() == 0 && registry->sessions().size() == 2; }));
    DeviceSession *b = registry->sessions().value(1);
    if (a && b) {
        registry->activate(a);
        d.waitIdle();
        d.spin(300);
        const QString tab = QStringLiteral("DeviceRow.touched.") + b->id();
        const int was = s.lit(tab);
        d.check("aivis: a change on a device that is not shown", s.ok("dspi_set_gain", gain(0, -27, b->id())));
        d.check("aivis: is in its model at once", d.waitFor(1000, [&] {
            return b->model()->state() && std::abs(double(b->model()->state()->output_gain_db[0]) + 27.0) < 0.01;
        }));
        d.check("aivis: the strip names that device, with Show", d.waitFor(2000, [&] { return s.strip().contains(b->title()); })
                && s.item(QStringLiteral("AiWorkBanner.show"))->isVisible());
        d.check("aivis: its tab lights up", was >= 0 && d.waitFor(1500, [&] { return s.lit(tab) > was; }));
        d.check("aivis: the notice names it too", notices->current().value(QStringLiteral("text")).toString().contains(b->title()));
        d.grab(d.mainWindow(), QStringLiteral("aivis-other-device.png"));
        d.check("aivis: a timed preview there is marked", [&] {
            QJsonObject args{ { QStringLiteral("device"), b->id() }, { QStringLiteral("guard_s"), 300 },
                              { QStringLiteral("changes"), QJsonArray{ QJsonObject{ { QStringLiteral("op"), QStringLiteral("set_gain") },
                                                                                    { QStringLiteral("output"), 1 },
                                                                                    { QStringLiteral("gain_db"), -28 } } } } };
            return s.ok("dspi_apply_changes", args) && d.waitFor(3000, [&] { return snapshots->guardsElsewhere().contains(b->title()); });
        }());
        d.check("aivis: Show shows it", click(s.item(QStringLiteral("AiWorkBanner.show"))) && d.waitFor(3000, [&] { return registry->active() == b; }));
        d.check("aivis: and its countdown", d.waitFor(3000, [&] { return snapshots->guardWaiting(); }));
        s.ok("dspi_revert_changes", { { QStringLiteral("device"), b->id() } });
        d.check("aivis: put back", d.waitFor(3000, [&] { return !snapshots->guardWaiting(); }));
        registry->activate(a);
        d.waitIdle();
        const QJsonObject told = s.result(s.call("dspi_get_user_activity")).value(QLatin1String("structuredContent")).toObject();
        d.check("aivis: the agent hears of the switch", QJsonDocument(told).toJson().contains("device_shown"));
        registry->close(b->id());
        d.waitFor(10000, [&] { return registry->sessions().size() == 1; });
        const QJsonObject r = s.result(s.call("dspi_list_devices"));
        d.check("aivis: and of the close, with its next call", d.waitFor(1, [&] {
            return QJsonDocument(r).toJson().contains("device_closed");
        }));
    }

    // ---- The agent is told what the user does, and gives way
    userGain(0, -35);
    QJsonObject r = s.result(s.call("dspi_list_devices"));
    const QString said = r.value(QLatin1String("content")).toArray().at(1).toObject().value(QLatin1String("text")).toString();
    d.check("aivis: the agent is told what the user changed", said.contains(QLatin1String("the user")) && said.contains(QLatin1String("-35 dB")));
    s.ok("dspi_set_gain", gain(1, -36));
    userGain(1, -45);
    r = s.result(s.call("dspi_set_gain", gain(1, -37)));
    d.check("aivis: a control the user took back is the user's",
            r.value(QLatin1String("isError")).toBool() && QJsonDocument(r).toJson().contains("the user changed this")
                && device->state() && std::abs(double(device->state()->output_gain_db[1]) + 45.0) < 0.01);

    // ---- Pause from the strip, Resume from the tray
    s.ok("dspi_get_overview");
    d.waitFor(2000, [&] { return !s.strip().isEmpty(); });
    d.check("aivis: Pause on the strip", click(s.item(QStringLiteral("AiWorkBanner.pause"))) && ai->paused() && !ai->working());
    r = s.result(s.call("dspi_set_gain", gain(2, -38)));
    d.check("aivis: the agent hears paused, not an error",
            !r.value(QLatin1String("isError")).toBool(true)
                && r.value(QLatin1String("structuredContent")).toObject().value(QLatin1String("status")).toString() == QLatin1String("paused"));
    d.check("aivis: the strip stays while they are paused", d.waitFor(2000, [&] { return s.strip().contains(QLatin1String("paused")); })
            && textOf(s.item(QStringLiteral("AiWorkBanner.pause"))) == QLatin1String("Resume"));
    d.grab(d.mainWindow(), QStringLiteral("aivis-paused.png"));
    if (trayPause) trayPause->trigger();
    d.check("aivis: Resume from the tray", !ai->paused());

    // ---- A question: the strip says it waits, in both words and mark
    ai->setConfirmMode(QStringLiteral("ask_everything"));
    auto held = s.call("dspi_set_gain", gain(2, -39));
    d.check("aivis: a held call is said to wait for the answer", d.waitFor(5000, [&] { return s.strip().contains(QLatin1String("Waiting for your answer")); }) && ai->working());
    if (!ai->confirmations().isEmpty()) ai->answer(ai->confirmations().first().toMap().value(QStringLiteral("id")).toDouble(), false);
    s.result(held);
    ai->setConfirmMode(QStringLiteral("never_ask"));
    d.check("aivis: the decline is in what the agent is told next",
            QJsonDocument(s.result(s.call("dspi_get_user_activity"))).toJson().contains("declined your change"));

    // ---- A measurement: the strip while it plays, and the user's change stops it
    user("aivis volume", [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
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
    for (int o = 0; o < 3; ++o) userGain(o, -10);
    QJsonObject args{ { QStringLiteral("outputs"), QJsonArray{ 0 } }, { QStringLiteral("sweep_ms"), 5000 }, { QStringLiteral("passes"), 8 } };
    r = s.result(s.call("dspi_measure_start", args));
    d.check("aivis: an agent starts a measurement", !r.value(QLatin1String("isError")).toBool(true));
    d.check("aivis: it runs in the Measure view", d.waitFor(15000, [&] { return mc->measuring(); }));
    d.check("aivis: the strip says it measures, with Stop Measuring", d.waitFor(3000, [&] {
        QQuickItem *stop = s.item(QStringLiteral("AiWorkBanner.stopJob"));
        return stop && stop->isVisible() && s.strip().contains(QLatin1String("easur"));
    }) && textOf(s.item(QStringLiteral("AiWorkBanner.agent"))) == QLatin1String("Smoke agent"));
    d.spin(21000);   // longer than any call lingers
    d.check("aivis: and stays while it plays", mc->measuring() && s.strip().contains(QLatin1String("easur")) && ai->working());
    d.grab(d.mainWindow(), QStringLiteral("aivis-measuring.png"));
    notices->clear();
    userGain(0, -12);
    d.check("aivis: a setting the user changes stops it", d.waitFor(5000, [&] { return !mc->measuring(); }));
    d.check("aivis: a notice says why", d.waitFor(3000, [&] {
        return notices->last().value(QStringLiteral("text")).toString().contains(QLatin1String("would not be valid"));
    }));
    r = s.result(s.call("dspi_measure_status"));
    const QJsonObject request = r.value(QLatin1String("structuredContent")).toObject().value(QLatin1String("request")).toObject();
    d.check("aivis: and the agent is told it does not count", !request.value(QLatin1String("valid")).toBool(true)
            && request.value(QLatin1String("note")).toString().contains(QLatin1String("the user changed")));
    r = s.result(s.call("dspi_measure_start", args));
    d.waitFor(15000, [&] { return mc->measuring(); });
    d.check("aivis: Stop Measuring on the strip stops it", d.waitFor(3000, [&] { return s.item(QStringLiteral("AiWorkBanner.stopJob"))->isVisible(); })
            && click(s.item(QStringLiteral("AiWorkBanner.stopJob"))) && d.waitFor(5000, [&] { return !mc->measuring(); }));
    r = s.result(s.call("dspi_measure_status"));
    d.check("aivis: and the agent is told the user stopped it", QJsonDocument(r).toJson().contains("the user stopped it"));

    // ---- It fades, and Stop
    go("overview", "");
    s.ok("dspi_get_overview");
    d.waitFor(1000, [&] { return !s.strip().isEmpty(); });
    d.check("aivis: a while after the last call it no longer counts as working",
            d.waitFor(AiController::kWorkingMs + 3000, [&] { return !ai->working(); }) && s.strip().contains(QLatin1String("Last")));
    d.check("aivis: and then the strip goes", d.waitFor(AiController::kLingerMs, [&] { return s.strip().isEmpty(); }));
    s.ok("dspi_get_overview");
    d.waitFor(1000, [&] { return !s.strip().isEmpty(); });
    d.check("aivis: Stop on the strip turns agents off", click(s.item(QStringLiteral("AiWorkBanner.stop")))
            && d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); }) && d.waitFor(1000, [&] { return s.strip().isEmpty(); }));

    // As it was
    tray->setShowInTray(trayWas);
    tray->setAvailabilityProbe({});
    ai->setConfirmMode(mode);
    ai->setPort(AiController::kDefaultPort);
    ai->revoke(ai->clients().value(0).toMap().value(QStringLiteral("id")).toDouble());
    notices->clear();
    layout->setLayout(QStringLiteral("classic"));
    d.waitFor(5000, [&] { return shell->pageHost() == nullptr; });
    d.waitIdle();
    d.spin(200);
}

const SmokeDriver::Section aivis("aivis-agent", 984, &aivisSection);

} // namespace
