// -- WP UB5 -- (UXP B5, D2) One question for a change across devices; the agent says
// when its work is unsaved. "ub5-agent" walks it in Modern at 1600×1000 on three demo
// DSPis with a fake agent on the real MCP endpoint, checks it and grabs each state in
// the dark and the light scheme into <out>/ub5/:
// - night mode on three devices (loudness and a leveller with gain on each, the
//   limiters lowered), the calls sent together: one sheet, each device once with its
//   rows, one Allow for all of them;
// - a question raised while a sheet is up: not on it, its Allow does not cover it, it
//   is the next sheet;
// - once the agent rests, the strip's last line: what it left unsaved, with Save All…
//   (Save to Device with one device), which asks the save question.
// New things are read through properties and object names only, so the same walk
// runs on a tree without this package (the report's "before" pictures).
#include "SmokeDriver.h"

#include "AiController.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "SessionModel.h"
#include "Registrar.h"
#include "Shell.h"

#include "dspi_core.h"

#include <QDebug>
#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTcpServer>
#include <QTcpSocket>
#include <cmath>
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
    bool failed() const { return body().value(QLatin1String("result")).toObject().value(QLatin1String("isError")).toBool(true); }
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

bool click(QObject *item) {
    auto *i = qobject_cast<QQuickItem *>(item);
    return i && i->isVisible() && i->isEnabled() && QMetaObject::invokeMethod(i, "clicked");
}

QString textOf(QObject *o) { return o ? o->property("text").toString() : QString(); }

// The window in both schemes, as <out>/ub5/<name>-<scheme>.png
void grabBoth(SmokeDriver &d, QQuickWindow *w, const QString &name) {
    Shell *shell = d.app().shell;
    const Qt::ColorScheme was = shell->colorSchemeOverride();
    for (const auto &[scheme, word] : { std::pair{ Qt::ColorScheme::Dark, "dark" }, std::pair{ Qt::ColorScheme::Light, "light" } }) {
        shell->setColorSchemeOverride(scheme);
        d.spin(300);
        d.grab(w, QStringLiteral("ub5/%1-%2.png").arg(name, QLatin1String(word)));
    }
    shell->setColorSchemeOverride(was);
    d.spin(150);
}

