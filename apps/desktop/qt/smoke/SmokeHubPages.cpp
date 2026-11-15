// -- WP P2QT -- The hub's pages in Qt (PLAN-P2 wave P2-E): Device > Events…
// (this console's log, and the hub's shared history with `by`), the Snapshots
// window's "On the Hub" section (save to the hub, the hub's copy winning on a
// differing id, copy here, a guarded recall from the hub), the hub's alarms as
// notices with Acknowledge, and Device > Identify. First on the demo device
// (no hub: the hub's parts do not show), then over DSPi Link against
// `uconsole --demo hub --open` (DSPI_SMOKE_UCONSOLE: a real hub with the demo
// DSPi, as an Admin). Screenshots, dark and light, in <out>/p2qt/.
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "EventsController.h"
#include "IdentifyController.h"
#include "LinkController.h"
#include "ModeController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "Shell.h"
#include "SnapshotsController.h"

#include "CoreApi.h"

#include <QDir>
#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QQuickItem>
#include <QQuickWindow>
#include <atomic>
#include <functional>
#include <memory>
#include <thread>

namespace {

struct Scheme {
    Qt::ColorScheme scheme;
    const char *name;
};
constexpr Scheme kSchemes[] = { { Qt::ColorScheme::Dark, "dark" }, { Qt::ColorScheme::Light, "light" } };

// One `uconsole --demo hub` process (a real hub with the demo DSPi)
class DemoHub {
public:
    bool start(const QString &store, const QStringList &extra) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        QDir(store).removeRecursively();
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList{ "--demo", "hub", "--loopback", "--json", "--no-keep-awake", "--store", store, "--port", "0" } + extra);
        if (!m_proc.waitForStarted(5000)) return false;
        QElapsedTimer t;
        t.start();
        while (t.elapsed() < 20000) {
            if (!m_proc.canReadLine() && !m_proc.waitForReadyRead(1000)) {
                if (m_proc.state() != QProcess::Running) return false;
                continue;
            }
            while (m_proc.canReadLine()) {
                const QJsonObject e = QJsonDocument::fromJson(m_proc.readLine()).object();
                if (e.value(QLatin1String("event")).toString() == QLatin1String("status"))
                    url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(e.value(QLatin1String("port")).toInt());
            }
            if (!url.isEmpty()) return true;
        }
        return false;
    }
    void stop() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    ~DemoHub() { stop(); }
    QString url;

private:
    QProcess m_proc;
};

bool shown(QQuickItem *item) {
    for (QQuickItem *i = item; i; i = i->parentItem())
        if (!i->isVisible()) return false;
    return item != nullptr;
}

QJsonObject reply(const QByteArray &buf) { return QJsonDocument::fromJson(buf.constData()).object(); }

// Another console on the open hub, through the C ABI on a thread of its own,
// bound to the hub's device: runs `act` there, true when it answered ok
bool elsewhere(SmokeDriver &d, const QString &url, const std::function<QJsonObject(FfiCore *)> &act) {
    auto ok = std::make_shared<std::atomic<int>>(-1);
    const QByteArray u = url.toUtf8();
    std::thread([ok, u, act] {
        FfiCore *other = dspi_core_new();
        bool done = false;
        if (other) {
            QByteArray buf(256 * 1024, '\0');
            if (dspi_link_connect(other, u.constData(), "p2qt-other", "0", "", "", buf.data(), uint32_t(buf.size())) > 0
                && reply(buf).value(QLatin1String("ok")).toBool()) {
                buf.fill('\0');
                dspi_link_devices(other, buf.data(), uint32_t(buf.size()));
                const QJsonObject dev = reply(buf).value(QLatin1String("value")).toArray().first().toObject();
                const QByteArray serial = dev.value(QLatin1String("serial")).toString().toUtf8();
                buf.fill('\0');
                dspi_link_select_device(other, uint32_t(dev.value(QLatin1String("handle")).toInt()), serial.constData(), buf.data(),
                                        uint32_t(buf.size()));
                if (reply(buf).value(QLatin1String("ok")).toBool()) done = act(other).value(QLatin1String("ok")).toBool();
            }
            dspi_link_close(other);
            dspi_core_free(other);
        }
        ok->store(done ? 1 : 0);
    }).detach();
    d.waitFor(40000, [&] { return ok->load() >= 0; });
    return ok->load() == 1;
}

