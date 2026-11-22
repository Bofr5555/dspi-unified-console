// -- WP LAY0 -- The layout switch and the panel API, on the demo device: the
// layout's default (Classic for a person who finished Getting Started, Modern
// for a new install) and its stored value; the arrangement's keys; PanelTable's
// rows against Registrar's windows; the two pilots (Stats, Interrupt Monitor)
// open by action in a PanelWindow named as before, and know when they show; a
// page host takes a panel's action instead of a window, and the window opens
// again once the host goes; a switch reloads the shell in the same window and
// keeps the device session, the selection and an open tool window; Settings >
// Appearance shows the layout and stays out of the sidebar.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "LayoutController.h"
#include "MonitorController.h"
#include "PrefsMap.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SidebarController.h"
#include "StatsController.h"

#include <QCoreApplication>
#include <QPointer>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QWindow>

namespace {

QString registeredType(const QString &action) {
    for (const Registrar::WindowEntry &w : Registrar::windows())
        if (action == QLatin1String(w.action)) return QString::fromLatin1(w.type);
    return {};
}

// A LayoutController made now, from the settings as they are: what a launch
// with these settings would choose
QString layoutAtLaunch() {
    LayoutController probe(AppContext{}, nullptr);
    return probe.layout();
}

void defaultsPart(SmokeDriver &d, LayoutController *layout) {
    QSettings settings;
    d.check(QStringLiteral("lay0: this run (Getting Started done) is Classic, and stored"),
            layout->layout() == QLatin1String("classic") && !layout->modern()
                && settings.value(QLatin1String(LayoutController::kLayoutKey)).toString() == QLatin1String("classic"));

    // The first launch's choice, from the settings it finds
    const QVariant setupDone = settings.value(QStringLiteral("onboarding/setupDone"));
    settings.remove(QLatin1String(LayoutController::kLayoutKey));
    settings.remove(QStringLiteral("onboarding/setupDone"));
    const QString fresh = layoutAtLaunch();
    d.check(QStringLiteral("lay0: a new install gets Modern, stored at once"),
            fresh == QLatin1String("modern")
                && settings.value(QLatin1String(LayoutController::kLayoutKey)).toString() == QLatin1String("modern"));
    d.check(QStringLiteral("lay0: a stored layout stays, Getting Started done or not"), [&] {
        settings.setValue(QStringLiteral("onboarding/setupDone"), true);
        return layoutAtLaunch() == QLatin1String("modern");
    }());
    settings.remove(QLatin1String(LayoutController::kLayoutKey));
    d.check(QStringLiteral("lay0: an existing install (Getting Started done) keeps Classic"),
            layoutAtLaunch() == QLatin1String("classic"));
    settings.setValue(QLatin1String(LayoutController::kLayoutKey), QStringLiteral("sideways"));
    d.check(QStringLiteral("lay0: an unknown stored layout counts as none"), layoutAtLaunch() == QLatin1String("classic"));
    if (setupDone.isValid()) settings.setValue(QStringLiteral("onboarding/setupDone"), setupDone);
    else settings.remove(QStringLiteral("onboarding/setupDone"));
    settings.setValue(QLatin1String(LayoutController::kLayoutKey), layout->layout());

    layout->setLayout(QStringLiteral("sideways"));
    d.check(QStringLiteral("lay0: an unknown layout is refused"), layout->layout() == QLatin1String("classic"));

    PrefsMap *arrangement = layout->arrangement();
    d.check(QStringLiteral("lay0: the arrangement's defaults (rail, device tabs in the top bar)"),
            arrangement && arrangement->value(QStringLiteral("navigation")).toString() == QLatin1String("rail")
                && arrangement->value(QStringLiteral("deviceTabs")).toString() == QLatin1String("topBar"));
    if (!arrangement) return;
    arrangement->setProperty("navigation", QStringLiteral("tabs"));
    d.check(QStringLiteral("lay0: an arrangement key is stored under ui/arrangement"),
            QSettings().value(QStringLiteral("ui/arrangement/navigation")).toString() == QLatin1String("tabs"));
    arrangement->restoreDefaults();
    d.check(QStringLiteral("lay0: the arrangement goes back to its defaults"),
            arrangement->value(QStringLiteral("navigation")).toString() == QLatin1String("rail")
                && !QSettings().contains(QStringLiteral("ui/arrangement/navigation")));
}

void tablePart(SmokeDriver &d, Shell *shell) {
    const QVariantList rows = shell->panels();
    d.check(QStringLiteral("lay0: PanelTable has the two pilots"),
            rows.size() >= 2 && !shell->panel(QStringLiteral("tools.stats")).isEmpty()
                && !shell->panel(QStringLiteral("tools.interruptMonitor")).isEmpty());
    d.check(QStringLiteral("lay0: a window that is no panel has no row"), shell->panel(QStringLiteral("smoke.window")).isEmpty());
    QStringList bad;
    QStringList seen;
    for (const QVariant &v : rows) {
        const QVariantMap row = v.toMap();
        const QString action = row.value(QStringLiteral("action")).toString();
        bool ok = !registeredType(action).isEmpty() && !seen.contains(action);
        for (const char *key : { "panel", "windowId", "section", "tab", "title", "icon" })
            ok = ok && !row.value(QLatin1String(key)).toString().isEmpty();
        ok = ok && row.value(QStringLiteral("width")).toInt() > 0 && row.value(QStringLiteral("height")).toInt() > 0;
        if (!ok) bad << action;
        seen << action;
    }
    if (!bad.isEmpty()) std::printf("smoke: lay0: bad PanelTable rows: %s\n", qPrintable(bad.join(QStringLiteral(", "))));
    d.check(QStringLiteral("lay0: every row is complete, once, for an action Registrar::Window registered"), bad.isEmpty());
}

void pilotsPart(SmokeDriver &d, Shell *shell) {
    struct Pilot {
        const char *action;
        const char *type;
        const char *panel;
        const char *item;
    };
    const Pilot pilots[] = { { "tools.stats", "StatsWindow", "StatsPanel", "StatsWindow.platform" },
                             { "tools.interruptMonitor", "MonitorWindow", "MonitorPanel", "MonitorWindow.status" } };
    for (const Pilot &p : pilots) {
        const QString action = QString::fromLatin1(p.action);
        const QVariantMap row = shell->panel(action);
        shell->trigger(action);
        QWindow *window = nullptr;
        const bool open = d.waitFor(3000, [&] {
            window = shell->openWindow(action);
            return window && window->isVisible();
        });
        auto *quick = qobject_cast<QQuickWindow *>(window);
        QQuickItem *content = quick ? quick->property("content").value<QQuickItem *>() : nullptr;
        d.check(QStringLiteral("lay0: %1 opens in a PanelWindow by its action").arg(action),
                open && quick && content && QString::fromLatin1(quick->metaObject()->className()).startsWith(QLatin1String("PanelWindow")));
        if (!open || !quick || !content) continue;
        d.check(QStringLiteral("lay0: %1's window keeps its name, and holds %2").arg(action, QLatin1String(p.panel)),
                quick->objectName() == QLatin1String(p.type) && content->objectName() == QLatin1String(p.panel)
                    && d.item(QLatin1String(p.item), quick->contentItem()) != nullptr);
        d.check(QStringLiteral("lay0: %1's window takes the row's title and least size").arg(action),
                quick->title() == row.value(QStringLiteral("title")).toString()
                    && quick->minimumWidth() == row.value(QStringLiteral("minWidth")).toInt()
                    && quick->minimumHeight() == row.value(QStringLiteral("minHeight")).toInt()
                    && quick->property("windowId").toString() == row.value(QStringLiteral("windowId")).toString());
        d.check(QStringLiteral("lay0: %1 knows it shows, and its action").arg(QLatin1String(p.panel)),
                content->property("shown").toBool() && content->property("actionId").toString() == action
                    && !content->property("locked").toBool());
        QObject *controller = action == QLatin1String("tools.stats") ? static_cast<QObject *>(Registrar::instance<StatsController>())
                                                                     : Registrar::instance<MonitorController>();
        d.check(QStringLiteral("lay0: %1's controller polls while it shows").arg(QLatin1String(p.panel)),
                controller && controller->property("windowShown").toBool());
        window->close();
        d.waitFor(2000, [&] { return !window->isVisible(); });
        d.check(QStringLiteral("lay0: %1 knows it no longer shows").arg(QLatin1String(p.panel)),
                !content->property("shown").toBool() && controller && !controller->property("windowShown").toBool());
    }
}

void pageHostPart(SmokeDriver &d, Shell *shell) {
    QQmlComponent probe(d.app().engine);
    probe.setData("import QtQuick\n"
                  "QtObject {\n"
                  "    property string action\n"
                  "    property string section\n"
                  "    property bool take: true\n"
                  "    function showPanel(row) { action = row.action; section = row.section; return take }\n"
                  "}\n",
                  QUrl(QStringLiteral("qrc:/smoke/lay0-pagehost.qml")));
    QPointer<QObject> host = probe.create();
    d.check(QStringLiteral("lay0: the page host probe loads"), host != nullptr);
    if (!host) {
        std::printf("smoke: lay0: %s\n", qPrintable(probe.errorString()));
        return;
    }
    const QString action = QStringLiteral("tools.stats");
    QWindow *window = shell->openWindow(action);
    d.waitFor(2000, [&] { return window && window->isVisible(); });
    if (window) window->close();
    d.check(QStringLiteral("lay0: the Stats window is closed"), d.waitFor(2000, [&] { return window && !window->isVisible(); }));
    int changes = 0;
    const auto counter = QObject::connect(shell, &Shell::pageHostChanged, [&] { ++changes; });
    shell->setPageHost(host);
    shell->trigger(action);
    d.spin(150);
    d.check(QStringLiteral("lay0: with a page host, the panel's action goes to its page, not a window"),
            shell->pageHost() == host && host->property("action").toString() == action
                && host->property("section").toString() == QLatin1String("analyse") && window && !window->isVisible());
    host->setProperty("action", QString());
    // -- WP LAY1a -- (was tools.matrix, now a panel)
    shell->trigger(QStringLiteral("smoke.window"));
    QWindow *plain = nullptr;
    d.waitFor(3000, [&] {
        plain = shell->openWindow(QStringLiteral("smoke.window"));
        return plain && plain->isVisible();
    });
    d.check(QStringLiteral("lay0: a window that is no panel still opens as a window"),
            plain && plain->isVisible() && host->property("action").toString().isEmpty());
    if (plain) plain->close();
    host->setProperty("take", false);
    shell->trigger(action);
    d.check(QStringLiteral("lay0: a page host that declines lets the window open"),
            d.waitFor(3000, [&] { return window && window->isVisible(); }));
    if (window) window->close();
    host->setProperty("take", true);
    delete host.data();
    d.spin(50);
    d.check(QStringLiteral("lay0: the page host clears itself when it goes"), shell->pageHost() == nullptr && changes == 2);
    QObject::disconnect(counter);
    shell->trigger(action);
    d.check(QStringLiteral("lay0: without a page host the window opens again"),
            d.waitFor(3000, [&] { return window && window->isVisible(); }));
    if (window) window->close();
    d.spin(100);
}

void switchPart(SmokeDriver &d, Shell *shell, LayoutController *layout) {
    QQuickWindow *main = d.mainWindow();
    DeviceModel *device = d.app().device;
    auto *sidebar = Registrar::instance<SidebarController>();
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check(QStringLiteral("lay0: on the demo"),
            d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    const QString serial = device->property("serial").toString();
    QObject *loader = d.object(QStringLiteral("MainWindow.shell"));
    const auto shellNow = [loader]() { return loader ? loader->property("item").value<QObject *>() : nullptr; };
    QPointer<QObject> before = shellNow();
    QPointer<QObject> menuBefore = main->property("menuBar").value<QObject *>();
    d.check(QStringLiteral("lay0: Classic shows ClassicShell, its menu bar on the window"),
            before && before->objectName() == QLatin1String("ClassicShell") && menuBefore
                && menuBefore->objectName() == QLatin1String("Menu.bar"));

    main->setProperty("selectedChannel", 0);
    d.spin(150);
    QWindow *stats = shell->openWindow(QStringLiteral("tools.stats"));
    d.waitFor(2000, [&] { return stats && stats->isVisible(); });

    layout->setLayout(QStringLiteral("modern"));
    d.check(QStringLiteral("lay0: the switch is stored"),
            layout->modern() && QSettings().value(QLatin1String(LayoutController::kLayoutKey)).toString() == QLatin1String("modern"));
    QObject *after = nullptr;
    const bool reloaded = d.waitFor(3000, [&] {
        after = shellNow();
        return after && after != before.data();
    });
    // The old shell goes by deleteLater, which an outer event loop runs (the
    // app's, not this section's spin)
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    d.check(QStringLiteral("lay0: the switch reloads the shell in the same window, the old one gone"),
            reloaded && !before && after->objectName() == QLatin1String("ModernShell") && d.mainWindow() == main);   // -- WP LAY3 --
    d.waitIdle();
    d.spin(200);
    d.check(QStringLiteral("lay0: Modern has no menu bar, Classic's is gone"),   // -- WP LAY3 --
            !menuBefore && !main->property("menuBar").value<QObject *>() && d.object(QStringLiteral("Menu.tools.stats")) == nullptr);
    d.check(QStringLiteral("lay0: the switch keeps the device session"),
            device->connected() && device->demo() && device->property("serial").toString() == serial);
    QQuickItem *bar = d.item(QStringLiteral("GraphHost"));   // -- WP LAY3, LAY4 -- on the Equaliser page
    d.check(QStringLiteral("lay0: the switch keeps the selection"),
            main->property("selectedChannel").toInt() == 0 && bar && bar->property("selectedChannel").toInt() == 0
                && sidebar && sidebar->selectedChannel() == 0);
    d.check(QStringLiteral("lay0: an open tool window stays open"), stats && stats->isVisible());
    shell->trigger(QStringLiteral("tools.stats"));
    d.check(QStringLiteral("lay0: Modern keeps a panel in its open window"),   // -- WP LAY3 --
            shell->pageHost() != nullptr && shell->pageAction().isEmpty() && stats && stats->isVisible());

    layout->setLayout(QStringLiteral("classic"));
    d.waitFor(3000, [&] { return shellNow() && shellNow() != after; });
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    d.waitIdle();
    d.spin(200);
    bar = d.item(QStringLiteral("Sidebar"));
    d.check(QStringLiteral("lay0: and back to Classic, with the selection"),
            !layout->modern() && shellNow() && shellNow()->objectName() == QLatin1String("ClassicShell") && bar
                && bar->property("selectedChannel").toInt() == 0
                && QSettings().value(QLatin1String(LayoutController::kLayoutKey)).toString() == QLatin1String("classic"));
    if (stats) stats->close();
    main->setProperty("selectedChannel", -1);
    if (sidebar) sidebar->select(-1);
    d.spin(150);
}

void appearancePart(SmokeDriver &d, Shell *shell) {
    auto *settings = Registrar::instance<SettingsController>();
    if (!settings) return;
    bool listed = false;
    for (const QVariant &row : settings->sidebarRows(QString()))
        listed = listed || row.toMap().value(QStringLiteral("id")).toString() == QLatin1String("appearance");
    d.check(QStringLiteral("lay0: Appearance is a page in the sidebar"),   // -- WP LAY3 -- the Modern shell is there
            listed && settings->pageType(QStringLiteral("appearance")) == QLatin1String("AppearancePage"));
    const QString page = settings->currentPage();
    settings->showPage(QStringLiteral("appearance"));
    QWindow *window = nullptr;
    d.waitFor(3000, [&] {
        window = shell->openWindow(QStringLiteral("edit.settings"));
        return window && window->isVisible();
    });
    auto *quick = qobject_cast<QQuickWindow *>(window);
    d.spin(300);
    QQuickItem *combo = quick ? d.item(QStringLiteral("Appearance.layout.combo"), quick->contentItem()) : nullptr;
    d.check(QStringLiteral("lay0: Settings > Appearance shows the layout in use"),
            settings->currentPage() == QLatin1String("appearance") && combo && combo->property("currentIndex").toInt() == 0);
    settings->open(page);
    if (window) window->close();
    d.spin(100);
}

void layoutSection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    auto *layout = Registrar::instance<LayoutController>();
    d.check(QStringLiteral("lay0: the layout controller"), layout != nullptr);
    if (!layout) return;
    defaultsPart(d, layout);
    tablePart(d, shell);
    pilotsPart(d, shell);
    pageHostPart(d, shell);
    switchPart(d, shell, layout);
    appearancePart(d, shell);
}

const SmokeDriver::Section layoutSmoke("lay0-layout", 880, &layoutSection);

} // namespace
