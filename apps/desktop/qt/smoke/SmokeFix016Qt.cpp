// -- FIX016 -- The 0.16 review's Qt findings (REVG), each checked where it was
// found ("fix016-qt", on the demo, last in the run; everything put back):
// - R1-QT: the demo's simulated microphone, once chosen, is the demo's again
//   when the demo is opened again (ua2 after mcp5-measure and ua5-settings).
// - R2-QT: an agent's questions raised together, one sheet: every asked row of
//   every question shows, or the grouped Allow is off; Allow answers exactly
//   the rows shown (three calls of seven changes each, one asked: the third
//   call's asked row was row 15, under "and 10 more").
#include "SmokeDriver.h"

#include "AiController.h"
#include "ChannelController.h"
#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "LayoutSetupController.h"
#include "LimitsController.h"
#include "MeasureController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SpeakersController.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QJSValue>
#include <QtQml/QQmlApplicationEngine>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QTcpServer>
#include <QTest>
#include <QTcpSocket>
#include <algorithm>
#include <cmath>
#include <functional>
#include <memory>
#include <vector>

namespace {

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// One request of a fake agent's, read on this thread's event loop (as ub5-agent's)
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

// ---- R1-QT: the simulated microphone comes back with the demo
void micWithTheDemo(SmokeDriver &d) {
    auto *mc = Registrar::instance<MeasureController>();
    DeviceModel *device = d.app().device;
    if (!mc) {
        d.check("fix016 R1-QT: the Measure controller", false);
        return;
    }
    if (!device->connected() || !device->demo()) device->openDemo();
    d.waitFor(10000, [&] { return device->connected() && device->demo(); });
    d.waitIdle();
    mc->panelShown(true);   // the measurement thread, as the Measure view starts it
    mc->setMicChoice(QStringLiteral("sim"));   // the person picks the simulated one
    d.check("fix016 R1-QT: the demo's simulated microphone is chosen",
            d.waitFor(5000, [&] { return mc->simAvailable() && mc->micChoice() == QLatin1String("sim"); }));
    // The demo closes (as Settings' and the reconnect paths do) and opens again
    d.app().core->worker()->post([](FfiCore *c) { dspi_disconnect(c); });
    device->refresh();
    d.waitFor(5000, [&] { return !device->connected(); });
    d.waitFor(3000, [&] { return !mc->simAvailable(); });
    device->openDemo();
    d.waitFor(10000, [&] { return device->connected() && device->demo(); });
    d.waitIdle();
    d.check(QStringLiteral("fix016 R1-QT: the demo opened again measures with its simulated microphone again (%1)").arg(mc->micChoice()),
            d.waitFor(5000, [&] { return mc->simAvailable() && mc->micChoice() == QLatin1String("sim"); }));
    mc->panelShown(false);
}

// ---- R2-QT: one Allow answers only rows the sheet showed
void groupedQuestions(SmokeDriver &d) {
    auto *ai = Registrar::instance<AiController>();
    auto *cc = Registrar::instance<ChannelController>();
    DeviceModel *device = d.app().device;
    QQuickWindow *main = d.mainWindow();
    if (!ai || !cc || !main || !device->state()) {
        d.check("fix016 R2-QT: the controllers", false);
        return;
    }
    const int outputs = std::min(cc->numOutputs(), 9);
    std::vector<float> gainsWas;
    for (int o = 0; o < outputs; ++o) gainsWas.push_back(device->state()->output_gain_db[o]);
    const auto setGains = [&](float db) {
        device->write(QStringLiteral("fix016 gains"), [db, outputs](FfiCore *c) {
            bool ok = true;
            for (int o = 0; o < outputs; ++o) ok = dspi_set_output_gain(c, uint8_t(o), db) && ok;
            return ok;
        });
        d.waitIdle();
    };
    const auto gain = [&](int o) { return device->state() ? double(device->state()->output_gain_db[o]) : 99.0; };
    setGains(-20.0f);

    // An agent, paired, asking about risky changes
    const QString modeWas = ai->confirmMode();
    const bool followWas = ai->followAgent();
    ai->setFollowAgent(false);
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("fix016-store")));
    const int port = freePort();
    ai->setPort(port);
    ai->setConfirmMode(QStringLiteral("ask_risky"));
    ai->setEnabled(true);
    d.waitFor(10000, [&] { return ai->running() && !ai->busy(); });
    auto asked = std::make_unique<Request>(port, "/pair", QByteArray(), QJsonObject{ { QStringLiteral("name"), QStringLiteral("Fix016 agent") } });
    QObject *pairing = d.object(QStringLiteral("AiPairingSheet"));
    d.waitFor(5000, [&] { return asked->done && opened(pairing); });
    ai->answerPairing(true);
    auto wait = std::make_unique<Request>(port, "/pair/wait", QByteArray(), QJsonObject{ { QStringLiteral("ticket"), asked->body().value(QLatin1String("ticket")) } });
    d.waitFor(10000, [&] { return wait->done; });
    const QByteArray token = wait->body().value(QLatin1String("token")).toString().toUtf8();
    d.check("fix016 R2-QT: an agent paired", !token.isEmpty());

    QObject *sheet = d.object(QStringLiteral("AiConfirmSheet"));
    const auto overlay = [&](const char *name) { return d.item(QString::fromLatin1(name), main->contentItem()->parentItem()); };
    const auto armed = [&] { return d.waitFor(3000, [&] { return sheet && sheet->property("armed").toBool(); }); };
    // The rows the sheet shows, and whether every asked row of its questions is among them
    const auto shownRows = [&] {
        QStringList rows;
        for (int i = 0; i < 64; ++i)
            if (QObject *r = overlay(QStringLiteral("AiConfirmSheet.risk.%1").arg(i).toLatin1().constData())) rows << textOf(r);
        return rows;
    };
    const auto missingAsked = [&] {
        const QStringList ids = sheet ? sheet->property("question").toString().split(QLatin1Char(','), Qt::SkipEmptyParts) : QStringList();
        const QStringList rows = shownRows();
        QStringList missing;
        for (const QVariant &v : ai->confirmations()) {
            const QVariantMap c = v.toMap();
            if (!ids.contains(QString::number(c.value(QStringLiteral("id")).toLongLong()))) continue;
            const QVariantList batch = c.value(QStringLiteral("batch")).toList();
            QStringList saids;
            for (const QVariant &b : batch)
                if (b.toMap().value(QStringLiteral("risky")).toBool()) saids << b.toMap().value(QStringLiteral("said")).toString();
            if (batch.isEmpty()) saids = c.value(QStringLiteral("riskLines")).toStringList();
            for (const QString &said : saids)
                if (!rows.contains(said)) missing << said;
        }
        return missing;
    };
    const auto allowOn = [&] {
        auto *allow = qobject_cast<QQuickItem *>(overlay("AiConfirmSheet.allow"));
        return allow && allow->isVisible() && allow->isEnabled();
    };
    // `louder`: a raise to each of these gains on outputs first+1, first+2, ... (each its
    // own words, so no two fold into one row); then `count` quieter ones from Out 4
    const auto changes = [&](const std::vector<double> &louder, int first, int count, double db) {
        QJsonArray list;
        for (size_t k = 0; k < louder.size(); ++k)
            list.append(QJsonObject{ { QStringLiteral("op"), QStringLiteral("set_gain") }, { QStringLiteral("output"), first + int(k) + 1 },
                                     { QStringLiteral("gain_db"), louder[k] } });
        for (int o = 3; o < 3 + count && o < outputs; ++o)
            list.append(QJsonObject{ { QStringLiteral("op"), QStringLiteral("set_gain") }, { QStringLiteral("output"), o + 1 },
                                     { QStringLiteral("gain_db"), db } });
        return QJsonObject{ { QStringLiteral("changes"), list } };
    };

    // Three calls sent together, each one louder change (Out 1, 2, 3) and six quieter
    // ones: one sheet of 22 rows, the third call's asked row beyond the twelfth
    std::vector<std::unique_ptr<Request>> sent;
    for (int k = 0; k < 3; ++k) sent.push_back(call(port, token, "dspi_apply_changes", changes({ -10.0 }, k, 6, -25.0)));
    const bool three = d.waitFor(8000, [&] { return ai->confirmations().size() >= 3 && opened(sheet); });
    armed();
    d.spin(300);
    const QStringList ids = sheet ? sheet->property("question").toString().split(QLatin1Char(','), Qt::SkipEmptyParts) : QStringList();
    QStringList missing = missingAsked();
    d.check(QStringLiteral("fix016 R2-QT: three calls raised together are one sheet (%1 questions)").arg(ids.size()), three && ids.size() == 3);
    d.check(QStringLiteral("fix016 R2-QT: every asked row shows, or the grouped Allow is off (missing: %1)").arg(missing.join(QStringLiteral("; "))),
            missing.isEmpty() || !allowOn());
    d.check("fix016 R2-QT: three asked rows of 22 fit: the grouped Allow is on", missing.isEmpty() && allowOn());
    d.check("fix016 R2-QT: the cut rows are said as not asked about",
            textOf(overlay("AiConfirmSheet.more")).contains(QLatin1String("not asked about")));
    d.grab(main, QStringLiteral("fix016-r2-grouped.png"));
    d.check("fix016 R2-QT: its Allow answers the three calls, each change made", click(overlay("AiConfirmSheet.allow")) && d.waitFor(8000, [&] {
                for (const auto &r : sent)
                    if (!r->done) return false;
                return ai->confirmations().isEmpty();
            }) && std::all_of(sent.begin(), sent.end(), [](const auto &r) { return !r->failed(); })
            && std::abs(gain(0) + 10) < 0.05 && std::abs(gain(1) + 10) < 0.05 && std::abs(gain(2) + 10) < 0.05
            && (outputs < 4 || std::abs(gain(3) + 25) < 0.05));

    // Three calls of five louder changes each: fifteen asked rows, more than a sheet
    // shows. Whatever the server groups, no sheet's Allow answers a row it hides.
    setGains(-20.0f);
    sent.clear();
    for (int k = 0; k < 3; ++k)
        sent.push_back(call(port, token, "dspi_apply_changes",
                            changes({ -5.0 - 5 * k, -6.0 - 5 * k, -7.0 - 5 * k, -8.0 - 5 * k, -9.0 - 5 * k }, 0, 0, 0)));
    d.waitFor(8000, [&] { return ai->confirmations().size() >= 3 && opened(sheet); });
    bool honest = true, tooMany = false;
    int sheets = 0;
    while (!ai->confirmations().isEmpty() && sheets < 8) {
        if (!d.waitFor(5000, [&] { return opened(sheet); })) break;
        armed();
        d.spin(300);
        ++sheets;
        missing = missingAsked();
        QObject *host = d.object(QStringLiteral("AiHost"));
        const bool fits = host && host->property("askedFit").toBool(), grouped = host && host->property("grouped").toBool();
        if (!missing.isEmpty() && allowOn()) {
            honest = false;
            std::printf("smoke: fix016 R2-QT: sheet %d hides asked rows with Allow on: %s\n", sheets, qPrintable(missing.join(QStringLiteral("; "))));
        }
        if (grouped && (!fits || !missing.isEmpty())) {
            tooMany = true;
            d.check("fix016 R2-QT: a sheet whose asked rows do not fit has no grouped Allow", !allowOn());
            d.check("fix016 R2-QT: a sheet whose asked rows do not fit says why and offers Answer One by One",
                    overlay("AiConfirmSheet.tooMany") && overlay("AiConfirmSheet.tooMany")->isVisible() && click(overlay("AiConfirmSheet.oneByOne")));
            continue;
        }
        click(overlay("AiConfirmSheet.deny"));
        d.waitFor(3000, [&] { return !opened(sheet) || sheet->property("question").toString().isEmpty() || !sheet->property("armed").toBool(); });
    }
    std::printf("smoke: fix016 R2-QT: fifteen asked rows took %d sheet(s)%s\n", sheets, tooMany ? ", one too many for a sheet" : "");
    d.check("fix016 R2-QT: fifteen asked rows: one by one, or sheets the server kept small", tooMany || sheets > 1);
    d.check("fix016 R2-QT: fifteen asked rows: no sheet's Allow answers a row it hides", honest && ai->confirmations().isEmpty());
    for (const auto &r : sent) d.waitFor(5000, [&] { return r->done; });
    d.check("fix016 R2-QT: denied, nothing changed", std::abs(gain(0) + 20) < 0.05 && std::abs(gain(4) + 20) < 0.05);

    // As it was
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setPort(AiController::kDefaultPort);
    ai->setConfirmMode(modeWas);
    ai->setFollowAgent(followWas);
    for (const QVariant &c : ai->clients()) ai->revoke(c.toMap().value(QStringLiteral("id")).toDouble());
    device->write(QStringLiteral("fix016 gains"), [gainsWas](FfiCore *c) {
        bool ok = true;
        for (size_t o = 0; o < gainsWas.size(); ++o) ok = dspi_set_output_gain(c, uint8_t(o), gainsWas[o]) && ok;
        return ok;
    });
    d.waitIdle();
}