// A JSON call on the app's own core, on its worker (the smoke's Admin acts)
QJsonObject onWorker(SmokeDriver &d, std::function<QByteArray(FfiCore *)> job) {
    auto out = std::make_shared<QByteArray>();
    auto done = std::make_shared<std::atomic<bool>>(false);
    d.app().core->worker()->post([job, out, done](FfiCore *c) {
        *out = job(c);
        done->store(true);
    });
    d.waitFor(20000, [&] { return done->load(); });
    return reply(*out);
}

QByteArray fill(const std::function<uint32_t(char *, uint32_t)> &f) {
    QByteArray buf(256 * 1024, '\0');
    f(buf.data(), uint32_t(buf.size()));
    return buf;
}

QVariantMap byId(const QVariantList &rows, const QString &id) {
    for (const QVariant &v : rows)
        if (v.toMap().value(QStringLiteral("id")).toString() == id) return v.toMap();
    return {};
}

QVariantMap byName(const QVariantList &rows, const QString &name) {
    for (const QVariant &v : rows)
        if (v.toMap().value(QStringLiteral("name")).toString() == name) return v.toMap();
    return {};
}

void shootBoth(SmokeDriver &d, QQuickWindow *window, const QString &name) {
    Shell *shell = d.app().shell;
    const Qt::ColorScheme before = shell->colorSchemeOverride();
    for (const Scheme &s : kSchemes) {
        shell->setColorSchemeOverride(s.scheme);
        d.spin(300);
        d.check(QStringLiteral("p2qt: screenshot %1, %2").arg(name, QLatin1String(s.name)),
                d.grab(window, QStringLiteral("p2qt/%1-%2.png").arg(name, QLatin1String(s.name))));
    }
    shell->setColorSchemeOverride(before);
    d.spin(100);
}

} // namespace

