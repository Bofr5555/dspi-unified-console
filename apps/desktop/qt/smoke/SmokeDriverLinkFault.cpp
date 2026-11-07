// WP QD10: the lost-connection banner on the fake hub's faults. A bound session
// loses its hub: the banner shows at once, the mirror stays readable, a write fails
// and is not sent when the hub is back, the analyser starts again, and the banner's
// button leaves the hub while a reconnect dial hangs.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "LinkController.h"
#include "LinkFakeHub.h"

#include <QElapsedTimer>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QThreadPool>
#include <atomic>

using namespace linksmoke;

void SmokeDriver::testLinkFault() {
    section("QD10: lost connection on the fake hub");
    auto *app = qobject_cast<LinkController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("linkSession")).value<QObject *>());
    check("QD10: registered as linkSession", app != nullptr);
    FakeHubProcess hub;
    const bool hubUp = app && hub.start({ "--rta", "--auth", "none" });
    check("QD10: the fake hub runs (DSPI_SMOKE_FAKEHUB)", hubUp);
    if (!hubUp) return;

    const auto settled = [this, app] { return waitFor(30000, [app] { return !app->busy(); }); };
    const auto phaseIs = [app](const char *phase) { return app->phase() == QLatin1String(phase); };
    auto *host = root()->findChild<QQuickItem *>(QStringLiteral("bannerHost"));
    auto *row = host ? findItem(host, QStringLiteral("linkLostRow")) : nullptr;
    auto *abort = host ? findItem(host, QStringLiteral("linkLostAbort")) : nullptr;
    check("QD10: the banner is loaded", row && abort);
    if (!row || !abort) return;
    auto *b = m_.bridge;
    const bool demoBefore = b->connected();

    // Bound, with the analyser running
    app->connectToHub(hub.url);
    check("QD10: session", settled() && waitFor(3000, [&] { return phaseIs("choose_device") && app->devices().size() == 1; }));
    const QVariantMap device = app->devices().value(0).toMap();
    const QString serial = device.value(QStringLiteral("serial")).toString();
    app->bindDevice(device.value(QStringLiteral("handle")).toInt(), serial);
    check("QD10: bound and shown", settled() && waitFor(3000, [&] { return phaseIs("bound"); })
                                       && b->connected() && b->selectedSerial() == serial);
    RtaRequest request = {};
    request.tap = RTA_TAP_INPUT;
    request.mask = 1;
    dspi_rta_set_request(b->core(), &request);
    const auto wait = [this](int ms, const std::function<bool()> &done) { return waitFor(ms, done); };
    check("QD10: the analyser runs", analyserFollowsHub(b->core(), hub, wait));
    // Master volume: a write the fake hub keeps (HubConfig::set_offsets)
    const auto volume = [b] { return dspi_get_state(b->core())->master_volume_db; };
    const float before = volume();
    const float written = before > -60.0f ? before - 6.0f : before + 6.0f;

    // Lost: the hub is cut off and turns every dial away for a while (503: try later)
    int back = 0;
    const QMetaObject::Connection backCount = QObject::connect(app, &LinkController::reconnected, b, [&] { back++; });
    check("QD10: the hub takes refuse 503", hub.fault("refuse 503"));
    QElapsedTimer lost;
    lost.start();
    check("QD10: the hub takes drop", hub.fault("drop"));
    check("QD10: banner at once", waitFor(2000, [&] { return phaseIs("reconnecting") && row->isVisible(); }));
    qInfo("smoke: QD10: banner %lld ms after the drop", qint64(lost.elapsed()));
    check("QD10: banner within a second", lost.elapsed() < 1000);
    check("QD10: banner text and button from CG10", app->phaseKey() == QLatin1String("reconnecting")
                                                         && app->actionKey() == QLatin1String("reconnecting.disconnect"));
    spin(300);
    check("QD10: still lost while the hub refuses", phaseIs("reconnecting") && row->isVisible());
    check("QD10: the mirror stays readable", dspi_is_connected(b->core()) && b->connected() && b->selectedSerial() == serial
                                                 && volume() == before);
    QElapsedTimer writing;
    writing.start();
    const bool wrote = dspi_set_master_volume(b->core(), written);
    b->setMasterVolume(written);   // the UI's path
    check("QD10: a write fails, at once", !wrote && writing.elapsed() < 500);
    check("QD10: the mirror keeps the device's value", b->masterVolumeDB() == before);

    // Back: the write was not queued, so the device never sees it
    check("QD10: the hub takes refuse ok", hub.fault("refuse ok"));
    app->retryNow();
    check("QD10: back and bound", waitFor(15000, [&] { return back == 1 && phaseIs("bound"); }) && !row->isVisible());
    QObject::disconnect(backCount);
    const auto reread = [&] {
        std::atomic<bool> fetched { false }, fetchOk { false };
        QThreadPool::globalInstance()->start([&] { fetchOk = dspi_fetch_all(b->core()); fetched = true; });
        return waitFor(20000, [&] { return fetched.load(); }) && fetchOk;
    };
    check("QD10: the device read again", reread());
    check("QD10: the failed write was not sent later", volume() == before && b->masterVolumeDB() == before);
    // The control: the hub keeps a write made now, so the check above can fail
    check("QD10: a write works once back", dspi_set_master_volume(b->core(), written) && reread() && volume() == written);
    check("QD10: and is put back", dspi_set_master_volume(b->core(), before) && reread() && volume() == before);
    check("QD10: the analyser starts again", analyserFollowsHub(b->core(), hub, wait));

    // The way out while a reconnect dial hangs: the hub takes the connection, then
    // says nothing, and the dial waits for its hello
    check("QD10: the hub takes refuse 503 again", hub.fault("refuse 503"));
    check("QD10: the hub takes drop again", hub.fault("drop"));
    check("QD10: lost again", waitFor(2000, [&] { return phaseIs("reconnecting") && row->isVisible(); }));
    check("QD10: the hub takes refuse ok and stall", hub.fault("refuse ok") && hub.fault("stall"));
    app->retryNow();
    spin(500);
    check("QD10: dialling, still lost", phaseIs("reconnecting") && row->isVisible());
    if (auto *main = qobject_cast<QQuickWindow *>(root())) main->grabWindow().save(m_out + QStringLiteral("/window-link-fault-dial.png"));
    QElapsedTimer leaving;
    leaving.start();
    QMetaObject::invokeMethod(abort, "clicked");
    check("QD10: the button leaves during the dial, at once", leaving.elapsed() < 1000 && phaseIs("idle") && !row->isVisible());
    check("QD10: the hub takes unstall", hub.fault("unstall"));
    spin(1500);
    check("QD10: the hung dial does not come back", phaseIs("idle") && !row->isVisible());
    check("QD10: the device goes with the session", waitFor(3000, [&] { return b->connected() == demoBefore && b->selectedSerial() != serial; }));
    settled();
}
