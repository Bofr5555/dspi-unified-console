// -- WP QP19 -- The launches run.sh starts besides the main run (behaviour.md
// §12.1, §23): the four tray launches, each with the tray settings on in its
// own settings file, and the demo chosen from an app started with nothing
// connected.
//
// Tray (DSPI_SMOKE_TRAY): offscreen has no tray, so DSPI_SMOKE_TRAY_PROBE=1|0
// answers for the desktop, and the start in the tray is made again once the
// stand-in says yes (TrayController::adoptWindow). Each launch must end the
// app by the way a user would; one that does not quit exits 3.
//   hidden       starts hidden in the tray, Open shows it, the tray's Quit ends it
//   hidden-quit  starts hidden and the tray's Quit ends it with no window shown
//   app-quit     shown with the tray on: closing hides it, File > Quit ends it
//   no-tray      the settings on and no tray: shown at start, closing quits
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "DevicesController.h"
#include "Registrar.h"
#include "Shell.h"
#include "TrayController.h"

#include <QAction>
#include <QGuiApplication>
#include <QMenu>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTimer>
#include <thread>

namespace {

constexpr int kQuitWithinMs = 15000;

QAction *trayAction(TrayController *tray, const char *name) {
    return tray->contextMenu() ? tray->contextMenu()->findChild<QAction *>(QLatin1String(name)) : nullptr;
}

void trayLaunch(SmokeDriver &d) {
    const QString launch = SmokeDriver::modeValue();
    const QString tag = QStringLiteral("qp19-tray %1: ").arg(launch);
    auto *tray = Registrar::instance<TrayController>();
    QQuickWindow *main = d.mainWindow();
    d.check(tag + QStringLiteral("the controller and the main window"), tray && main);
    if (!tray || !main) {
        d.expectQuit(0);
        return;
    }
    const bool hasTray = qEnvironmentVariable("DSPI_SMOKE_TRAY_PROBE") == QLatin1String("1");
    // run.sh sets start-in-tray for every launch but app-quit
    d.check(tag + QStringLiteral("the settings file turned the tray on"),
            tray->showInTray() && tray->startMinimised() == (launch != QLatin1String("app-quit"))
                && QSettings().value(QStringLiteral("tray/showInTray")).toBool());
    tray->setAvailabilityProbe([hasTray]() { return hasTray; });
    tray->adoptWindow(main);
    d.spin(200);
    d.check(tag + QStringLiteral("the tray is there as the stand-in says"), tray->trayAvailable() == hasTray && tray->iconShown() == hasTray);

    if (launch == QLatin1String("hidden") || launch == QLatin1String("hidden-quit")) {
        d.check(tag + QStringLiteral("starts hidden in the tray"), d.waitFor(2000, [&] { return tray->windowHidden() && !main->isVisible(); }));
        QAction *quit = trayAction(tray, "Tray.quit");
        d.check(tag + QStringLiteral("the tray menu has Quit"), quit != nullptr);
        if (launch == QLatin1String("hidden")) {
            QAction *open = trayAction(tray, "Tray.open");
            d.check(tag + QStringLiteral("the tray menu has Open"), open != nullptr);
            if (open) open->trigger();
            d.check(tag + QStringLiteral("Open shows the window"), d.waitFor(2000, [&] { return !tray->windowHidden() && main->isVisible(); }));
            d.grab(main, QStringLiteral("window-main.png"));
        }
        d.expectQuit(kQuitWithinMs);
        if (quit) quit->trigger();
    } else if (launch == QLatin1String("app-quit")) {
        d.check(tag + QStringLiteral("shown at start"), main->isVisible() && !tray->windowHidden());
        main->close();
        d.check(tag + QStringLiteral("closing hides it to the tray"), d.waitFor(2000, [&] { return tray->windowHidden() && !main->isVisible(); }));
        tray->restore();
        d.check(tag + QStringLiteral("and it comes back"), d.waitFor(2000, [&] { return main->isVisible(); }));
        d.expectQuit(kQuitWithinMs);
        d.app().shell->trigger(QStringLiteral("file.quit"));
    } else if (launch == QLatin1String("no-tray")) {
        d.check(tag + QStringLiteral("shown at start, start-in-tray set or not"),
                tray->startMinimised() && main->isVisible() && !tray->windowHidden() && !tray->iconShown());
        d.check(tag + QStringLiteral("closing will quit"), QGuiApplication::quitOnLastWindowClosed());
        d.grab(main, QStringLiteral("window-main.png"));
        d.expectQuit(kQuitWithinMs);
        main->close();
    } else {
        d.check(tag + QStringLiteral("DSPI_SMOKE_TRAY names a launch"), false);
        d.expectQuit(0);
    }
}

const SmokeDriver::Section tray("qp19-tray-launch", 10, &trayLaunch, SmokeDriver::Mode::Tray);

// ---- The demo from an app started with nothing connected (QD7) ----

bool click(QQuickItem *item) {
    return item && item->isVisible() && item->isEnabled() && QMetaObject::invokeMethod(item, "clicked");
}

void demoEntry(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    auto *list = Registrar::instance<DevicesController>();
    // -- WP LAY7 -- Classic's popover, or Modern's device switcher
    const bool modern = d.app().shell->pageHost() != nullptr;
    QQuickItem *host = list ? list->host() : nullptr;
    QObject *popup = modern ? d.object(QStringLiteral("DeviceSwitcher")) : host ? host->findChild<QObject *>(QStringLiteral("DevicesPopup")) : nullptr;
    d.check("qp19-demo: the layout the launch asks for", modern == (qEnvironmentVariable("DSPI_SMOKE_DEMO_ENTRY") == QLatin1String("modern")));
    d.check("qp19-demo: the device list is loaded", popup != nullptr);
    if (!popup) return;
    const auto in = [&](const QString &name) {
        auto *content = popup->property("contentItem").value<QQuickItem *>();
        QQuickItem *scope = content && content->parentItem() ? content->parentItem() : content;
        return scope ? d.item(name, scope) : nullptr;
    };

    // ci/run.sh hides USB from the container: a device found here would get the writes below
    d.spin(1200);   // the hotplug poll, twice
    d.check("qp19-demo: nothing is connected", !device->connected() && list->usbDevices().isEmpty());
    if (device->connected() || !list->usbDevices().isEmpty()) return;
    d.check("qp19-demo: and it is not the demo", !device->demo());
    d.grab(d.mainWindow(), QStringLiteral("window-disconnected.png"));

    d.app().shell->trigger(QStringLiteral("device.devices"));
    d.check("qp19-demo: Devices... opens the list", d.waitFor(2000, [&] { return popup->property("opened").toBool(); }));
    QQuickItem *demoRow = in(QStringLiteral("DevicesPopup.demo"));
    d.check("qp19-demo: the demo is offered, not open", demoRow && demoRow->isVisible() && !demoRow->property("current").toBool());
    d.spin(200);
    d.grab(d.mainWindow(), QStringLiteral("window-demo-offered.png"));

    d.check("qp19-demo: choosing it", click(demoRow));
    d.check("qp19-demo: the demo opens with no hardware",
            d.waitFor(5000, [&] { return device->connected() && device->demo() && device->channels()->rowCount() > 0; }) && d.waitIdle());
    d.check("qp19-demo: writable", !device->readOnly());
    if (popup->property("opened").toBool()) QMetaObject::invokeMethod(popup, "close");
    d.spin(300);
    d.grab(d.mainWindow(), QStringLiteral("window-demo-active.png"));

    // A write takes, and a scan and the hotplug poll leave the demo open
    const float before = device->state() ? device->state()->master_volume_db : 0.0f;
    const float want = before > -40.0f ? before - 6.0f : before + 6.0f;
    int outcome = -1;
    device->write(QStringLiteral("master volume"), [want](FfiCore *c) { return dspi_set_master_volume(c, want); },
                  [&outcome](bool ok) { outcome = ok ? 1 : 0; });
    d.check("qp19-demo: a write takes", d.waitFor(5000, [&] { return outcome >= 0; }) && outcome == 1 && d.waitIdle()
                                             && device->state() && device->state()->master_volume_db == want);
    list->rescan();
    d.spin(1300);
    d.check("qp19-demo: a scan and the hotplug poll keep it open",
            d.waitIdle() && device->connected() && device->demo() && device->state()->master_volume_db == want);
}

const SmokeDriver::Section demo("qp19-demo-entry", 10, &demoEntry, SmokeDriver::Mode::DemoEntry);

// ---- The driver's own failure paths (DSPI_SMOKE_SELFTEST; not in run.sh) ----
//   thread  a timer started from a thread with no event loop: the run must end
//           with "FAIL thread:" and exit 1, every check passing
//   noquit  a launch that expects a quit that never comes: exit 3
void selfTest(SmokeDriver &d) {
    const QString what = SmokeDriver::modeValue();
    if (what == QLatin1String("thread")) {
        QTimer *timer = new QTimer;   // lives on the GUI thread
        std::thread([timer]() { timer->start(10); }).join();
        delete timer;
        d.check("qp19-selftest: the run goes on", true);
    } else if (what == QLatin1String("noquit")) {
        d.expectQuit(2000);
    }
}

const SmokeDriver::Section selftest("qp19-selftest", 10, &selfTest, SmokeDriver::Mode::SelfTest);

} // namespace
