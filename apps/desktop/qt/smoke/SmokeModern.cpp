// -- WP LAY3 -- The Modern shell on the demo device: every PanelTable action
// shows its panel on its page (and only there: one panel, one place), the rail,
// the page tabs, the arrangements, the device tabs (a second demo opened beside
// the first, as Modern does with keepOpen on; -- WP INT3 -- an edit on each
// stays on its own device), the
// device switcher, the top bar, the dock, the status strip, the notification
// drawer, and the way back to Classic. Then the Modern gallery: every page at
// 1280x800 and the sections at 1000x660, dark and light, to gallery/modern-*.
#include "SmokeDriver.h"

#include "ChannelController.h"   // -- WP INT3 --
#include "Core.h"
#include "DeviceInfoController.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"   // -- WP INT3 --
#include "DevicesController.h"
#include "LayoutController.h"
#include "NoticeCenter.h"
#include "OnboardingController.h"
#include "Registrar.h"
#include "SessionModel.h"   // -- WP INT3 --
#include "SettingsController.h"
#include "Shell.h"
#include "SidebarController.h"
#include "Snapshot.h"   // -- WP INT3 --
#include "StatsController.h"

#include <QCoreApplication>
#include <QDir>
#include <QPointer>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QSignalSpy>
#include <QTest>
#include <QWindow>
#include <algorithm>
#include <cmath>   // -- WP INT3 --
#include <functional>
#include <memory>

namespace {

struct Modern {
    SmokeDriver &d;
    Shell *shell;
    LayoutController *layout;
    QQuickWindow *main;

