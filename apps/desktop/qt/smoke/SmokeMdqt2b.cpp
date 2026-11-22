// -- WP MDQT2b -- The library, state and safety controllers with several open
// devices: two demos (and, for the System panel, a hub's unit beside them).
// The history and the A/B compare are each device's own; a safe apply left
// waiting on a device switched away from is driven there and reverts at its
// timeout; closing a device asks about that device's unsaved changes, and
// quitting about every open one; a firmware update or a calibration pins its
// device; the lock state, the Device menu's questions and the System panel's
// two tabs follow the active device.
#include "SmokeDriver.h"

#include "CoreVoltageController.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "DevicesController.h"
#include "FilesController.h"
#include "FirmwareController.h"
#include "HistoryController.h"
#include "InstallationController.h"
#include "LimitsController.h"
#include "LinkController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"
#include "SnapshotsController.h"
#include "SystemController.h"
#include "UnsavedGuard.h"

extern "C" {
#include "dspi_core.h"
}

#include <QDir>
#include <QElapsedTimer>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPointer>
#include <QProcess>
#include <QQuickItem>
#include <QSignalSpy>
#include <cmath>
#include <cstdio>
#include <memory>

namespace {

constexpr const char *kPin = "2468";

float volumeOf(const DeviceSession *s) {
    const DspState *state = s->model()->state();
    return state ? state->master_volume_db : NAN;
}

float gainOf(const DeviceSession *s) {
    const DspState *state = s->model()->state();
    return state ? state->output_gain_db[0] : NAN;
}

bool near(float a, float b) { return std::fabs(a - b) < 0.05f; }

QObject *find(QQuickItem *host, const char *name) { return host ? host->findChild<QObject *>(QLatin1String(name)) : nullptr; }
bool opened(QObject *popup) { return popup && popup->property("opened").toBool(); }
void finish(QObject *popup, const char *answer) {
    if (popup) QMetaObject::invokeMethod(popup, "finish", Q_ARG(QVariant, QString::fromLatin1(answer)));
}

// Until `s`'s worker has nothing queued or running
bool quiet(SmokeDriver &d, DeviceSession *s, int ms = 5000) {
    return d.waitFor(ms, [s] { return s->worker()->pending() == 0; });
}

// A master volume write on whichever device is active now
bool setVolume(SmokeDriver &d, float db) {
    bool done = false;
    d.app().device->write(QStringLiteral("master volume"), [db](FfiCore *c) { return dspi_set_master_volume(c, db); },
                          [&done](bool ok) { done = ok; });
    return d.waitFor(5000, [&] { return done; }) && d.waitIdle();
}

// Output 1's gain on the active device: a preset change (unsaved)
bool setGain(SmokeDriver &d, float db) {
    bool done = false;
    d.app().device->write(QStringLiteral("mdqt2b gain"), [db](FfiCore *c) { return dspi_set_output_gain(c, 0, db); },
                          [&done](bool ok) { done = ok; });
    return d.waitFor(5000, [&] { return done; }) && d.waitIdle();
}

QString idNamed(const SnapshotsController *snaps, const QString &name) {
    for (const QVariant &v : snaps->snapshots())
        if (v.toMap().value(QStringLiteral("name")).toString() == name) return v.toMap().value(QStringLiteral("id")).toString();
    return {};
}

// One `uconsole --demo hub --alink` (as instq-installation's)
class AlinkHub {
public:
    bool start(const QString &store) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList{ "--demo", "hub", "--alink", "--loopback", "--json", "--no-keep-awake", "--forget-clients",
                                          "--store", store, "--port", "0", "--open" });
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
                if (e.value(QLatin1String("event")).toString() == QLatin1String("status")) {
                    url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(e.value(QLatin1String("port")).toInt());
                    return true;
                }
            }
        }
        return false;
    }
    ~AlinkHub() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    QString url;

private:
    QProcess m_proc;
};

