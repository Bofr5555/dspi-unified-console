// -- WP ADETQ -- Speaker auto-detect on the Qt desktop (AUTODES 4, 5.1), on the
// demo DSPi and its simulated microphone.
//
// adet-speakers: a layout the detection wrote (seeded through the core, as the
// detection writes it) is "autoDetected": the banner, the stamp, the pills with
// their reasons, the summary and its details, the main screen's line (Classic
// and Modern) and the channel inspector's word for an output nothing was heard
// from; an edited role is "you"; the app's own Detect… asks its one question
// (the agent's warning, word for word), plays, says where it is, and Stop keeps
// what it wrote; Confirm ends it, and a confirmed layout is not detected over.
//
// adet-agent: a fake agent over HTTP asks for dspi_detect_speakers: one
// question, headed "Detect the speakers on …", with the Speaker detection line,
// the server's warning (the app's own copy is the same words) and the grant's
// sentence; Allow runs it in the Measure view ("Requested by AI · …" in the Stop
// bar, "Detecting speakers: …" in the strip), the grant shows "speaker
// detection"; under "never ask" it is refused until Settings › AI Agents ›
// Agents may detect speakers is on, which asks with the warning first and is
// kept in the settings.
//
// Both sections leave the speaker layout as they found it.
#include "SmokeDriver.h"

#include "AiController.h"
#include "ChannelListModel.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "MeasureController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SpeakersController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QDateTime>
#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
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

QJsonObject detectState(SmokeDriver &d) {
    return onWorker(d, [](FfiCore *c) { return fill([c](char *b, uint32_t l) { return dspi_detect_state(c, b, l); }); });
}

int outputCount(SmokeDriver &d) {
    ChannelListModel *ch = d.app().device->channels();
    int n = 0;
    for (int r = 0; r < ch->rowCount(); ++r)
        if (ch->get(r, QStringLiteral("kind")).toString() == QLatin1String("output")) ++n;
    return n;
}

QJsonObject row(int output, const char *role, const char *source) {
    return { { QStringLiteral("output"), output }, { QStringLiteral("role"), QLatin1String(role) },
             { QStringLiteral("speaker"), QStringLiteral("L") }, { QStringLiteral("group"), 0 },
             { QStringLiteral("source"), QLatin1String(source) } };
}

QJsonObject found(int output, const char *klass, const char *confidence, const char *heard, const char *note) {
    QJsonObject o{ { QStringLiteral("output"), output }, { QStringLiteral("class"), QLatin1String(klass) },
                   { QStringLiteral("confidence"), QLatin1String(confidence) }, { QStringLiteral("heard"), QLatin1String(heard) },
                   { QStringLiteral("floorHz"), 1000 }, { QStringLiteral("probedMs"), QDateTime::currentMSecsSinceEpoch() } };
    if (note) o.insert(QStringLiteral("note"), QLatin1String(note));
    return o;
}

// What the detection writes (AUTODES 3.1): Out 1 a woofer, Out 2 a tweeter
// (not sure), nothing heard on Out 3 (no row), Out 4 off
QJsonObject detectedLayout(SmokeDriver &d) {
    QJsonObject woofer = found(0, "woofer", "high", "top", "detect.woofer");
    woofer.insert(QStringLiteral("topFallDb"), 21.3);
    woofer.insert(QStringLiteral("lowestHz"), 20);
    QJsonObject tweeter = found(1, "tweeter", "medium", "top", "detect.tweeter_unsure");
    tweeter.insert(QStringLiteral("topFallDb"), 0.4);
    tweeter.insert(QStringLiteral("lowestHz"), 1000);
    const QJsonObject detection{
        { QStringLiteral("atMs"), QDateTime::currentMSecsSinceEpoch() },
        { QStringLiteral("by"), QStringLiteral("Adet agent") },
        { QStringLiteral("placement"), QStringLiteral("listeningPosition") },
        { QStringLiteral("floorHz"), 1000 },
        { QStringLiteral("outputs"), QJsonArray{ woofer, tweeter, found(2, "nothing", "high", "never", "detect.nothing"),
                                                 found(3, "off", "high", "never", "detect.off") } },
        { QStringLiteral("summary"), QStringLiteral("1 tweeter?, 1 woofer; nothing heard on Out 3; Out 4 off, not probed.") },
        { QStringLiteral("deviceOutputs"), outputCount(d) },
    };
    return { { QStringLiteral("outputs"), QJsonArray{ row(0, "low", "detected"), row(1, "high", "detected") } },
             { QStringLiteral("confirmedByUser"), false },
             { QStringLiteral("detected"), detection } };
}