    QQuickItem *host() const { return qobject_cast<QQuickItem *>(shell->pageHost()); }
    QString section() const { return host() ? host()->property("section").toString() : QString(); }
    QString tab() const { return host() ? host()->property("tab").toString() : QString(); }
    QQuickItem *page(const QString &key) const { return d.item(QStringLiteral("PageHost.page.") + key); }
    QQuickItem *content(const QString &key) const {
        QQuickItem *p = page(key);
        return p ? p->property("content").value<QQuickItem *>() : nullptr;
    }
    void go(const char *section, const char *tab = "") const {
        QVariant ok;
        QMetaObject::invokeMethod(host(), "go", Q_RETURN_ARG(QVariant, ok), Q_ARG(QVariant, QString::fromLatin1(section)),
                                  Q_ARG(QVariant, QString::fromLatin1(tab)));
    }
    bool click(const QString &name, Qt::MouseButton button = Qt::LeftButton) const {
        QQuickItem *it = d.item(name);
        if (!it || !it->isVisible() || it->width() <= 0) return false;
        const QPointF at = it->mapToScene(QPointF(it->width() / 2, it->height() / 2));
        QTest::mouseClick(main, button, Qt::NoModifier, at.toPoint());
        d.spin(80);
        return true;
    }
    bool visible(const QString &name) const {
        QQuickItem *it = d.item(name);
        return it && it->isVisible();
    }
    void check(const char *what, bool ok) const { d.check(QStringLiteral("lay3: ") + QString::fromUtf8(what), ok); }
};

bool switchTo(Modern &m, const QString &layout, const char *shellName) {
    m.layout->setLayout(layout);
    QObject *loader = m.d.object(QStringLiteral("MainWindow.shell"));
    const bool ok = m.d.waitFor(3000, [&] {
        QObject *now = loader ? loader->property("item").value<QObject *>() : nullptr;
        return now && now->objectName() == QLatin1String(shellName);
    });
    // The old shell goes by deleteLater (SmokeLayout)
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    m.d.waitIdle();
    m.d.spin(200);
    return ok;
}

void shellPart(Modern &m) {
    m.check("Modern shows ModernShell, with no menu bar",
            switchTo(m, QStringLiteral("modern"), "ModernShell") && !m.main->property("menuBar").value<QObject *>()
                && m.d.item(QStringLiteral("Sidebar")) == nullptr);
    m.check("the page host is the shell's, on the Overview", m.host() && m.section() == QLatin1String("overview"));
    m.check("the dashboard is the Overview", m.visible(QStringLiteral("OverviewSection")) && m.visible(QStringLiteral("Dashboard")));
    auto *registry = DeviceRegistry::instance();
    // -- WP INT3 -- KEEPOPEN is on: MDQT2a-c rebound every controller
    m.check("choosing a device opens it beside the others (keepOpen)", registry && registry->keepOpen());
    for (const char *name : { "AppMenuButton", "DeviceTabs", "TopBar.preset", "TopBar.ab", "TopBar.search", "TopBar.ai", "TopBar.theme",
                              "TopBar.undo", "TopBar.redo", "TopBar.history", "TopBar.save", "TopBar.bell", "NavRail", "Dock.source",
                              "Dock.mode", "Dock.volume", "Dock.mute", "Dock.bypass", "Dock.meters" })
        m.d.check(QStringLiteral("lay3: %1 shows").arg(QLatin1String(name)), m.visible(QString::fromLatin1(name)));
    QQuickItem *channels = m.d.item(QStringLiteral("Dock.meters"));
    m.check("the dock has a meter per channel", channels && m.d.item(QStringLiteral("Dock.meter.0")) != nullptr);
}

// Every PanelTable action goes to a page; exactly one panel knows it is shown
void routesPart(Modern &m) {
    QString before;
    for (const QVariant &v : m.shell->panels()) {
        const QVariantMap row = v.toMap();
        const QString action = row.value(QStringLiteral("action")).toString();
        const QString key = row.value(QStringLiteral("section")).toString() + QLatin1Char('/') + row.value(QStringLiteral("tab")).toString();
        m.shell->trigger(action);
        QQuickItem *panel = nullptr;
        const bool shown = m.d.waitFor(3000, [&] {
            panel = m.content(key);
            return panel && panel->property("shown").toBool();
        });
        bool window = false;
        for (QWindow *w : QGuiApplication::topLevelWindows())
            window = window || (w->isVisible() && w != m.main && w->property("actionId").toString() == action);
        m.d.check(QStringLiteral("lay3: %1 shows %2 on its page, in no window").arg(action, row.value(QStringLiteral("panel")).toString()),
                  shown && QString::fromLatin1(panel->metaObject()->className()).startsWith(row.value(QStringLiteral("panel")).toString()) && !window
                      && m.section() + QLatin1Char('/') + m.tab() == key && m.shell->pageAction() == action
                      && panel->property("actionId").toString() == action && panel->property("inPage").toBool()
                      && panel->isVisible() && panel->width() > 100 && panel->height() > 100);
        if (!before.isEmpty() && before != key) {
            QQuickItem *left = m.content(before);
            m.d.check(QStringLiteral("lay3: %1 knows it is left").arg(before), left && !left->property("shown").toBool() && !left->isVisible());
        }
        before = key;
        m.d.waitIdle();
    }
    // Every section of the rail has something to show
    for (const char *s : { "overview", "equaliser", "routing", "effects", "measure", "analyse", "library", "system", "devices", "settings" }) {
        const bool clicked = m.click(QStringLiteral("NavRail.") + QLatin1String(s));
        m.d.check(QStringLiteral("lay3: the rail goes to %1").arg(QLatin1String(s)),
                  clicked && m.section() == QLatin1String(s) && m.content(m.section() + QLatin1Char('/') + m.tab()) != nullptr);
    }
    auto *devices = Registrar::instance<DevicesController>();
    m.go("devices", "devices");
    m.d.spin(50);
    m.check("the Devices page keeps the list live", devices && devices->property("listShown").toBool());
    QObject *switcher = m.d.object(QStringLiteral("DeviceSwitcher"));
    m.check("and the switcher stays closed over it", switcher && !switcher->property("visible").toBool());
    m.go("settings");
    m.d.spin(50);
    m.check("left, the list is not", devices && !devices->property("listShown").toBool());
    m.check("a page's panel has no close button of its own",
            m.d.item(QStringLiteral("Settings.close")) && !m.d.item(QStringLiteral("Settings.close"))->isVisible());
}

void tabsPart(Modern &m) {
    m.go("effects", "crossfeed");
    m.d.spin(80);
    const bool clicked = m.click(QStringLiteral("PageTabs.tube"));
    m.check("a page tab goes to its panel", clicked && m.tab() == QLatin1String("tube") && m.shell->pageAction() == QLatin1String("tools.tube"));
    m.click(QStringLiteral("NavRail.routing"));
    m.click(QStringLiteral("NavRail.effects"));
    m.check("a section comes back on the tab it was on", m.section() == QLatin1String("effects") && m.tab() == QLatin1String("tube"));
    m.check("What's New has no tab", m.d.item(QStringLiteral("PageTabs.whatsNew")) == nullptr);
    m.shell->trigger(QStringLiteral("help.whatsNew"));
    m.d.spin(80);
    QQuickItem *panel = m.content(QStringLiteral("settings/whatsNew"));
    m.check("its action shows it", panel && panel->property("shown").toBool());
    if (panel) QMetaObject::invokeMethod(panel, "close");
    m.d.spin(80);
    m.check("a panel that closes goes back to the page before", m.section() == QLatin1String("effects") && m.tab() == QLatin1String("tube"));
    QTest::keyClick(m.main, Qt::Key_T, Qt::ControlModifier | Qt::ShiftModifier);
    m.check("the menu table's shortcuts work, and go to the page",
            m.d.waitFor(1000, [&] { return m.shell->pageAction() == QLatin1String("tools.stats"); }));
    QObject *overflow = m.d.object(QStringLiteral("AppMenu.tools.matrix"));
    m.check("the app menu holds the menu table", overflow != nullptr && m.d.object(QStringLiteral("AppMenu.file.quit")) != nullptr);
    if (overflow) QMetaObject::invokeMethod(overflow, "triggered");
    m.check("and its rows go to their pages", m.d.waitFor(1000, [&] { return m.shell->pageAction() == QLatin1String("tools.matrix"); }));
    m.click(QStringLiteral("AppMenuButton"));
    QObject *menu = m.d.object(QStringLiteral("AppMenu"));
    m.check("the app button opens its menu", menu && m.d.waitFor(1000, [&] { return menu->property("opened").toBool(); }));
    m.d.grab(m.main, QStringLiteral("modern-app-menu.png"));
    if (menu) QMetaObject::invokeMethod(menu, "close");
    m.d.spin(200);
}

// "Open in window": the panel is in one place at a time, its polls never stop by it
void windowPart(Modern &m) {
    auto *stats = Registrar::instance<StatsController>();
    const QString key = QStringLiteral("analyse/stats");
    m.shell->trigger(QStringLiteral("tools.stats"));
    m.d.waitFor(1000, [&] { return m.content(key) && m.content(key)->property("shown").toBool(); });
    m.check("the page's panel polls", stats && stats->property("windowShown").toBool());
    const bool clicked = m.click(QStringLiteral("PageTabs.openWindow"));
    QWindow *window = nullptr;
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (w->objectName() == QLatin1String("StatsWindow")) window = w;
    const bool open = window && m.d.waitFor(2000, [&] { return window->isVisible(); });
    QQuickItem *page = m.page(key);
    m.check("Open in window opens it", clicked && open);
    m.check("then the page's panel is not shown, the window's is, and it polls",
            page && page->property("inWindow").toBool() && !m.content(key)->property("shown").toBool() && stats->property("windowShown").toBool()
                && m.shell->shownWindows().contains(QStringLiteral("tools.stats")) && m.shell->pageAction() == QLatin1String("tools.stats"));
    m.d.grab(m.main, QStringLiteral("modern-in-window.png"));
    m.go("overview");
    m.shell->trigger(QStringLiteral("tools.stats"));
    m.d.spin(100);
    m.check("its action raises the window", window && window->isVisible() && m.section() == QLatin1String("overview"));
    m.go("analyse", "stats");
    m.d.spin(50);
    m.click(QStringLiteral("PageHost.page.analyse/stats.bringBack"));
    m.check("Show Here closes the window and shows it here, polling",
            m.d.waitFor(2000, [&] { return window && !window->isVisible() && m.content(key)->property("shown").toBool(); })
                && stats->property("windowShown").toBool());
    m.shell->openWindow(QStringLiteral("tools.stats"));
    m.d.waitFor(2000, [&] { return window && window->isVisible(); });
    if (window) window->close();
    m.check("the window closed by hand gives the page its panel back, polling",
            m.d.waitFor(2000, [&] { return m.content(key)->property("shown").toBool(); }) && stats->property("windowShown").toBool());
    m.main->requestActivate();
    m.d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == m.main; });
    m.go("overview");
    m.d.spin(100);
    m.check("left, it stops polling", !stats->property("windowShown").toBool());
}

