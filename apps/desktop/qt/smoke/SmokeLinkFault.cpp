// -- WP QP13 -- The lost-connection banner, on the fake hub's faults, with the
// app's own controllers. A bound session loses its hub: the banner shows at once
// in the core's words, the app keeps showing the last values read, a write fails
// at once and is not sent when the hub is back, the analyser starts again, and the
// banner's button leaves the hub while a reconnect dial hangs. Adapted from the
// previous desktop app's QD10 checks.
#include "SmokeDriver.h"

#include "Core.h"
#include "DeviceModel.h"
#include "LinkController.h"
#include "LinkFakeHub.h"
#include "Registrar.h"

#include <QElapsedTimer>
#include <QQuickItem>
#include <QQuickWindow>
#include <cmath>
#include <cstdio>

namespace {
using namespace linksmoke;

bool click(QQuickItem *item) {
    return item && item->isVisible() && item->isEnabled() && QMetaObject::invokeMethod(item, "clicked");
}

void linkFaultSection(SmokeDriver &d) {
    auto *link = Registrar::instance<LinkController>();
    DeviceModel *device = d.app().device;
    FfiCore *core = d.app().core->handle();
    FakeHubProcess hub;
    const bool hubUp = link && hub.start({ "--rta", "--auth", "none" });
    d.check("qp13-fault: the fake hub runs (DSPI_SMOKE_FAKEHUB)", hubUp);
    if (!hubUp) return;

    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    const auto phaseIs = [link](const char *phase) { return link->phase() == QLatin1String(phase); };
    QQuickItem *banner = d.item(QStringLiteral("LinkBanner"));
    QQuickItem *abort = d.item(QStringLiteral("LinkBanner.disconnect"));
    d.check("qp13-fault: the banner is loaded, and hidden", banner && abort && !banner->isVisible());
    if (!banner || !abort) return;
    const bool demoBefore = device->demo() && device->connected();

    // Bound, with the analyser running
    link->connectToHub(hub.url);
    d.check("qp13-fault: session", settled() && d.waitFor(3000, [&] { return phaseIs("choose_device") && link->devices().size() == 1; }));
    const QVariantMap hubDevice = link->devices().value(0).toMap();
    const QString serial = hubDevice.value(QStringLiteral("serial")).toString();
    link->bindDevice(hubDevice.value(QStringLiteral("handle")).toInt(), serial);
    d.check("qp13-fault: bound and shown", settled() && d.waitFor(5000, [&] {
        return phaseIs("bound") && device->remote() && device->connected() && device->serial() == serial;
    }) && d.waitIdle());
    RtaRequest request = {};
    request.tap = RTA_TAP_INPUT;
    request.mask = 1;
    dspi_rta_set_request(core, &request);
    RtaCapsInfo caps{};
    d.check("qp13-fault: binding starts the analyser", d.waitFor(5000, [&] {
        dspi_rta_get_caps(core, &caps);
        return caps.supported;
    }));
    const auto wait = [&d](int ms, const std::function<bool()> &done) { return d.waitFor(ms, done); };
    d.check("qp13-fault: the analyser runs", analyserFollowsHub(core, hub, wait));
    // Master volume: a write the fake hub keeps
    const auto volume = [device] { return device->state() ? device->state()->master_volume_db : NAN; };
    const float before = volume();
    const float written = before > -60.0f ? before - 6.0f : before + 6.0f;
    const auto writeVolume = [&](float db) {
        int outcome = -1;
        device->write(QStringLiteral("master volume"), [db](FfiCore *c) { return dspi_set_master_volume(c, db); },
                      [&outcome](bool ok) { outcome = ok ? 1 : 0; });
        d.waitFor(10000, [&] { return outcome >= 0; });
        return outcome == 1;
    };

    // Lost: the hub is cut off and turns every dial away for a while (503)
    int back = 0;
    const QMetaObject::Connection counting = QObject::connect(link, &LinkController::reconnected, link, [&] { back++; });
    d.check("qp13-fault: the hub takes refuse 503", hub.fault("refuse 503"));
    QElapsedTimer lost;
    lost.start();
    d.check("qp13-fault: the hub takes drop", hub.fault("drop"));
    d.check("qp13-fault: banner at once", d.waitFor(2000, [&] { return phaseIs("reconnecting") && banner->isVisible(); }));
    std::printf("smoke: qp13-fault: banner %lld ms after the drop\n", qint64(lost.elapsed()));
    d.check("qp13-fault: banner within a second", lost.elapsed() < 1000);
    d.check("qp13-fault: banner text and button are the core's",
            link->phaseKey() == QLatin1String("reconnecting") && banner->property("text").toString().endsWith(link->phaseText())
                && link->actionKey() == QLatin1String("reconnecting.disconnect")
                && abort->property("text").toString() == link->actionText());
    d.spin(300);
    d.check("qp13-fault: still lost while the hub refuses", phaseIs("reconnecting") && banner->isVisible());
    d.check("qp13-fault: the app keeps showing the hub's device", device->remote() && device->connected()
                                                                       && device->serial() == serial && volume() == before);
    d.grab(d.mainWindow(), QStringLiteral("window-link-banner.png"));
    QElapsedTimer writing;
    writing.start();
    const bool wrote = writeVolume(written);
    d.check("qp13-fault: a write fails, at once", !wrote && writing.elapsed() < 1000);
    d.check("qp13-fault: the shown value stays the device's", volume() == before);

    // Back: the write was not queued, so the device never sees it
    d.check("qp13-fault: the hub takes refuse ok", hub.fault("refuse ok"));
    link->retryNow();
    d.check("qp13-fault: back and bound", d.waitFor(15000, [&] { return back == 1 && phaseIs("bound"); }) && !banner->isVisible());
    QObject::disconnect(counting);
    device->refresh();
    d.check("qp13-fault: the device read again", d.waitIdle(20000));
    d.check("qp13-fault: the failed write was not sent later", volume() == before);
    // The control: the hub keeps a write made now, so the check above can fail
    d.check("qp13-fault: a write works once back", writeVolume(written) && d.waitIdle() && volume() == written);
    d.check("qp13-fault: and is put back", writeVolume(before) && d.waitIdle() && volume() == before);
    d.check("qp13-fault: the analyser starts again", analyserFollowsHub(core, hub, wait));

    // The way out while a reconnect dial hangs: the hub takes the connection,
    // then says nothing, and the dial waits for its hello
    d.check("qp13-fault: the hub takes refuse 503 again", hub.fault("refuse 503"));
    d.check("qp13-fault: the hub takes drop again", hub.fault("drop"));
    d.check("qp13-fault: lost again", d.waitFor(2000, [&] { return phaseIs("reconnecting") && banner->isVisible(); }));
    d.check("qp13-fault: the hub takes refuse ok and stall", hub.fault("refuse ok") && hub.fault("stall"));
    link->retryNow();
    d.spin(500);
    d.check("qp13-fault: dialling, still lost", phaseIs("reconnecting") && banner->isVisible());
    d.grab(d.mainWindow(), QStringLiteral("window-link-fault-dial.png"));
    QElapsedTimer leaving;
    leaving.start();
    d.check("qp13-fault: the banner's button", click(abort));
    d.check("qp13-fault: leaves during the dial, at once",
            leaving.elapsed() < 1000 && d.waitFor(1000, [&] { return phaseIs("idle") && !banner->isVisible(); }));
    d.check("qp13-fault: the hub takes unstall", hub.fault("unstall"));
    d.spin(1500);
    d.check("qp13-fault: the hung dial does not come back", phaseIs("idle") && !banner->isVisible());
    d.check("qp13-fault: the device goes with the session", d.waitFor(5000, [&] {
        return !device->remote() && device->demo() == demoBefore && device->connected() == demoBefore;
    }));
    settled();
    d.waitIdle();
}

const SmokeDriver::Section linkFault("qp13-link-fault", 132, &linkFaultSection);
} // namespace
