// -- WP MCP5 -- An AI agent's measurement through the app's Measure view, with a
// fake agent over HTTP (the real server, on loopback) and the demo DSPi's
// simulated microphone: the request opens the Measure view, runs there as the
// view's own run said as "Requested by AI · name", ends with its result shown,
// and the agent reads that it ended; an unconfirmed speaker layout is refused
// before anything reaches the app; the "imported" banner no longer claims a
// configuration file (review RB C4).
// -- WP INT5 -- and in Modern, on a device that is not the one shown and whose
// measurement thread sleeps (LAY5b).
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
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"
#include "SpeakersController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTcpServer>
#include <QTcpSocket>
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

QByteArray fill(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(1 << 16, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

// A core call on the active device's worker, waited for
QJsonObject onWorker(SmokeDriver &d, std::function<QByteArray(FfiCore *)> job) {
    QByteArray out;
    bool done = false;
    QObject ctx;
    d.app().core->worker()->call(&ctx, std::move(job), [&](const QByteArray &r) {
        out = r;
        done = true;
    });
    d.waitFor(5000, [&] { return done; });
    return QJsonDocument::fromJson(out).object();
}

// The demo's speaker layout: output 1 a woofer, confirmed or not
QJsonObject setLayout(SmokeDriver &d, bool confirmed) {
    const QJsonObject layout{ { QStringLiteral("outputs"),
                                QJsonArray{ QJsonObject{ { QStringLiteral("output"), 0 },
                                                         { QStringLiteral("role"), QStringLiteral("low") },
                                                         { QStringLiteral("speaker"), QStringLiteral("L") },
                                                         { QStringLiteral("group"), 0 } } } },
                              { QStringLiteral("confirmedByUser"), confirmed } };
    const QByteArray text = QJsonDocument(layout).toJson(QJsonDocument::Compact);
    return onWorker(d, [text](FfiCore *c) {
        return fill([c, &text](char *b, uint32_t l) { return dspi_speaker_layout_set(c, text.constData(), b, l); });
    });
}

// -- WP INT5 -- Modern, its Measure page not shown, the request for a second
// device that is not shown either and was never measured on
void otherDevice(SmokeDriver &d, MeasureController *mc, const std::function<QJsonObject(const char *, const QJsonObject &)> &call,
                 QJsonObject args) {
    auto *layout = Registrar::instance<LayoutController>();
    auto *registry = DeviceRegistry::instance();
    Shell *shell = d.app().shell;
    layout->setLayout(QStringLiteral("modern"));
    d.check("mcp5: in Modern", d.waitFor(5000, [&] { return shell->pageHost() != nullptr; }));
    if (!shell->pageHost()) return;
    d.spin(200);
    const auto go = [&](const char *section, const char *tab) {
        QVariant ok;
        QMetaObject::invokeMethod(shell->pageHost(), "go", Q_RETURN_ARG(QVariant, ok), Q_ARG(QVariant, QString::fromLatin1(section)),
                                  Q_ARG(QVariant, QString::fromLatin1(tab)));
        d.waitIdle();
        d.spin(100);
    };
    go("measure", "documents");
    go("overview", "");
    DeviceSession *a = registry->active();
    registry->openDemo();
    d.check("mcp5: a second demo", d.waitFor(10000, [&] { return registry->opening() == 0 && registry->sessions().size() == 2; }));
    DeviceSession *b = registry->sessions().value(1);
    if (!a || !b) return;
    registry->activate(b);
    d.waitIdle();
    b->model()->write(QStringLiteral("mcp5 volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
    setLayout(d, true);
    registry->activate(a);
    d.waitIdle();
    d.spin(200);
    MeasureSession *on = nullptr;
    for (MeasureSession *m : mc->sessions())
        if (m->session() == b) on = m;
    d.check("mcp5: nothing was measured on it", !on || !on->awake());

    args.insert(QStringLiteral("device"), b->id());
    QJsonObject r = call("dspi_measure_start", args);
    d.check("mcp5: a start on the other device is a request too", !r.value(QLatin1String("isError")).toBool(true));
    d.check("mcp5: it runs there, with its simulated microphone", d.waitFor(15000, [&] {
        for (MeasureSession *m : mc->sessions())
            if (m->session() == b) on = m;
        return on && on->measuring();
    }) && on->micChoice() == QLatin1String("sim"));
    if (!on) return;
    QObject *page = shell->pageHost();
    // -- WP LAY7 -- (INT5 follow-up 3) Measure was last on Documents, and A was shown
    d.check("mcp5: the asked device is shown, on Measure's measuring tab",
            registry->active() == b && mc->measuring() && page->property("pageKey").toString() == QLatin1String("measure/measure")
                && page->property("tab").toString() == QLatin1String("measure") && !d.object(QStringLiteral("MeasureWindow")));
    d.check("mcp5: it says who asked for it there", mc->runningText().startsWith(QString::fromUtf8("Requested by AI · Smoke agent")));
    call("dspi_measure_stop", { { QStringLiteral("device"), b->id() } });
    d.check("mcp5: stopped", d.waitFor(10000, [&] { return !on->running(); }));

    // -- WP LAY7 -- Something runs on the device shown: that stays shown
    registry->activate(a);
    d.waitIdle();
    const QJsonObject here = call("dspi_measure_start", QJsonObject{ { QStringLiteral("device"), a->id() }, { QStringLiteral("outputs"), QJsonArray{ 0 } },
                                                                     { QStringLiteral("sweep_ms"), 4000 }, { QStringLiteral("passes"), 8 } });
    d.check("mcp5: a run on the device shown", !here.value(QLatin1String("isError")).toBool(true) && d.waitFor(15000, [&] { return mc->measuring(); }));
    go("measure", "documents");
    go("overview", "");
    r = call("dspi_measure_start", args);
    // -- WP INTG -- (FFIX bug 6) the app has one microphone: an agent's start
    // beside a run on another device is refused, naming it
    d.check("mcp5: the agent's start on the other device waits for the one microphone",
            r.value(QLatin1String("isError")).toBool() &&
                r.value(QLatin1String("content")).toArray().first().toObject().value(QLatin1String("text")).toString().contains(
                    QLatin1String("one microphone")) &&
                !on->running() && registry->active() == a);
    call("dspi_measure_stop", { { QStringLiteral("device"), a->id() } });
    d.check("mcp5: the shown device's run stopped", d.waitFor(10000, [&] { return !mc->running(); }));

    // -- WP INTG -- The agent's run on the other device, then the person's own
    // on the device shown, beside it (the one-microphone rule is the agent's)
    QJsonObject longer = args;
    longer.insert(QStringLiteral("sweep_ms"), 4000);
    longer.insert(QStringLiteral("passes"), 8);
    r = call("dspi_measure_start", longer);
    d.check("mcp5: the other device's run", !r.value(QLatin1String("isError")).toBool(true) && d.waitFor(15000, [&] { return on->measuring(); }));
    registry->activate(a);
    d.waitIdle();
    go("measure", "measure");
    mc->setMicChoice(QStringLiteral("sim"));
    d.waitFor(5000, [&] { return mc->simAvailable() && mc->micChoice() == QLatin1String("sim") && !mc->setupOutputs().isEmpty(); });
    bool one = false;
    for (const QVariant &v : mc->setupOutputs()) {
        const QVariantMap m = v.toMap();
        mc->setOutputOn(m.value(QStringLiteral("output")).toInt(), !one);
        one = true;
    }
    mc->setPlacement(QStringLiteral("listeningPosition"));
    mc->setSweepMs(4000);
    mc->setPasses(8);
    mc->start();
    d.check("mcp5: the person's run on the device shown, beside it, on the measuring tab",
            d.waitFor(15000, [&] { return mc->measuring(); }) && on->measuring() && registry->active() == a &&
                page->property("tab").toString() == QLatin1String("measure"));
    d.check("mcp5: the device shown says what runs on the other, and who asked", d.waitFor(3000, [&] {
        const QVariantList others = mc->otherRuns();
        return others.size() == 1 && others.first().toMap().value(QStringLiteral("text")).toString().startsWith(QString::fromUtf8("Requested by AI · Smoke agent"));
    }));
    QQuickItem *row = d.item(QStringLiteral("Measure.otherRun"));
    d.check("mcp5: with its Stop on every part of the page", row && row->isVisible());
    d.grab(d.mainWindow(), QStringLiteral("mcp5-other-device.png"));
    go("measure", "tune");
    d.check("mcp5: it goes on over a tab", on->measuring() && row && row->isVisible());
    go("overview", "");
    d.check("mcp5: and over a section", on->measuring());
    shell->openWindow(QStringLiteral("tools.measure"));
    d.waitFor(3000, [&] { return shell->shownWindows().contains(QStringLiteral("tools.measure")); });
    shell->closeWindow(QStringLiteral("tools.measure"));
    d.spin(300);
    d.check("mcp5: the window's close stops the shown device's only", on->measuring() && d.waitFor(10000, [&] { return !mc->running(); }));
    const QJsonObject stopped = call("dspi_measure_stop", { { QStringLiteral("device"), b->id() } });
    d.check("mcp5: the agent's Stop stops it there",
            stopped.value(QLatin1String("structuredContent")).toObject().value(QLatin1String("stopped")).toArray().contains(
                QStringLiteral("measurement")) &&
                d.waitFor(10000, [&] { return !on->running(); }));

    registry->close(b->id());
    d.waitFor(10000, [&] { return registry->sessions().size() == 1; });
    go("overview", "");
    layout->setLayout(QStringLiteral("classic"));
    d.waitFor(5000, [&] { return shell->pageHost() == nullptr; });
    d.spin(200);
}

void measureSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    auto *ai = Registrar::instance<AiController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *sc = Registrar::instance<SpeakersController>();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    d.check("mcp5: the controllers exist", ai && mc && sc);
    if (!ai || !mc || !sc) return;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check("mcp5: the demo DSPi", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    const QJsonObject before = onWorker(d, [](FfiCore *c) { return fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); }); });
    device->write(QStringLiteral("mcp5 volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
    d.waitIdle();

    // Agents on, never asking (the start's question is dspi-mcp's, tested there)
    const int port = freePort();
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("mcp5-store")));
    QDir().mkpath(QDir(d.outDir()).filePath(QStringLiteral("mcp5-store")));
    const QString mode = ai->confirmMode();
    ai->setPort(port);
    ai->setConfirmMode(QStringLiteral("never_ask"));
    ai->setEnabled(true);
    d.check("mcp5: AI agents on", d.waitFor(5000, [&] { return ai->running() && !ai->busy(); }));
    ai->addClient(QStringLiteral("Smoke agent"));
    d.waitFor(3000, [&] { return !ai->newClient().isEmpty(); });
    const QByteArray token = ai->newClient().value(QStringLiteral("token")).toString().toUtf8();
    ai->clearNewClient();
    const auto call = [&](const char *tool, const QJsonObject &args) {
        Call c(port, token, tool, args);
        d.waitFor(10000, [&] { return c.done; });
        return c.result();
    };
    const QJsonObject outs{ { QStringLiteral("outputs"), QJsonArray{ 0 } },
                            { QStringLiteral("sweep_ms"), 1000 },
                            { QStringLiteral("passes"), 2 } };

    // An unconfirmed layout: refused, nothing reaches the app
    setLayout(d, false);
    QJsonObject r = call("dspi_measure_start", outs);
    d.check("mcp5: an unconfirmed speaker layout is refused, with where to confirm it",
            r.value(QLatin1String("isError")).toBool() &&
                r.value(QLatin1String("content")).toArray().first().toObject().value(QLatin1String("text")).toString().contains(
                    QString::fromUtf8("Settings › Speakers")) &&
                !mc->running());
    sc->reload();   // as a device switch or a Settings page does
    d.check("mcp5: the banner says the layout was set outside this app, not that a file brought it",
            d.waitFor(3000, [&] { return sc->status() == QLatin1String("imported"); }) &&
                sc->bannerText(true).contains(QLatin1String("set outside this app")) &&
                !sc->bannerText(true).contains(QLatin1String("came with a configuration file")));

    // -- WP INTG -- (review RE1) the agents' SPL cap on, and the simulated
    // microphone has no dB SPL calibration: nothing plays, and the agent is
    // told what the user can do. Off for the rest: the level ceiling holds.
    setLayout(d, true);
    const bool capOn = ai->agentSplCapOn();
    ai->setAgentSplCapOn(true);
    r = call("dspi_measure_start", outs);
    d.check("intg: the SPL cap without a dB SPL calibration: nothing plays, the agent is told what to do",
            !r.value(QLatin1String("isError")).toBool(true) && d.waitFor(10000, [&] {
                const QByteArray st = QJsonDocument(call("dspi_measure_status", {})).toJson(QJsonDocument::Compact);
                return st.contains("calibrates it in Measure") && st.contains("turns the cap off");
            }) && !mc->running());
    ai->setAgentSplCapOn(false);

    // Confirmed: the request runs in the Measure view
    r = call("dspi_measure_start", outs);
    d.check("mcp5: the agent's start is a request to the app",
            !r.value(QLatin1String("isError")).toBool(true) &&
                r.value(QLatin1String("structuredContent")).toObject().value(QLatin1String("status")).toString() ==
                    QLatin1String("waiting_for_app"));
    d.check("mcp5: the Measure view comes up and runs it",
            d.waitFor(10000, [&] { return mc->measuring(); }) && shell->shownWindows().contains(QStringLiteral("tools.measure")));
    d.check("mcp5: the simulated microphone is the Measure view's", mc->simAvailable() && mc->micChoice() == QLatin1String("sim"));
    d.check("mcp5: it says who asked for it", mc->runningText().startsWith(QString::fromUtf8("Requested by AI · Smoke agent")));
    if (auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("tools.measure"))))
        d.grab(w, QStringLiteral("mcp5-requested.png"));
    QJsonObject st = call("dspi_measure_status", {}).value(QLatin1String("structuredContent")).toObject();
    d.check("mcp5: the agent reads it running, as its own",
            st.value(QLatin1String("request")).toObject().value(QLatin1String("state")).toString() == QLatin1String("running") &&
                st.value(QLatin1String("measurement")).toObject().value(QLatin1String("by_this_agent")).toBool());
    d.check("mcp5: it ends with its result shown",
            d.waitFor(180000, [&] { return !mc->measuring() && !mc->running(); }) && d.waitFor(5000, [&] { return mc->hasResult(); }));
    d.check("mcp5: and nobody is named once it is over", mc->runningText().isEmpty());
    st = call("dspi_measure_status", {}).value(QLatin1String("structuredContent")).toObject();
    d.check("mcp5: the agent reads that it ended",
            st.value(QLatin1String("request")).toObject().value(QLatin1String("state")).toString() == QLatin1String("ended") &&
                st.value(QLatin1String("measurement")).toObject().value(QLatin1String("phase")).toString() == QLatin1String("done"));

    // The agent's Stop, while its next one runs
    r = call("dspi_measure_start", outs);
    d.waitFor(10000, [&] { return mc->measuring(); });
    r = call("dspi_measure_stop", {});
    d.check("mcp5: the agent's Stop stops its run",
            r.value(QLatin1String("structuredContent")).toObject().value(QLatin1String("stopped")).toArray().contains(
                QStringLiteral("measurement")) &&
                d.waitFor(10000, [&] { return !mc->running(); }));

    // -- WP RCFIX -- (review RC, RC4) a start whose request was withdrawn meanwhile
    // is stopped; one that could not start has nothing to stop
    int withdrawn = 0;
    ai->reportMeasure(999999, false, QString(), [&withdrawn] { withdrawn += 10; });
    ai->reportMeasure(999999, true, QString(), [&withdrawn] { ++withdrawn; });
    d.check("mcp5: the app hears that a started request was withdrawn, and stops it", d.waitFor(5000, [&] { return withdrawn == 1; }));

    shell->closeWindow(QStringLiteral("tools.measure"));
    otherDevice(d, mc, call, outs);

    // As it was
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setAgentSplCapOn(capOn);   // -- WP INTG --
    ai->setConfirmMode(mode);
    ai->setPort(AiController::kDefaultPort);
    for (const QVariant &c : ai->clients()) ai->revoke(c.toMap().value(QStringLiteral("id")).toDouble());
    // The layout it had (none: the confirmed one stays; this is the last section)
    if (before.value(QLatin1String("ok")).toBool() && !before.value(QLatin1String("outputs")).toArray().isEmpty()) {
        QJsonObject l = before;
        l.remove(QStringLiteral("ok"));
        const QByteArray text = QJsonDocument(l).toJson(QJsonDocument::Compact);
        onWorker(d, [text](FfiCore *c) { return fill([c, &text](char *b, uint32_t l) { return dspi_speaker_layout_set(c, text.constData(), b, l); }); });
    }
    d.waitIdle();
    d.spin(200);
}

const SmokeDriver::Section measure("mcp5-measure", 984, &measureSection);

} // namespace
