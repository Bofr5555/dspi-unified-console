// -- WP UAG -- (UXP A6, A7, A9, D2) The report's pictures: "uag-pictures" walks
// what a person sees of an agent, in Modern at 1600×1000 on the demo, and grabs
// each in the dark and the light scheme into <out>/uag/:
// - the AI Agents page with no agent paired (Connect an Agent's place), and the
//   pairing sheet;
// - a question about a call of two changes, one asked (an output named by the
//   speaker setup), a flash write (a preset with a name), and two questions
//   raised together;
// - a refusal (a tweeter's high-pass taken away under "never ask") in the strip.
// It reads and drives the app through properties and the agents' endpoint only,
// so the same walk runs on a tree without this package (the "before" pictures).
// The checks of the behaviour are in mcp4-ai, adet-agent and the dspi-mcp tests.
#include "SmokeDriver.h"

#include "AiController.h"
#include "DeviceModel.h"
#include "LayoutController.h"   // -- FIX016 --
#include "Registrar.h"
#include "Shell.h"

#include "dspi_core.h"

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
#include <utility>

namespace {

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// One request, read on this thread's event loop
struct Request {
    QTcpSocket socket;
    QByteArray in;
    bool done = false;
    Request(int port, const QByteArray &path, const QByteArray &headers, const QJsonObject &body) {
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
    QJsonObject body() const { return QJsonDocument::fromJson(in.mid(in.indexOf("\r\n\r\n") + 4)).object(); }
};

std::unique_ptr<Request> call(int port, const QByteArray &token, const char *tool, const QJsonObject &args) {
    const QJsonObject meta{ { QStringLiteral("io.modelcontextprotocol/protocolVersion"), QStringLiteral("2026-07-28") },
                            { QStringLiteral("io.modelcontextprotocol/clientCapabilities"), QJsonObject() } };
    const QJsonObject body{ { QStringLiteral("jsonrpc"), QStringLiteral("2.0") },
                            { QStringLiteral("id"), 1 },
                            { QStringLiteral("method"), QStringLiteral("tools/call") },
                            { QStringLiteral("params"), QJsonObject{ { QStringLiteral("name"), QLatin1String(tool) },
                                                                     { QStringLiteral("arguments"), args },
                                                                     { QStringLiteral("_meta"), meta } } } };
    return std::make_unique<Request>(port, "/mcp",
                                     "Authorization: Bearer " + token + "\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: tools/call\r\nMcp-Name: " +
                                         QByteArray(tool) + "\r\n",
                                     body);
}

bool opened(QObject *o) { return o && o->property("opened").toBool(); }

// The window in both schemes, as <out>/uag/<name>-<scheme>.png
void grabBoth(SmokeDriver &d, QQuickWindow *w, const QString &name) {
    Shell *shell = d.app().shell;
    const Qt::ColorScheme was = shell->colorSchemeOverride();
    for (const auto &[scheme, word] : { std::pair{ Qt::ColorScheme::Dark, "dark" }, std::pair{ Qt::ColorScheme::Light, "light" } }) {
        shell->setColorSchemeOverride(scheme);
        d.spin(300);
        d.grab(w, QStringLiteral("uag/%1-%2.png").arg(name, QLatin1String(word)));
    }
    shell->setColorSchemeOverride(was);
    d.spin(150);
}

void picturesSection(SmokeDriver &d) {
    auto *ai = Registrar::instance<AiController>();
    Shell *shell = d.app().shell;
    DeviceModel *device = d.app().device;
    QQuickWindow *main = d.mainWindow();
    if (!ai || !shell || !device || !main) {
        d.check("uag-pictures: the controllers exist", false);
        return;
    }
    QDir(QDir(d.outDir()).filePath(QStringLiteral("uag"))).removeRecursively();
    QDir().mkpath(QDir(d.outDir()).filePath(QStringLiteral("uag")));
    const bool modernWas = Registrar::instance<LayoutController>()->modern();   // -- FIX016 -- (R1-QT)
    d.switchLayout("modern");
    const QSize size = main->size();
    main->resize(1600, 1000);
    d.spin(300);
    const auto write = [&](const QString &what, std::function<bool(FfiCore *)> job) {
        device->write(what, std::move(job));
        d.waitIdle();
    };
    // Out 2 the user's "Right woofer", Out 4 their "Sub", Out 3 a 2 kHz high-pass (a tweeter's)
    {
        QJsonArray outputs;
        for (int o = 0; o < 9; ++o)
            outputs.append(QJsonObject{ { QStringLiteral("output"), o },
                                        { QStringLiteral("role"), o == 1 ? QStringLiteral("low") : o == 3 ? QStringLiteral("sub") : QStringLiteral("full") },
                                        { QStringLiteral("speaker"), o == 1 ? QStringLiteral("R") : o == 3 ? QStringLiteral("Sub") : QStringLiteral("Spk%1").arg(o + 1) },
                                        { QStringLiteral("group"), 0 } });
        const QByteArray json = QJsonDocument(QJsonObject{ { QStringLiteral("outputs"), outputs }, { QStringLiteral("confirmedByUser"), true } })
                                    .toJson(QJsonDocument::Compact);
        write(QStringLiteral("uag layout"), [json](FfiCore *c) {
            QByteArray buf(1 << 16, '\0');
            return dspi_speaker_layout_set(c, json.constData(), buf.data(), uint32_t(buf.size())) > 0;
        });
    }
    write(QStringLiteral("uag gains"), [](FfiCore *c) {
        return dspi_set_output_gain(c, 1, -5.0f) && dspi_set_output_gain(c, 3, 0.0f) && dspi_set_output_gain(c, 6, -20.0f)
               && dspi_set_output_gain(c, 7, -20.0f);
    });
    write(QStringLiteral("uag preset"), [](FfiCore *c) { return dspi_set_preset_name(c, 1, "Lunch"); });

    // ---- No agent yet: the page, then the pairing sheet
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("store")));
    const int port = freePort();
    ai->setPort(port);
    ai->setConfirmMode(QStringLiteral("ask_risky"));
    ai->setEnabled(true);
    d.waitFor(10000, [&] { return ai->running() && !ai->busy(); });
    ai->openSettings();
    auto *settings = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    if (settings) {
        settings->resize(1100, 900);
        d.waitFor(3000, [&] { return d.item(QStringLiteral("AiAgentsPage"), settings->contentItem()) != nullptr; });
        d.spin(500);
        grabBoth(d, settings, QStringLiteral("1-page-no-agent"));
        settings->close();
    }
    QObject *pairing = d.object(QStringLiteral("AiPairingSheet"));
    auto asked = std::make_unique<Request>(port, "/pair", QByteArray(), QJsonObject{ { QStringLiteral("name"), QStringLiteral("Claude Code") } });
    d.check("uag-pictures: the pairing sheet", d.waitFor(5000, [&] { return asked->done && opened(pairing); }));
    d.waitFor(3000, [&] { return pairing && pairing->property("armed").toBool(); });
    grabBoth(d, main, QStringLiteral("2-pairing"));
    ai->answerPairing(true);
    auto wait = std::make_unique<Request>(port, "/pair/wait", QByteArray(), QJsonObject{ { QStringLiteral("ticket"), asked->body().value(QLatin1String("ticket")) } });
    d.waitFor(10000, [&] { return wait->done; });
    const QByteArray token = wait->body().value(QLatin1String("token")).toString().toUtf8();
    d.check("uag-pictures: paired", !token.isEmpty());