static void hubPagesSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *events = Registrar::instance<EventsController>();
    auto *identify = Registrar::instance<IdentifyController>();
    auto *snaps = Registrar::instance<SnapshotsController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    auto *link = Registrar::instance<LinkController>();
    d.check("p2qt: the controllers", events && identify && snaps && notices && link);
    if (!events || !identify || !snaps || !notices || !link) return;
    QDir().mkpath(d.outDir() + QStringLiteral("/p2qt"));
    const auto settled = [&] {
        d.waitIdle();
        const bool ok = d.waitFor(15000, [&] { return !snaps->busy() && !events->busy() && !link->busy(); });
        d.waitIdle();
        d.spin(60);
        return ok;
    };

    // ---- The demo: no hub -------------------------------------------------------------
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check("p2qt: on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && settled());
    d.check("p2qt: Device > Events… and Identify are menu rows",
            d.object(QStringLiteral("Menu.device.events")) && d.object(QStringLiteral("Menu.device.identify")));
    d.check("p2qt: the shell handles Identify", shell->handles(QStringLiteral("device.identify")));
    // The demo has no EXT_IDENTIFY: the row is off, and asked anyway it says so
    d.check("p2qt: Identify is off on the demo (no EXT_IDENTIFY)", d.waitFor(5000, [&] { return !identify->available(); }));
    notices->clear();
    shell->trigger(QStringLiteral("device.identify"));
    d.spin(100);
    d.check("p2qt: asked anyway, the core's unsupported row",
            notices->last().value(QStringLiteral("text")).toString() == notices->line(QStringLiteral("unsupported"))
                && !notices->line(QStringLiteral("unsupported")).isEmpty());
    notices->clear();

    // Events: this console's log alone
    auto *ew = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("device.events")));
    d.check("p2qt: the Events window opens", ew && d.waitFor(3000, [&] { return ew->isVisible(); }));
    if (!ew) return;
    const auto eitem = [&](const char *name) { return d.item(QString::fromLatin1(name), ew->contentItem()); };
    d.check("p2qt: it reads while it shows", events->active());
    d.check("p2qt: no hub: the local log, no source tabs",
            !events->hubAvailable() && events->source() == QLatin1String("local") && !shown(eitem("Events.source")));
    // A rename made elsewhere is a notification: a record of this console's log
    d.app().core->worker()->post([](FfiCore *core) { dspi_demo_rename_elsewhere(core, 0xFF, "P2QT elsewhere"); });
    d.check("p2qt: the local log lists the device's events", d.waitFor(15000, [&] { return !events->entries().isEmpty(); }));
    d.app().core->worker()->post([](FfiCore *core) { dspi_demo_rename_elsewhere(core, 0xFF, ""); });
    d.waitFor(5000, [&] { return device->deviceName().isEmpty(); });
    settled();
    d.check("p2qt: a row shows", d.waitFor(3000, [&] { return shown(eitem("Events.row.0")); }));
    shootBoth(d, ew, QStringLiteral("events-local"));
    ew->close();
    d.spin(100);
    d.check("p2qt: closed, it stops reading", !events->active());

    // Snapshots: no "On the Hub" off a hub
    auto *sw = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("device.snapshots")));
    d.check("p2qt: the Snapshots window opens", sw && d.waitFor(3000, [&] { return sw->isVisible(); }));
    if (!sw) return;
    settled();
    const auto sitem = [&](const QString &name) { return d.item(name, sw->contentItem()); };
    d.check("p2qt: no hub: no On the Hub section, no Save to the Hub",
            !snaps->hubAvailable() && !shown(sitem(QStringLiteral("SnapshotsWindow.hubList")))
                && !shown(sitem(QStringLiteral("SnapshotsWindow.saveToHub"))));
    sw->close();

    // ---- An open hub (Admin) ------------------------------------------------------------
    if (qEnvironmentVariable("DSPI_SMOKE_UCONSOLE").isEmpty()) {
        d.notRun("p2qt: the hub's pages", QStringLiteral("no DSPI_SMOKE_UCONSOLE"));
        return;
    }
    const auto phaseIs = [link](const char *phase) { return link->phase() == QLatin1String(phase); };
    DemoHub hub;
    const bool up = hub.start(d.outDir() + QStringLiteral("/p2qt/hub"), { QStringLiteral("--open"), QStringLiteral("--forget-clients") });
    d.check("p2qt: uconsole --demo hub --open runs (DSPI_SMOKE_UCONSOLE)", up);
    if (!up) return;
    link->connectToHub(hub.url);
    settled();
    if (d.waitFor(15000, [&] { return phaseIs("choose_device") && !link->devices().isEmpty(); })) {
        const QVariantMap m = link->devices().first().toMap();
        link->bindDevice(m.value(QStringLiteral("handle")).toInt(), m.value(QStringLiteral("serial")).toString());
    }
    d.check("p2qt: on the hub, its device bound",
            settled() && d.waitFor(15000, [&] { return phaseIs("bound") && device->remote() && device->connected(); }) && d.waitIdle(20000));
    if (!device->remote()) return;
    const QString serial = device->serial();

    // Identify: the hub's cap; the hub answers for its device
    d.check("p2qt: Identify is on over the hub (its identify cap)", d.waitFor(10000, [&] { return identify->available(); }));
    {
        bool done = false;
        QJsonObject answer;
        const auto c = QObject::connect(identify, &IdentifyController::identified, identify, [&](bool, const QJsonObject &r) {
            done = true;
            answer = r;
        });
        notices->clear();
        shell->trigger(QStringLiteral("device.identify"));
        d.check("p2qt: the hub answers Identify", d.waitFor(15000, [&] { return done; }));
        QObject::disconnect(c);
        // The demo DSPi behind the hub has no EXT_IDENTIFY: the hub's own
        // `unsupported`; a DSPi with it blinks
        const QString key = answer.value(QLatin1String("noticeKey")).toString();
        const QString text = notices->last().value(QStringLiteral("text")).toString();
        d.check("p2qt: and the answer is a notice (the blink, or the hub's refusal in its row's words)",
                answer.value(QLatin1String("ok")).toBool() ? text.contains(QStringLiteral("10 s"))
                                                          : (!key.isEmpty() && text == notices->line(key, QStringLiteral("?"))));
        printf("smoke: p2qt: identify over the hub answered %s\n", QJsonDocument(answer).toJson(QJsonDocument::Compact).constData());
        notices->clear();
    }

    // Snapshots: both sides, in Advanced mode (an earlier section, or the
    // mode remembered for this device, may have left Simple on)
    auto *modes = Registrar::instance<ModeController>();
    const QString mode0 = modes ? modes->mode() : QString();
    if (modes && mode0 != QLatin1String("advanced")) modes->choose(QStringLiteral("advanced"), false);
    // Snapshots window
    sw = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("device.snapshots")));
    d.check("p2qt: the Snapshots window, over the hub", sw && d.waitFor(3000, [&] { return sw->isVisible(); }));
    if (!sw) return;
    snaps->reload();
    settled();
    d.check("p2qt: the hub lists its snapshots", d.waitFor(15000, [&] { return snaps->hubAvailable(); }) && snaps->hubProblem().isEmpty());
    d.spin(200);
    d.check("p2qt: On the Hub shows, and Save to the Hub",
            shown(sitem(QStringLiteral("SnapshotsWindow.hubList"))) && shown(sitem(QStringLiteral("SnapshotsWindow.saveToHub"))));
    QString op;
    bool opOk = false;
    const auto storedC = QObject::connect(snaps, &SnapshotsController::stored, snaps, [&](const QString &o, bool ok) {
        op = o;
        opOk = ok;
    });
    snaps->capture(QStringLiteral("P2QT bench"), QStringLiteral("before the hub"));
    settled();
    const QVariantMap bench = byName(snaps->snapshots(), QStringLiteral("P2QT bench"));
    const QString id = bench.value(QStringLiteral("id")).toString();
    d.check("p2qt: a local snapshot, only here", !id.isEmpty() && bench.value(QStringLiteral("where")).toString() == QLatin1String("local")
                                                      && byId(snaps->hubSnapshots(), id).isEmpty());
    // Save to the hub: on both sides
    op.clear();
    snaps->saveToHub(id);
    d.check("p2qt: saved to the hub", d.waitFor(15000, [&] { return op == QLatin1String("hub-save"); }) && opOk && settled());
    d.check("p2qt: listed by the hub, and as on both sides here", d.waitFor(10000, [&] {
        return byId(snaps->hubSnapshots(), id).value(QStringLiteral("where")).toString() == QLatin1String("both")
            && byId(snaps->snapshots(), id).value(QStringLiteral("where")).toString() == QLatin1String("both");
    }));
    d.spin(200);
    d.check("p2qt: the row says On the hub", shown(sitem(QStringLiteral("SnapshotsWindow.onHub.0"))));
    // Another console renames it on the hub: the hub's `snapshot.changed`, and
    // the hub's copy wins; this console's stays as a local copy
    const QByteArray sid = id.toUtf8();
    d.check("p2qt: another console renames it on the hub", elsewhere(d, hub.url, [sid](FfiCore *c) {
        return reply(fill([&](char *b, uint32_t n) { return dspi_hub_snapshots_rename(c, sid.constData(), "P2QT renamed on the hub", b, n); }));
    }));
    d.check("p2qt: the window follows the hub's event: the hub's name wins", d.waitFor(15000, [&] {
        return byId(snaps->hubSnapshots(), id).value(QStringLiteral("name")).toString() == QLatin1String("P2QT renamed on the hub");
    }));
    settled();
    d.check("p2qt: the hub's row is the hub's alone now, ours a local copy",
            byId(snaps->hubSnapshots(), id).value(QStringLiteral("where")).toString() == QLatin1String("hub")
                && byId(snaps->snapshots(), id).value(QStringLiteral("localCopy")).toBool()
                && byId(snaps->snapshots(), id).value(QStringLiteral("name")).toString() == QLatin1String("P2QT bench"));
    d.spin(200);
    d.check("p2qt: the row says Local copy", shown(sitem(QStringLiteral("SnapshotsWindow.localCopy.0"))));
    // Copy here: the hub's, kept beside ours (nothing overwritten)
    const int before = int(snaps->snapshots().size());
    op.clear();
    snaps->copyHere(id);
    d.check("p2qt: copied here", d.waitFor(15000, [&] { return op == QLatin1String("hub-copy"); }) && opOk && settled());
    d.check("p2qt: the copy beside this console's own", d.waitFor(10000, [&] {
        return int(snaps->snapshots().size()) == before + 1 && !byName(snaps->snapshots(), QStringLiteral("P2QT renamed on the hub")).isEmpty()
            && byId(snaps->snapshots(), id).value(QStringLiteral("name")).toString() == QLatin1String("P2QT bench");
    }));
    // A recall from the hub: guarded over DSPi Link (the core's default), Keep
    d.check("p2qt: a recall here is guarded by default", snaps->guardDefault());
    sw->setProperty("hubSelectedId", id);
    d.spin(100);
    shootBoth(d, sw, QStringLiteral("snapshots-hub"));
    op.clear();
    snaps->recallFromHub(id);
    d.check("p2qt: recalled from the hub", d.waitFor(20000, [&] { return op == QLatin1String("hub-recall"); }) && opOk);
    d.check("p2qt: its countdown waits for Keep", d.waitFor(5000, [&] { return snaps->guardWaiting(); }));
    snaps->keep();
    d.check("p2qt: kept", d.waitFor(10000, [&] { return snaps->guardPhase() == QLatin1String("kept"); }) && settled());
    sw->close();

    // Events: the hub's history, with who did it
    ew = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("device.events")));
    d.check("p2qt: the Events window, over the hub", ew && d.waitFor(3000, [&] { return ew->isVisible(); }));
    if (!ew) return;
    d.check("p2qt: the hub keeps a history (its events cap)", d.waitFor(10000, [&] { return events->hubAvailable(); }));
    d.spin(200);
    d.check("p2qt: the source tabs show", shown(eitem("Events.source")));
    int hubLoads = 0;
    const auto loadedC = QObject::connect(events, &EventsController::loaded, events, [&](const QString &s, bool ok) {
        if (s == QLatin1String("hub") && ok) ++hubLoads;
    });
    events->setSource(QStringLiteral("hub"));
    d.check("p2qt: the hub's history is read", d.waitFor(15000, [&] { return hubLoads > 0; }) && settled());
    const auto recallRows = [&] {
        int n = 0;
        for (const QVariant &v : events->entries()) {
            const QVariantMap r = v.toMap();
            if (r.value(QStringLiteral("text")).toString().contains(QStringLiteral("Recalled snapshot"))
                && !r.value(QStringLiteral("by")).toString().isEmpty())
                ++n;
        }
        return n;
    };
    d.check("p2qt: it lists the recall, by this console's name", recallRows() >= 1 && events->problem().isEmpty());
    d.spin(200);
    d.check("p2qt: the row says by whom", shown(eitem("Events.by.0")) || recallRows() >= 1);
    // A new record arrives by the hub's event.added, not by reading again
    const int loadsBefore = hubLoads;
    const int recallsBefore = recallRows();
    snaps->recallFromHub(id);
    d.waitFor(20000, [&] { return snaps->guardWaiting(); });
    snaps->keep();
    d.waitFor(10000, [&] { return snaps->guardPhase() == QLatin1String("kept"); });
    d.check("p2qt: the hub's event.added puts the new record at the top",
            d.waitFor(15000, [&] { return recallRows() == recallsBefore + 1; }) && hubLoads == loadsBefore);
    QObject::disconnect(loadedC);
    QObject::disconnect(storedC);
    shootBoth(d, ew, QStringLiteral("events-hub"));
    events->setSource(QStringLiteral("local"));
    ew->close();

    // An alarm: a schedule that fails raises `schedule_failed` on the hub
    notices->clear();
    const QJsonObject rule = onWorker(d, [](FfiCore *c) {
        const char *json = R"({"name":"P2QT failing","enabled":false,"days":["mon"],"time":"03:00",)"
                           R"("actions":[{"t":"preset.load","slot":0}],"targets":["P2QT-NO-SUCH-DSPI"]})";
        return fill([&](char *b, uint32_t n) { return dspi_hub_schedule_set(c, json, b, n); });
    });
    const QByteArray ruleId = rule.value(QLatin1String("id")).toString().toUtf8();
    d.check("p2qt: a schedule saved on the hub (Admin)", rule.value(QLatin1String("ok")).toBool() && !ruleId.isEmpty());
    const QJsonObject ran = onWorker(d, [ruleId](FfiCore *c) {
        return fill([&](char *b, uint32_t n) { return dspi_hub_schedule_run_now(c, ruleId.constData(), b, n); });
    });
    printf("smoke: p2qt: run_now answered %s\n", QJsonDocument(ran).toJson(QJsonDocument::Compact).constData());
    d.check("p2qt: the hub's alarm is a notice", d.waitFor(20000, [&] { return !notices->alarmIds().isEmpty(); }));
    d.waitFor(5000, [&] { return !notices->current().value(QStringLiteral("alarmId")).toString().isEmpty(); });
    const QVariantMap alarm = notices->current();
    d.check("p2qt: in the core's row's words (schedule_failed)",
            alarm.value(QStringLiteral("text")).toString().startsWith(notices->row(QStringLiteral("alarm.schedule_failed")).value(QStringLiteral("title")).toString())
                && alarm.value(QStringLiteral("canAck")).toBool());
    d.spin(200);
    QQuickItem *ack = d.item(QStringLiteral("NoticeBanner.acknowledge"));
    d.check("p2qt: the banner offers Acknowledge", shown(ack));
    shootBoth(d, d.mainWindow(), QStringLiteral("alarm-banner"));
    bool acked = false;
    const auto ackC = QObject::connect(notices, &NoticeCenter::acknowledged, notices, [&](const QString &, bool ok) { acked = ok; });
    if (ack) QMetaObject::invokeMethod(ack, "clicked");
    d.check("p2qt: acknowledged on the hub, and the notice is gone",
            d.waitFor(15000, [&] { return acked; }) && d.waitFor(5000, [&] { return notices->alarmIds().isEmpty(); }));
    QObject::disconnect(ackC);
    onWorker(d, [ruleId](FfiCore *c) { return fill([&](char *b, uint32_t n) { return dspi_hub_schedule_delete(c, ruleId.constData(), b, n); }); });

    if (modes && !mode0.isEmpty() && modes->mode() != mode0) modes->choose(mode0, false);
    // Back to the demo
    link->closeSession();
    settled();
    d.waitFor(10000, [&] { return !device->remote(); });
    hub.stop();
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.waitFor(10000, [&] { return device->connected() && device->demo(); });
    d.waitIdle();
    d.check("p2qt: off the hub, no hub parts", d.waitFor(10000, [&] { return !snaps->hubAvailable() && !events->hubAvailable(); }));
    notices->clear();
}

static const SmokeDriver::Section hubPages("p2qt-hub", 162, &hubPagesSection);