// The quick strip's state (RA): a panel's page counts as its window
void stripPart(Modern &m) {
    auto *sidebar = Registrar::instance<SidebarController>();
    if (!sidebar) return;
    m.go("overview");
    sidebar->toggleWindow(QStringLiteral("tools.stats"));
    m.d.spin(80);
    m.check("the quick strip's button shows the page as its window, active",
            m.shell->pageAction() == QLatin1String("tools.stats") && sidebar->openWindows().contains(QStringLiteral("tools.stats")));
    sidebar->toggleWindow(QStringLiteral("tools.stats"));
    m.d.spin(80);
    m.check("and again leaves it", m.section() == QLatin1String("overview") && !sidebar->openWindows().contains(QStringLiteral("tools.stats")));
}

void selectionPart(Modern &m) {
    auto *sidebar = Registrar::instance<SidebarController>();
    m.go("equaliser");
    m.d.waitIdle();
    m.d.spin(150);
    m.check("the Equaliser has a channel selected, its page under the graph",
            sidebar && sidebar->selectedChannel() >= 0 && m.visible(QStringLiteral("GraphHost")) && m.visible(QStringLiteral("EqualiserSection.channels")));
    m.check("the window's selection follows", m.main->property("selectedChannel").toInt() == sidebar->selectedChannel());
    m.go("overview");
    m.d.spin(50);
    m.check("the Overview selects none", sidebar->selectedChannel() == -1);
    sidebar->select(1);
    m.d.spin(80);
    m.check("a card's channel opens the Equaliser", m.section() == QLatin1String("equaliser") && sidebar->selectedChannel() == 1);
    m.go("overview");
    m.d.spin(50);
}