void rebindSection(SmokeDriver &d) {
    DeviceRegistry *r = DeviceRegistry::instance();
    DeviceModel *device = d.app().device;
    auto *history = Registrar::instance<HistoryController>();
    auto *snaps = Registrar::instance<SnapshotsController>();
    auto *guard = Registrar::instance<UnsavedGuard>();
    auto *list = Registrar::instance<DevicesController>();
    auto *firmware = Registrar::instance<FirmwareController>();
    auto *voltage = Registrar::instance<CoreVoltageController>();
    auto *limits = Registrar::instance<LimitsController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    auto *files = Registrar::instance<FilesController>();
    d.check("mdqt2b: the controllers", r && history && snaps && guard && list && firmware && voltage && limits && notices && files);
    if (!r || !history || !snaps || !guard || !list || !firmware || !voltage || !limits || !notices || !files) return;
    d.allowSectionTime(240);
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.waitIdle();
    DeviceSession *a = r->mainSession();
    r->openDemo();
    d.check("mdqt2b: a second demo beside the first",
            d.waitFor(10000, [&] { return r->opening() == 0 && r->sessions().size() == 2; }) && a->model()->connected());
    if (r->sessions().size() != 2) return;
    DeviceSession *b = r->sessions().at(1);
    d.waitIdle();

    // ---- The history is each device's own
    r->activate(a);
    d.check("mdqt2b: a change on the first", setVolume(d, -41.0f));
    d.check("mdqt2b: its history lists it", d.waitFor(3000, [&] { return history->canUndo() && !history->entries().isEmpty(); }));
    const QVariantList historyA = history->entries();
    r->activate(b);
    d.check("mdqt2b: the second shows its own history", d.waitFor(3000, [&] { return history->entries() != historyA; }));
    d.check("mdqt2b: two changes on the second", setVolume(d, -37.0f) && setVolume(d, -36.0f));
    d.check("mdqt2b: listed there", d.waitFor(3000, [&] {
        return history->entries().size() >= 2 && history->entries() != historyA;
    }));
    const QVariantList historyB = history->entries();
    r->activate(a);
    d.check("mdqt2b: back on the first, its history as it was", d.waitFor(3000, [&] { return history->entries() == historyA; }));
    history->undo();
    d.check("mdqt2b: Undo undoes the first's change, on the first",
            d.waitFor(5000, [&] { return !near(volumeOf(a), -41.0f); }) && d.waitIdle() && near(volumeOf(b), -36.0f));
    r->activate(b);
    d.check("mdqt2b: the second's history is untouched", d.waitFor(3000, [&] { return history->entries() == historyB; }));

    // ---- The A/B compare is each device's own
    r->activate(a);
    QSignalSpy stored(snaps, &SnapshotsController::stored);
    const auto capture = [&](const QString &name) {
        stored.clear();
        snaps->capture(name, QString());
        return d.waitFor(5000, [&] { return !stored.isEmpty() && !idNamed(snaps, name).isEmpty(); });
    };
    const float gainA = gainOf(a);
    const float gainB = gainA > -10.0f ? gainA - 3.0f : gainA + 3.0f;
    d.check("mdqt2b: two snapshots of the first", capture(QStringLiteral("MDQT2b A")) && setGain(d, gainB) &&
                                                     capture(QStringLiteral("MDQT2b B")));
    const QString idA = idNamed(snaps, QStringLiteral("MDQT2b A"));
    const QString idB = idNamed(snaps, QStringLiteral("MDQT2b B"));
    snaps->abStart(QString(), idB);
    d.check("mdqt2b: a compare runs on the first", d.waitFor(5000, [&] { return snaps->abActive(); }));
    r->activate(b);
    d.check("mdqt2b: none on the second", d.waitFor(3000, [&] { return !snaps->abActive(); }) && d.waitIdle() && !snaps->abActive());
    r->activate(a);
    d.check("mdqt2b: back on the first, its compare again",
            d.waitFor(3000, [&] { return snaps->abActive() && snaps->abBName() == QLatin1String("MDQT2b B"); }));
    snaps->abKeep(QStringLiteral("a"));
    d.check("mdqt2b: the compare ends", d.waitFor(5000, [&] { return !snaps->abActive(); }) && d.waitIdle());

    // ---- A safe apply left waiting is driven on its own device
    snaps->setRollback(true);
    snaps->setGuardTimeoutS(SnapshotsController::kGuardTimeoutMinS);
    snaps->recall(idA);
    d.check("mdqt2b: a recall with rollback waits on the first",
            d.waitFor(8000, [&] { return snaps->guardWaiting(); }) && d.waitFor(3000, [&] { return near(gainOf(a), gainA); }));
    const int noticesBefore = notices->serial();
    r->activate(b);
    d.check("mdqt2b: the second has no countdown", d.waitFor(3000, [&] { return !snaps->guardWaiting(); }));
    d.check("mdqt2b: the first is named as waiting", snaps->guardsElsewhere() == QStringList{ a->title() } && snaps->guardWaitingOn(a));
    d.check("mdqt2b: at its timeout it reverts there", d.waitFor(15000, [&] { return snaps->guardsElsewhere().isEmpty(); }) &&
                                                          quiet(d, a) && d.waitFor(3000, [&] { return near(gainOf(a), gainB); }));
    d.check("mdqt2b: said with the device's name", d.waitFor(3000, [&] {
        return notices->serial() > noticesBefore && notices->last().value(QStringLiteral("text")).toString().startsWith(a->title());
    }));
    d.check("mdqt2b: the second is as it was", near(volumeOf(b), -36.0f) && !snaps->guardWaiting());
    snaps->setRollback(false);
    snaps->setGuardTimeoutS(SnapshotsController::kGuardTimeoutDefaultS);
    r->activate(a);
    d.waitIdle();
    for (const QString &id : { idA, idB }) {
        stored.clear();
        snaps->remove(id);
        d.waitFor(3000, [&] { return !stored.isEmpty(); });
    }

    // ---- The lock state follows the active device
    r->activate(b);
    d.waitFor(3000, [&] { return !limits->busy(); });
    limits->setPin(QString::fromLatin1(kPin), QString::fromLatin1(kPin));
    d.check("mdqt2b: a PIN on the second", d.waitFor(5000, [&] { return !limits->busy() && limits->pinSet(); }));
    limits->dismissRecovery();
    r->activate(a);
    d.check("mdqt2b: the first has none", d.waitFor(3000, [&] { return !limits->pinSet(); }) && d.waitIdle() && !limits->pinSet());
    r->activate(b);
    d.check("mdqt2b: back on the second, its PIN", d.waitFor(3000, [&] { return limits->pinSet(); }));
    limits->clearPin();
    d.check("mdqt2b: cleared there", d.waitFor(5000, [&] { return !limits->busy() && !limits->pinSet(); }));

    // ---- A Device menu question goes to the device it was asked about
    r->activate(b);
    d.waitIdle();
    QObject *deviceQuestion = find(files->host(), "FilesHost.deviceQuestion");
    d.app().shell->trigger(QStringLiteral("device.revert"));
    d.check("mdqt2b: Revert to Saved asks", d.waitFor(2000, [&] { return opened(deviceQuestion); }));
    r->activate(a);
    const int writesA = a->model()->settersRun(), writesB = b->model()->settersRun();
    finish(deviceQuestion, "accept");
    d.waitIdle();
    d.check("mdqt2b: answered after a switch, nothing is written anywhere",
            quiet(d, b) && a->model()->settersRun() == writesA && b->model()->settersRun() == writesB);
    // -- WP INT3 -- and says so in Files' notice, which is modal: closed here, or
    // the next section's mouse lands on its dimmer (mdqt2a's held slider)
    QObject *filesNotice = find(files->host(), "FilesHost.notice");
    d.check("mdqt2b: the notice says another device is shown", d.waitFor(2000, [&] { return opened(filesNotice); }) &&
                                                                  filesNotice->property("text").toString().contains(QStringLiteral("Another device")));
    if (filesNotice) QMetaObject::invokeMethod(filesNotice, "close");
    d.waitFor(2000, [&] { return !filesNotice || !filesNotice->property("visible").toBool(); });

    // ---- A firmware update pins its device
    firmware->pretendRunForTest(b);
    d.check("mdqt2b: a run on the second pins it", !list->pinnedText(b->id()).isEmpty() && list->pinnedText(a->id()).isEmpty());
    d.check("mdqt2b: and names it while another is active", firmware->runDevice() == b->title());
    const QString idOfB = b->id();
    int noticeNow = notices->serial();
    list->closeOpen(idOfB);
    d.spin(300);
    d.check("mdqt2b: it is not closed, and that is said",
            r->session(idOfB) == b && notices->serial() > noticeNow && list->pinnedText(idOfB).contains(b->title()));
    // The first's snapshots are not taken for the second coming back
    r->activate(b);
    r->activate(a);
    d.waitIdle();
    d.spin(300);
    d.check("mdqt2b: another device's connection is not the one coming back",
            firmware->stateValue() == FirmwareController::WaitingForDevice && firmware->runsOn(b));
    // Classic replaces the active device: not the pinned one
    r->activate(b);
    noticeNow = notices->serial();
    list->chooseUsb(QStringLiteral("MDQT2B-NONE"));
    d.spin(300);
    d.check("mdqt2b: Classic does not replace it", r->active() == b && b->model()->demo() && notices->serial() > noticeNow);
    firmware->pretendRunForTest(nullptr);
    d.check("mdqt2b: the run over, nothing pins it", list->pinnedText(idOfB).isEmpty());

    // ---- A core voltage calibration pins its device
    r->activate(b);
    d.waitIdle();
    if (d.waitFor(5000, [&] { return voltage->supported(); })) {
        if (voltage->phase() != QLatin1String("idle")) {
            voltage->endTrial();
            d.waitFor(3000, [&] { return !voltage->busy(); });
            voltage->closeFlow();
            d.waitFor(3000, [&] { return !voltage->busy(); });
        }
        voltage->openFlow();
        d.check("mdqt2b: calibration review on the second", d.waitFor(5000, [&] { return voltage->phase() == QLatin1String("review"); }));
        int plain = -1;
        for (const QVariant &c : voltage->view().value(QStringLiteral("choices")).toList())
            if (!c.toMap().value(QStringLiteral("needsWord")).toBool() && plain < 0) plain = c.toMap().value(QStringLiteral("code")).toInt();
        const QVariantList soaks = voltage->soakChoices();
        voltage->start(plain, soaks.isEmpty() ? 0 : soaks.first().toInt(), QString());
        d.check("mdqt2b: soaking on the second", d.waitFor(8000, [&] { return voltage->calibrating(); }));
        r->activate(a);
        d.check("mdqt2b: the first's page is its own", d.waitFor(3000, [&] { return !voltage->calibrating(); }));
        d.check("mdqt2b: the calibration pins the second", voltage->calibratingOn(b) && !list->pinnedText(idOfB).isEmpty());
        list->closeOpen(idOfB);
        d.spin(300);
        d.check("mdqt2b: it is not closed", r->session(idOfB) == b);
        r->activate(b);
        d.waitFor(3000, [&] { return voltage->calibrating(); });
        voltage->endTrial();
        // The flow moves on when it is read (the page polls it while it shows)
        const auto ended = [&] {
            if (!voltage->busy()) voltage->refresh();
            d.spin(100);
            return !voltage->busy() && !voltage->calibrating();
        };
        d.waitFor(8000, ended);
        voltage->closeFlow();
        d.check("mdqt2b: the calibration ended, nothing pins it",
                d.waitFor(8000, ended) && !voltage->calibratingOn(b) && list->pinnedText(idOfB).isEmpty());
    } else {
        d.check("mdqt2b: the demo has the core voltage extension", false);
    }

    // ---- Quitting asks about every open device, the active one first
    r->activate(b);
    d.check("mdqt2b: the second has unsaved changes", setGain(d, gainOf(b) - 1.0f));
    r->activate(a);
    d.check("mdqt2b: and so has the first", setGain(d, gainOf(a) - 1.0f));
    QObject *unsaved = find(files->host(), "FilesHost.unsavedQuestion");
    QSignalSpy question(guard, &UnsavedGuard::question);
    QStringList about;
    const auto aboutConnection = QObject::connect(guard, &UnsavedGuard::question, guard,
                                                  [&about, guard](const QString &, const QString &) { about << guard->deviceTitle(); });
    bool quit = false;
    guard->ask(QStringLiteral("quit"), guard, [&quit] { quit = true; });
    d.check("mdqt2b: quit asks about the first", d.waitFor(3000, [&] { return question.size() == 1; }) && about.value(0) == a->title());
    d.check("mdqt2b: the question names it", d.waitFor(2000, [&] { return opened(unsaved); }) &&
                                                 unsaved->property("text").toString().contains(a->title()));
    finish(unsaved, "alternative");
    d.check("mdqt2b: then about the second", d.waitFor(3000, [&] { return question.size() == 2; }) && about.value(1) == b->title() && !quit);
    d.waitFor(2000, [&] { return opened(unsaved); });
    finish(unsaved, "alternative");
    d.check("mdqt2b: then it goes on", d.waitFor(3000, [&] { return quit; }) && !guard->asking());
    // A Cancel stops there
    quit = false;
    guard->ask(QStringLiteral("quit"), guard, [&quit] { quit = true; });
    d.waitFor(3000, [&] { return question.size() == 3; });
    d.waitFor(2000, [&] { return opened(unsaved); });
    finish(unsaved, "reject");
    d.spin(300);
    d.check("mdqt2b: Cancel stops", !quit && !guard->asking() && question.size() == 3);

    // ---- Closing a device asks about that one
    list->closeOpen(idOfB);
    d.check("mdqt2b: closing the second (not the active one) asks about it",
            d.waitFor(3000, [&] { return question.size() == 4; }) && about.value(3) == b->title());
    d.waitFor(2000, [&] { return opened(unsaved); });
    finish(unsaved, "reject");
    d.spin(300);
    d.check("mdqt2b: Cancel keeps it open", r->session(idOfB) == b);
    // Save goes to its own active slot
    list->closeOpen(idOfB);
    d.waitFor(3000, [&] { return question.size() == 5; });
    QPointer<DeviceSession> goneB(b);
    const int writesNowA = a->model()->settersRun();
    d.waitFor(2000, [&] { return opened(unsaved); });
    finish(unsaved, "accept");
    d.check("mdqt2b: Save saves it there and closes it", d.waitFor(10000, [&] { return goneB.isNull(); }) &&
                                                            r->sessions().size() == 1 && r->active() == a);
    d.check("mdqt2b: nothing was written to the first", a->model()->settersRun() == writesNowA);
    QObject::disconnect(aboutConnection);

    // ---- The System panel's two tabs follow the active device, together
    auto *link = Registrar::instance<LinkController>();
    auto *inst = Registrar::instance<InstallationController>();
    auto *sys = Registrar::instance<SystemController>();
    if (!link || !inst || !sys) {
        d.check("mdqt2b: the System panel's controllers", false);
        return;
    }
    AlinkHub hub;
    const QString store = d.outDir() + QStringLiteral("/mdqt2b-hub");
    QDir().mkpath(store);
    d.check("mdqt2b: uconsole --demo hub --alink runs (DSPI_SMOKE_UCONSOLE)", hub.start(store));
    if (hub.url.isEmpty()) return;
    r->setKeepOpen(true);
    link->connectToHub(hub.url);
    d.check("mdqt2b: the hub's units are listed", d.waitFor(30000, [&] {
        return !link->busy() && link->phase() == QLatin1String("choose_device") && !link->devices().isEmpty();
    }));
    int handle = -1;
    for (const QVariant &v : link->devices())
        if (v.toMap().value(QStringLiteral("serial")).toString() == QLatin1String("DEMO-ALINK-A"))
            handle = v.toMap().value(QStringLiteral("handle")).toInt();
    std::printf("smoke: mdqt2b: hub units %d, handle %d\n", int(link->devices().size()), handle);
    list->chooseHubDevice(handle, QStringLiteral("DEMO-ALINK-A"));
    d.check("mdqt2b: its unit opens beside the demo, on its own core",
            d.waitFor(20000, [&] { return r->opening() == 0 && r->active() != a && device->remote(); }) && r->sessions().size() == 2);
    if (r->active() == a) {
        r->setKeepOpen(false);
        return;
    }
    DeviceSession *u = r->active();
    inst->panelShown(true);
    d.check("mdqt2b: System and Installation read the unit's hub", d.waitFor(20000, [&] {
        return sys->available() && inst->available() && !inst->unitIds().isEmpty();
    }));
    r->activate(a);
    d.check("mdqt2b: on the demo, neither shows the hub's at once", !sys->available() && !inst->available() && inst->unitIds().isEmpty());
    d.spin(1500);
    d.check("mdqt2b: and none comes back late", !sys->available() && !inst->available());
    r->activate(u);
    d.check("mdqt2b: back on the unit, both again", d.waitFor(20000, [&] { return sys->available() && inst->available(); }));
    inst->panelShown(false);
    QPointer<DeviceSession> goneU(u);
    r->close(u->id());
    d.waitFor(10000, [&] { return goneU.isNull(); });
    r->setKeepOpen(false);
    d.waitIdle();
}

const SmokeDriver::Section rebind("mdqt2b-rebind", 962, &rebindSection);
} // namespace