    QObject *sheet = d.object(QStringLiteral("AiConfirmSheet"));
    const auto question = [&](const QString &name, const std::function<void()> &ask, int n) {
        ask();
        const bool shown = d.waitFor(5000, [&] { return int(ai->confirmations().size()) >= n && opened(sheet); });
        d.check(QStringLiteral("uag-pictures: %1 is asked").arg(name), shown);
        d.waitFor(3000, [&] { return sheet && sheet->property("armed").toBool(); });
        d.spin(400);
        grabBoth(d, main, name);
        for (const QVariant &q : ai->confirmations()) ai->answer(q.toMap().value(QStringLiteral("id")).toDouble(), false);
        d.waitFor(5000, [&] { return ai->confirmations().isEmpty(); });
        d.spin(300);
    };
    std::vector<std::unique_ptr<Request>> calls;
    const auto send = [&](const char *tool, const QJsonObject &args) { calls.push_back(call(port, token, tool, args)); };

    // ---- A call of two changes: one louder (Right woofer), one quieter (Sub)
    question(QStringLiteral("3-question-batch"), [&] {
        const auto change = [](int output, double db) {
            return QJsonObject{ { QStringLiteral("op"), QStringLiteral("set_gain") }, { QStringLiteral("output"), output }, { QStringLiteral("gain_db"), db } };
        };
        send("dspi_apply_changes", { { QStringLiteral("changes"), QJsonArray{ change(2, -2.0), change(4, -6.0) } } });
    }, 1);
    // ---- A flash write: the startup preset, preset 2 "Lunch"
    question(QStringLiteral("4-question-flash"), [&] {
        send("dspi_preset_set_startup", { { QStringLiteral("mode"), QStringLiteral("slot") }, { QStringLiteral("slot"), 2 } });
    }, 1);
    // ---- Two questions raised together
    question(QStringLiteral("5-question-together"), [&] {
        send("dspi_set_gain", { { QStringLiteral("output"), 7 }, { QStringLiteral("gain_db"), -10.0 } });
        send("dspi_set_gain", { { QStringLiteral("output"), 8 }, { QStringLiteral("gain_db"), -10.0 } });
    }, 2);

    // ---- A refusal: a tweeter's high-pass taken away under "never ask"
    ai->setConfirmMode(QStringLiteral("never_ask"));
    const QJsonObject hp{ { QStringLiteral("family"), QStringLiteral("linkwitz_riley") }, { QStringLiteral("order"), 4 }, { QStringLiteral("freq_hz"), 2000 } };
    auto set = call(port, token, "dspi_set_crossover", { { QStringLiteral("output"), 3 }, { QStringLiteral("high_pass"), hp } });
    d.waitFor(5000, [&] { return set->done; });
    d.spin(500);
    auto refused = call(port, token, "dspi_set_crossover", { { QStringLiteral("output"), 3 }, { QStringLiteral("high_pass"), QJsonValue(QJsonValue::Null) } });
    d.waitFor(5000, [&] { return refused->done; });
    d.spin(600);
    grabBoth(d, main, QStringLiteral("6-refusal-strip"));

    // As it was
    for (auto &c : calls) d.waitFor(3000, [&] { return c->done; });
    ai->setConfirmMode(QStringLiteral("ask_risky"));
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setPort(AiController::kDefaultPort);
    for (const QVariant &c : ai->clients()) ai->revoke(c.toMap().value(QStringLiteral("id")).toDouble());
    main->resize(size);
    if (!modernWas) d.switchLayout("classic");   // -- FIX016 -- (R1-QT) the layout as it found it
    d.waitIdle();
}

const SmokeDriver::Section pictures("uag-pictures", 999, &picturesSection);

} // namespace