void topBarPart(Modern &m) {
    auto *sidebar = Registrar::instance<SidebarController>();
    DeviceModel *device = m.d.app().device;
    // The question a preset switch asks, which Classic's sidebar holds
    if (sidebar->unsaved()) {
        sidebar->savePreset();
        m.d.waitFor(5000, [&] { return !sidebar->unsaved(); });
    }
    const int start = device->presets()->property("activeSlot").toInt();
    QQuickItem *box = m.d.item(QStringLiteral("TopBar.preset"));
    m.check("the preset picker shows the active slot", box && box->property("shownName").toString() == sidebar->presetTitle(start));
    const float gain = device->state()->output_gain_db[0];
    device->write(QStringLiteral("lay3 dirty"), [gain](FfiCore *c) { return dspi_set_output_gain(c, 0, gain - 3.0f); });
    m.check("an unsaved change counts on Save to Device", m.d.waitFor(3000, [&] {
        QQuickItem *save = m.d.item(QStringLiteral("TopBar.save"));
        return sidebar->unsaved() && save && save->property("count").toInt() > 0 && save->property("primary").toBool();
    }));
    sidebar->requestPresetSwitch(start == 0 ? 1 : 0);
    QObject *question = m.d.object(QStringLiteral("GlobalControls.unsavedQuestion"));
    m.check("a preset switch asks about unsaved changes in Modern",
            question && m.d.waitFor(3000, [&] { return question->property("opened").toBool(); }));
    if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    m.d.waitFor(1000, [&] { return question && !question->property("opened").toBool(); });
    device->write(QStringLiteral("lay3 clean"), [gain](FfiCore *c) { return dspi_set_output_gain(c, 0, gain); });
    m.d.waitIdle();

    // The theme
    const bool dark = m.shell->darkScheme();
    const QString theme = m.layout->theme();
    m.layout->setTheme(dark ? QStringLiteral("light") : QStringLiteral("dark"));
    m.check("the theme switches at once and is kept",
            m.shell->darkScheme() != dark && QSettings().value(QLatin1String(LayoutController::kThemeKey)).toString() == m.layout->theme());
    m.layout->setTheme(QStringLiteral("sideways"));
    m.check("an unknown theme is refused", m.layout->theme() == (dark ? QLatin1String("light") : QLatin1String("dark")));
    m.layout->setTheme(theme);
    m.click(QStringLiteral("TopBar.theme"));
    QObject *themeMenu = m.d.object(QStringLiteral("TopBar.themeMenu"));
    m.check("the top bar's theme button offers it", themeMenu && m.d.waitFor(1000, [&] { return themeMenu->property("opened").toBool(); }));
    if (themeMenu) QMetaObject::invokeMethod(themeMenu, "close");
    m.d.spin(200);

    m.click(QStringLiteral("TopBar.ai"));
    QObject *ai = m.d.object(QStringLiteral("TopBar.aiPopover"));
    m.check("the AI button opens its placeholder", ai && m.d.waitFor(1000, [&] { return ai->property("opened").toBool(); }));
    m.d.grab(m.main, QStringLiteral("modern-ai.png"));
    if (ai) QMetaObject::invokeMethod(ai, "close");
    m.d.spin(200);

    m.click(QStringLiteral("TopBar.search"));
    m.d.spin(100);
    QQuickItem *search = m.d.item(QStringLiteral("Settings.search"));
    m.check("the search goes to the settings' search", m.shell->pageAction() == QLatin1String("edit.settings") && search
            && m.main->activeFocusItem() && (m.main->activeFocusItem() == search || search->isAncestorOf(m.main->activeFocusItem())));
    if (m.main->activeFocusItem()) m.main->activeFocusItem()->setFocus(false);

    // The dock
    sidebar->setVolumeMode(QStringLiteral("user"));
    sidebar->setUserVolume(-20);
    QObject *readout = m.d.object(QStringLiteral("Dock.readout"));
    if (!readout) readout = m.d.item(QStringLiteral("Dock.readout"));
    m.check("the dock's volume follows the device", m.d.waitFor(3000, [&] {
        return readout && readout->property("text").toString() == SidebarController::formatUserDb(sidebar->userVolumeDb())
            && qAbs(sidebar->userVolumeDb() + 20) < 0.1;
    }));
    sidebar->setUserVolume(0);
    QQuickItem *source = m.d.item(QStringLiteral("Dock.source"));
    m.check("the dock's source is the device's", source && source->property("shownName").toString() == sidebar->sourceTitle(sidebar->source()));
    m.d.waitIdle();
    m.go("overview");
}

void noticesPart(Modern &m) {
    auto *notices = Registrar::instance<NoticeCenter>();
    QQuickItem *strip = m.d.item(QStringLiteral("StatusStrip"));
    QQuickItem *list = m.d.item(QStringLiteral("NotificationDrawer.banners"));
    const auto types = [](QQuickItem *host) { return host ? host->property("types").toStringList() : QStringList(); };
    m.check("the status strip holds the blocking banners, the drawer the notices",
            types(strip).contains(QStringLiteral("DemoBanner")) && types(strip).contains(QStringLiteral("LockBanner"))
                && !types(strip).contains(QStringLiteral("NoticeBanner")) && types(list).contains(QStringLiteral("NoticeBanner"))
                && types(list).contains(QStringLiteral("UpdateBanner")) && !types(list).contains(QStringLiteral("DemoBanner"))
                && !types(strip).contains(QStringLiteral("SystemStatusStrip")) && !types(list).contains(QStringLiteral("SystemStatusStrip")));
    QStringList both = types(strip) + types(list);
    both << QStringLiteral("SystemStatusStrip");
    QStringList all = m.shell->banners();
    std::sort(both.begin(), both.end());
    std::sort(all.begin(), all.end());
    m.check("every banner has one place in Modern", both == all);
    m.check("the demo says so in the strip", strip && strip->height() > 0 && strip->property("bannersShown").toInt() >= 1);

    QQuickItem *bell = m.d.item(QStringLiteral("TopBar.bell"));
    notices->clear();
    m.d.spin(100);
    const int before = bell ? bell->property("shownCount").toInt() : -1;
    notices->post(QStringLiteral("LAY3 warning"), QStringLiteral("warning"));
    m.check("a notice counts on the bell and shows as a toast", bell && m.d.waitFor(1000, [&] {
        return bell->property("shownCount").toInt() == before + 1 && m.visible(QStringLiteral("ModernShell.toast"));
    }));
    m.d.spin(100);
    m.d.grab(m.main, QStringLiteral("modern-toast.png"));
    QQuickItem *drawer = m.d.item(QStringLiteral("NotificationDrawer"));
    m.click(QStringLiteral("TopBar.bell"));
    m.check("the bell opens the drawer, the toast makes way", drawer && drawer->property("open").toBool() && !m.visible(QStringLiteral("ModernShell.toast")));
    m.d.spin(100);
    m.d.grab(m.main, QStringLiteral("modern-drawer.png"));
    QQuickItem *sheet = m.d.item(QStringLiteral("NotificationDrawer.sheet"));
    m.check("it lies within the window", sheet && sheet->mapToScene(QPointF(sheet->width(), 0)).x() <= m.main->width() + 1);
    QTest::keyClick(m.main, Qt::Key_Escape);
    m.check("Escape closes it", drawer && m.d.waitFor(1000, [&] { return !drawer->property("open").toBool(); }));
    notices->clear();
    m.d.spin(100);
    m.check("a dismissed notice is kept under Earlier", bell && bell->property("shownCount").toInt() == before && drawer && drawer->property("kept").toInt() >= 1);
}

