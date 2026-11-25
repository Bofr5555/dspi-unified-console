// -- WP SHOWFIX -- Following an agent (the 0.15.0 films' findings), with a fake
// agent over HTTP against the real server, in Modern with two demo devices:
// - finding 3: a change on a channel's EQ, crossover or delay shows that channel
//   on the Equaliser's tab of the change, a PEQ band selected;
// - finding 14: the app's first change by an agent is followed; an agent's
//   measurement asked for right after a key or click, once the input rests;
// the user who went elsewhere is left there.
#include "SmokeDriver.h"

#include "AiController.h"
#include "ChannelController.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "MeasureController.h"
#include "PeqEditorController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"

#include <QCoreApplication>
#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QTcpServer>
#include <QTcpSocket>
#include <memory>

extern "C" {
#include "dspi_core.h"
}

namespace {

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// One tool call of the fake agent's, read on this thread's event loop (as SmokeOdec's)
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

QJsonObject lr4(double hz) {
    return { { QStringLiteral("family"), QStringLiteral("linkwitz_riley") }, { QStringLiteral("order"), 4 }, { QStringLiteral("freq_hz"), hz } };
}

void showfixFollowSection(SmokeDriver &d) {
    d.allowSectionTime(120);
    auto *ai = Registrar::instance<AiController>();
    auto *sidebar = Registrar::instance<SidebarController>();
    auto *peq = Registrar::instance<PeqEditorController>();
    auto *channels = Registrar::instance<ChannelController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *registry = DeviceRegistry::instance();
    Shell *shell = d.app().shell;
    d.check("showfix: the controllers exist", ai && sidebar && peq && channels && registry);
    if (!ai || !sidebar || !peq || !channels || !registry) return;

    // The journal's places as the follow reads them
    const AiController::FollowAt band = AiController::followAtOf(QStringLiteral("eq.gain"), { 2, 3 });
    const AiController::FollowAt cross = AiController::followAtOf(QStringLiteral("eq.freq"), { 9, 21 });
    const AiController::FollowAt gain = AiController::followAtOf(QStringLiteral("out.gain"), { 4 });
    const AiController::FollowAt route = AiController::followAtOf(QStringLiteral("mix"), { 0, 1 });
    d.check("showfix 3: a PEQ band is its channel and band on the PEQ tab",
            band.section == QLatin1String("equaliser") && band.tab == QLatin1String("peq") && band.channel == 2 && band.band == 3);
    d.check("showfix 3: a crossover band is its channel on the Crossover tab",
            cross.tab == QLatin1String("crossover") && cross.channel == 9 && cross.band == -1);
    d.check("showfix 3: an output's gain is its output on Delay & Gain",
            gain.tab == QLatin1String("delay") && gain.output == 4 && gain.channel == -1);
    d.check("showfix 3: a route has no channel to show", route.section == QLatin1String("routing") && route.channel < 0 && route.output < 0);

    // ---- An agent, never asked
    const QString mode = ai->confirmMode();
    const bool followWas = ai->followAgent();
    const bool capOn = ai->agentSplCapOn();
    const QString store = QDir(d.outDir()).filePath(QStringLiteral("showfix-store"));
    QDir().mkpath(store);
    ai->setStoreDir(store);
    Scene s{ d, freePort(), {} };
    ai->setPort(s.port);
    ai->setConfirmMode(QStringLiteral("never_ask"));
    ai->setFollowAgent(false);
    ai->setAgentSplCapOn(false);   // the simulated microphone reads no dB SPL
    ai->setEnabled(true);
    d.check("showfix: the server runs", d.waitFor(5000, [&] { return ai->running() && !ai->busy(); }));
    ai->addClient(QStringLiteral("Showfix agent"));
    d.waitFor(3000, [&] { return !ai->newClient().isEmpty(); });
    s.token = ai->newClient().value(QStringLiteral("token")).toString().toUtf8();
    ai->clearNewClient();

    d.check("showfix: in Modern", d.switchLayout("modern") && shell->pageHost() != nullptr);
    DeviceSession *a = registry->active();
    registry->openDemo();
    d.check("showfix: a second demo", d.waitFor(10000, [&] { return registry->opening() == 0 && registry->sessions().size() == 2; }));
    DeviceSession *b = registry->sessions().value(1);
    if (a && b && b != a && shell->pageHost()) {
        const auto section = [&] { return shell->pageHost()->property("section").toString(); };
        const auto tab = [&] { return shell->pageHost()->property("tab").toString(); };
        // The demo's channels on b, read while it is shown
        registry->activate(b);
        d.waitIdle();
        d.waitFor(3000, [&] { return channels->numInputs() > 2; });
        const int out0 = channels->outputChannel(0);
        const int out1 = channels->outputChannel(1);
        registry->activate(a);
        d.goTo(QStringLiteral("overview/overview"));
        d.spin(AiController::kFollowMs + 300);
        ai->setFollowAgent(true);
        // Finding 14: the agent's first call is this change; in a run of this section
        // alone it is the app's first read of the activity, followed too

        // ---- Finding 3: a PEQ band on input C of the other device: that device, the
        // Equaliser's PEQ tab, input C, the band selected
        const bool peqSet = s.ok("dspi_set_peq_band", { { QStringLiteral("device"), b->id() }, { QStringLiteral("input"), 2 },
                                                        { QStringLiteral("slot"), 3 }, { QStringLiteral("type"), QStringLiteral("peaking") },
                                                        { QStringLiteral("freq_hz"), 2000 }, { QStringLiteral("q"), 1 },
                                                        { QStringLiteral("gain_db"), -3 } });
        d.check("showfix 3: the agent's PEQ band is set", peqSet);
        d.check("showfix 3: its device comes to the front", d.waitFor(3000, [&] { return registry->active() == b; }));
        d.check("showfix 3: on the Equaliser's PEQ tab", d.waitFor(2000, [&] {
            return section() == QLatin1String("equaliser") && tab() == QLatin1String("peq");
        }));
        d.check("showfix 3: showing input C, not the channel selected before",
                d.waitFor(3000, [&] { return sidebar->selectedChannel() == 2; }));
        d.check("showfix 3: with the band selected", d.waitFor(3000, [&] {
            return peq->editedChannel() == 2 && peq->selected() == QList<int>{ 3 };
        }));
        QObject *page = d.object(QStringLiteral("EqualiserPeq"));
        d.check("showfix 3: the page's band is the agent's", page && d.waitFor(2000, [&] { return page->property("band").toInt() == 3; }));

        // A crossover on an output of the device shown: the Crossover tab, that output
        const bool xoSet = s.ok("dspi_set_crossover", { { QStringLiteral("device"), b->id() }, { QStringLiteral("output"), 0 },
                                                        { QStringLiteral("low_pass"), lr4(2000) } });
        d.check("showfix 3: the agent's crossover is set", xoSet);
        d.check("showfix 3: the Crossover tab shows that output (after the rate limit)",
                d.waitFor(AiController::kFollowMs + 3000, [&] {
                    return tab() == QLatin1String("crossover") && sidebar->selectedChannel() == out0;
                }));

        // A delay: Delay & Gain, that output
        const bool delaySet = s.ok("dspi_set_delay", { { QStringLiteral("device"), b->id() }, { QStringLiteral("output"), 1 },
                                                       { QStringLiteral("delay_ms"), 2.0 } });
        d.check("showfix 3: the agent's delay is set", delaySet);
        d.check("showfix 3: Delay & Gain shows that output",
                d.waitFor(AiController::kFollowMs + 3000, [&] {
                    return tab() == QLatin1String("delay") && sidebar->selectedChannel() == out1;
                }));

        // The user went elsewhere: the next change on the same page leaves them there
        registry->activate(a);
        d.goTo(QStringLiteral("overview/overview"));
        d.spin(AiController::kFollowMs + 300);
        s.ok("dspi_set_peq_band", { { QStringLiteral("device"), b->id() }, { QStringLiteral("input"), 2 },
                                    { QStringLiteral("slot"), 4 }, { QStringLiteral("type"), QStringLiteral("peaking") },
                                    { QStringLiteral("freq_hz"), 3000 }, { QStringLiteral("q"), 1 }, { QStringLiteral("gain_db"), -2 } });
        d.spin(800);
        d.check("showfix 3: the user who left is left where they went", registry->active() == a && section() == QLatin1String("overview"));

        // ---- Finding 14: an agent's measurement on the other device, asked for right
        // after a key (as right after a click on the question's Allow): followed once
        // the keyboard rests, to that device's Measure
        if (mc) {
            registry->activate(b);
            d.waitIdle();
            d.app().device->write(QLatin1String("showfix volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
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
            registry->activate(a);
            d.goTo(QStringLiteral("overview/overview"));
            d.spin(AiController::kFollowMs + 300);
            QObject sink;
            QKeyEvent key(QEvent::KeyPress, Qt::Key_Right, Qt::NoModifier);
            QCoreApplication::sendEvent(&sink, &key);
            auto run = s.call("dspi_measure_start", { { QStringLiteral("device"), b->id() }, { QStringLiteral("outputs"), QJsonArray{ 0 } },
                                                      { QStringLiteral("sweep_ms"), 5000 }, { QStringLiteral("passes"), 8 } });
            d.check("showfix 14: the measurement starts", !s.result(run).value(QLatin1String("isError")).toBool(true));
            d.check("showfix 14: its device and Measure come up once the keyboard rests",
                    d.waitFor(AiController::kInputIdleMs + 4000, [&] {
                        return registry->active() == b && section() == QLatin1String("measure");
                    }));
            s.ok("dspi_measure_stop", { { QStringLiteral("device"), b->id() } });
            d.waitFor(10000, [&] { return !mc->measuring(); });
        }

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

const SmokeDriver::Section showfixFollow("showfix-follow", 986, &showfixFollowSection);

} // namespace