// ==== R4-QT ====
// ---- R4-QT: Settings › Speakers is read-only under the installer's lock
const QString kPin = QStringLiteral("2468");

bool settled(SmokeDriver &d, LimitsController *lc) {
    d.waitIdle();
    const bool ok = d.waitFor(8000, [&] { return !lc->busy(); });
    d.waitIdle();
    d.spin(60);
    return ok;
}

// The installer's lock on for a while (as ub2-home sets it), off and the PIN gone after
struct InstallerLock {
    SmokeDriver &d;
    LimitsController *lc;
    InstallerLock(SmokeDriver &driver, LimitsController *limits) : d(driver), lc(limits) {
        lc->setPin(kPin, kPin);
        settled(d, lc);
        lc->dismissRecovery();
        lc->setCovers(true);
        settled(d, lc);
        lc->lockNow();
        settled(d, lc);
    }
    void open() {
        if (!lc->locked()) return;
        lc->unlock(kPin);
        settled(d, lc);
    }
    ~InstallerLock() {
        open();
        lc->clearPin();
        settled(d, lc);
        lc->clearLimits();
        settled(d, lc);
    }
};

QQuickItem *within(SmokeDriver &d, QQuickWindow *w, const char *name) { return w ? d.item(QString::fromLatin1(name), w->contentItem()) : nullptr; }