// -- WP INT3 -- Several devices on (KEEPOPEN): an edit on A, a switch to B by
// its tab, an edit there, back to A. Each edit is on the device it was made on
// (the core's state, and each session's own setter count), and both survive.
void editsPart(Modern &m, const QString &first, const QString &second, const std::function<bool(const QString &)> &clickTab) {
    auto *registry = DeviceRegistry::instance();
    auto *cc = Registrar::instance<ChannelController>();
    DeviceSession *a = registry->session(first), *b = registry->session(second);
    m.check("int3: two sessions", a && b && a != b && cc);
    if (!a || !b || !cc) return;
    const auto idle = [&](DeviceSession *s) {
        return m.d.waitFor(8000, [&] { return !s->model()->busy() && s->worker()->pending() == 0; });
    };
    const auto near = [](double x, double y, double tol) { return std::abs(x - y) <= tol; };
    const auto shown = [&](double freq) { return near(cc->band(0, 0, false).value(QStringLiteral("freq")).toDouble(), freq, 0.5); };
    const auto show = [&](const QString &id, DeviceSession *s) {
        return clickTab(id) && m.d.waitFor(3000, [&] { return registry->active() == s; }) && m.d.waitIdle() && idle(s);
    };
    m.check("int3: A's tab makes A active", show(first, a));
    const int setsB = b->model()->settersRun();
    cc->setBandType(0, 0, false, FILTER_PEAKING);
    cc->setBandValue(0, 0, false, QStringLiteral("freq"), 432);
    cc->setBandValue(0, 0, false, QStringLiteral("gain"), 4.5);
    // B's tab at once: A's writes may still be on their way
    m.check("int3: B's tab makes B active", show(second, b));
    m.check("int3: B shows its own band, not A's edit", !shown(432));
    m.check("int3: A's edit landed on A and nothing on B", idle(a) && b->model()->settersRun() == setsB);
    const int setsA = a->model()->settersRun();
    cc->setBandType(0, 0, false, FILTER_LOWSHELF);
    cc->setBandValue(0, 0, false, QStringLiteral("freq"), 1234);
    cc->setBandValue(0, 0, false, QStringLiteral("gain"), -2.5);
    m.check("int3: B's edit landed on B and nothing on A",
            idle(b) && b->model()->settersRun() > setsB && a->model()->settersRun() == setsA);
    const int setsB2 = b->model()->settersRun();
    m.check("int3: back on A by its tab", show(first, a));
    m.check("int3: A shows its own edit", m.d.waitFor(2000, [&] { return shown(432); }));
    m.check("int3: nothing more on either", idle(a) && idle(b) && a->model()->settersRun() == setsA && b->model()->settersRun() == setsB2);
    const FilterParams fa = a->model()->state()->filters[0][0], fb = b->model()->state()->filters[0][0];
    m.check("int3: A keeps its edit", fa.filter_type == FILTER_PEAKING && near(fa.freq, 432, 0.5) && near(fa.gain, 4.5, 0.05));
    m.check("int3: B keeps its own", fb.filter_type == FILTER_LOWSHELF && near(fb.freq, 1234, 0.5) && near(fb.gain, -2.5, 0.05));
    m.check("int3: and B's page shows B's", show(second, b) && m.d.waitFor(2000, [&] { return shown(1234); }));
}

