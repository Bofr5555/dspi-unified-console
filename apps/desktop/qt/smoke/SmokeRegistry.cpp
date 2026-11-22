// -- WP MDQT1 -- Several open devices through the app's own objects: two demos
// and a fake hub's unit, each with its own model and core, one of them active.
// Choosing (the registry, Classic's "Open devices" list), writes that stay with
// the device they were asked of, polling, a background device's lost hub,
// one session per serial, Classic's replace, and closing.
#include "SmokeDriver.h"

#include "Core.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "DevicesController.h"
#include "LinkController.h"
#include "LinkFakeHub.h"
#include "NoticeCenter.h"
#include "Poller.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"

#include <QPointer>
#include <QQuickItem>
#include <QThread>
#include <algorithm>
#include <atomic>
#include <cmath>
#include <memory>

namespace {
using namespace linksmoke;

float volumeOf(const DeviceSession *s) {
    const DspState *state = s->model()->state();
    return state ? state->master_volume_db : NAN;
}

QVariantMap rowOf(const DeviceRegistry *r, const QString &id) {
    for (const QVariant &v : r->devices())
        if (v.toMap().value(QStringLiteral("id")).toString() == id) return v.toMap();
    return {};
}

bool near(float a, float b) { return std::fabs(a - b) < 0.05f; }

void registrySection(SmokeDriver &d) {
    DeviceRegistry *r = DeviceRegistry::instance();
    DeviceModel *device = d.app().device;
    Core *core = d.app().core;
    Poller *poller = Poller::instance();
    auto *link = Registrar::instance<LinkController>();
    auto *list = Registrar::instance<DevicesController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    QQuickItem *host = list ? list->host() : nullptr;
    QObject *popup = host ? host->findChild<QObject *>(QStringLiteral("DevicesPopup")) : nullptr;
    d.check("mdqt1: the registry and the list exist", r && link && list && notices && popup);
    if (!r || !link || !list || !notices || !popup) return;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.waitIdle();
    DeviceSession *a = r->mainSession();
    d.check("mdqt1: the demo is the main session's, and active", r->active() == a && a->model()->demo());
    d.check("mdqt1: and the one row", d.waitFor(3000, [&] { return r->devices().size() == 1; }) && !r->keepOpen());
    const QString idA = r->activeId();
    d.check("mdqt1: its id is a demo's", idA.startsWith(QLatin1String("demo:")));
    const auto in = [&](const QString &name) {
        auto *content = popup->property("contentItem").value<QQuickItem *>();
        return content ? d.item(name, content) : nullptr;
    };
    QQuickItem *heading = in(QStringLiteral("DevicesPopup.openHeading"));
    d.check("mdqt1: one device: no Open devices list", heading && !heading->property("visible").toBool());
    const float volumeA = volumeOf(a);
    const int writesBefore = a->model()->settersRun();

    // ---- A second demo beside it
    int switches = 0, snapshots = 0;
    const auto c1 = QObject::connect(r, &DeviceRegistry::activeChanged, r, [&] { ++switches; });
    const auto c2 = QObject::connect(device, &DeviceModel::snapshotApplied, r, [&] { ++snapshots; });
    r->openDemo();
    d.check("mdqt1: the second demo opens", d.waitFor(10000, [&] { return r->opening() == 0 && r->sessions().size() == 2; }));
    if (r->sessions().size() != 2) return;
    DeviceSession *b = r->sessions().at(1);
    d.check("mdqt1: and is active, the first stays open",
            r->active() == b && switches == 1 && snapshots >= 1 && a->model()->connected() && b->model()->connected());
    d.check("mdqt1: the app shows it", device->active() == b->model() && core->handle() == b->handle() &&
                                           core->worker() == b->worker() && device->snapshot().generation == b->model()->snapshot().generation);
    d.check("mdqt1: another core and connection", a->handle() != b->handle() &&
                                                      a->model()->snapshot().generation != b->model()->snapshot().generation);
    d.check("mdqt1: two rows, one active", d.waitFor(3000, [&] {
        return r->devices().size() == 2 && rowOf(r, idA).value(QStringLiteral("active")).toBool() == false &&
               rowOf(r, b->id()).value(QStringLiteral("active")).toBool();
    }));
    d.check("mdqt1: the demos are told apart", a->title() != b->title());

    // ---- Writes stay with the device they were asked of
    bool doneB = false;
    const float wanted = volumeA > -40.0f ? -47.0f : -33.0f;
    device->write(QStringLiteral("master volume"), [wanted](FfiCore *c) { return dspi_set_master_volume(c, wanted); },
                  [&doneB](bool ok) { doneB = ok; });
    // Switched away before it ran: it is B's all the same
    r->activate(a);
    d.check("mdqt1: the first is active again at once", r->active() == a && device->active() == a->model());
    d.check("mdqt1: the write landed on the second", d.waitFor(5000, [&] { return doneB && near(volumeOf(b), wanted); }));
    d.waitIdle();
    d.check("mdqt1: and not on the first", near(volumeOf(a), volumeA) && a->model()->settersRun() == writesBefore);
    d.check("mdqt1: the app shows the first's own snapshot", device->state() && near(device->state()->master_volume_db, volumeA));
    const float wantedA = wanted - 3.0f;
    bool doneA = false;
    device->write(QStringLiteral("master volume"), [wantedA](FfiCore *c) { return dspi_set_master_volume(c, wantedA); },
                  [&doneA](bool ok) { doneA = ok; });
    d.check("mdqt1: a write to the first", d.waitFor(5000, [&] { return doneA; }) && d.waitIdle());
    r->activate(b->id());
    d.check("mdqt1: each keeps its own", near(volumeOf(a), wantedA) && near(volumeOf(b), wanted) &&
                                             near(device->state()->master_volume_db, wanted));
    d.check("mdqt1: an id that is not open changes nothing", !r->activate(QStringLiteral("usb:NONE")) && r->active() == b);

    // A drag held over a switch ends with it
    device->beginDrag();
    r->activate(a);
    d.check("mdqt1: a drag ends with the switch", !b->model()->dragging() && !device->dragging());

    // ---- Polling: the active one at full rate, the other once a second
    const int meterBefore = poller->counts(poller->meterPoll()).completed;
    const int readsBefore = b->backgroundReads();
    const int readsOfActive = a->backgroundReads();
    d.spin(2600);
    const int reads = b->backgroundReads() - readsBefore;
    d.check("mdqt1: the active one's meters are read at full rate", poller->counts(poller->meterPoll()).completed - meterBefore > 10);
    d.check(QStringLiteral("mdqt1: the other is read once a second (%1 in 2.6 s)").arg(reads), reads >= 1 && reads <= 3);
    d.check("mdqt1: the active one is not read twice", a->backgroundReads() == readsOfActive);
    d.check("mdqt1: the meters start over on a switch", !poller->peaks().isEmpty());
    r->activate(b);
    d.check("mdqt1: none of the other's are shown", poller->peaks().isEmpty());
    d.check("mdqt1: then the new one's are", d.waitFor(3000, [&] { return !poller->peaks().isEmpty(); }));

    // ---- A hub's unit beside them, on its own core
    FakeHubProcess hub;
    const bool hubUp = hub.start({ "--auth", "none" });
    d.check("mdqt1: the fake hub runs (DSPI_SMOKE_FAKEHUB)", hubUp);
    if (!hubUp) return;
    r->setKeepOpen(true);
    const auto phaseIs = [link](const char *phase) { return link->phase() == QLatin1String(phase); };
    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    d.check("mdqt1: the hub is dialled", list->connectAddress(hub.address));
    d.check("mdqt1: its unit is listed, the open devices as they were",
            settled() && d.waitFor(5000, [&] { return phaseIs("choose_device") && link->devices().size() == 1; }) &&
                r->active() == b && b->model()->connected() && !b->model()->remote());
    const QVariantMap unit = link->devices().value(0).toMap();
    const QString serial = unit.value(QStringLiteral("serial")).toString();
    const int handle = unit.value(QStringLiteral("handle")).toInt();
    list->chooseHubDevice(handle, serial);
    d.check("mdqt1: the unit opens beside them", d.waitFor(20000, [&] { return r->sessions().size() == 3 && r->opening() == 0; }));
    if (r->sessions().size() != 3) return;
    DeviceSession *u = r->sessions().at(2);
    d.check("mdqt1: it is active and a hub's", r->active() == u && device->remote() && device->connected() && device->serial() == serial);
    d.check("mdqt1: its id", u->id().startsWith(QLatin1String("hub:")) && u->id().endsWith(QLatin1Char('/') + serial));
    d.check("mdqt1: its own session is bound", d.waitFor(5000, [&] { return phaseIs("bound") && link->boundSerial() == serial; }));
    d.check("mdqt1: polls slow down over the hub", poller->slowTransport());
    d.check("mdqt1: three devices, all live", d.waitFor(3000, [&] {
        const QVariantList rows = r->devices();
        return rows.size() == 3 && std::all_of(rows.begin(), rows.end(), [](const QVariant &v) {
                   return v.toMap().value(QStringLiteral("state")).toString() == QLatin1String("live");
               });
    }));
    d.check("mdqt1: each model keeps its snapshot", near(volumeOf(a), wantedA) && near(volumeOf(b), wanted) && u->model()->state());

    r->activate(a);
    d.check("mdqt1: the demo's own Link session shows: none",
            phaseIs("idle") && link->boundSerial().isEmpty() && !device->remote() && !poller->slowTransport());
    r->activate(u);
    d.check("mdqt1: the hub's shows again", phaseIs("bound") && device->remote() && poller->slowTransport());

    // ---- One session per serial
    r->activate(a);
    const int noticesBefore = notices->serial();
    QString failedCode;
    r->open({ { QStringLiteral("kind"), QStringLiteral("usb") }, { QStringLiteral("serial"), serial } },
            [&failedCode](DeviceSession *, const QString &code, const QString &) { failedCode = code; });
    d.check("mdqt1: the unit over USB too is refused", d.waitFor(10000, [&] { return !failedCode.isEmpty(); }) &&
                                                           failedCode == QLatin1String("already_open"));
    d.check("mdqt1: with a notice", notices->serial() > noticesBefore &&
                                        notices->last().value(QStringLiteral("text")).toString().contains(serial));
    const int noticesNow = notices->serial();
    list->chooseUsb(serial);
    d.waitIdle();
    d.check("mdqt1: the list refuses it as well", notices->serial() > noticesNow && r->active() == a && r->sessions().size() == 3 &&
                                                      a->model()->demo() && near(volumeOf(a), wantedA));
    // On the active core, past the list: the core's own refusal
    // (its notice is the first one's, still on show, so it is not posted again)
    QString refused;
    const auto c4 = QObject::connect(r, &DeviceRegistry::openFailed, r, [&refused](const QString &code, const QString &) { refused = code; });
    device->selectDevice(serial);
    d.check("mdqt1: the core refuses it, and it is said",
            d.waitFor(5000, [&] { return refused == QLatin1String("already_open"); }) && d.waitIdle());
    QObject::disconnect(c4);
    d.check("mdqt1: what was open stays open", a->model()->connected() && a->model()->demo() && near(volumeOf(a), wantedA));
    // The same way: chosen again, it becomes the active one
    list->chooseHubDevice(handle, serial);
    d.check("mdqt1: chosen again it is made active", r->active() == u && r->sessions().size() == 3);

    // ---- A background device's lost connection marks its entry
    r->activate(a);
    d.check("mdqt1: the hub takes refuse 503", hub.fault("refuse 503"));
    d.check("mdqt1: the hub takes drop", hub.fault("drop"));
    const auto stateOf = [&](const DeviceSession *s) { return rowOf(r, s->id()).value(QStringLiteral("state")).toString(); };
    d.check("mdqt1: lost in the background: its entry says so",
            d.waitFor(5000, [&] { return stateOf(u) == QLatin1String("reconnecting") || stateOf(u) == QLatin1String("lost"); }));
    d.check("mdqt1: the others are live", stateOf(a) == QLatin1String("live") && stateOf(b) == QLatin1String("live") &&
                                              device->connected() && phaseIs("idle"));
    d.check("mdqt1: the hub takes refuse ok", hub.fault("refuse ok"));
    dspi_link_retry_now(u->handle());
    d.check("mdqt1: back in the background: live", d.waitFor(20000, [&] { return stateOf(u) == QLatin1String("live"); }));
    d.check("mdqt1: the same session", r->sessions().contains(u) && r->active() == a);

    // ---- Classic: the Open devices list, and choosing replaces
    r->setKeepOpen(false);
    list->openList();
    d.check("mdqt1: the list shows the open devices", d.waitFor(3000, [&] {
        return popup->property("opened").toBool() && heading && heading->isVisible();
    }));
    QQuickItem *rowU = in(QStringLiteral("DevicesPopup.open.") + u->id());
    QQuickItem *rowA = in(QStringLiteral("DevicesPopup.open.") + a->id());
    QQuickItem *rowB = in(QStringLiteral("DevicesPopup.open.") + b->id());
    d.check("mdqt1: a row each, the active one marked", rowU && rowA && rowB && rowA->property("current").toBool() &&
                                                            !rowU->property("current").toBool());
    d.check("mdqt1: the rows' screenshot", d.grab(d.mainWindow(), QStringLiteral("window-mdqt1-open-devices.png")));
    if (rowB) QMetaObject::invokeMethod(rowB, "clicked");
    d.check("mdqt1: choosing one makes it active", r->active() == b && r->sessions().size() == 3);
    const QString idB = b->id();
    const uint64_t generationB = b->model()->snapshot().generation;
    // The demo row: the active device's replaced (a demo by a new one), on its core
    device->openDemo();
    d.check("mdqt1: replaced on its own core", d.waitIdle(10000) && b->model()->snapshot().generation != generationB &&
                                                   r->active() == b && r->sessions().size() == 3);
    d.check("mdqt1: a demo stays the same entry", d.waitFor(3000, [&] { return b->id() == idB && r->devices().size() == 3; }));
    // A hub's unit replaced by the demo: its id follows, the hub is left
    r->activate(u);
    const QString idU = u->id();
    link->leaveForLocal();
    device->openDemo();
    d.check("mdqt1: the id follows the device", d.waitFor(10000, [&] {
        return u->model()->demo() && u->id() != idU && u->id().startsWith(QLatin1String("demo:")) && rowOf(r, idU).isEmpty();
    }));
    d.check("mdqt1: the hub's serial is free again", !r->holder(serial));
    list->closeList();

    // ---- Classic: a hub's unit on the active device's own core
    d.waitIdle();
    link->connectToHub(hub.url);
    d.check("mdqt1: dialled on the active core", settled() && d.waitFor(5000, [&] { return phaseIs("choose_device") && link->devices().size() == 1; }) &&
                                                     r->sessions().size() == 3);
    const QVariantMap again = link->devices().value(0).toMap();
    link->bindDevice(again.value(QStringLiteral("handle")).toInt(), serial);
    d.check("mdqt1: bound in place of the demo", settled() && d.waitFor(5000, [&] {
        return phaseIs("bound") && r->active() == u && device->remote() && u->id() == idU;
    }) && r->sessions().size() == 3);
    d.waitIdle();

    // ---- Closing
    QObject *closing = nullptr;
    const auto c3 = QObject::connect(r, &DeviceRegistry::sessionClosing, r, [&closing](DeviceSession *s) { closing = s; });
    QPointer<DeviceSession> goneU(u);
    r->close(idU);
    d.check("mdqt1: closed: its neighbour is active at once", r->active() == b && closing == u && r->sessions().size() == 2);
    d.check("mdqt1: and the app shows that", device->active() == b->model() && !device->remote() && phaseIs("idle"));
    d.check("mdqt1: the session goes once its worker is quiet", d.waitFor(10000, [&] { return goneU.isNull(); }));
    list->chooseOpen(idU);
    d.check("mdqt1: its id is gone", r->active() == b && rowOf(r, idU).isEmpty());
    // The main one is closed without going
    r->close(a->id());
    d.check("mdqt1: the main session stays, with no device", d.waitFor(5000, [&] { return !a->model()->connected() && a->id().isEmpty(); }) &&
                                                                 r->sessions().size() == 2 && r->active() == b);
    d.check("mdqt1: one device left", d.waitFor(3000, [&] { return r->devices().size() == 1; }));
    QPointer<DeviceSession> goneB(b);
    // A job on its worker when it is closed: it ends first, on an open device
    auto late = std::make_shared<std::atomic<int>>(0);
    b->worker()->post([late](FfiCore *c) {
        QThread::msleep(300);
        late->store(dspi_is_connected(c) ? 1 : 2);
    });
    r->close(b->id());
    d.check("mdqt1: its worker ran dry before its core was closed", late->load() == 0 && d.waitFor(5000, [&] { return late->load() != 0; }) && late->load() == 1);
    d.check("mdqt1: the last one closed: nothing open", r->active() == a && d.waitFor(5000, [&] { return goneB.isNull(); }) &&
                                                            !device->connected() && r->devices().isEmpty() && r->isEmpty());
    r->setKeepOpen(true);
    r->openDemo();
    d.check("mdqt1: the next opens on the main session", d.waitFor(5000, [&] { return device->connected() && device->demo(); }) &&
                                                             r->sessions().size() == 1 && r->active() == a);
    r->setKeepOpen(false);
    d.waitIdle();
    d.waitFor(3000, [&] { return r->devices().size() == 1; });
    QObject::disconnect(c1);
    QObject::disconnect(c2);
    QObject::disconnect(c3);
    hub.kill();
}

const SmokeDriver::Section registry("mdqt1-registry", 960, &registrySection);
} // namespace
