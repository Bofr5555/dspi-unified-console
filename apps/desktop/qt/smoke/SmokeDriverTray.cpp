// WP QD12: the tray's state machine, on the real main window and menu. Offscreen has
// no tray, so a probe stands in for one. What a real panel does with the icon, its
// menu and its clicks is not covered here.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "TrayController.h"

#include <QAction>
#include <QApplication>
#include <QKeyEvent>
#include <QMenu>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickWindow>
#include <QSettings>
#include <QSystemTrayIcon>
#include <QTimer>
#include <cstdio>

namespace {

class FakeHub : public TrayHub {
public:
    using TrayHub::TrayHub;
    State state;
    int allowed = 0, cancelled = 0;
    bool refuse = false;
    State trayState() const override { return state; }
    void setSharing(bool on) override {
        if (refuse) return;
        state.sharing = on;
        state.running = on;
        emit trayStateChanged();
    }
    void allowNewClient() override { allowed++; state.pairingCode = QStringLiteral("123456"); emit trayStateChanged(); }
    void cancelPairing() override { cancelled++; state.pairingCode.clear(); emit trayStateChanged(); }
};

TrayController *trayOf(QQmlApplicationEngine *engine) {
    return qobject_cast<TrayController *>(engine->rootContext()->contextProperty(QStringLiteral("tray")).value<QObject *>());
}

QAction *item(TrayController *tray, const char *name) {
    return tray->contextMenu() ? tray->contextMenu()->findChild<QAction *>(QLatin1String(name)) : nullptr;
}

QString itemText(TrayController *tray, const char *name) {
    QAction *a = item(tray, name);
    return a ? a->text() : QString();
}

bool itemVisible(TrayController *tray, const char *name) {
    QAction *a = item(tray, name);
    return a && a->isVisible();
}

// Evaluated in main.qml, where its ids are in scope
QVariant inMain(QObject *root, const QString &expression) {
    QQmlExpression e(QQmlEngine::contextForObject(root), root, expression);
    return e.evaluate();
}

} // namespace