void devicesPart(Modern &m) {
    auto *registry = DeviceRegistry::instance();
    auto *devices = Registrar::instance<DevicesController>();
    const QString first = registry->activeId();
    // The tabs' place: the top bar ("DeviceTabs"), the row of several ("DeviceRow"), the rail ("DeviceRail")
    QString place = QStringLiteral("DeviceTabs");
    const auto tabName = [&place](const QString &id) { return place + QStringLiteral(".tab.") + id; };
    QQuickItem *tab = m.d.item(tabName(first));
    m.check("one device is one tab in the top bar", tab && tab->isVisible() && tab->property("checked").toBool()
            && tab->property("shownName").toString() == registry->devices().value(0).toMap().value(QStringLiteral("title")).toString());
    m.click(QStringLiteral("DeviceTabs.add"));
    QObject *switcher = m.d.object(QStringLiteral("DeviceSwitcher"));
    m.check("+ opens the switcher", switcher && m.d.waitFor(1000, [&] { return switcher->property("opened").toBool(); }) && devices->property("listShown").toBool());
    QObject *popup = m.d.object(QStringLiteral("DevicesPopup"));
    m.check("and not Classic's popover", !popup || !popup->property("visible").toBool());
    m.d.spin(200);
    m.d.grab(m.main, QStringLiteral("modern-switcher.png"));
    if (switcher) QMetaObject::invokeMethod(switcher, "close");
    m.check("closed, the list is not live", m.d.waitFor(1000, [&] { return !devices->property("listShown").toBool(); }));
    m.shell->trigger(QStringLiteral("device.devices"));
    m.check("Device > Devices opens it too", switcher && m.d.waitFor(1000, [&] { return switcher->property("opened").toBool(); }));
    if (switcher) QMetaObject::invokeMethod(switcher, "manage");
    m.d.spin(300);
    m.check("Manage goes to the Devices page", m.section() == QLatin1String("devices") && !switcher->property("visible").toBool());
    m.go("overview");
    m.d.spin(100);

    // A second device beside it, as Open Demo does (-- WP INT3 -- with keepOpen on)
    devices->chooseDemo();
    const bool two = m.d.waitFor(15000, [&] { return registry->devices().size() == 2 && registry->activeId() != first; });
    m.d.waitIdle();
    m.d.spin(200);
    const QString second = registry->activeId();
    const auto shownTab = [&](const QString &id) {
        QQuickItem *t = m.d.item(tabName(id));
        return t && t->isVisible();
    };
    place = QStringLiteral("DeviceRow");
    m.check("two devices are two tabs, the new one active", two && shownTab(first) && shownTab(second));
    m.d.grab(m.main, QStringLiteral("modern-two-devices.png"));
    m.click(tabName(first));
    m.check("a tab makes its device active", m.d.waitFor(3000, [&] { return registry->activeId() == first; }) && m.d.app().device->connected());
    m.d.waitIdle();
    m.click(tabName(second));
    m.check("and back", m.d.waitFor(3000, [&] { return registry->activeId() == second; }));
    m.d.waitIdle();
    editsPart(m, first, second, [&](const QString &id) { return m.click(tabName(id)); });   // -- WP INT3 --
    // Device tabs at the rail's top
    m.layout->arrangement()->setProperty("deviceTabs", QStringLiteral("rail"));
    m.d.spin(150);
    place = QStringLiteral("DeviceRail");
    m.check("at the rail's top when arranged so", shownTab(first) && shownTab(second));
    m.d.grab(m.main, QStringLiteral("modern-devices-rail.png"));
    m.layout->arrangement()->setProperty("deviceTabs", QStringLiteral("topBar"));
    m.d.spin(150);
    const bool closed = m.click(QStringLiteral("DeviceRow.close.") + second);
    // -- WP INT3 -- the tab's device has editsPart's change: the question, Discard
    QObject *unsaved = m.d.object(QStringLiteral("FilesHost.unsavedQuestion"));
    m.check("int3: closing an edited tab asks first",
            unsaved && m.d.waitFor(3000, [&] { return unsaved->property("opened").toBool(); }));
    if (unsaved) QMetaObject::invokeMethod(unsaved, "finish", Q_ARG(QVariant, QStringLiteral("alternative")));
    m.check("a tab closes its device, the neighbour is active", closed && m.d.waitFor(8000, [&] { return registry->devices().size() == 1 && registry->activeId() == first; }));
    m.d.waitIdle();
    m.d.spin(200);
    place = QStringLiteral("DeviceTabs");
    m.check("back to one tab", shownTab(first) && m.d.app().device->connected() && m.d.app().device->demo());
}

// No device: the tab says so, the strip offers the demo
void emptyPart(Modern &m) {
    auto *registry = DeviceRegistry::instance();
    DeviceModel *device = m.d.app().device;
    registry->close(registry->activeId());
    m.check("the last device closes", m.d.waitFor(5000, [&] { return !device->connected() && registry->devices().isEmpty(); }));
    m.d.waitIdle();
    m.d.spin(200);
    m.check("with none, the tab says so", m.visible(QStringLiteral("DeviceTabs.tab.none")));
    m.go("equaliser");
    m.d.spin(150);
    m.d.grab(m.main, QStringLiteral("modern-no-device.png"));
    m.check("the strip offers the demo", m.click(QStringLiteral("DemoBanner.open")));
    m.check("and opens it", m.d.waitFor(10000, [&] { return device->connected() && device->demo(); }));
    m.d.waitIdle();
    m.d.spin(200);
    auto *sidebar = Registrar::instance<SidebarController>();
    m.check("a channel is selected again", sidebar->selectedChannel() >= 0);
    m.go("overview");
    // Another device while the Overview shows: its cards choose as before
    registry->close(registry->activeId());
    m.d.waitFor(5000, [&] { return !device->connected() && registry->devices().isEmpty(); });
    device->openDemo();
    m.d.waitFor(10000, [&] { return device->connected() && !registry->devices().isEmpty(); });
    m.d.waitIdle();
    m.d.spin(200);
    m.check("another device leaves the Overview on show", m.section() == QLatin1String("overview") && sidebar->selectedChannel() == -1);
    sidebar->select(0);
    m.d.spin(100);
    m.check("and its card opens the Equaliser", m.section() == QLatin1String("equaliser") && sidebar->selectedChannel() == 0);
    m.go("overview");
}

