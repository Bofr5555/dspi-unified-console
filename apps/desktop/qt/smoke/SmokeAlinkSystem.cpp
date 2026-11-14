// -- WP LKUI1 -- Window > System... against a real hub with an audio link
// installation: `uconsole --demo hub --alink` (DSPI_SMOKE_UCONSOLE), with
// the app's own LinkController, SystemController and QML. Locked units are
// shown with their audio link delay; the audio link delays are written; a scene is
// recalled and the next one, before its frame, is refused in words naming the
// held frame; an edit made while the hub's installation changes withdraws Save
// and offers Reload, and a Save sent stale is a conflict; Simple shows the
// status only. Then a PIN hub: paired as Control the editor() is read only, and
// made a Viewer (`uconsole hub role`, the hub restarted on its store) nothing
// that writes is offered. -- WP HWLK1F -- Then a hub with a unit whose firmware has
// no audio link (`--with-plain-unit`): said so, left out of the locked count, a
// scene applied at once on it, skipped by the alignment with the reason.
// -- WP AUSB1 -- The demo installation's two USB-direct units (Rear L/R, Height):
// phase-locked, their delay in samples and range, the statistics line, counted
// apart in the summary, and a recall at a USB frame on them. -- WP AUSB2 -- A
// hub whose Rear L/R can't lock (`--with-usb-lock-error`): the row says it is
// muted and why, the hub's alarm is shown; the Installer sets it to play
// unaligned and keeps the USB-direct units aligned, and the hub's own poll
// writes the setting to the unit, which then says so.
// Screenshots in <out>/lkui1/, dark and light.
#include "SmokeDriver.h"

#include "Core.h"
#include "DeviceModel.h"
#include "LinkController.h"
#include "ModeController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SystemController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QDir>
#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QQuickItem>
#include <QQuickWindow>
#include <cstdio>

namespace {

// One `uconsole --demo hub --alink` process
class UconsoleHub {
public:
    // `pairing`: wait for the PIN of the pairing window `--pair` opens
    bool start(const QString &store, const QStringList &extra, bool pairing = false, int port0 = 0) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList{ "--demo", "hub", "--alink", "--loopback", "--json", "--no-keep-awake",
                                          "--store", store, "--port", QString::number(port0) }
                                 + extra);
        if (!m_proc.waitForStarted(5000)) return false;
        QElapsedTimer t;
        t.start();
        pin.clear();
        port = 0;
        while (t.elapsed() < 20000) {
            if (!m_proc.canReadLine() && !m_proc.waitForReadyRead(1000)) {
                if (m_proc.state() != QProcess::Running) return false;
                continue;
            }
            while (m_proc.canReadLine()) {
                const QJsonObject e = QJsonDocument::fromJson(m_proc.readLine()).object();
                const QString event = e.value(QLatin1String("event")).toString();
                if (event == QLatin1String("status")) port = e.value(QLatin1String("port")).toInt();
                if (event == QLatin1String("pairing")) pin = e.value(QLatin1String("pin")).toString();
            }
            if (port > 0 && (!pairing || !pin.isEmpty())) {
                url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(port);
                return true;
            }
        }
        return false;
    }
    void stop() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    ~UconsoleHub() { stop(); }
    int port = 0;
    QString pin, url;

private:
    QProcess m_proc;
};

// `uconsole <args>` run to its end: whether it succeeded, and its last stdout line
bool uconsole(const QStringList &args, QString *last = nullptr) {
    QProcess p;
    p.start(qEnvironmentVariable("DSPI_SMOKE_UCONSOLE"), args);
    if (!p.waitForFinished(15000) || p.exitStatus() != QProcess::NormalExit || p.exitCode() != 0) return false;
    const QList<QByteArray> lines = p.readAllStandardOutput().trimmed().split('\n');
    if (last) *last = QString::fromUtf8(lines.isEmpty() ? QByteArray() : lines.last());
    return true;
}