void SmokeDriver::testTray() {
    section("QD12: tray");
    TrayController *tray = trayOf(m_.engine);
    auto *main = qobject_cast<QQuickWindow *>(root());
    check("QD12: controller registered as tray", tray && main);
    if (!tray || !main) return;
    DSPiBridge *b = m_.bridge;

    // Real quits go nowhere here: Qt.quit() only counts, and the last window closing
    // only ends the app when this says so
    int quits = 0;
    QObject::disconnect(m_.engine, &QQmlEngine::quit, nullptr, nullptr);
    const auto counting = connect(m_.engine, &QQmlEngine::quit, this, [&quits]() { quits++; });
    const auto closeAndCount = [&]() {
        const bool quitOnLast = QApplication::quitOnLastWindowClosed();
        QApplication::setQuitOnLastWindowClosed(false);
        const int before = quits;
        main->close();
        spin(50);
        QApplication::setQuitOnLastWindowClosed(quitOnLast);
        return quits - before;
    };
    const auto bringBack = [&]() {
        if (!main->isVisible()) main->show();
        return waitFor(1000, [&]() { return main->isVisible(); });
    };

    // Off, the default: as before this WP
    check("QD12: off by default", !tray->showInTray() && !tray->startMinimised());
    check("QD12: off: no icon, and asks the desktop nothing", !tray->iconShown() && !tray->contextMenu() && !tray->rechecking());
    check("QD12: off: the last window closing still quits", QApplication::quitOnLastWindowClosed());
    check("QD12: off: close quits", closeAndCount() == 1);
    check("QD12: off: the window comes back", bringBack());

    // On, with no tray: offscreen's own answer, not the probe's
    tray->setShowInTray(true);
    tray->setStartMinimised(true);
    check("QD12: no tray: the desktop says so", !QSystemTrayIcon::isSystemTrayAvailable() && !tray->trayAvailable());
    check("QD12: no tray: no icon", !tray->iconShown());
    check("QD12: no tray: keeps asking", tray->rechecking());
    check("QD12: no tray: start minimised is ignored", !TrayController::startsHidden());
    tray->minimise();
    check("QD12: no tray: minimise refused", main->isVisible() && !tray->windowHidden());
    check("QD12: no tray: close quits", closeAndCount() == 1);
    check("QD12: no tray: never hidden", !tray->windowHidden());
    check("QD12: no tray: the window comes back", bringBack());
    check("QD12: settings stored", QSettings().value(QStringLiteral("tray/showInTray")).toBool()
                                       && QSettings().value(QStringLiteral("tray/startMinimised")).toBool());

    // A tray turns up
    bool available = true;
    tray->setAvailabilityProbe([&available]() { return available; });
    check("QD12: tray arrives: icon up", tray->iconShown() && tray->icon() && tray->icon()->isVisible());
    check("QD12: tray up: the last window closing no longer quits", !QApplication::quitOnLastWindowClosed());

    // The Mac's menu, in order, with the hub's parts hidden
    QStringList order;
    for (QAction *a : tray->contextMenu()->actions()) order << (a->isSeparator() ? QStringLiteral("-") : a->objectName());
    check("QD12: the menu's order",
          order == QStringList({ "trayStatus", "trayAddress", "trayPairingCode", "-", "trayOpen", "trayShare", "trayPair", "-", "trayQuit" }));
    check("QD12: no hub: status is the device alone", itemText(tray, "trayStatus") == QLatin1String("DSPi connected") && !item(tray, "trayStatus")->isEnabled());
    check("QD12: no hub: hub items hidden", !itemVisible(tray, "trayShare") && !itemVisible(tray, "trayPair") && !itemVisible(tray, "trayAddress") && !itemVisible(tray, "trayPairingCode"));
    check("QD12: shown: says Show", itemText(tray, "trayOpen") == QLatin1String("Show DSPi Console"));
    check("QD12: says Quit", itemText(tray, "trayQuit") == QLatin1String("Quit DSPi Console"));

    // Close hides, with a tool window, with no prompt though changes are unsaved
    b->setFilter(0, 0, 1, b->filterFreq(0, 0) + 13.0f, 1.0f, 0.9f);
    check("QD12: there are unsaved changes", waitFor(1000, [&]() { return b->presetDirty(); }));
    auto *tool = root()->findChild<QQuickWindow *>(QStringLiteral("matrixWindow"));
    check("QD12: a tool window opens", tool != nullptr);
    if (!tool) return;
    tool->show();
    waitFor(1000, [&]() { return tool->isVisible(); });
    check("QD12: close asks nothing and does not quit", closeAndCount() == 0 && !inMain(root(), "unsavedDialog.visible").toBool());
    check("QD12: close hides the window", !main->isVisible() && tray->windowHidden());
    check("QD12: and the tool window", !tool->isVisible());
    check("QD12: hidden: says Open", itemText(tray, "trayOpen") == QLatin1String("Open DSPi Console"));

    // Everything else carries on: the session answers, the app runs
    check("QD12: hidden: the device stays connected", b->connected());
    b->setMasterVolume(-32.0f);
    spin(100);
    check("QD12: hidden: writes still reach it", qAbs(b->masterVolumeDB() + 32.0f) < 0.01f);

    // Restored from the menu, as it was
    item(tray, "trayOpen")->trigger();
    check("QD12: Open shows the window", main->isVisible() && !tray->windowHidden());
    check("QD12: and the tool window", tool->isVisible());
    check("QD12: shown again: says Show", itemText(tray, "trayOpen") == QLatin1String("Show DSPi Console"));
    tool->hide();

    main->showMaximized();
    waitFor(1000, [&]() { return main->visibility() == QWindow::Maximized; });
    tray->minimise();
    check("QD12: minimise hides", !main->isVisible());
    emit tray->icon()->activated(QSystemTrayIcon::Trigger);
    check("QD12: a click on the icon shows it, still maximised", main->visibility() == QWindow::Maximized);
    main->showNormal();
    tray->minimise();
    emit tray->icon()->activated(QSystemTrayIcon::Context);
    check("QD12: the context click only opens the menu", !main->isVisible());
    emit tray->icon()->activated(QSystemTrayIcon::DoubleClick);
    check("QD12: a double click shows it", main->isVisible() && main->visibility() != QWindow::Maximized);

    // The tray goes away while hidden: the only way back is gone
    tray->minimise();
    available = false;
    tray->recheckTray();
    check("QD12: tray lost: the window comes back", main->isVisible() && !tray->windowHidden());
    check("QD12: tray lost: no icon", !tray->iconShown() && !tray->contextMenu());
    check("QD12: tray lost: the last window closing quits", QApplication::quitOnLastWindowClosed());
    // Changes are unsaved here, so a close that is no longer a hide asks about them
    check("QD12: tray lost: close is a quit again", closeAndCount() == 0 && inMain(root(), "unsavedDialog.visible").toBool() && main->isVisible());
    inMain(root(), "unsavedDialog.chosen('cancel'); unsavedDialog.close()");
    spin(300);
    available = true;
    tray->recheckTray();
    check("QD12: tray back: icon up, window left alone", tray->iconShown() && main->isVisible());

    // The switch off while hidden
    tray->minimise();
    tray->setShowInTray(false);
    check("QD12: switched off while hidden: the window comes back", main->isVisible() && !tray->windowHidden() && !tray->iconShown());
    check("QD12: switched off: stops asking", !tray->rechecking());
    tray->minimise();
    check("QD12: minimise turns it on, as the Mac's does", tray->showInTray() && tray->iconShown() && !main->isVisible());
    tray->restore();

    // The hub's items
    {
        FakeHub hub;
        TrayController::setHub(&hub);
        const qint64 plainIcon = tray->icon()->icon().cacheKey();
        check("QD12: hub: Share appears", itemVisible(tray, "trayShare") && !item(tray, "trayShare")->isChecked());
        check("QD12: hub off: not sharing", itemText(tray, "trayStatus") == QLatin1String("DSPi connected, not sharing"));
        check("QD12: hub, not PIN: Allow hidden", !itemVisible(tray, "trayPair"));
        hub.state.pinAccess = true;
        emit hub.trayStateChanged();
        check("QD12: hub, PIN: Allow shows, off while not running",
              itemVisible(tray, "trayPair") && !item(tray, "trayPair")->isEnabled() && itemText(tray, "trayPair") == QLatin1String("Allow a New Client..."));

        hub.refuse = true;
        item(tray, "trayShare")->trigger();
        check("QD12: hub refuses: the tick goes back", !item(tray, "trayShare")->isChecked());
        hub.refuse = false;
        hub.state.address = QStringLiteral("192.0.2.1:7878");
        item(tray, "trayShare")->trigger();
        check("QD12: Share turns it on", hub.state.sharing && item(tray, "trayShare")->isChecked());
        check("QD12: running: status and address", itemText(tray, "trayStatus") == QLatin1String("DSPi connected, sharing, 0 clients")
                                                       && itemVisible(tray, "trayAddress") && itemText(tray, "trayAddress") == hub.state.address);
        check("QD12: running: the icon shows it", tray->icon()->icon().cacheKey() != plainIcon);
        hub.state.clients = 1;
        emit hub.trayStateChanged();
        check("QD12: one client", itemText(tray, "trayStatus").endsWith(QLatin1String("sharing, 1 client")));

        check("QD12: running: Allow is on", item(tray, "trayPair")->isEnabled());
        item(tray, "trayPair")->trigger();
        check("QD12: Allow asks the hub, and shows the code", hub.allowed == 1 && itemVisible(tray, "trayPairingCode")
                                                                  && itemText(tray, "trayPairingCode") == QLatin1String("Pairing code 123456"));
        check("QD12: while pairing: says Cancel", itemText(tray, "trayPair") == QLatin1String("Cancel Pairing"));
        item(tray, "trayPair")->trigger();
        check("QD12: Cancel asks the hub", hub.cancelled == 1 && !itemVisible(tray, "trayPairingCode"));

        // Hiding the window is presentation: the hub hears nothing of it
        const bool wasRunning = hub.state.running;
        tray->minimise();
        check("QD12: hidden: the hub keeps serving", hub.state.running == wasRunning && hub.state.sharing && hub.allowed == 1 && hub.cancelled == 1);
        check("QD12: hidden: the menu still reads the hub", itemText(tray, "trayStatus").endsWith(QLatin1String("1 client")));
        tray->restore();
    }
    check("QD12: hub gone: its items hide", !itemVisible(tray, "trayShare") && !itemVisible(tray, "trayPair") && itemText(tray, "trayStatus") == QLatin1String("DSPi connected"));

    // The Quit shortcut is a real close, unlike the button that sends the same call
    QObject receiver;
    QShortcutEvent otherKey(QKeySequence(Qt::CTRL | Qt::Key_W), 1);
    QCoreApplication::sendEvent(&receiver, &otherKey);
    check("QD12: another shortcut, then close: hides", closeAndCount() == 0 && !main->isVisible());
    tray->restore();
    QShortcutEvent quitKey(QKeySequence(Qt::CTRL | Qt::Key_Q), 1);
    QCoreApplication::sendEvent(&receiver, &quitKey);
    main->close();
    spin(50);
    check("QD12: the Quit shortcut asks about unsaved changes", inMain(root(), "unsavedDialog.visible").toBool() && main->isVisible() && !tray->windowHidden());
    inMain(root(), "unsavedDialog.chosen('cancel'); unsavedDialog.close()");
    spin(300);
    check("QD12: the armed shortcut lasts one turn", closeAndCount() == 0 && !main->isVisible());

    // Quit from the tray, with changes unsaved: from hidden, the prompt brings the window back
    const int quitsBefore = quits;
    item(tray, "trayQuit")->trigger();
    spin(50);
    check("QD12: Quit, unsaved: the window comes back for the prompt", main->isVisible() && !tray->windowHidden() && inMain(root(), "unsavedDialog.visible").toBool());
    check("QD12: Quit, unsaved: has not quit", quits == quitsBefore);
    inMain(root(), "unsavedDialog.chosen('cancel'); unsavedDialog.close()");
    spin(300);
    check("QD12: Cancel stays open, and close hides again", main->isVisible() && closeAndCount() == 0 && !main->isVisible());
    item(tray, "trayQuit")->trigger();
    spin(50);
    int count = quits;
    inMain(root(), "unsavedDialog.chosen('discard'); unsavedDialog.close()");
    spin(100);
    check("QD12: Discard quits", quits == count + 1);
    inMain(root(), "root.closeConfirmed = false");
    check("QD12: the window comes back after", bringBack());

    // Clean: quits with the window still hidden
    if (b->presetDirty()) {
        check("QD12: a clean preset", b->savePreset(b->activePresetSlot()) == 0 && waitFor(1000, [&]() { return !b->presetDirty(); }));
    }
    tray->minimise();
    count = quits;
    bool shownOnQuit = false;
    const auto shown = connect(main, &QWindow::visibleChanged, this, [&](bool v) { shownOnQuit = shownOnQuit || v; });
    const bool quitOnLast = QApplication::quitOnLastWindowClosed();
    item(tray, "trayQuit")->trigger();
    spin(50);
    disconnect(shown);
    QApplication::setQuitOnLastWindowClosed(quitOnLast);
    check("QD12: Quit, clean: quits", quits == count + 1);
    check("QD12: Quit, clean: without showing the window", !shownOnQuit);
    tray->restore();
    check("QD12: the window comes back after", bringBack());

    // Leave things as found
    tray->setShowInTray(false);
    tray->setStartMinimised(false);
    tray->setAvailabilityProbe([]() { return false; });
    disconnect(counting);
    connect(m_.engine, &QQmlEngine::quit, QCoreApplication::instance(), &QCoreApplication::quit, Qt::QueuedConnection);
}