void arrangementPart(Modern &m) {
    m.layout->arrangement()->setProperty("navigation", QStringLiteral("tabs"));
    m.d.spin(150);
    m.check("sections across the top replace the rail", m.visible(QStringLiteral("NavTabs")) && !m.visible(QStringLiteral("NavRail")));
    QQuickItem *tabs = m.d.item(QStringLiteral("NavTabs"));
    QQuickItem *effects = tabs ? m.d.item(QStringLiteral("NavTabs.effects"), tabs) : nullptr;
    if (effects) QMetaObject::invokeMethod(effects, "clicked");
    m.d.spin(150);
    m.check("and navigate, the page's tabs as pills", m.section() == QLatin1String("effects") && m.d.item(QStringLiteral("PageTabs"))->property("pills").toBool());
    m.d.grab(m.main, QStringLiteral("modern-top-tabs.png"));
    m.layout->arrangement()->setProperty("navigation", QStringLiteral("rail"));
    m.d.spin(100);
    m.check("back to the rail", m.visible(QStringLiteral("NavRail")) && !m.visible(QStringLiteral("NavTabs")));
}

void wizardPart(Modern &m) {
    auto *ob = Registrar::instance<OnboardingController>();
    if (!ob) return;
    ob->requestSetup();
    m.d.spin(200);
    m.check("Getting Started takes the page area", ob->wizardShown() && m.visible(QStringLiteral("ModernShell.wizard")) && !m.visible(QStringLiteral("NavRail"))
            && m.shell->pageAction().isEmpty());
    m.d.grab(m.main, QStringLiteral("modern-wizard.png"));
    ob->finishSetup();
    m.d.spin(200);
    m.check("and gives it back", m.visible(QStringLiteral("NavRail")) && m.visible(QStringLiteral("PageHost")));
}

// The alink sender, which has no DSP: its page, and only what works on it
void senderPart(Modern &m) {
    auto *info = Registrar::instance<DeviceInfoController>();
    m.go("effects");
    m.d.app().core->worker()->post([](FfiCore *c) { dspi_demo_open_generation(c, 8); });
    m.d.app().device->refresh();
    m.check("on the sender", info && m.d.waitFor(6000, [&] { return info->known() && info->sender(); }));
    m.d.waitIdle();
    m.d.spin(150);
    const auto enabled = [&m](const char *name) {
        QQuickItem *it = m.d.item(QString::fromLatin1(name));
        return it && it->isEnabled();
    };
    m.check("the sender's page takes the DSP pages' place", m.visible(QStringLiteral("SenderPage")) && !m.visible(QStringLiteral("PageHost"))
            && m.shell->pageAction().isEmpty() && !m.visible(QStringLiteral("Dock")));
    m.check("the rail greys what needs a DSP, and only that",
            !enabled("NavRail.equaliser") && !enabled("NavRail.effects") && !enabled("NavRail.measure") && !enabled("NavRail.library")
                && enabled("NavRail.overview") && enabled("NavRail.system") && enabled("NavRail.devices") && enabled("NavRail.settings"));
    m.d.grab(m.main, QStringLiteral("modern-sender.png"));
    m.shell->trigger(QStringLiteral("edit.settings"));
    m.d.spin(100);
    m.check("what it keeps still shows its page", m.shell->pageAction() == QLatin1String("edit.settings") && m.visible(QStringLiteral("SettingsPanel")));
    m.shell->trigger(QStringLiteral("tools.matrix"));
    m.d.spin(100);
    m.check("a DSP action is refused", m.shell->pageAction() == QLatin1String("edit.settings"));
    m.go("overview");
    m.d.app().device->openDemo();
    m.check("back on the demo", m.d.waitFor(5000, [&] { return m.d.app().device->connected() && info->known() && !info->sender(); }));
    m.d.waitIdle();
    m.d.spin(150);
    m.check("the pages are back", m.visible(QStringLiteral("PageHost")) && m.visible(QStringLiteral("Dock")) && enabled("NavRail.effects"));
}

void backPart(Modern &m) {
    m.shell->trigger(QStringLiteral("tools.measure"));
    m.d.waitFor(1000, [&] { return m.shell->pageAction() == QLatin1String("tools.measure"); });
    QPointer<QQuickItem> panel = m.content(QStringLiteral("measure/measure"));
    m.go("overview");
    m.d.spin(50);
    // closing(): Measure's panel stops what it started, so only when it goes for good
    std::unique_ptr<QSignalSpy> closing;
    if (panel) closing = std::make_unique<QSignalSpy>(panel.data(), SIGNAL(closing()));
    m.go("measure");
    m.go("overview");
    m.d.spin(50);
    m.check("a tab switch does not close a panel", closing && closing->isValid() && closing->count() == 0);
    QPointer<QObject> host = m.shell->pageHost();
    m.check("back in Classic", switchTo(m, QStringLiteral("classic"), "ClassicShell"));
    m.check("a panel that goes with its page hears closing() once", closing && closing->count() == 1);
    m.check("the pages are gone, the menu bar is back",
            !host && !panel && !m.shell->pageHost() && m.shell->pageAction().isEmpty() && m.main->property("menuBar").value<QObject *>());
    m.shell->trigger(QStringLiteral("tools.stats"));
    QWindow *window = m.shell->openWindow(QStringLiteral("tools.stats"));
    m.check("panels open in windows again", window && m.d.waitFor(2000, [&] { return window->isVisible(); }));
    auto *stats = Registrar::instance<StatsController>();
    m.check("polling", stats && stats->property("windowShown").toBool());
    // A window left open over a switch keeps its panel; the page does not show it
    m.check("to Modern with a window open", switchTo(m, QStringLiteral("modern"), "ModernShell"));
    m.go("analyse", "stats");
    m.d.spin(100);
    QQuickItem *page = m.page(QStringLiteral("analyse/stats"));
    m.check("the window keeps its panel", window && window->isVisible() && page && page->property("inWindow").toBool() && stats->property("windowShown").toBool());
    m.check("and back with it open", switchTo(m, QStringLiteral("classic"), "ClassicShell"));
    m.check("it still polls", window && window->isVisible() && stats->property("windowShown").toBool());
    if (window) window->close();
    m.d.waitFor(1000, [&] { return m.shell->shownWindows().isEmpty() || !m.shell->shownWindows().contains(QStringLiteral("tools.stats")); });
    m.check("closed, it does not", !stats->property("windowShown").toBool());
}