bool click(QQuickItem *item) {
    return item && item->isVisible() && item->isEnabled() && QMetaObject::invokeMethod(item, "clicked");
}

// A second Link client on the same hub, through the C ABI: another installer
// saving while this console edits
bool saveElsewhere(const QString &url, const QByteArray &edits) {
    FfiCore *other = dspi_core_new();
    if (!other) return false;
    QByteArray buf(256 * 1024, '\0');
    const QByteArray u = url.toUtf8();
    bool ok = false;
    if (dspi_link_connect(other, u.constData(), "lkui1-other", "0", "", "", buf.data(), uint32_t(buf.size())) > 0
        && QJsonDocument::fromJson(buf.constData()).object().value(QLatin1String("ok")).toBool()) {
        buf.fill('\0');
        dspi_alink_system_view(other, buf.data(), uint32_t(buf.size()));
        const QJsonObject view = QJsonDocument::fromJson(buf.constData()).object();
        const quint64 revision = quint64(view.value(QLatin1String("revision")).toInteger());
        buf.fill('\0');
        dspi_alink_system_save(other, revision, edits.constData(), buf.data(), uint32_t(buf.size()));
        ok = QJsonDocument::fromJson(buf.constData()).object().value(QLatin1String("ok")).toBool();
    }
    dspi_link_close(other);
    dspi_core_free(other);
    return ok;
}

