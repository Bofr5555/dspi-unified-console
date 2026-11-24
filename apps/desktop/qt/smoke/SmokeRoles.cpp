// -- WP ROLES -- Speaker roles an AI agent sets from the person's words (owner
// 2026-11-23), on the demo DSPi and its simulated microphone.
//
// roles-agent: a fake agent over HTTP sends the owner's sentence as a table
// (dspi_set_speaker_roles): one question, headed "Set the speaker roles on …",
// with the whole table and the rest of the server's words; Allow keeps it as a
// stated layout: Settings › Speakers and the main screen's line say "set by AI
// from your words, not checked", each stated row's pill "from your words", an
// output the words leave out "unknown", a row whose top held like a tweeter's
// "tweeter-like". The agent's low-end search on a stated midrange runs the
// top-fall check first in the Measure view ("Checking that … is not a
// tweeter"), then the search. Under "never ask" it is refused until Settings ›
// AI Agents › Agents may set speaker roles from my words is on, which asks
// first and is kept in the settings.
//
// The section leaves the speaker layout and the settings as it found them.
#include "SmokeDriver.h"

#include "AiController.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "MeasureController.h"
#include "MeasureSession.h"
#include "Registrar.h"
#include "Shell.h"
#include "SpeakersController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QDateTime>
#include <QDir>
#include <QFile>   // -- WP REVDFIX --
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QStandardPaths>   // -- WP REVDFIX --
#include <QTcpServer>
#include <QTcpSocket>
#include <functional>
#include <memory>

namespace {

using Job = std::function<QByteArray(FfiCore *)>;

QJsonObject onWorker(SmokeDriver &d, Job job) {
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

QByteArray fill(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(1 << 16, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

QJsonObject storedLayout(SmokeDriver &d) {
    return onWorker(d, [](FfiCore *c) { return fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); }); });
}

QJsonObject setLayout(SmokeDriver &d, QJsonObject layout) {
    layout.remove(QStringLiteral("ok"));
    const QByteArray text = QJsonDocument(layout).toJson(QJsonDocument::Compact);
    return onWorker(d, [text](FfiCore *c) {
        return fill([c, &text](char *b, uint32_t l) { return dspi_speaker_layout_set(c, text.constData(), b, l); });
    });
}

// -- WP REVDFIX -- (REVD B3) The user's set keeps the core's statement, never
// the caller's, so a forged check goes into the layout's file (main.cpp's
// <AppData>/speaker-layouts, the folder snapshot_store::dir_name names), as
// another process on the host could write it.
bool writeLayoutFile(QJsonObject layout) {
    const QString serial = layout.value(QLatin1String("serial")).toString();
    bool safe = !serial.isEmpty();
    for (const QChar c : serial)
        safe = safe && c.unicode() < 128 && (c.isLetterOrNumber() || c == QLatin1Char('-') || c == QLatin1Char('_'));
    const QString folder = safe ? QStringLiteral("s-") + serial : QStringLiteral("x-") + QString::fromLatin1(serial.toUtf8().toHex());
    const QString dir = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation) + QStringLiteral("/speaker-layouts/") + folder;
    layout.remove(QStringLiteral("ok"));
    layout.insert(QStringLiteral("kind"), QStringLiteral("speakerLayout"));
    layout.insert(QStringLiteral("version"), 2);
    QFile f(dir + QStringLiteral("/speaker_layout.json"));
    return QDir().mkpath(dir) && f.open(QIODevice::WriteOnly | QIODevice::Truncate)
           && f.write(QJsonDocument(layout).toJson()) > 0;
}

QVariantMap rowFor(SpeakersController *sc, int output) {
    for (const QVariant &v : sc->rows())
        if (v.toMap().value(QStringLiteral("output")).toInt() == output) return v.toMap();
    return {};
}

QString textOf(QObject *item) { return item ? item->property("text").toString() : QString(); }

bool opened(QObject *dialog) { return dialog && dialog->property("opened").toBool(); }

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// One tool call of the fake agent's (as SmokeAdetq's)
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

struct Agent {
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
};

QString words(const QJsonObject &result) { return QString::fromUtf8(QJsonDocument(result).toJson(QJsonDocument::Compact)); }