QVariantMap rowFor(SpeakersController *sc, int output) {
    for (const QVariant &v : sc->rows())
        if (v.toMap().value(QStringLiteral("output")).toInt() == output) return v.toMap();
    return {};
}

bool click(QObject *item) {
    auto *i = qobject_cast<QQuickItem *>(item);
    return i && i->isVisible() && i->isEnabled() && QMetaObject::invokeMethod(i, "clicked");
}

QString textOf(QObject *item) { return item ? item->property("text").toString() : QString(); }

bool opened(QObject *dialog) { return dialog && dialog->property("opened").toBool(); }

// The demo as the Measure view needs it: quiet enough to play the generator
void quieten(SmokeDriver &d) {
    d.app().device->write(QLatin1String("adet volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
    d.waitIdle();
}

// ---- adet-speakers ----------------------------------------------------------------------

void speakersSection(SmokeDriver &d) {
    d.allowSectionTime(150);
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo()) device->openDemo();
    d.check(QStringLiteral("adet: the demo DSPi is open"), d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    auto *sc = Registrar::instance<SpeakersController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    d.check(QStringLiteral("adet: the controllers exist"), sc && mc && notices);
    if (!sc || !mc || !notices) return;
    d.switchLayout("classic");
    quieten(d);
    const QJsonObject was = storedLayout(d);
    // -- INT016 -- (UB7's finding) from an empty, unconfirmed layout, whatever an
    // earlier section (roleq-speakers, ub7-speakers) left confirmed
    setLayout(d, { { QStringLiteral("outputs"), QJsonArray{} }, { QStringLiteral("confirmedByUser"), false } });
    sc->reload();
    d.waitFor(3000, [&] { return sc->loaded() && sc->status() != QLatin1String("confirmed"); });

    // ---- A layout the detection wrote: auto-detected ---------------------------------
    const QJsonObject kept = setLayout(d, detectedLayout(d));
    d.check(QStringLiteral("adet: the core keeps the detected layout"), kept.value(QLatin1String("ok")).toBool());
    sc->reload();
    d.check(QStringLiteral("adet: its status is autoDetected"), d.waitFor(3000, [&] { return sc->status() == QLatin1String("autoDetected"); }));
    d.check(QStringLiteral("adet: elsewhere it says detected by AI, not confirmed"),
            sc->bannerText(false) == QStringLiteral("Your speaker layout was detected by AI and is not confirmed. Check it in Settings › Speakers."));
    d.check(QStringLiteral("adet: here it names who, when and where"),
            sc->bannerText(true).startsWith(QStringLiteral("Detected by AI, not confirmed: Adet agent probed this DSPi's outputs today at "))
                && sc->bannerText(true).contains(QStringLiteral("(microphone at the listening position)")));
    d.check(QStringLiteral("adet: the stamp says detected today"),
            sc->stampText().startsWith(QStringLiteral("Detected by AI, not confirmed, today at ")));

    // The main screen's line: Classic's banners, Modern's status strip
    QQuickItem *noteC = nullptr;
    d.check(QStringLiteral("adet: the main screen says so (Classic)"), d.waitFor(3000, [&] {
        noteC = d.item(QStringLiteral("SpeakersLayoutNote"));
        return noteC && noteC->isVisible() && textOf(noteC) == sc->bannerText(false);
    }));
    if (QQuickWindow *mw = d.mainWindow()) d.grab(mw, QStringLiteral("adet-main-classic.png"));
    d.check(QStringLiteral("adet: in Modern"), d.switchLayout("modern"));
    d.check(QStringLiteral("adet: the status strip says so (Modern)"), d.waitFor(3000, [&] {
        QQuickItem *strip = d.item(QStringLiteral("StatusStrip"));
        QQuickItem *n = strip ? d.item(QStringLiteral("SpeakersLayoutNote"), strip) : nullptr;
        return n && n->isVisible() && textOf(n) == sc->bannerText(false);
    }));
    if (QQuickWindow *mw = d.mainWindow()) d.grab(mw, QStringLiteral("adet-main-modern.png"));
    d.check(QStringLiteral("adet: the inspector's note for nothing heard is for Out 3 only"), sc->heardNothing(2) && !sc->heardNothing(0)
            && !sc->heardNothing(1));
    d.switchLayout("classic");

    // ---- Settings › Speakers: pills, summary, details --------------------------------
    sc->openSettings();
    auto *settings = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    QQuickItem *page = nullptr;
    d.check(QStringLiteral("adet: Settings › Speakers opens"), settings && d.waitFor(3000, [&] {
        page = settings->isVisible() ? d.item(QStringLiteral("SpeakersPage"), settings->contentItem()) : nullptr;
        return page != nullptr;
    }));
    if (!page) return;
    const auto in = [&](const QString &name) { return d.item(name, page); };
    d.check(QStringLiteral("adet: the page's banner"), d.waitFor(2000, [&] { return textOf(in(QStringLiteral("SpeakersPage.banner"))) == sc->bannerText(true); }));
    d.check(QStringLiteral("adet: and its stamp"), textOf(in(QStringLiteral("SpeakersPage.stamp"))) == sc->stampText());
    const auto pill = [&](int o) { return rowFor(sc, o).value(QStringLiteral("pill")).toString(); };
    const auto pillItem = [&](int o) { return in(QStringLiteral("Speakers.pill.%1").arg(o)); };
    d.check(QStringLiteral("adet: Out 1 detected, Out 2 detected?"), pill(0) == QLatin1String("detected") && pill(1) == QLatin1String("detected?")
            && pillItem(0) && pillItem(0)->isVisible() && pillItem(1) && pillItem(1)->isVisible());
    d.check(QStringLiteral("adet: Out 2's reason is the notice's"),
            rowFor(sc, 1).value(QStringLiteral("pillReason")).toString() == notices->sentence(QStringLiteral("detect.tweeter_unsure"))
                && rowFor(sc, 1).value(QStringLiteral("pillReason")).toString().contains(QLatin1String("full-range")));
    d.check(QStringLiteral("adet: Out 3 nothing heard, left out, its role not offered"), pill(2) == QLatin1String("nothing heard")
            && !rowFor(sc, 2).value(QStringLiteral("included")).toBool()
            && in(QStringLiteral("Speakers.role.2")) && !in(QStringLiteral("Speakers.role.2"))->isEnabled());
    d.check(QStringLiteral("adet: Out 4 off, not probed"), pill(3) == QLatin1String("off, not probed"));
    d.check(QStringLiteral("adet: the summary is under the table"),
            textOf(in(QStringLiteral("SpeakersPage.summary"))).contains(QLatin1String("nothing heard on Out 3")));
    QQuickItem *table = in(QStringLiteral("SpeakersPage.detailsTable"));
    d.check(QStringLiteral("adet: details are on request"), table && !table->isVisible() && click(in(QStringLiteral("SpeakersPage.details")))
            && d.waitFor(1000, [&] { return table->isVisible(); }));
    d.check(QStringLiteral("adet: the details say what was found"),
            textOf(in(QStringLiteral("SpeakersPage.detail.0.1"))) == QLatin1String("Woofer")
                && textOf(in(QStringLiteral("SpeakersPage.detail.1.1"))) == QLatin1String("Tweeter?")
                && textOf(in(QStringLiteral("SpeakersPage.detail.0.3"))).contains(QLatin1String("21.3"))
                && textOf(in(QStringLiteral("SpeakersPage.detail.3.2"))) == QLatin1String("not probed"));
    d.grab(settings, QStringLiteral("adet-settings-speakers.png"));

    // ---- The app's own Detect…: its question, a run, Stop ------------------------------
    QObject *question = page->property("detectDialog").value<QObject *>();
    QQuickItem *detectButton = in(QStringLiteral("SpeakersPage.detect"));
    d.check(QStringLiteral("adet: Detect… is offered on a layout not confirmed"), detectButton && detectButton->isEnabled()
            && sc->detectBlocker().isEmpty());
    d.check(QStringLiteral("adet: it asks first"), click(detectButton) && d.waitFor(2000, [&] { return opened(question); }));
    const QString asked = question ? question->property("text").toString() : QString();
    d.check(QStringLiteral("adet: the question is headed for the device"),
            question && question->property("title").toString().startsWith(QLatin1String("Detect the speakers on ")));
    d.check(QStringLiteral("adet: it says the levels, and the agent's warning word for word"),
            asked.contains(QString::fromUtf8("at most −24 dBFS")) && asked.contains(QString::fromUtf8("at most −50 dBFS"))
                && asked.contains(QStringLiteral("⚠ ") + AiController::detectWarning())
                && !asked.contains(QLatin1String("Allow also")));
    d.spin(400);
    d.grab(settings, QStringLiteral("adet-detect-question.png"));
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.spin(300);
    d.check(QStringLiteral("adet: Cancel plays nothing"), !sc->detecting() && !mc->running());
    click(detectButton);
    d.waitFor(2000, [&] { return opened(question); });
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    const bool runs = d.waitFor(10000, [&] { return sc->detecting(); });
    if (!runs) qInfo().noquote() << "adet: not started:" << sc->said() << "|" << mc->error();
    d.check(QStringLiteral("adet: Detect runs it"), runs);
    d.check(QStringLiteral("adet: the page says where it is"), d.waitFor(10000, [&] {
        return textOf(in(QStringLiteral("SpeakersPage.detectText"))).startsWith(QLatin1String("Detecting the speakers: "));
    }));
    d.check(QStringLiteral("adet: the Stop bar's words are the detection's"), mc->runningText().startsWith(QLatin1String("Detecting the speakers")));
    d.check(QStringLiteral("adet: Detect… waits while it runs"), !detectButton->isEnabled() && !sc->detectBlocker().isEmpty());
    // It wrote its rows from the first sound: auto-detected, the output under way unknown
    d.waitFor(15000, [&] { return detectState(d).value(QLatin1String("stage")).toString() != QLatin1String("ramp"); });
    d.grab(settings, QStringLiteral("adet-detecting.png"));
    d.check(QStringLiteral("adet: Stop ends it"), click(in(QStringLiteral("SpeakersPage.stopDetect")))
            && d.waitFor(10000, [&] { return !sc->detecting() && !mc->running(); }));
    const QJsonObject st = detectState(d);
    d.check(QStringLiteral("adet: as stopped"), st.value(QLatin1String("stage")).toString() == QLatin1String("stopped"));
    d.check(QStringLiteral("adet: what it wrote is kept, auto-detected"), d.waitFor(3000, [&] {
        return sc->status() == QLatin1String("autoDetected") && sc->detection().value(QStringLiteral("stopped")).toBool();
    }));
    d.check(QStringLiteral("adet: the app's own detection says no AI"), sc->bannerText(false) == QStringLiteral(
            "Your speaker layout was detected and is not confirmed. Check it in Settings › Speakers."));
    d.check(QStringLiteral("adet: a notice says it stopped"), notices->current().value(QStringLiteral("text")).toString().contains(QLatin1String("stopped"))
            || notices->waiting() > 0);

    // -- WP INTK -- a row corrected through the write-back (the Measure tables, the
    // function test's user) keeps the layout auto-detected until Confirm, never "imported"
    const QString corrected = rowFor(sc, 1).value(QStringLiteral("role")).toString() == QLatin1String("sub")
                                  ? QStringLiteral("mid") : QStringLiteral("sub");
    sc->writeBack(1, corrected);
    d.check(QStringLiteral("intk: a corrected row keeps the layout auto-detected"), d.waitFor(3000, [&] {
        return rowFor(sc, 1).value(QStringLiteral("role")).toString() == corrected && sc->status() == QLatin1String("autoDetected");
    }));
    d.check(QStringLiteral("intk: the corrected row is \"you\""), pill(1) == QLatin1String("you")
            && storedLayout(d).value(QLatin1String("outputs")).toArray().at(1).toObject().value(QLatin1String("source")).toString() == QLatin1String("user"));
    sc->reload();
    d.check(QStringLiteral("intk: still auto-detected when read again"), d.waitFor(3000, [&] { return sc->status() == QLatin1String("autoDetected"); })
            && sc->bannerText(false).contains(QLatin1String("detected and is not confirmed")));

    // ---- The person's word: an edited role is theirs; Confirm ends it -----------------
    sc->setRole(0, QStringLiteral("mid"));
    d.check(QStringLiteral("adet: an edited role is \"you\""), pill(0) == QLatin1String("you")
            && rowFor(sc, 0).value(QStringLiteral("source")).toString() == QLatin1String("user"));
    sc->confirm();
    d.check(QStringLiteral("adet: Confirm confirms it"), d.waitFor(3000, [&] { return sc->status() == QLatin1String("confirmed"); }));
    const QJsonObject confirmed = storedLayout(d);
    d.check(QStringLiteral("adet: the detection's record is kept with it"), confirmed.value(QLatin1String("detected")).isObject());
    d.check(QStringLiteral("adet: the main screen's line is gone"), d.waitFor(2000, [&] {
        QQuickItem *n = d.item(QStringLiteral("SpeakersLayoutNote"));
        return !n || !n->isVisible();
    }));
    d.check(QStringLiteral("adet: a confirmed layout is not detected over"), d.waitFor(2000, [&] {
        return !detectButton->isEnabled() && sc->detectBlocker().contains(QLatin1String("confirmed"));
    }));
    d.check(QStringLiteral("adet: no pills on the confirmed rows"), pill(0).isEmpty() && pill(1).isEmpty());

    // As it was
    settings->close();
    if (!was.isEmpty() && was.value(QLatin1String("outputs")).isArray()) setLayout(d, was);
    sc->reload();
    d.waitIdle();
    d.spin(200);
}

// ---- adet-agent -------------------------------------------------------------------------

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// One tool call of the fake agent's (as SmokeOdec's)
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

void agentSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    auto *ai = Registrar::instance<AiController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *sc = Registrar::instance<SpeakersController>();
    auto *settings = Registrar::instance<SettingsController>();
    d.check(QStringLiteral("adet-agent: the controllers exist"), ai && mc && sc && settings);
    if (!ai || !mc || !sc || !settings) return;
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo()) device->openDemo();
    d.waitFor(10000, [&] { return device->connected() && device->demo(); });
    d.switchLayout("classic");
    // -- FIX016 -- (R1-QT) put back at the end: the volume, and no Measure window
    // the agent's detection opened (ua2's "in view" checks after it read the main
    // window while the Measure view was in that window)
    const float volumeWas = device->state() ? device->state()->user_volume_db : -20.0f;
    const bool measureWas = d.app().shell->shownWindows().contains(QStringLiteral("tools.measure"));
    quieten(d);
    const QJsonObject was = storedLayout(d);
    // Nobody's word yet: the proposal kept unconfirmed (a confirmed layout is not detected over)
    QJsonObject proposal = onWorker(d, [](FfiCore *c) {
        return fill([c](char *b, uint32_t l) { return dspi_speaker_layout_propose(c, b, l); });
    });
    proposal.insert(QStringLiteral("confirmedByUser"), false);
    setLayout(d, proposal);
    sc->reload();

    // ---- On, asking about risky changes
    const QString mode = ai->confirmMode();
    const bool capOn = ai->agentSplCapOn();
    const bool followWas = ai->followAgent();
    const bool detectWas = ai->agentDetect();
    d.check(QStringLiteral("adet-agent: agents may not detect under never-ask by default"),
            !QSettings().contains(QStringLiteral("mcp/agentDetect")) ? !detectWas : true);
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("adet-store")));
    QDir().mkpath(QDir(d.outDir()).filePath(QStringLiteral("adet-store")));
    Agent a{ d, freePort(), {} };
    ai->setPort(a.port);
    ai->setConfirmMode(QStringLiteral("ask_risky"));
    ai->setAgentSplCapOn(false);   // the simulated microphone reads no dB SPL
    ai->setFollowAgent(false);
    ai->setEnabled(true);
    d.check(QStringLiteral("adet-agent: the server runs"), d.waitFor(5000, [&] { return ai->running() && !ai->busy(); }));
    ai->addClient(QStringLiteral("Adet agent"));
    d.waitFor(3000, [&] { return !ai->newClient().isEmpty(); });
    a.token = ai->newClient().value(QStringLiteral("token")).toString().toUtf8();
    ai->clearNewClient();

    // ---- One question, with the warning and the grant
    const QJsonObject args{ { QStringLiteral("outputs"), QJsonArray{ 1 } } }; // Out 1
    auto held = a.call("dspi_detect_speakers", args);
    d.check(QStringLiteral("adet-agent: the detection is asked"), d.waitFor(5000, [&] { return !ai->confirmations().isEmpty(); }));
    const QVariantMap q = ai->confirmations().value(0).toMap();
    const QStringList lines = q.value(QStringLiteral("riskLines")).toStringList();
    d.check(QStringLiteral("adet-agent: one question with both risks"), lines.size() == 2 && lines.value(0).startsWith(QLatin1String("Test signal: "))
            && lines.value(1).startsWith(QLatin1String("Speaker detection: ")) && q.value(QStringLiteral("detect")).toBool());
    d.check(QStringLiteral("adet-agent: the server's warning is the app's, word for word"),
            q.value(QStringLiteral("warning")).toString() == AiController::detectWarning());
    d.check(QStringLiteral("adet-agent: Allow also covers detecting again"),
            q.value(QStringLiteral("grantText")).toString().contains(QLatin1String("play test signals and detect the speakers on"))
                && q.value(QStringLiteral("grant")).toMap().value(QStringLiteral("detect")).toBool());
    d.check(QStringLiteral("adet-agent: the sheet is headed for the detection"), d.waitFor(2000, [&] {
        QObject *sheet = d.object(QStringLiteral("AiConfirmSheet"));
        return sheet && sheet->property("heading").toString().startsWith(QLatin1String("Detect the speakers on "));
    }));
    // -- WP UAG -- (UXP A6) what to do comes first; the mechanism waits under Details
    d.check(QStringLiteral("uag: the detection says what to do first, the warning under Details"), d.waitFor(2000, [&] {
        auto *todo = qobject_cast<QQuickItem *>(d.object(QStringLiteral("AiConfirmSheet.todo")));
        auto *w = qobject_cast<QQuickItem *>(d.object(QStringLiteral("AiConfirmSheet.warning")));
        return todo && todo->isVisible() && textOf(todo).contains(QLatin1String("Keep the volume where it is")) && w && !w->isVisible();
    }));
    if (QQuickWindow *mw = d.mainWindow()) d.grab(mw, QStringLiteral("uag-detect-question.png"));
    click(d.object(QStringLiteral("AiConfirmSheet.details")));
    d.check(QStringLiteral("adet-agent: the sheet shows the warning"), d.waitFor(2000, [&] {
        auto *w = qobject_cast<QQuickItem *>(d.object(QStringLiteral("AiConfirmSheet.warning")));
        return w && w->isVisible() && textOf(w) == QStringLiteral("⚠ ") + AiController::detectWarning();
    }));
    if (QQuickWindow *mw = d.mainWindow()) d.grab(mw, QStringLiteral("adet-agent-question.png"));
    ai->answer(q.value(QStringLiteral("id")).toDouble(), true);
    const QJsonObject r = a.result(held);
    d.check(QStringLiteral("adet-agent: Allow sends it to the app"), !r.value(QLatin1String("isError")).toBool(true));

    // ---- It runs in the Measure view, said as the agent's
    d.check(QStringLiteral("adet-agent: the Measure view runs it"), d.waitFor(15000, [&] { return mc->session()->detecting(); }));
    d.check(QStringLiteral("adet-agent: the Stop bar says requested by AI"), d.waitFor(5000, [&] {
        return mc->runningText().startsWith(QLatin1String("Requested by AI")) && mc->runningText().contains(QLatin1String("Detecting the speakers"));
    }));
    d.check(QStringLiteral("adet-agent: the strip says which output"), d.waitFor(5000, [&] {
        QQuickItem *t = d.item(QStringLiteral("AiWorkBanner.text"));
        return t && t->isVisible() && textOf(t).startsWith(QLatin1String("Detecting speakers"));
    }));
    d.check(QStringLiteral("adet-agent: the grant says speaker detection"), d.waitFor(3000, [&] {
        QQuickItem *g = d.item(QStringLiteral("AiWorkBanner.grant"));
        return !ai->grants().isEmpty() && ai->grants().value(0).toMap().value(QStringLiteral("detect")).toBool() && g
               && textOf(g).startsWith(QLatin1String("Test signals and speaker detection allowed"));
    }));
    if (QQuickWindow *mw = d.mainWindow()) d.grab(mw, QStringLiteral("adet-agent-running.png"));
    // To its end (Out 1 only)
    d.check(QStringLiteral("adet-agent: it ends"), d.waitFor(150000, [&] { return !mc->session()->detecting(); }));
    const QJsonObject st = detectState(d);
    d.check(QStringLiteral("adet-agent: done, Out 1 found"), st.value(QLatin1String("stage")).toString() == QLatin1String("done")
            && !st.value(QLatin1String("detection")).toObject().value(QLatin1String("outputs")).toArray().isEmpty());
    d.check(QStringLiteral("adet-agent: the layout is auto-detected, by the agent"), d.waitFor(3000, [&] {
        return sc->status() == QLatin1String("autoDetected") && sc->bannerText(false).contains(QLatin1String("detected by AI"));
    }));
    d.check(QStringLiteral("adet-agent: Out 1 is the detection's"), rowFor(sc, 0).value(QStringLiteral("source")).toString() == QLatin1String("detected")
            && !rowFor(sc, 0).value(QStringLiteral("pill")).toString().isEmpty());

    // ---- Never ask: refused until the switch is on, which asks with the warning
    ai->setConfirmMode(QStringLiteral("never_ask"));
    ai->revokeGrant(ai->grants().value(0).toMap().value(QStringLiteral("id")).toDouble());
    d.waitFor(3000, [&] { return ai->grants().isEmpty(); });
    QJsonObject refused = a.result(a.call("dspi_detect_speakers", args));
    d.check(QStringLiteral("adet-agent: never-ask without the switch refuses"), refused.value(QLatin1String("isError")).toBool()
            && words(refused).contains(QLatin1String("Agents may detect speakers")));
    ai->openSettings();
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    QQuickItem *page = nullptr;
    d.check(QStringLiteral("adet-agent: Settings › AI Agents opens"), w && d.waitFor(3000, [&] {
        page = w->isVisible() ? d.item(QStringLiteral("AiAgentsPage"), w->contentItem()) : nullptr;
        return page != nullptr;
    }));
    QQuickItem *toggle = page ? d.item(QStringLiteral("AiAgents.detect.switch"), page) : nullptr;
    QObject *question = page ? page->property("detectDialog").value<QObject *>() : nullptr;
    const auto flip = [&] {
        toggle->setProperty("checked", !toggle->property("checked").toBool());
        QMetaObject::invokeMethod(toggle, "toggled");
    };
    d.check(QStringLiteral("adet-agent: the switch is there, off"), toggle && toggle->isVisible() && !toggle->property("checked").toBool());
    if (toggle && question) {
        flip();
        d.check(QStringLiteral("adet-agent: turning it on shows the warning first"), d.waitFor(2000, [&] { return opened(question); })
                && !ai->agentDetect() && question->property("text").toString().contains(AiController::detectWarning()));
        d.spin(400);
        d.grab(w, QStringLiteral("adet-agent-switch-question.png"));
        QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
        d.spin(300);
        d.check(QStringLiteral("adet-agent: Cancel leaves it off"), !ai->agentDetect() && !toggle->property("checked").toBool());
        flip();
        d.waitFor(2000, [&] { return opened(question); });
        QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
        d.check(QStringLiteral("adet-agent: I Understand turns it on, kept"), d.waitFor(2000, [&] { return ai->agentDetect(); })
                && toggle->property("checked").toBool() && QSettings().value(QStringLiteral("mcp/agentDetect")).toBool());
    }
    auto again = a.call("dspi_detect_speakers", args);
    const QJsonObject r2 = a.result(again);
    d.check(QStringLiteral("adet-agent: with the switch on it runs without a question"), !r2.value(QLatin1String("isError")).toBool(true)
            && ai->confirmations().isEmpty());
    d.check(QStringLiteral("adet-agent: and the Measure view takes it"), d.waitFor(15000, [&] { return mc->session()->detecting(); }));
    const QJsonObject stopped = a.result(a.call("dspi_measure_stop"));
    d.check(QStringLiteral("adet-agent: the agent's Stop ends it"), !stopped.value(QLatin1String("isError")).toBool(true)
            && d.waitFor(10000, [&] { return !mc->session()->detecting() && !mc->running(); }));
    if (toggle) {
        flip();
        d.check(QStringLiteral("adet-agent: off again at once, no question"), d.waitFor(2000, [&] { return !ai->agentDetect(); })
                && !opened(question) && !QSettings().value(QStringLiteral("mcp/agentDetect")).toBool());
    }
    if (w) w->close();

    // As it was
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setAgentDetect(detectWas);
    ai->setFollowAgent(followWas);
    ai->setConfirmMode(mode);
    ai->setAgentSplCapOn(capOn);
    ai->setPort(AiController::kDefaultPort);
    ai->revoke(ai->clients().value(0).toMap().value(QStringLiteral("id")).toDouble());
    if (!was.isEmpty() && was.value(QLatin1String("outputs")).isArray()) setLayout(d, was);
    sc->reload();
    if (!measureWas) d.app().shell->closeWindow(QStringLiteral("tools.measure"));   // -- FIX016 --
    device->write(QLatin1String("adet volume"), [volumeWas](FfiCore *c) { return dspi_set_user_volume(c, volumeWas); });   // -- FIX016 --
    d.waitIdle();
    d.spin(200);
}

const SmokeDriver::Section speakers("adet-speakers", 986, &speakersSection);
const SmokeDriver::Section agent("adet-agent", 987, &agentSection);

} // namespace