void alinkSystemSection(SmokeDriver &d) {
    auto *link = Registrar::instance<LinkController>();
    auto *sys = Registrar::instance<SystemController>();
    auto *modes = Registrar::instance<ModeController>();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    d.check("lkui1: the controllers", link && sys && modes);
    if (!link || !sys || !modes) return;
    const QString out = d.outDir() + QStringLiteral("/lkui1");
    QDir().mkpath(out);
    const bool demoBefore = device->demo() && device->connected();
    const QString modeBefore = modes->mode();
    modes->choose(QStringLiteral("advanced"), false);

    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    const auto phaseIs = [link](const char *phase) { return link->phase() == QLatin1String(phase); };
    const auto bind = [&](const QString &serial) {
        if (!d.waitFor(10000, [&] { return phaseIs("choose_device") && link->devices().size() >= 6; })) return false;
        for (const QVariant &v : link->devices()) {
            const QVariantMap m = v.toMap();
            if (m.value(QStringLiteral("serial")).toString() == serial) link->bindDevice(m.value(QStringLiteral("handle")).toInt(), serial);
        }
        return settled() && d.waitFor(10000, [&] { return phaseIs("bound") && device->remote() && device->connected(); }) && d.waitIdle(20000);
    };
    const auto leave = [&] {
        link->closeSession();
        settled();
        d.waitFor(10000, [&] { return !device->remote(); });
        d.waitIdle();
    };
    QQuickWindow *window = nullptr;
    const auto in = [&](const QString &name) { return window ? d.item(name, window->contentItem()) : nullptr; };
    const auto shoot = [&](const QString &name) {
        // Tall enough for the whole window: the editor and its buttons too
        window->resize(880, 1180);
        d.spin(200);
        const Qt::ColorScheme before = shell->colorSchemeOverride();
        for (const auto &[scheme, label] : { std::pair{ Qt::ColorScheme::Dark, "dark" }, std::pair{ Qt::ColorScheme::Light, "light" } }) {
            shell->setColorSchemeOverride(scheme);
            d.spin(300);
            d.check(QStringLiteral("lkui1: screenshot %1-%2").arg(name, QLatin1String(label)),
                    d.grab(window, QStringLiteral("lkui1/%1-%2.png").arg(name, QLatin1String(label))));
        }
        shell->setColorSchemeOverride(before);
        d.spin(100);
    };
    const auto fresh = [&] {
        // The hub answers a session's status once a second: past that, a fresh view
        d.spin(1100);
        sys->refresh();
        return d.waitFor(10000, [&] { return !sys->view().value(QLatin1String("stale")).toBool() && !sys->busy(); });
    };

    // ---- An open hub: Admin ----------------------------------------------------
    UconsoleHub hub;
    const bool up = hub.start(out + QStringLiteral("/hub-open"), { QStringLiteral("--open"), QStringLiteral("--forget-clients") });
    d.check("lkui1: uconsole --demo hub --alink runs (DSPI_SMOKE_UCONSOLE)", up);
    if (!up) return;
    d.check("lkui1: no view while not on a hub", !sys->available());
    link->connectToHub(hub.url);
    d.check("lkui1: on the hub, DEMO-ALINK-A bound", settled() && bind(QStringLiteral("DEMO-ALINK-A")));
    d.check("lkui1: the installation is read", d.waitFor(15000, [&] { return sys->available() && sys->installed(); }));
    d.check("lkui1: Admin on an open hub", sys->role() == QLatin1String("admin") && sys->mayEdit() && sys->mayRecall());
    int units = 0, locked = 0, usbUnits = 0, usbLocked = 0;
    for (const QVariant &v : sys->units()) {
        const QVariantMap u = v.toMap();
        if (u.value(QStringLiteral("kind")).toString() != QLatin1String("unit")) continue;
        // -- WP AUSB1 -- the two USB-direct units, counted apart
        if (u.value(QStringLiteral("transport")).toString() == QLatin1String("usb_direct")) {
            ++usbUnits;
            usbLocked += u.value(QStringLiteral("stateWord")).toString() == QLatin1String("Phase-locked (USB)")
                         && u.value(QStringLiteral("usbDelaySamples")).toInt() >= u.value(QStringLiteral("usbDelayMin")).toInt()
                         && u.value(QStringLiteral("usbDelayMin")).toInt() == 170
                         && u.value(QStringLiteral("usbStats")).toString().startsWith(QLatin1String("host -11.5 ppm"));
            continue;
        }
        ++units;
        locked += u.value(QStringLiteral("stateWord")).toString() == QLatin1String("Locked")
                  && u.value(QStringLiteral("alinkDelayFrames")).toInt() > 0
                  && u.value(QStringLiteral("alinkDelayMax")).toInt() >= u.value(QStringLiteral("alinkDelayMin")).toInt();
    }
    std::printf("smoke: lkui1: %s\n", qPrintable(sys->summary()));
    d.check("lkui1: four units, all locked with an audio link delay and its range", units == 4 && locked == 4);
    d.check("lkui1: the summary says so", sys->summary().startsWith(QLatin1String("Audio link: 4 of 4 locked")));
    d.check("ausb1: two USB-direct units, phase-locked, with their delay, range and statistics", usbUnits == 2 && usbLocked == 2);
    d.check("ausb1: the summary counts them apart",
            sys->summary().startsWith(QLatin1String("Audio link: 4 of 4 locked, USB-direct 2 of 2 locked")));
    QQuickItem *strip = d.item(QStringLiteral("SystemStatusStrip"));
    d.check("lkui1: the status line under the menu bar", strip && d.waitFor(3000, [&] { return strip->isVisible(); })
                                                            && strip->property("shownName").toString().startsWith(QLatin1String("Audio link:")));
    d.grab(d.mainWindow(), QStringLiteral("lkui1/main-strip.png"));

    shell->trigger(QStringLiteral("window.system"));
    d.check("lkui1: Window > System opens", d.waitFor(3000, [&] {
        window = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("window.system")));
        return window && window->isVisible();
    }));
    if (!window) return;
    d.spin(400);
    const auto rowA = [&] { return in(QStringLiteral("SystemWindow.unit.A")); };
    d.check("lkui1: a unit row shows its name and lock",
            rowA() && rowA()->property("shownName").toString() == QLatin1String("Main left, Locked"));
    // -- WP AUSB1 --
    const auto rowR = [&] { return in(QStringLiteral("SystemWindow.unit.R")); };
    const auto statsR = [&] { return in(QStringLiteral("SystemWindow.usbStats.R")); };
    const auto delayR = [&] { return in(QStringLiteral("SystemWindow.delay.R")); };
    d.check("ausb1: a USB-direct row shows its state, its delay in samples and its statistics",
            rowR() && rowR()->property("shownName").toString() == QLatin1String("Rear L/R, Phase-locked (USB)") && statsR()
                && statsR()->isVisible() && statsR()->property("text").toString().contains(QLatin1String("stream 144 samples early"))
                && delayR() && delayR()->property("text").toString().contains(QLatin1String(" smp")));

    // -- fixup lk1 -- the device held read only (G1, as a mismatched firmware): Admin on
    // the hub, yet no System write is offered, and none is sent if asked
    sys->setReadOnlyForTest(true);
    d.spin(200);
    const auto item = [&](const char *name) { return in(QString::fromLatin1(name)); };
    const int callsBefore = sys->lastRecall().size();
    sys->recall(QStringLiteral("evening"));
    sys->align();
    d.spin(300);
    d.check("lkui1: read only: nothing offered that writes",
            !sys->mayEdit() && !sys->mayRecall() && item("SystemWindow.scene.evening")
                && !item("SystemWindow.scene.evening")->isEnabled() && item("SystemWindow.align")
                && !item("SystemWindow.align")->isEnabled() && item("SystemWindow.latency.B")
                && !item("SystemWindow.latency.B")->isEnabled() && item("SystemWindow.recallNote")
                && item("SystemWindow.recallNote")->isVisible());
    d.check("lkui1: read only: said why", sys->recallNote().contains(QLatin1String("read only")));
    d.check("lkui1: read only: a recall or an alignment asked for is not sent",
            !sys->busy() && sys->lastRecall().size() == callsBefore && sys->actionMessage().isEmpty());
    shoot(QStringLiteral("read-only"));
    sys->setReadOnlyForTest(false);
    d.spin(200);
    d.check("lkui1: writable again", sys->mayEdit() && sys->mayRecall());

    // Align the audio link delays: none pending afterwards
    sys->align();
    d.check("lkui1: the audio link delays are written", d.waitFor(20000, [&] {
        return !sys->busy() && sys->actionMessage().startsWith(QLatin1String("The audio link delays were written"));
    }) && fresh() && sys->pendingUnits() == 0);
    shoot(QStringLiteral("system"));

    // A recall, then a second before the first one's frame: refused, in words
    const auto evening = [&] { return in(QStringLiteral("SystemWindow.scene.evening")); };
    const auto closed = [&] { return in(QStringLiteral("SystemWindow.scene.closed")); };
    const auto result = [&] { return in(QStringLiteral("SystemWindow.recallResult")); };
    d.check("lkui1: the scene bar", evening() && closed() && result());
    d.check("lkui1: recall Evening", click(evening()) && d.waitFor(15000, [&] { return !sys->busy() && !sys->lastRecall().isEmpty(); }));
    // -- WP AUSB1 -- at a USB frame on the USB-direct units, through the sender on the others
    d.check("lkui1: Evening recalled through the sender",
            sys->lastRecall().value(QStringLiteral("taken")).toBool() && sys->recalledScene() == QLatin1String("Evening")
                && result() && result()->property("text").toString().startsWith(QLatin1String("Evening recalled: at USB frame "))
                && result()->property("text").toString().contains(QLatin1String(" on Rear L/R, Height; at audio link frame "))
                && result()->property("text").toString().endsWith(QLatin1String(" through the sender on the others.")));
    d.check("lkui1: recall Closed at once", click(closed()) && d.waitFor(15000, [&] {
        return !sys->busy() && sys->lastRecall().value(QStringLiteral("name")).toString() == QLatin1String("Closed");
    }));
    const QString refused = result() ? result()->property("text").toString() : QString();
    std::printf("smoke: lkui1: %s\n", qPrintable(refused));
    d.check("lkui1: Closed refused, naming the frame the sender holds",
            !sys->lastRecall().value(QStringLiteral("taken")).toBool()
                && refused.startsWith(QLatin1String("Closed was refused: a scene is already scheduled on the sender for audio link frame "))
                && sys->recalledScene() == QLatin1String("Evening"));
    shoot(QStringLiteral("scene-refused"));

    // An edit while the installation changes on the hub: Save withdrawn, Reload offered
    const auto latencyB = [&] { return in(QStringLiteral("SystemWindow.latency.B")); };
    const auto save = [&] { return in(QStringLiteral("SystemWindow.save")); };
    const auto reload = [&] { return in(QStringLiteral("SystemWindow.reload")); };
    const auto staleNote = [&] { return in(QStringLiteral("SystemWindow.stale")); };
    d.check("lkui1: the editor's fields and buttons", latencyB() && save() && reload() && staleNote());
    if (!latencyB() || !save() || !reload() || !staleNote()) return;
    latencyB()->setProperty("text", QStringLiteral("150"));
    QMetaObject::invokeMethod(latencyB(), "accepted");
    d.check("lkui1: an edit makes a draft, Save offered", sys->dirty() && !sys->stale() && save()->isVisible() && save()->isEnabled());
    d.check("lkui1: another installer saves meanwhile",
            saveElsewhere(hub.url, R"({"units":[{"unitId":"C","measuredLatencyUs":90}]})"));
    d.check("lkui1: the draft is seen stale", fresh() && d.waitFor(5000, [&] { return sys->stale(); }));
    d.check("lkui1: Save withdrawn, the warning and Reload shown",
            !save()->isVisible() && staleNote()->isVisible() && reload()->property("text").toString() == QLatin1String("Reload")
                && reload()->isEnabled());
    shoot(QStringLiteral("save-stale"));
    d.check("lkui1: Reload drops the draft", click(reload()) && d.waitFor(5000, [&] { return !sys->dirty() && !sys->stale(); })
                                                 && latencyB()->property("text").toString().isEmpty());
    // A Save sent before the poll sees the change: the hub's conflict, the same outcome
    d.check("lkui1: a view after Reload", fresh());
    latencyB()->setProperty("text", QStringLiteral("160"));
    QMetaObject::invokeMethod(latencyB(), "accepted");
    d.check("lkui1: another installer saves again",
            saveElsewhere(hub.url, R"({"units":[{"unitId":"C","measuredLatencyUs":95}]})"));
    d.check("lkui1: Save sent stale is a conflict", click(save()) && d.waitFor(15000, [&] { return !sys->busy() && sys->stale(); })
                                                       && sys->actionFailed() && sys->actionMessage().contains(QLatin1String("Reload")));
    d.check("lkui1: Reload again", click(reload()) && d.waitFor(5000, [&] { return !sys->dirty(); }));
    // Saved for real
    d.check("lkui1: a view to edit", fresh());
    latencyB()->setProperty("text", QStringLiteral("170"));
    QMetaObject::invokeMethod(latencyB(), "accepted");
    const int before = sys->revision();
    d.check("lkui1: Save", click(save()) && d.waitFor(15000, [&] { return !sys->busy() && !sys->dirty(); }) && !sys->actionFailed());
    d.check("lkui1: the hub has it", fresh() && sys->revision() == before + 1
                                         && sys->unitLatencyText(QStringLiteral("B")) == QLatin1String("170"));

    // Simple: the status only
    const auto scenes = [&] { return in(QStringLiteral("SystemWindow.scenes")); };
    const auto editor = [&] { return in(QStringLiteral("SystemWindow.editor")); };
    modes->choose(QStringLiteral("simple"), false);
    d.spin(200);
    d.check("lkui1: Simple shows the status only", scenes() && editor() && !scenes()->isVisible() && !editor()->isVisible()
                                                       && rowA() && rowA()->isVisible());
    modes->choose(QStringLiteral("advanced"), false);
    d.spin(200);
    d.check("lkui1: Advanced has the scene bar and the editor again", scenes() && editor() && scenes()->isVisible() && editor()->isVisible());
    window->close();
    leave();
    hub.stop();
    d.check("lkui1: off the hub, no view and no status line", d.waitFor(5000, [&] { return !sys->available(); }) && strip
                                                                && !strip->isVisible());

    // ---- A PIN hub: Control, then Viewer ---------------------------------------
    const QString store = out + QStringLiteral("/hub-pin");
    UconsoleHub pinHub;
    d.check("lkui1: a PIN hub runs", pinHub.start(store, { QStringLiteral("--pair"), QStringLiteral("--forget-clients") }, true));
    link->connectToHub(pinHub.url);
    d.check("lkui1: the PIN hub asks for its PIN", settled() && d.waitFor(5000, [&] { return phaseIs("needs_pairing"); }));
    link->pair(pinHub.pin);
    d.check("lkui1: paired, DEMO-ALINK-B bound", settled() && bind(QStringLiteral("DEMO-ALINK-B")));
    d.check("lkui1: the last hub's recall is not shown on this one", sys->lastRecall().isEmpty() && sys->recalledScene().isEmpty());
    d.check("lkui1: Control on a PIN hub", d.waitFor(15000, [&] { return sys->available() && sys->role() == QLatin1String("control"); })
                                              && !sys->mayEdit() && sys->mayRecall());
    shell->trigger(QStringLiteral("window.system"));
    d.waitFor(3000, [&] { return window->isVisible(); });
    d.spin(400);
    const auto align = [&] { return in(QStringLiteral("SystemWindow.align")); };
    const auto editNote = [&] { return in(QStringLiteral("SystemWindow.editNote")); };
    d.check("lkui1: Control may recall, not edit",
            evening() && evening()->isEnabled() && align() && !align()->isEnabled() && !latencyB()->isEnabled() && editNote() && editNote()->isVisible());
    shoot(QStringLiteral("control-role"));

    // Made a Viewer: the hub stopped, the role changed on its store, the same hub again
    const int port = pinHub.port;
    pinHub.stop();
    QString listed;
    uconsole({ "hub", "clients", "--store", store, "--json" }, &listed);
    const QJsonArray clients = QJsonDocument::fromJson(listed.toUtf8())
                                   .object()
                                   .value(QLatin1String("clients"))
                                   .toArray();
    const QString cid = QString::number(clients.isEmpty() ? -1 : clients.first().toObject().value(QLatin1String("cid")).toInt());
    d.check("lkui1: one client, made a Viewer", clients.size() == 1
                                                   && uconsole({ "hub", "role", cid, "viewer", "--store", store }));
    UconsoleHub again;
    d.check("lkui1: the same hub again", again.start(store, {}, false, port));
    link->retryNow();
    d.check("lkui1: back as a Viewer", d.waitFor(30000, [&] {
        if (phaseIs("reconnecting")) link->retryNow();
        return phaseIs("bound") && sys->available() && sys->role() == QLatin1String("viewer");
    }));
    d.spin(300);
    const auto recallNote = [&] { return in(QStringLiteral("SystemWindow.recallNote")); };
    const auto delayAuto = [&] { return in(QStringLiteral("SystemWindow.delayAuto")); };
    d.check("lkui1: a Viewer is offered nothing that writes",
            !sys->mayEdit() && !sys->mayRecall() && evening() && !evening()->isEnabled() && closed() && !closed()->isEnabled()
                && align() && !align()->isEnabled() && !latencyB()->isEnabled() && delayAuto() && !delayAuto()->isEnabled()
                && save() && !save()->isEnabled() && recallNote() && recallNote()->isVisible());
    d.check("lkui1: and the reads stay", rowA() && rowA()->isVisible() && sys->installed());
    shoot(QStringLiteral("viewer-role"));
    window->close();
    leave();
    again.stop();

    // ---- -- WP HWLK1F -- a unit without the audio link (hw-lk1's K340) -----------
    // `--with-plain-unit`: unit P, a simulated DSPi whose firmware has no audio link
    UconsoleHub mixed;
    d.check("hwlk1f: a hub with a unit without the audio link runs",
            mixed.start(out + QStringLiteral("/hub-mixed"),
                        { QStringLiteral("--open"), QStringLiteral("--forget-clients"), QStringLiteral("--with-plain-unit") }));
    link->connectToHub(mixed.url);
    d.check("hwlk1f: on the hub, DEMO-ALINK-A bound", settled() && bind(QStringLiteral("DEMO-ALINK-A")));
    d.check("hwlk1f: the installation is read", d.waitFor(15000, [&] { return sys->available() && sys->installed() && sys->role() == QLatin1String("admin"); }));
    QVariantMap plain;
    for (const QVariant &v : sys->units())
        if (v.toMap().value(QStringLiteral("id")).toString() == QLatin1String("P")) plain = v.toMap();
    d.check("hwlk1f: P is said to have no audio link, not Unknown nor a Receiver",
            plain.value(QStringLiteral("stateWord")).toString() == QLatin1String("No audio link (firmware)")
                && plain.value(QStringLiteral("role")).toString() == QLatin1String("No audio link")
                && plain.value(QStringLiteral("connected")).toBool());
    std::printf("smoke: hwlk1f: %s\n", qPrintable(sys->summary()));
    d.check("hwlk1f: and left out of N of M locked",
            sys->summary().startsWith(
                QLatin1String("Audio link: 4 of 4 locked, USB-direct 2 of 2 locked, 1 without the audio link (firmware)")));
    shell->trigger(QStringLiteral("window.system"));
    d.waitFor(3000, [&] { return window->isVisible(); });
    d.spin(400);
    const auto rowP = [&] { return in(QStringLiteral("SystemWindow.unit.P")); };
    d.check("hwlk1f: P's row", rowP() && rowP()->property("shownName").toString() == QLatin1String("Plain unit, No audio link (firmware)"));
    d.check("hwlk1f: recall Evening", click(evening()) && d.waitFor(15000, [&] {
        return !sys->busy() && sys->lastRecall().value(QStringLiteral("name")).toString() == QLatin1String("Evening");
    }));
    const QString mixedRecall = result() ? result()->property("text").toString() : QString();
    std::printf("smoke: hwlk1f: %s\n", qPrintable(mixedRecall));
    d.check("hwlk1f: Evening applied now on P, at the frame on the others, not 'on time on every unit'",
            sys->lastRecall().value(QStringLiteral("taken")).toBool() && !sys->lastRecall().value(QStringLiteral("onTime")).toBool()
                && mixedRecall.startsWith(QLatin1String("Evening recalled: at USB frame "))
                && mixedRecall.contains(QLatin1String(" through the sender on the others; "))
                && mixedRecall.endsWith(QLatin1String("; applied now on Plain unit (no audio link).")));
    sys->align();
    d.check("hwlk1f: align says P was skipped and why", d.waitFor(20000, [&] {
        return !sys->busy() && sys->actionMessage().contains(QLatin1String("skipped on Plain unit (no audio link (firmware))"));
    }));
    std::printf("smoke: hwlk1f: %s\n", qPrintable(sys->actionMessage()));
    shoot(QStringLiteral("hwlk1f-plain-unit"));
    window->close();
    leave();
    mixed.stop();

    // ---- -- WP AUSB2 -- a USB-direct unit that can't lock -----------------------
    UconsoleHub locking;
    d.check("ausb2: a hub whose Rear L/R can't lock runs",
            locking.start(out + QStringLiteral("/hub-lock-error"),
                          { QStringLiteral("--open"), QStringLiteral("--forget-clients"), QStringLiteral("--with-usb-lock-error") }));
    link->connectToHub(locking.url);
    d.check("ausb2: on the hub, DEMO-ALINK-A bound", settled() && bind(QStringLiteral("DEMO-ALINK-A")));
    const auto unitR = [&] {
        for (const QVariant &v : sys->units())
            if (v.toMap().value(QStringLiteral("id")).toString() == QLatin1String("R")) return v.toMap();
        return QVariantMap();
    };
    const auto lockAlarm = [&](const char *doing) {
        for (const QVariant &v : sys->alarms()) {
            const QString m = v.toMap().value(QStringLiteral("message")).toString();
            if (m == QStringLiteral("USB-direct unit Rear L/R can't lock \u2014 %1 (the host's rate is out of range)").arg(QLatin1String(doing)))
                return true;
        }
        return false;
    };
    d.check("ausb2: Rear L/R is said to be muted, can't lock", d.waitFor(15000, [&] {
        return sys->available() && unitR().value(QStringLiteral("stateWord")).toString() == QLatin1String("Can't lock (USB): muted");
    }) && unitR().value(QStringLiteral("usbLockError")).toBool());
    d.check("ausb2: the hub's alarm is shown", lockAlarm("muted"));
    std::printf("smoke: ausb2: %s\n", qPrintable(sys->summary()));
    shell->trigger(QStringLiteral("window.system"));
    d.waitFor(3000, [&] { return window->isVisible(); });
    d.spin(400);
    const auto lockR = [&] { return in(QStringLiteral("SystemWindow.lockError.R")); };
    d.check("ausb2: the row says why",
            lockR() && lockR()->isVisible()
                && lockR()->property("text").toString() == QLatin1String("Can't lock: the host's rate is out of range"));
    shoot(QStringLiteral("ausb2-lock-error-muted"));
    // Installer: Rear L/R plays unaligned, the USB-direct units kept aligned
    const auto actionR = [&] { return in(QStringLiteral("SystemWindow.onLockError.R")); };
    const auto keep = [&] { return in(QStringLiteral("SystemWindow.keepUsbAligned")); };
    d.check("ausb2: the Installer's USB-direct settings",
            actionR() && actionR()->isVisible() && actionR()->isEnabled() && keep() && keep()->isEnabled()
                && in(QStringLiteral("SystemWindow.usbMode.H")) && actionR()->property("currentIndex").toInt() == 0);
    if (actionR() && keep()) {
        actionR()->setProperty("currentIndex", 1);
        QMetaObject::invokeMethod(actionR(), "activated", Q_ARG(int, 1));
        keep()->setProperty("checked", true);
        QMetaObject::invokeMethod(keep(), "toggled");
    }
    d.check("ausb2: a draft of both", sys->dirty() && sys->onLockErrorOf(QStringLiteral("R")) == QLatin1String("play_unaligned")
                                          && sys->keepUsbAligned());
    d.check("ausb2: saved", click(save()) && d.waitFor(15000, [&] { return !sys->busy() && !sys->dirty(); }) && !sys->actionFailed());
    // The hub's own poll (every 5 s) writes it to the unit, which then plays unaligned
    d.check("ausb2: written to the unit by the hub", d.waitFor(30000, [&] {
        fresh();
        return unitR().value(QStringLiteral("usbOnLockError")).toString() == QLatin1String("play_unaligned")
               && unitR().value(QStringLiteral("stateWord")).toString() == QLatin1String("Can't lock (USB): playing unaligned");
    }));
    d.check("ausb2: kept aligned, and the alarm says it plays", sys->keepUsbAligned() && lockAlarm("playing unaligned"));
    shoot(QStringLiteral("ausb2-lock-error-playing"));
    window->close();
    leave();
    locking.stop();
    modes->choose(modeBefore, false);
    if (demoBefore && !device->connected()) device->openDemo();
    d.waitFor(10000, [&] { return !demoBefore || device->connected(); });
    d.waitIdle();
}

} // namespace

const SmokeDriver::Section alinkSystem("lkui1-system", 133, &alinkSystemSection);