const char *const kOwner = "L and r front speakers, both on DSPi named main front, full range three way, tweeter,mid,woofer, "
                           "l then right, output 1-6";

QJsonObject stated(int output, const char *role, const char *speaker) {
    return { { QStringLiteral("output"), output }, { QStringLiteral("role"), QLatin1String(role) },
             { QStringLiteral("speaker"), QLatin1String(speaker) } };
}

// The owner's sentence as its table (outputs from 0)
QJsonObject ownersTable() {
    return { { QStringLiteral("user_words"), QLatin1String(kOwner) },
             { QStringLiteral("rows"), QJsonArray{ stated(0, "tweeter", "L"), stated(1, "midrange", "L"), stated(2, "woofer", "L"),
                                                   stated(3, "tweeter", "R"), stated(4, "midrange", "R"), stated(5, "woofer", "R") } } };
}

void rolesSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    auto *ai = Registrar::instance<AiController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *sc = Registrar::instance<SpeakersController>();
    d.check(QStringLiteral("roles: the controllers exist"), ai && mc && sc);
    if (!ai || !mc || !sc) return;
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo()) device->openDemo();
    d.waitFor(10000, [&] { return device->connected() && device->demo(); });
    d.switchLayout("classic");
    d.app().device->write(QLatin1String("roles volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
    d.waitIdle();
    const QJsonObject was = storedLayout(d);
    // Nobody's word yet: the proposal kept unconfirmed
    QJsonObject proposal = onWorker(d, [](FfiCore *c) {
        return fill([c](char *b, uint32_t l) { return dspi_speaker_layout_propose(c, b, l); });
    });
    proposal.insert(QStringLiteral("confirmedByUser"), false);
    setLayout(d, proposal);
    sc->reload();

    const QString mode = ai->confirmMode();
    const bool capOn = ai->agentSplCapOn();
    const bool followWas = ai->followAgent();
    const bool rolesWas = ai->agentRoles();
    d.check(QStringLiteral("roles: agents may not set roles under never-ask by default"),
            !QSettings().contains(QStringLiteral("mcp/agentRoles")) ? !rolesWas : true);
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("roles-store")));
    QDir().mkpath(QDir(d.outDir()).filePath(QStringLiteral("roles-store")));
    Agent a{ d, freePort(), {} };
    ai->setPort(a.port);
    ai->setConfirmMode(QStringLiteral("ask_risky"));
    ai->setAgentSplCapOn(false);   // the simulated microphone reads no dB SPL
    ai->setFollowAgent(false);
    ai->setEnabled(true);
    d.check(QStringLiteral("roles: the server runs"), d.waitFor(5000, [&] { return ai->running() && !ai->busy(); }));
    ai->addClient(QStringLiteral("Roles agent"));
    d.waitFor(3000, [&] { return !ai->newClient().isEmpty(); });
    a.token = ai->newClient().value(QStringLiteral("token")).toString().toUtf8();
    ai->clearNewClient();

    // ---- One question showing the whole table
    auto held = a.call("dspi_set_speaker_roles", ownersTable());
    d.check(QStringLiteral("roles: the roles are asked"), d.waitFor(5000, [&] { return !ai->confirmations().isEmpty(); }));
    const QVariantMap q = ai->confirmations().value(0).toMap();
    const QStringList cells = q.value(QStringLiteral("roleCells")).toStringList();
    d.check(QStringLiteral("roles: the table, every row"), cells.size() == 18 && cells.mid(0, 3) == QStringList{ "Out 1", "L", "Tweeter" }
            && cells.mid(12, 3) == QStringList{ "Out 5", "R", "Midrange" } && cells.mid(15, 3) == QStringList{ "Out 6", "R", "Woofer" });
    d.check(QStringLiteral("roles: then the outputs not named, the words, not checked"),
            q.value(QStringLiteral("rolesAfter")).toString().contains(QLatin1String("not named: unknown, treated as tweeters"))
                && q.value(QStringLiteral("rolesAfter")).toString().contains(QLatin1String("full range three way"))
                && q.value(QStringLiteral("rolesAfter")).toString().contains(QLatin1String("Not checked")));
    d.check(QStringLiteral("roles: one risk, no grant"), q.value(QStringLiteral("riskLines")).toStringList().size() == 1
            && q.value(QStringLiteral("riskLines")).toStringList().value(0).startsWith(QStringLiteral("Speaker roles from your words: Out 1 · L · tweeter"))
            && q.value(QStringLiteral("grantText")).toString().isEmpty());
    d.check(QStringLiteral("roles: the sheet is headed for the roles"), d.waitFor(2000, [&] {
        QObject *sheet = d.object(QStringLiteral("AiConfirmSheet"));
        return sheet && sheet->property("heading").toString().startsWith(QLatin1String("Set the speaker roles on "));
    }));
    d.check(QStringLiteral("roles: the sheet shows the table"), d.waitFor(2000, [&] {
        QQuickItem *grid = d.item(QStringLiteral("AiConfirmSheet.roles"));
        QObject *first = d.object(QStringLiteral("AiConfirmSheet.roles.3"));
        QObject *after = d.object(QStringLiteral("AiConfirmSheet.rolesAfter"));
        return grid && grid->isVisible() && textOf(first) == QLatin1String("Out 1") && textOf(after).contains(QLatin1String("not named"));
    }));
    d.check(QStringLiteral("roles: the sheet promises a correction, not an undo"),
            textOf(d.object(QStringLiteral("AiConfirmSheet.after"))).contains(QLatin1String("confirm or correct the roles afterwards")));
    if (QQuickWindow *mw = d.mainWindow()) d.grab(mw, QStringLiteral("roles-question.png"));
    ai->answer(q.value(QStringLiteral("id")).toDouble(), true);
    const QJsonObject r = a.result(held);
    d.check(QStringLiteral("roles: Allow sets them"), !r.value(QLatin1String("isError")).toBool(true)
            && words(r).contains(QLatin1String("speaker_setup_stated")));

    qInfo().noquote() << "smoke: roles: risk lines" << q.value(QStringLiteral("riskLines")).toStringList();
    // ---- Settings › Speakers and the main screen: set by AI from your words, not checked
    d.check(QStringLiteral("roles: the layout is stated"), d.waitFor(5000, [&] { return sc->status() == QLatin1String("stated"); }));
    d.check(QStringLiteral("roles: elsewhere it says set by AI from your words"),
            sc->bannerText(false) == QStringLiteral("Your speaker roles were set by AI from your words and are not checked. Check them in Settings › Speakers."));
    qInfo().noquote() << "smoke: roles: banner" << sc->bannerText(true);
    d.check(QStringLiteral("roles: here it names who, when and the words"),
            sc->bannerText(true).startsWith(QStringLiteral("Set by AI from your words, not checked: Roles agent set these roles today at "))
                && sc->bannerText(true).contains(QStringLiteral("(“L and r front speakers")));
    d.check(QStringLiteral("roles: the stamp says so"), sc->stampText().startsWith(QStringLiteral("Set by AI from your words, not checked, today at ")));
    d.check(QStringLiteral("roles: a stated row's pill"), rowFor(sc, 1).value(QStringLiteral("pillKind")).toString() == QLatin1String("stated")
            && rowFor(sc, 1).value(QStringLiteral("pill")).toString() == QLatin1String("from your words")
            && rowFor(sc, 1).value(QStringLiteral("role")).toString() == QLatin1String("mid"));
    const QVariantMap unnamed = rowFor(sc, 6);
    d.check(QStringLiteral("roles: an output the words left out is unknown"),
            unnamed.isEmpty() || unnamed.value(QStringLiteral("pillKind")).toString() == QLatin1String("notNamed"));
    QQuickItem *note = nullptr;
    d.check(QStringLiteral("roles: the main screen says so"), d.waitFor(3000, [&] {
        note = d.item(QStringLiteral("SpeakersLayoutNote"));
        return note && note->isVisible() && textOf(note) == sc->bannerText(false);
    }));
    if (QQuickWindow *mw = d.mainWindow()) d.grab(mw, QStringLiteral("roles-main.png"));
    sc->openSettings();
    auto *sw = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    if (sw) {
        d.spin(600);
        d.grab(sw, QStringLiteral("roles-speakers-page.png"));
        sw->close();
    }

    // ---- Trust but check: the low-end search on a stated woofer checks its top first
    // (the demo plays Out 1 and 2 only; the person now says Out 1 is the woofer)
    auto restate = a.call("dspi_set_speaker_roles", QJsonObject{ { QStringLiteral("user_words"), QStringLiteral("output 1 is the woofer") },
                                                                 { QStringLiteral("rows"), QJsonArray{ stated(0, "woofer", "L") } } });
    d.check(QStringLiteral("roles: another table is asked again"), d.waitFor(5000, [&] { return !ai->confirmations().isEmpty(); }));
    ai->answer(ai->confirmations().value(0).toMap().value(QStringLiteral("id")).toDouble(), true);
    d.check(QStringLiteral("roles: and set"), !a.result(restate).value(QLatin1String("isError")).toBool(true));
    auto search = a.call("dspi_measure_rolloff_start", QJsonObject{ { QStringLiteral("output"), 0 } });
    const bool asked = d.waitFor(5000, [&] { return !ai->confirmations().isEmpty() || search->done; });
    if (search->done) qInfo().noquote() << "smoke: roles: the search answered" << words(search->result());
    d.check(QStringLiteral("roles: the test signals are asked"), asked && !ai->confirmations().isEmpty());
    ai->answer(ai->confirmations().value(0).toMap().value(QStringLiteral("id")).toDouble(), true);
    const QJsonObject s = a.result(search);
    d.check(QStringLiteral("roles: the check goes first"), !s.value(QLatin1String("isError")).toBool(true)
            && words(s).contains(QLatin1String("First a check that Out 1 (stated woofer) is not a tweeter")));
    d.check(QStringLiteral("roles: the Measure view checks it"), d.waitFor(15000, [&] { return mc->session()->detecting(); }));
    d.check(QStringLiteral("roles: the Stop bar says what it checks"), d.waitFor(5000, [&] {
        return mc->runningText().startsWith(QLatin1String("Requested by AI")) && mc->runningText().contains(QLatin1String("is not a tweeter, above 1 kHz only"));
    }));
    d.check(QStringLiteral("roles: the strip says it checks"), d.waitFor(5000, [&] {
        QQuickItem *t = d.item(QStringLiteral("AiWorkBanner.text"));
        return t && t->isVisible() && textOf(t).startsWith(QLatin1String("Checking the stated speaker roles"));
    }));
    if (QQuickWindow *mw = d.mainWindow()) d.grab(mw, QStringLiteral("roles-check-running.png"));
    d.check(QStringLiteral("roles: the check ends"), d.waitFor(120000, [&] { return !mc->session()->detecting(); }));
    const QJsonObject l = storedLayout(d);
    const QJsonArray checks = l.value(QLatin1String("stated")).toObject().value(QLatin1String("checks")).toArray();
    d.check(QStringLiteral("roles: the check is kept with the roles"), checks.size() == 1
            && checks.at(0).toObject().value(QLatin1String("output")).toInt() == 0
            && checks.at(0).toObject().value(QLatin1String("verdict")).toString() != QLatin1String("unfinished"));
    d.check(QStringLiteral("roles: then the search runs as its own request"), d.waitFor(20000, [&] { return mc->running(); }));
    const QJsonObject stopped = a.result(a.call("dspi_measure_stop"));
    d.check(QStringLiteral("roles: the agent's Stop ends it"), !stopped.value(QLatin1String("isError")).toBool(true)
            && d.waitFor(15000, [&] { return !mc->running(); }));

    // ---- A row whose top held like a tweeter's
    QJsonObject tweeterLike = storedLayout(d);
    QJsonObject st = tweeterLike.value(QLatin1String("stated")).toObject();
    st.insert(QStringLiteral("checks"), QJsonArray{ QJsonObject{ { QStringLiteral("output"), 0 }, { QStringLiteral("role"), QStringLiteral("low") },
                                                                 { QStringLiteral("verdict"), QStringLiteral("topHolds") },
                                                                 { QStringLiteral("topFallDb"), 0.4 }, { QStringLiteral("floorHz"), 1000 } } });
    tweeterLike.insert(QStringLiteral("stated"), st);
    // -- WP REVDFIX -- the user's set takes no statement (B3): in its file
    const QJsonObject forged = setLayout(d, tweeterLike);
    d.check(QStringLiteral("roles: the user's set keeps the core's checks, not the caller's"),
            forged.value(QLatin1String("stated")).toObject().value(QLatin1String("checks")).toArray().at(0).toObject()
                    .value(QLatin1String("verdict")).toString() != QLatin1String("topHolds"));
    d.check(QStringLiteral("roles: a forged file is the test's way in"), writeLayoutFile(tweeterLike));
    sc->reload();
    d.check(QStringLiteral("roles: a top that held reads tweeter-like"), d.waitFor(3000, [&] {
        return rowFor(sc, 0).value(QStringLiteral("pillKind")).toString() == QLatin1String("statedTweeter")
               && rowFor(sc, 0).value(QStringLiteral("pill")).toString() == QLatin1String("tweeter-like");
    }));

    // ---- Never ask: refused until the switch is on, which asks first
    ai->setConfirmMode(QStringLiteral("never_ask"));
    for (const QVariant &g : ai->grants()) ai->revokeGrant(g.toMap().value(QStringLiteral("id")).toDouble());
    const QJsonObject refused = a.result(a.call("dspi_set_speaker_roles", ownersTable()));
    d.check(QStringLiteral("roles: never-ask without the switch refuses"), refused.value(QLatin1String("isError")).toBool()
            && words(refused).contains(QLatin1String("Agents may set speaker roles from my words")));
    ai->openSettings();
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    QQuickItem *page = nullptr;
    d.check(QStringLiteral("roles: Settings › AI Agents opens"), w && d.waitFor(3000, [&] {
        page = w->isVisible() ? d.item(QStringLiteral("AiAgentsPage"), w->contentItem()) : nullptr;
        return page != nullptr;
    }));
    QQuickItem *toggle = page ? d.item(QStringLiteral("AiAgents.roles.switch"), page) : nullptr;
    QObject *question = page ? page->property("rolesDialog").value<QObject *>() : nullptr;
    const auto flip = [&] {
        toggle->setProperty("checked", !toggle->property("checked").toBool());
        QMetaObject::invokeMethod(toggle, "toggled");
    };
    d.check(QStringLiteral("roles: the switch is there, off"), toggle && toggle->isVisible() && !toggle->property("checked").toBool());
    if (toggle && question) {
        flip();
        d.check(QStringLiteral("roles: turning it on asks first"), d.waitFor(2000, [&] { return opened(question); }) && !ai->agentRoles()
                && question->property("text").toString().contains(QLatin1String("not checked")));
        d.spin(400);
        d.grab(w, QStringLiteral("roles-switch-question.png"));
        QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
        d.spin(300);
        d.check(QStringLiteral("roles: Cancel leaves it off"), !ai->agentRoles() && !toggle->property("checked").toBool());
        flip();
        d.waitFor(2000, [&] { return opened(question); });
        QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
        d.check(QStringLiteral("roles: I Understand turns it on, kept"), d.waitFor(2000, [&] { return ai->agentRoles(); })
                && toggle->property("checked").toBool() && QSettings().value(QStringLiteral("mcp/agentRoles")).toBool());
    }
    const QJsonObject r2 = a.result(a.call("dspi_set_speaker_roles", ownersTable()));
    d.check(QStringLiteral("roles: with the switch on it sets them without a question"),
            !r2.value(QLatin1String("isError")).toBool(true) && ai->confirmations().isEmpty());
    if (toggle) {
        flip();
        d.check(QStringLiteral("roles: off again at once, no question"), d.waitFor(2000, [&] { return !ai->agentRoles(); }) && !opened(question)
                && !QSettings().value(QStringLiteral("mcp/agentRoles")).toBool());
    }
    if (w) w->close();

    // As it was
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setAgentRoles(rolesWas);
    ai->setFollowAgent(followWas);
    ai->setConfirmMode(mode);
    ai->setAgentSplCapOn(capOn);
    ai->setPort(AiController::kDefaultPort);
    ai->revoke(ai->clients().value(0).toMap().value(QStringLiteral("id")).toDouble());
    if (!was.isEmpty() && was.value(QLatin1String("outputs")).isArray()) setLayout(d, was);
    sc->reload();
    d.waitIdle();
    d.spin(200);
}

const SmokeDriver::Section roles("roles-agent", 988, &rolesSection);

} // namespace