void speakersUnderTheLock(SmokeDriver &d) {
    auto *lc = Registrar::instance<LimitsController>();
    auto *sc = Registrar::instance<SpeakersController>();
    auto *ls = Registrar::instance<LayoutSetupController>();
    auto *settings = Registrar::instance<SettingsController>();
    Shell *shell = d.app().shell;
    if (!lc || !sc || !ls || !settings) {
        d.check("fix016 R4-QT: the controllers", false);
        return;
    }
    const bool shownWas = shell->shownWindows().contains(QStringLiteral("edit.settings"));
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    settings->open(QStringLiteral("speakers"));
    sc->reload();
    d.waitFor(3000, [&] { return within(d, w, "SpeakersPage.suggestion") != nullptr && sc->loaded(); });
    d.spin(200);
    const auto on = [&](const char *name) {
        QQuickItem *i = within(d, w, name);
        return i && i->isVisible() && i->isEnabled();
    };
    d.check("fix016 R4-QT: unlocked: the speaker layout can be changed, Suggest is on",
            sc->editable() && on("SpeakersPage.suggestion"));
    {
        InstallerLock lock(d, lc);
        d.check("fix016 R4-QT: locked", lc->locked());
        d.spin(200);
        QQuickItem *said = within(d, w, "SpeakersPage.readOnly");
        d.check("fix016 R4-QT: locked: the speaker layout is read-only (the table, Suggest, Confirm, Detect…)",
                !sc->editable() && !on("SpeakersPage.suggestion") && !on("SpeakersPage.confirm") && !on("SpeakersPage.detect"));
        d.check("fix016 R4-QT: locked: the page says why", said && said->isVisible()
                && said->property("text").toString().startsWith(QLatin1String("Locked by the installer")));
        d.check("fix016 R4-QT: locked: no Crossovers and Routing… (LayoutSetupController.writable)",
                !ls->writable() && !on("SpeakersPage.fromLayout"));
        d.check("fix016 R4-QT: locked: Detect says why", sc->detectBlocker().startsWith(QLatin1String("Locked by the installer")));
        d.grab(w, QStringLiteral("fix016-r4-speakers-locked.png"));
        lock.open();
        d.spin(200);
        d.check("fix016 R4-QT: unlocked again: the layout can be changed again", sc->editable() && on("SpeakersPage.suggestion"));
    }
    if (!shownWas) shell->closeWindow(QStringLiteral("edit.settings"));
    d.waitIdle();
}
// ==== end R4-QT ====





void fix016Section(SmokeDriver &d) {
    d.allowSectionTime(240);
    micWithTheDemo(d);
    groupedQuestions(d);
    speakersUnderTheLock(d);   // R4-QT
}

const SmokeDriver::Section fix016("fix016-qt", 1000, &fix016Section);

} // namespace