// Nothing of `bar` is cut off or pushed out at the window's width
bool fits(QQuickItem *bar) {
    if (!bar) return false;
    const std::function<bool(QQuickItem *)> inside = [&](QQuickItem *it) {
        for (QQuickItem *c : it->childItems()) {
            if (!c->isVisible() || c->width() <= 0 || c->inherits("QQuickFlickable")) continue;
            const qreal left = c->mapToItem(bar, QPointF(0, 0)).x(), right = c->mapToItem(bar, QPointF(c->width(), 0)).x();
            if (left < -1 || right > bar->width() + 1) return false;
        }
        return true;
    };
    for (QQuickItem *row : bar->childItems())
        if (row->isVisible() && !inside(row)) return false;
    return inside(bar);
}

void galleryPart(Modern &m) {
    const QSize was = m.main->size();
    const Qt::ColorScheme before = m.shell->colorSchemeOverride();
    QDir(m.d.outDir()).mkpath(QStringLiteral("gallery"));
    struct Size {
        int w, h;
        bool every;
    };
    for (const Size &size : { Size{ 1280, 800, true }, Size{ 1000, 660, false } }) {
        m.main->resize(size.w, size.h);
        m.d.waitFor(1000, [&] { return m.main->contentItem()->width() == size.w; });
        for (const bool dark : { true, false }) {
            m.shell->setColorSchemeOverride(dark ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
            m.d.spin(300);
            m.d.check(QStringLiteral("lay3: the top bar fits %1 px").arg(size.w), fits(m.d.item(QStringLiteral("TopBar"))));
            m.d.check(QStringLiteral("lay3: the dock fits %1 px").arg(size.w), fits(m.d.item(QStringLiteral("Dock"))));
            QStringList keys{ QStringLiteral("overview/overview"), QStringLiteral("equaliser/peq"), QStringLiteral("devices/devices") };
            QStringList sections;
            for (const QVariant &v : m.shell->panels()) {
                const QVariantMap row = v.toMap();
                const QString s = row.value(QStringLiteral("section")).toString();
                if (!size.every && sections.contains(s)) continue;
                sections << s;
                keys << s + QLatin1Char('/') + row.value(QStringLiteral("tab")).toString();
            }
            for (const QString &key : std::as_const(keys)) {
                m.go(qPrintable(key.section(QLatin1Char('/'), 0, 0)), qPrintable(key.section(QLatin1Char('/'), 1)));
                m.d.waitIdle();
                m.d.spin(250);
                const QString name = QStringLiteral("gallery/modern-%1-%2-%3.png")
                                         .arg(size.w).arg(QString(key).replace(QLatin1Char('/'), QLatin1Char('-')),
                                                          dark ? QStringLiteral("dark") : QStringLiteral("light"));
                m.d.check(QStringLiteral("lay3: screenshot %1").arg(name), m.d.grab(m.main, name));
                QQuickItem *scroll = m.d.item(QStringLiteral("PageHost.page.%1.scroll").arg(key));
                m.d.check(QStringLiteral("lay3: %1 at %2 px needs no sideways scrolling").arg(key).arg(size.w),
                          scroll && scroll->property("contentWidth").toReal() <= scroll->width() + 0.5);
            }
        }
    }
    m.shell->setColorSchemeOverride(before);
    m.main->resize(was);
    m.go("overview");
    m.d.spin(300);
}

void modernSection(SmokeDriver &d) {
    d.allowSectionTime(300);
    Modern m{ d, d.app().shell, Registrar::instance<LayoutController>(), d.mainWindow() };
    auto *sidebar = Registrar::instance<SidebarController>();
    auto *settings = Registrar::instance<SettingsController>();
    m.check("the controllers", m.shell && m.layout && m.main && sidebar && settings && DeviceRegistry::instance());
    if (!m.layout || !m.main || !sidebar || !settings) return;
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    m.check("on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    // Windows earlier sections left open would keep their panels
    for (const QString &action : m.shell->shownWindows()) m.shell->closeWindow(action);
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (w->isVisible() && w != m.main && w->inherits("QQuickWindow")) w->close();
    m.main->requestActivate();
    d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == m.main; });
    const QString page = settings->currentPage();
    const QSize was = m.main->size();
    m.main->resize(1280, 800);
    d.spin(200);
    sidebar->select(-1);

    shellPart(m);
    if (!m.host()) return;
    routesPart(m);
    tabsPart(m);
    windowPart(m);
    stripPart(m);
    selectionPart(m);
    topBarPart(m);
    noticesPart(m);
    devicesPart(m);
    emptyPart(m);
    arrangementPart(m);
    wizardPart(m);
    senderPart(m);
    galleryPart(m);
    backPart(m);

    settings->open(page);
    sidebar->select(-1);
    m.main->resize(was);
    d.spin(200);
}

const SmokeDriver::Section modernSmoke("lay3-modern", 890, &modernSection);

} // namespace
