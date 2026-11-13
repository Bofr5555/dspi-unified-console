// -- WP QP14 -- The tray's states (behaviour.md §12.1) on the real main window:
// off, wanted with no tray, a tray that comes and goes, closing to the tray
// with a tool window open, Open and the icon's clicks, minimise, the switch off
// while hidden, the menu's items in the Mac's order with the hub's (sharing,
// address, pairing) on a loopback hub, the quick volume, and starting hidden.
// Offscreen has no tray, so a stand-in answers for the desktop; what a real
// panel does with the icon is QP21's hand check. Adapted from the previous
// desktop app's QD12 checks.
#include "SmokeDriver.h"

#include "AppIdentity.h"
#include "DeviceModel.h"
#include "HubController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"
#include "TrayController.h"

#include "CoreApi.h"

#include <QAction>
#include <QDir>
#include <QGuiApplication>
#include <QImage>
#include <QMenu>
#include <QQuickWindow>
#include <QSettings>
#include <QSystemTrayIcon>
#include <QTcpServer>
#include <cmath>

namespace {

QAction *action(TrayController *tray, const char *name) {
    return tray->contextMenu() ? tray->contextMenu()->findChild<QAction *>(QLatin1String(name)) : nullptr;
}
QString text(TrayController *tray, const char *name) {
    QAction *a = action(tray, name);
    return a ? a->text() : QString();
}
bool shown(TrayController *tray, const char *name) {
    QAction *a = action(tray, name);
    return a && a->isVisible();
}

void traySection(SmokeDriver &d) {
    auto *tray = Registrar::instance<TrayController>();
    auto *hub = Registrar::instance<HubController>();
    auto *bar = Registrar::instance<SidebarController>();
    QQuickWindow *main = d.mainWindow();
    DeviceModel *device = d.app().device;
    d.check(QStringLiteral("qp14: the tray's controller exists"), tray && hub && bar && main);
    if (!tray || !hub || !bar || !main) return;
    const QString name = AppIdentity::displayName();

    // A close that would quit is counted, not obeyed: the last window closing
    // only ends the app when Qt is told so
    const auto closeMain = [&]() {
        const bool quitOnLast = QGuiApplication::quitOnLastWindowClosed();
        QGuiApplication::setQuitOnLastWindowClosed(false);
        main->close();
        d.spin(50);
        QGuiApplication::setQuitOnLastWindowClosed(quitOnLast);
    };
    const auto bringBack = [&]() {
        if (!main->isVisible()) main->show();
        return d.waitFor(1000, [&]() { return main->isVisible(); });
    };

    // ---- Off, the default ----
    d.check(QStringLiteral("qp14: the tray is off by default"), !tray->showInTray() && !tray->startMinimised());
    d.check(QStringLiteral("qp14: off: no icon, and the desktop is not asked again"),
            !tray->iconShown() && !tray->contextMenu() && !tray->rechecking());
    d.check(QStringLiteral("qp14: off: closing the last window quits"), QGuiApplication::quitOnLastWindowClosed());
    closeMain();
    d.check(QStringLiteral("qp14: off: close is a real close"), !main->isVisible() && !tray->windowHidden());
    d.check(QStringLiteral("qp14: the window comes back"), bringBack());

    // ---- Wanted, with no tray (offscreen's own answer) ----
    tray->setShowInTray(true);
    tray->setStartMinimised(true);
    d.check(QStringLiteral("qp14: the settings are stored"),
            QSettings().value(QStringLiteral("tray/showInTray")).toBool() && QSettings().value(QStringLiteral("tray/startMinimised")).toBool());
    d.check(QStringLiteral("qp14: no tray: the desktop says so"), !QSystemTrayIcon::isSystemTrayAvailable() && !tray->trayAvailable());
    d.check(QStringLiteral("qp14: no tray: no icon, and it keeps asking"), !tray->iconShown() && tray->rechecking());
    d.check(QStringLiteral("qp14: no tray: closing still quits"), QGuiApplication::quitOnLastWindowClosed());
    tray->minimise();
    d.check(QStringLiteral("qp14: no tray: minimise does not hide"), !tray->windowHidden());
    main->showNormal();
    closeMain();
    d.check(QStringLiteral("qp14: no tray: close is a real close"), !main->isVisible() && !tray->windowHidden());
    d.check(QStringLiteral("qp14: the window comes back"), bringBack());

    // ---- A tray turns up ----
    bool available = true;
    tray->setAvailabilityProbe([&available]() { return available; });
    d.check(QStringLiteral("qp14: tray: the icon shows"), tray->iconShown() && tray->icon() && tray->icon()->isVisible());
    d.check(QStringLiteral("qp14: tray: the app outlives its windows"), !QGuiApplication::quitOnLastWindowClosed() && !tray->rechecking());

    QStringList order;
    for (QAction *a : tray->contextMenu()->actions()) order << (a->isSeparator() ? QStringLiteral("-") : a->objectName());
    d.check(QStringLiteral("qp14: the menu in the Mac's order, the volume before Quit"),
            order == QStringList({ "Tray.status", "Tray.address", "Tray.pairingCode", "-", "Tray.open", "Tray.share", "Tray.pair", "-",
                                   "Tray.volume", "-", "Tray.quit" }));
    d.check(QStringLiteral("qp14: the status says what runs"),
            text(tray, "Tray.status") == QLatin1String("DSPi connected, not sharing") && !action(tray, "Tray.status")->isEnabled());
    d.check(QStringLiteral("qp14: not sharing: no address, no code; pairing is off"),
            !shown(tray, "Tray.address") && !shown(tray, "Tray.pairingCode") && shown(tray, "Tray.share")
                && !action(tray, "Tray.share")->isChecked() && shown(tray, "Tray.pair") && !action(tray, "Tray.pair")->isEnabled());
    d.check(QStringLiteral("qp14: shown: it says Show"), text(tray, "Tray.open") == QStringLiteral("Show %1").arg(name));
    d.check(QStringLiteral("qp14: and Quit names the app"), text(tray, "Tray.quit") == QStringLiteral("Quit %1").arg(name));

    // ---- Close hides, with a tool window open ----
    QWindow *tool = d.app().shell->openWindow(QStringLiteral("edit.settings"));
    d.check(QStringLiteral("qp14: a tool window is open"), tool && d.waitFor(2000, [&]() { return tool->isVisible(); }));
    if (!tool) return;
    closeMain();
    d.check(QStringLiteral("qp14: close hides the window instead"), !main->isVisible() && tray->windowHidden());
    d.check(QStringLiteral("qp14: and the tool window with it"), !tool->isVisible());
    d.check(QStringLiteral("qp14: hidden: it says Open"), text(tray, "Tray.open") == QStringLiteral("Open %1").arg(name));
    d.check(QStringLiteral("qp14: hidden: the device stays open"), device->connected());

    // The quick volume, while hidden: the volume the sidebar shows
    bar->setVolumeMode(QStringLiteral("user"));
    bar->setUserVolume(-20.0);
    d.waitIdle();
    d.check(QStringLiteral("qp14: the volume shows while a device is open"),
            shown(tray, "Tray.volume") && text(tray, "Tray.volumeLevel") == QLatin1String("User Volume: -20.0 dB"));
    action(tray, "Tray.volumeUp")->trigger();
    d.check(QStringLiteral("qp14: hidden: Volume Up reaches the device"),
            d.waitIdle() && d.waitFor(2000, [&]() { return std::abs(bar->userVolumeDb() + 17.0) < 0.05; }));
    action(tray, "Tray.volumeDown")->trigger();
    d.waitIdle();
    action(tray, "Tray.volumeDown")->trigger();
    d.check(QStringLiteral("qp14: Volume Down twice"),
            d.waitIdle() && d.waitFor(2000, [&]() { return std::abs(bar->userVolumeDb() + 23.0) < 0.05; })
                && text(tray, "Tray.volumeLevel") == QLatin1String("User Volume: -23.0 dB"));
    bar->setUserVolume(0.0);
    d.waitIdle();
    action(tray, "Tray.volumeUp")->trigger();
    d.check(QStringLiteral("qp14: not past 0 dB"), d.waitIdle() && std::abs(bar->userVolumeDb()) < 0.05);

    action(tray, "Tray.open")->trigger();
    d.check(QStringLiteral("qp14: Open shows the window"), main->isVisible() && !tray->windowHidden());
    d.check(QStringLiteral("qp14: and the tool window"), tool->isVisible());
    d.check(QStringLiteral("qp14: shown again: it says Show"), text(tray, "Tray.open") == QStringLiteral("Show %1").arg(name));
    tool->close();

    // The icon's clicks, and the window comes back as it was
    main->showMaximized();
    d.waitFor(1000, [&]() { return main->visibility() == QWindow::Maximized; });
    tray->minimise();
    d.check(QStringLiteral("qp14: minimise hides"), !main->isVisible() && tray->windowHidden());
    emit tray->icon()->activated(QSystemTrayIcon::Trigger);
    d.check(QStringLiteral("qp14: a click on the icon shows it, still maximised"), main->visibility() == QWindow::Maximized);
    main->showNormal();
    tray->minimise();
    emit tray->icon()->activated(QSystemTrayIcon::Context);
    d.check(QStringLiteral("qp14: the context click only opens the menu"), !main->isVisible());
    emit tray->icon()->activated(QSystemTrayIcon::DoubleClick);
    d.check(QStringLiteral("qp14: a double click shows it"), main->isVisible() && main->visibility() != QWindow::Maximized);
    d.check(QStringLiteral("qp14: Window > Minimise to Tray hides it"), [&]() {
        d.app().shell->trigger(QStringLiteral("window.minimiseToTray"));
        return !main->isVisible() && tray->windowHidden();
    }());
    tray->restore();

    // ---- The tray goes away while hidden: the only way back is gone ----
    tray->minimise();
    available = false;
    tray->recheckTray();
    d.check(QStringLiteral("qp14: tray lost: the window comes back"), main->isVisible() && !tray->windowHidden());
    d.check(QStringLiteral("qp14: tray lost: no icon, and closing quits again"),
            !tray->iconShown() && QGuiApplication::quitOnLastWindowClosed() && tray->rechecking());
    available = true;
    tray->recheckTray();
    d.check(QStringLiteral("qp14: tray back: the icon, the window left alone"), tray->iconShown() && main->isVisible());

    // The switch off while hidden; minimise turns it on again, as the Mac's does
    tray->minimise();
    tray->setShowInTray(false);
    d.check(QStringLiteral("qp14: switched off while hidden: the window comes back"),
            main->isVisible() && !tray->windowHidden() && !tray->iconShown() && !tray->rechecking());
    tray->minimise();
    d.check(QStringLiteral("qp14: minimise turns the tray on"), tray->showInTray() && tray->iconShown() && !main->isVisible());
    tray->restore();

    // ---- The hub's items, on a hub of the test's own ----
    hub->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("qp14-tray-store")));
    hub->setExtraOptions(HUB_OPT_LOOPBACK);
    {
        QTcpServer probe;
        if (probe.listen(QHostAddress::LocalHost, 0)) {
            const int port = int(probe.serverPort());
            probe.close();
            hub->setPort(port);
        }
    }
    const auto iconImage = [&]() { return tray->icon()->icon().pixmap(32, 32).toImage(); };
    const QImage plainIcon = iconImage();
    action(tray, "Tray.share")->trigger();
    d.check(QStringLiteral("qp14: Share on This Network starts the hub"),
            d.waitFor(15000, [&]() { return hub->running() && !hub->busy(); }) && action(tray, "Tray.share")->isChecked());
    d.check(QStringLiteral("qp14: sharing: the status counts clients, the address shows"),
            text(tray, "Tray.status") == QLatin1String("DSPi connected, sharing, 0 clients") && shown(tray, "Tray.address")
                && text(tray, "Tray.address") == hub->address());
    d.check(QStringLiteral("qp14: sharing: the icon says so"), !plainIcon.isNull() && iconImage() != plainIcon);
    d.check(QStringLiteral("qp14: sharing: Allow a New Client is on"),
            action(tray, "Tray.pair")->isEnabled() && text(tray, "Tray.pair") == QStringLiteral("Allow a New Client…"));
    action(tray, "Tray.pair")->trigger();
    d.check(QStringLiteral("qp14: Allow opens a window and shows its code"), d.waitFor(5000, [&]() {
        return shown(tray, "Tray.pairingCode") && text(tray, "Tray.pairingCode") == QStringLiteral("Pairing code %1").arg(hub->pairingCode());
    }));
    d.check(QStringLiteral("qp14: while pairing: it says Cancel Pairing"), text(tray, "Tray.pair") == QLatin1String("Cancel Pairing"));
    action(tray, "Tray.pair")->trigger();
    d.check(QStringLiteral("qp14: Cancel Pairing closes it"), hub->pairingCode().isEmpty() && !shown(tray, "Tray.pairingCode"));

    // Hiding is presentation only: the hub serves on
    tray->minimise();
    d.spin(200);
    d.check(QStringLiteral("qp14: hidden: the hub keeps serving"), hub->running() && hub->sharing());
    tray->restore();
    hub->setAccess(QStringLiteral("none"));
    d.check(QStringLiteral("qp14: an open hub has no pairing item"),
            d.waitFor(15000, [&]() { return hub->running() && !hub->busy(); }) && !shown(tray, "Tray.pair"));
    hub->setAccess(QStringLiteral("pin"));
    d.waitFor(15000, [&]() { return hub->running() && !hub->busy(); });
    action(tray, "Tray.share")->trigger();
    d.check(QStringLiteral("qp14: Share again stops it"),
            d.waitFor(15000, [&]() { return !hub->running() && !hub->busy(); }) && !action(tray, "Tray.share")->isChecked()
                && text(tray, "Tray.status") == QLatin1String("DSPi connected, not sharing"));
    d.check(QStringLiteral("qp14: stopped: the plain icon again"), iconImage() == plainIcon);

    // ---- Starting in the tray: both settings on and a tray, the window hidden ----
    tray->setStartMinimised(true);
    tray->adoptWindow(main);
    d.check(QStringLiteral("qp14: start in the tray: the window is hidden at start"), !main->isVisible() && tray->windowHidden());
    d.check(QStringLiteral("qp14: start in the tray: it says Open"), text(tray, "Tray.open") == QStringLiteral("Open %1").arg(name));
    action(tray, "Tray.open")->trigger();
    d.check(QStringLiteral("qp14: Open from a start in the tray shows it"), main->isVisible() && !tray->windowHidden());
    tray->setStartMinimised(false);
    tray->adoptWindow(main);
    d.check(QStringLiteral("qp14: without start in the tray it stays shown"), main->isVisible());
    available = false;
    tray->setStartMinimised(true);
    tray->recheckTray();
    tray->adoptWindow(main);
    d.check(QStringLiteral("qp14: no tray: start in the tray is ignored"), main->isVisible() && !tray->windowHidden());

    // As found
    tray->setShowInTray(false);
    tray->setStartMinimised(false);
    tray->setAvailabilityProbe({});
    hub->setPort(11915);
    QSettings().remove(QStringLiteral("tray"));
    QSettings().remove(QStringLiteral("hub"));
    d.check(QStringLiteral("qp14: the tray is off again"), !tray->iconShown() && QGuiApplication::quitOnLastWindowClosed() && bringBack());
}

} // namespace

static const SmokeDriver::Section trayChecks("qp14-tray", 141, &traySection);
