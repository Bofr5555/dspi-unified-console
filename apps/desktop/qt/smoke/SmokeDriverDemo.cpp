// WP QD7: the demo chosen from the running app. testDemoEntry is its own run of the
// app, without --demo (run.sh, DSPI_SMOKE_DEMO_ENTRY), since only that starts with
// nothing connected and the demo not on the device list. testDemoLabel is on --demo.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "DemoController.h"
#include "LinkController.h"

#include <QCoreApplication>
#include <QMouseEvent>
#include <QProcess>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>

namespace {
const char *const kDemoSerial = "DEMO0000DSPI0001";

QQuickItem *findItem(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findItem(c, name)) return found;
    return nullptr;
}

// A click where the user would make it: what covers the item gets it instead
void click(QQuickItem *item) {
    QQuickWindow *w = item->window();
    const QPointF at = item->mapToScene(QPointF(item->width() / 2, item->height() / 2));
    const QPointF global(w->mapToGlobal(at.toPoint()));
    QMouseEvent press(QEvent::MouseButtonPress, at, at, global, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
    QCoreApplication::sendEvent(w, &press);
    QMouseEvent release(QEvent::MouseButtonRelease, at, at, global, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
    QCoreApplication::sendEvent(w, &release);
}

// The bridge's real hotplug callback, which only the core can reach, for a USB device
// this container doesn't have. An explicit instantiation may name a private member.
using HotplugFn = void (*)(uint8_t, const char *, void *);
HotplugFn realHotplugCallback();
template <HotplugFn F>
struct HotplugAccess {
    friend HotplugFn realHotplugCallback() { return F; }
};
template struct HotplugAccess<&DSPiBridge::hotplugCallback>;
} // namespace

void SmokeDriver::testDemoLabel() {
    section("QD7: labelled under --demo");
    auto *b = m_.bridge;
    auto *d = qobject_cast<DemoController *>(m_.engine->rootContext()->contextProperty(QStringLiteral("demo")).value<QObject *>());
    auto *host = root()->findChild<QQuickItem *>(QStringLiteral("bannerHost"));
    auto *text = host ? findItem(host, QStringLiteral("demoBannerText")) : nullptr;
    auto *leave = host ? findItem(host, QStringLiteral("demoLeave")) : nullptr;
    check("QD7: registered as demo, with its banner", d && text && leave);
    if (!d || !text || !leave) return;
    spin(50);
    check("QD7: --demo counts as the demo", d->active() && !d->offered());
    check("QD7: --demo is labelled", text->isVisible() && text->property("text").toString().startsWith(QLatin1String("Demo:")));
    check("QD7: --demo has no way out: a scan would reopen it", !d->leavable() && !leave->isVisible() && !d->leave() && b->connected());
    check("QD7: entering the open demo changes nothing", d->enter() && b->selectedSerial() == QLatin1String(kDemoSerial));
}

void SmokeDriver::testDemoEntry() {
    section("QD7: demo from a disconnected app");
    auto *b = m_.bridge;
    QQmlContext *context = m_.engine->rootContext();
    auto *d = qobject_cast<DemoController *>(context->contextProperty(QStringLiteral("demo")).value<QObject *>());
    auto *link = qobject_cast<LinkController *>(context->contextProperty(QStringLiteral("linkSession")).value<QObject *>());
    auto *host = root()->findChild<QQuickItem *>(QStringLiteral("bannerHost"));
    auto *text = host ? findItem(host, QStringLiteral("demoBannerText")) : nullptr;
    auto *enter = host ? findItem(host, QStringLiteral("demoEnter")) : nullptr;
    auto *leave = host ? findItem(host, QStringLiteral("demoLeave")) : nullptr;
    check("QD7: controller and banner", d && link && text && enter && leave);
    if (!d || !link || !text || !enter || !leave) return;
    const auto labelled = [&]() {
        return text->isVisible() && text->property("text").toString().startsWith(QLatin1String("Demo:"));
    };

    // ci/run.sh hides USB from the container; a device found here would get writes below
    check("QD7: no device here", !b->connected() && b->availableSerials().isEmpty());
    if (b->connected() || !b->availableSerials().isEmpty()) return;

    spin(100);
    check("QD7: disconnected: not writable", !b->writable());
    check("QD7: the demo is not on the device list", !b->availableSerials().contains(QLatin1String(kDemoSerial)));
    check("QD7: the way in shows", d->offered() && !d->active() && enter->isVisible() && !leave->isVisible());
    check("QD7: and says what it is", text->isVisible() && !labelled());
    if (auto *main = qobject_cast<QQuickWindow *>(root())) main->grabWindow().save(m_out + QStringLiteral("/window-demo-offered.png"));

    click(enter);
    check("QD7: selectable with no hardware", b->connected() && b->selectedSerial() == QLatin1String(kDemoSerial));
    check("QD7: compat is fine", b->compat() == COMPAT_OK);
    check("QD7: writable at once", b->writable());
    check("QD7: no error left", b->connectionError().isEmpty());
    check("QD7: clearly labelled", waitFor(1000, labelled) && d->active() && !d->offered());
    check("QD7: with the way out", leave->isVisible() && !enter->isVisible());
    if (auto *main = qobject_cast<QQuickWindow *>(root())) main->grabWindow().save(m_out + QStringLiteral("/window-demo-active.png"));
    b->setMasterVolume(-12.0f);
    check("QD7: a write takes", waitFor(1000, [&]() { return b->masterVolumeDB() == -12.0f; }));

    section("QD7: scans and hotplug leave it alone");
    {
        int changing = 0, arrived = 0, departed = 0;
        QObject scope;
        QObject::connect(b, &DSPiBridge::deviceChanging, &scope, [&]() { changing++; });
        QObject::connect(b, &DSPiBridge::deviceArrived, &scope, [&]() { arrived++; });
        QObject::connect(b, &DSPiBridge::deviceDeparted, &scope, [&]() { departed++; });
        const auto unchanged = [&]() {
            return changing == 0 && b->connected() && b->selectedSerial() == QLatin1String(kDemoSerial)
                && b->writable() && labelled() && b->masterVolumeDB() == -12.0f;
        };
        b->scanDevices();
        check("QD7: a scan", unchanged());
        spin(1300);   // the 500 ms hotplug poll, twice
        check("QD7: hotplug polls", unchanged());
        realHotplugCallback()(0, "USBFAKE000000001", b);
        spin(50);
        check("QD7: a DSPi arrives: the bridge hears it", arrived == 1);
        check("QD7: and the demo stays", unchanged());
        realHotplugCallback()(1, "USBFAKE000000001", b);
        spin(50);
        check("QD7: it departs: the bridge hears it", departed == 1);
        check("QD7: and the demo stays", unchanged());
    }

    section("QD7: leaving");
    click(leave);
    check("QD7: closed", !b->connected() && b->selectedSerial().isEmpty() && !b->writable());
    check("QD7: the way back", waitFor(1000, [&]() { return d->offered() && enter->isVisible() && !leave->isVisible() && !labelled(); }));
    spin(1300);
    check("QD7: no scan brings it back", !b->connected() && !d->active());
    check("QD7: open again", d->enter() && b->connected() && b->writable() && labelled());
    b->rescan();
    check("QD7: a rescan is the way out too, and leaves nothing behind",
          !b->connected() && b->selectedSerial().isEmpty() && !b->writable() && b->connectionError().isEmpty() && d->offered());

    section("QD7: beside a hub session");
    QProcess hub;
    hub.setProcessChannelMode(QProcess::ForwardedErrorChannel);
    hub.start(qEnvironmentVariable("DSPI_SMOKE_FAKEHUB"), { "--bind", "127.0.0.1:0", "--rta", "--auth", "none" });
    const bool hubUp = hub.waitForStarted(5000) && hub.waitForReadyRead(5000);
    const auto address = QRegularExpression(QStringLiteral(" on ([0-9.:]+) id ")).match(QString::fromUtf8(hub.readLine()));
    check("QD7: the fake hub runs (DSPI_SMOKE_FAKEHUB)", hubUp && address.hasMatch());
    if (hubUp && address.hasMatch()) {
        const QString url = QStringLiteral("ws://%1/dspi/v1").arg(address.captured(1));
        const auto phase = [&](const char *want) {
            return waitFor(5000, [&]() { return link->phase() == QLatin1String(want); });
        };
        const auto hubClosed = [&]() {
            return waitFor(5000, [&]() { return link->phase() != QLatin1String("bound") && link->phase() != QLatin1String("choose_device") && !link->busy(); });
        };

        // From nothing: a session that is choosing keeps the way in shut
        link->connectToHub(url);
        check("QD7: session up", phase("choose_device"));
        check("QD7: no way in while a hub session is up", waitFor(1000, [&]() { return !d->offered(); }) && !enter->isVisible());
        check("QD7: and enter refuses", !d->enter() && !b->connected());
        link->closeSession();
        check("QD7: session down", hubClosed());
        check("QD7: the way in is back", waitFor(1000, [&]() { return d->offered(); }));

        // From the demo: the session may take over, and hands back a clean app
        check("QD7: demo open", d->enter() && b->writable());
        link->connectToHub(url);
        check("QD7: session up with the demo open", phase("choose_device"));
        spin(700);
        check("QD7: choosing leaves the demo open", b->connected() && d->active() && labelled());
        const QVariantMap device = link->devices().value(0).toMap();
        const QString serial = device.value(QStringLiteral("serial")).toString();
        link->bindDevice(device.value(QStringLiteral("handle")).toInt(), serial);
        check("QD7: the bound device replaces the demo", phase("bound") && waitFor(3000, [&]() { return b->selectedSerial() == serial; }));
        check("QD7: no demo label on a real device", waitFor(1000, [&]() { return !d->active() && !d->offered() && !text->isVisible(); }));
        check("QD7: with none shown the banner takes no space", !link->tokenWarning().isEmpty() || !host->isVisible());
        check("QD7: enter refuses on a bound session", !d->enter() && b->selectedSerial() == serial);
        link->closeSession();
        check("QD7: session closed", hubClosed());
        check("QD7: nothing connected, the way in is back",
              waitFor(3000, [&]() { return !b->connected() && b->selectedSerial().isEmpty() && d->offered(); }) && !b->writable());
        check("QD7: demo opens after a session", d->enter() && b->writable() && labelled());
    }
    hub.kill();
    hub.waitForFinished(2000);
}