void agentSection(SmokeDriver &d) {
    auto *ai = Registrar::instance<AiController>();
    auto *registry = DeviceRegistry::instance();
    Shell *shell = d.app().shell;
    DeviceModel *device = d.app().device;
    QQuickWindow *main = d.mainWindow();
    if (!ai || !registry || !shell || !device || !main) {
        d.check("ub5: the controllers exist", false);
        return;
    }
    QDir(QDir(d.outDir()).filePath(QStringLiteral("ub5"))).removeRecursively();
    QDir().mkpath(QDir(d.outDir()).filePath(QStringLiteral("ub5")));
    d.switchLayout("modern");
    const QSize size = main->size();
    main->resize(1600, 1000);
    d.spin(300);
    if (!device->connected() || !device->demo()) device->openDemo();
    d.waitFor(10000, [&] { return device->connected(); });
    // Three devices
    while (registry->sessions().size() < 3) {
        const qsizetype had = registry->sessions().size();
        registry->openDemo();
        if (!d.waitFor(10000, [&] { return registry->opening() == 0 && registry->sessions().size() > had; })) break;
    }
    const QList<DeviceSession *> sessions = registry->sessions();
    d.check("ub5: three devices", sessions.size() == 3);
    if (sessions.size() < 3) return;
    DeviceSession *first = sessions.first();
    registry->activate(first);
    d.waitIdle();
    const auto gain = [&](DeviceSession *s, int o) {
        return s->model()->state() ? double(s->model()->state()->output_gain_db[o]) : 99.0;
    };
    const auto setGains = [&](float db) {
        device->write(QStringLiteral("ub5 gains"), [db](FfiCore *c) { return dspi_set_output_gain(c, 6, db) && dspi_set_output_gain(c, 7, db); });
        d.waitIdle();
    };
    setGains(-20.0f);

    // ---- An agent, paired
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("ub5-store")));
    const int port = freePort();
    ai->setPort(port);
    ai->setConfirmMode(QStringLiteral("ask_risky"));
    ai->setEnabled(true);
    d.waitFor(10000, [&] { return ai->running() && !ai->busy(); });
    auto asked = std::make_unique<Request>(port, "/pair", QByteArray(), QJsonObject{ { QStringLiteral("name"), QStringLiteral("Claude Code") } });
    QObject *pairing = d.object(QStringLiteral("AiPairingSheet"));
    d.waitFor(5000, [&] { return asked->done && opened(pairing); });
    ai->answerPairing(true);
    auto wait = std::make_unique<Request>(port, "/pair/wait", QByteArray(), QJsonObject{ { QStringLiteral("ticket"), asked->body().value(QLatin1String("ticket")) } });
    d.waitFor(10000, [&] { return wait->done; });
    const QByteArray token = wait->body().value(QLatin1String("token")).toString().toUtf8();
    d.check("ub5: paired", !token.isEmpty());

    QObject *sheet = d.object(QStringLiteral("AiConfirmSheet"));
    const auto overlay = [&](const char *name) { return d.item(QString::fromLatin1(name), main->contentItem()->parentItem()); };
    const auto armed = [&] { return d.waitFor(3000, [&] { return sheet && sheet->property("armed").toBool(); }); };
    const auto rowTexts = [&] {
        QStringList rows;
        for (int i = 0; i < 16; ++i)
            if (QObject *r = overlay(QStringLiteral("AiConfirmSheet.risk.%1").arg(i).toLatin1().constData())) rows << textOf(r);
        return rows;
    };

    // ---- Night mode on three devices, sent together
    std::vector<std::unique_ptr<Request>> night;
    for (DeviceSession *s : sessions) {
        const QString id = s->id();
        night.push_back(call(port, token, "dspi_set_loudness", { { QStringLiteral("device"), id }, { QStringLiteral("enabled"), true } }));
        night.push_back(call(port, token, "dspi_set_leveller",
                             { { QStringLiteral("device"), id }, { QStringLiteral("enabled"), true }, { QStringLiteral("max_gain_db"), 6 } }));
        night.push_back(call(port, token, "dspi_set_limiter",
                             { { QStringLiteral("device"), id }, { QStringLiteral("output"), QStringLiteral("all") },
                               { QStringLiteral("enabled"), true }, { QStringLiteral("threshold_db"), -6 } }));
        d.spin(50);   // (the server takes a few connections at once; a held call gives its own back)
    }
    const bool six = d.waitFor(8000, [&] { return ai->confirmations().size() >= 6 && opened(sheet); });
    armed();
    d.spin(400);
    const QString heading = sheet ? sheet->property("heading").toString() : QString();
    const QStringList ids = sheet ? sheet->property("question").toString().split(QLatin1Char(','), Qt::SkipEmptyParts) : QStringList();
    const QStringList rows = rowTexts();
    d.check("ub5: night mode on three devices is six questions, one sheet", six && ids.size() == 6
            && heading.contains(QLatin1String("wants to make 6 changes")));
    d.check("ub5: each device once, with its rows", rows.count(sessions.at(0)->title()) == 1 && rows.count(sessions.at(1)->title()) == 1
                                                      && rows.count(sessions.at(2)->title()) == 1 && rows.size() >= 9);
    d.check("ub5: with Answer One by One", overlay("AiConfirmSheet.oneByOne") && overlay("AiConfirmSheet.oneByOne")->isVisible());
    grabBoth(d, main, QStringLiteral("1-night-three-devices"));
    d.check("ub5: one Allow answers all six", click(overlay("AiConfirmSheet.allow")) && d.waitFor(8000, [&] {
        for (const auto &r : night)
            if (!r->done) return false;
        return ai->confirmations().isEmpty();
    }));
    bool allMade = true;
    for (const auto &r : night) {
        if (r->failed()) qWarning().noquote() << "ub5: not made:" << QJsonDocument(r->body()).toJson(QJsonDocument::Compact).left(600);
        allMade = allMade && !r->failed();
    }
    d.check("ub5: and every call is made", allMade);

    // ---- A question raised while a sheet is up is not on it
    registry->activate(first);
    d.waitIdle();
    auto shown = call(port, token, "dspi_set_gain", { { QStringLiteral("output"), 7 }, { QStringLiteral("gain_db"), -10.0 } });
    d.waitFor(5000, [&] { return opened(sheet) && ai->confirmations().size() == 1; });
    const QString before = sheet ? sheet->property("question").toString() : QString();
    auto later = call(port, token, "dspi_set_gain", { { QStringLiteral("output"), 8 }, { QStringLiteral("gain_db"), -10.0 } });
    d.waitFor(5000, [&] { return ai->confirmations().size() == 2; });
    armed();
    d.spin(300);
    const QString left = textOf(overlay("AiConfirmSheet.left"));
    d.check("ub5: a question raised while a sheet is up is not on it",
            ai->confirmations().size() == 2 && sheet && sheet->property("question").toString() == before
                && !sheet->property("heading").toString().contains(QLatin1String("changes")) && left.contains(QLatin1String("1 more waiting")));
    grabBoth(d, main, QStringLiteral("2-later-question"));
    click(overlay("AiConfirmSheet.allow"));
    d.check("ub5: the Allow covers the question shown, not the later one", d.waitFor(5000, [&] { return shown->done; }) && !shown->failed()
            && d.waitFor(3000, [&] { return std::abs(gain(first, 6) + 10.0) < 0.05; }) && std::abs(gain(first, 7) + 20.0) < 0.05 && !later->done);
    d.check("ub5: which is the next sheet", d.waitFor(3000, [&] {
        return opened(sheet) && ai->confirmations().size() == 1 && sheet->property("question").toString() != before;
    }));
    armed();
    click(overlay("AiConfirmSheet.deny"));
    d.waitFor(5000, [&] { return later->done; });

    // ---- Once the agent rests: what it left unsaved, with Save
    QObject *line = d.object(QStringLiteral("AiWorkBanner.text"));
    const bool said = d.waitFor(AiController::kWorkingMs + AiController::kLingerMs + 3000,
                                [&] { return textOf(line).contains(QLatin1String("not saved yet")); });
    d.check("ub5: the strip's last line says what the agent left unsaved", said
            && textOf(line).startsWith(QLatin1String("Changed ")) && textOf(line).contains(QLatin1String("on 3 devices")));
    QObject *save = overlay("AiWorkBanner.save");
    if (!save) save = d.item(QStringLiteral("AiWorkBanner.save"));
    d.check("ub5: with Save All… beside it", save && qobject_cast<QQuickItem *>(save)->isVisible() && textOf(save) == QStringLiteral("Save All…"));
    grabBoth(d, main, QStringLiteral("3-unsaved-strip"));
    QObject *allQuestion = d.object(QStringLiteral("TopBar.saveAllQuestion"));
    d.check("ub5: which asks the save question", click(save) && d.waitFor(3000, [&] { return opened(allQuestion); })
            && textOf(allQuestion).startsWith(QLatin1String("Save the unsaved changes of 3 devices?")));
    grabBoth(d, main, QStringLiteral("4-save-question"));
    if (allQuestion) QMetaObject::invokeMethod(allQuestion, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    d.check("ub5: saved, the line goes", d.waitFor(8000, [&] { return !textOf(line).contains(QLatin1String("not saved yet")); }));

    // As it was
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setPort(AiController::kDefaultPort);
    for (const QVariant &c : ai->clients()) ai->revoke(c.toMap().value(QStringLiteral("id")).toDouble());
    for (DeviceSession *s : registry->sessions())
        if (s != first) registry->close(s->id());
    d.waitFor(10000, [&] { return registry->sessions().size() == 1; });
    setGains(-20.0f);
    main->resize(size);
    d.waitIdle();
}

const SmokeDriver::Section agent("ub5-agent", 999, &agentSection);

} // namespace