// One launch with both settings stored (run.sh writes them), ending in a real quit.
// Passing means the process ends with 0: the watchdog's 3 says it never did.
void SmokeDriver::runTrayLaunch(const QString &mode) {
    section(qPrintable(QStringLiteral("QD12: launch, ") + mode));
    TrayController *tray = trayOf(m_.engine);
    auto *main = qobject_cast<QQuickWindow *>(root());
    if (!tray || !main) {
        check("QD12: launch: controller and window", false);
        QCoreApplication::exit(1);
        return;
    }
    spin(300);
    if (mode == QLatin1String("no-tray")) {
        check("QD12: no tray: the window shows at start", main->isVisible() && !tray->windowHidden() && !tray->iconShown());
        check("QD12: no tray: settings kept", tray->showInTray() && tray->startMinimised());
    } else {
        check("QD12: start minimised: never shown", !main->isVisible() && tray->windowHidden() && tray->iconShown());
        check("QD12: start minimised: says Open", itemText(tray, "trayOpen") == QLatin1String("Open DSPi Console"));
        check("QD12: start minimised: the device is up all the same", waitFor(3000, [&]() { return m_.bridge->connected(); }));
    }
    if (mode == QLatin1String("hidden") || mode == QLatin1String("app-quit")) {
        item(tray, "trayOpen")->trigger();
        check("QD12: Open from a cold start shows the window", waitFor(1000, [&]() { return main->isVisible(); }) && !tray->windowHidden());
        spin(200);
        if (mode == QLatin1String("hidden")) {
            main->close();
            check("QD12: then close hides it", !main->isVisible() && tray->windowHidden());
        }
    }
    // -- WP QD38 --
    sectionTimes();
    printf("smoke: tray launch %s: %d checks, %d failures, quitting\n", qPrintable(mode), m_checks, m_failures + qmlErrors());
    fflush(stdout);
    if (m_failures + qmlErrors()) {
        QCoreApplication::exit(1);
        return;
    }
    QTimer::singleShot(20000, this, []() {
        printf("FAIL QD12: the app did not quit\n");
        fflush(stdout);
        QCoreApplication::exit(3);
    });
    if (mode == QLatin1String("no-tray")) main->close();
    // A quit from outside the window, as a session ending is, with the window showing
    else if (mode == QLatin1String("app-quit")) QCoreApplication::quit();
    else item(tray, "trayQuit")->trigger();
}
