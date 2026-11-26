// -- WP LAY5b -- Modern's Measure, Library, System, Devices and Settings pages:
// every tab opens from the rail, from its tab, as the palette will (PageHost.go)
// and by its action; a panel's parts share one page, and a window of it starts
// on the part; a run goes on over a tab switch and Show Here, and the window's
// close stops the shown device's only; the Settings groups and the search over
// them; the presets and the files; pairing on Devices > Hubs, with no dialog
// while its page shows; System > Installation with "Open in tab", "Make Them
// Again" after another's save, and two panels at once. Pictures of the pages go
// to gallery/lay5b-*.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "DevicesController.h"
#include "GraphController.h"   // -- WP DL7 --
#include "FilesController.h"
#include "InstallationController.h"
#include "LayoutController.h"
#include "LinkController.h"
#include "MeasureController.h"
#include "ModeController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SessionModel.h"
#include "SidebarController.h"
#include "SpeakersController.h"
#include "SystemController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QGuiApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPointer>
#include <QProcess>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <QThread>

namespace {

struct Pages {
    SmokeDriver &d;
    Shell *shell;
    LayoutController *layout;
    MeasureController *mc;
    SettingsController *settings;
    QQuickWindow *main;

    void check(const char *what, bool ok) const { d.check(QStringLiteral("lay5b: ") + QString::fromUtf8(what), ok); }
    void check(const QString &what, bool ok) const { d.check(QStringLiteral("lay5b: ") + what, ok); }
    QQuickItem *host() const { return qobject_cast<QQuickItem *>(shell->pageHost()); }
    QString at() const {
        return host() ? host()->property("section").toString() + QLatin1Char('/') + host()->property("tab").toString() : QString();
    }
    void go(const char *section, const char *tab = "") const {
        QVariant ok;
        QMetaObject::invokeMethod(host(), "go", Q_RETURN_ARG(QVariant, ok), Q_ARG(QVariant, QString::fromLatin1(section)),
                                  Q_ARG(QVariant, QString::fromLatin1(tab)));
        settle();
    }
    // A tool window that was open had it
    void activate() const {
        main->requestActivate();
        d.waitFor(1500, [&] { return QGuiApplication::focusWindow() == main; });
    }
    void settle() const {
        d.spin(40);
        d.waitIdle();
        d.spin(80);
    }
    // The shown item named `name`: pages are kept, and one shows
    QQuickItem *shown(const QString &name, QQuickItem *from = nullptr) const {
        QQuickItem *i = from ? from : main->contentItem();
        if (!i->isVisible()) return nullptr;
        if (i->objectName() == name) return i;
        for (QQuickItem *c : i->childItems())
            if (QQuickItem *f = shown(name, c)) return f;
        return nullptr;
    }
    QQuickItem *shown(const char *name) const { return shown(QString::fromLatin1(name)); }
    bool click(const QString &name, Qt::MouseButton button = Qt::LeftButton) const {
        QQuickItem *i = shown(name);
        if (!i || !i->isEnabled() || i->width() <= 0) return false;
        QTest::mouseClick(main, button, Qt::NoModifier, i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint());
        settle();
        return true;
    }
    bool click(const char *name) const { return click(QString::fromLatin1(name)); }
    // As a press on it, when the page has it scrolled out of sight
    bool press(const QString &name) const {
        QQuickItem *i = shown(name);
        const bool ok = i && i->isEnabled() && SmokeDriver::press(i);   // -- INTC -- a ListRow too
        settle();
        return ok;
    }
    QString text(const char *name) const {
        QQuickItem *i = shown(name);
        return i ? i->property("text").toString() : QString();
    }
    QQuickItem *content(const char *key) const {
        QVariant page;
        if (!host()) return nullptr;
        QMetaObject::invokeMethod(host(), "page", Q_RETURN_ARG(QVariant, page), Q_ARG(QVariant, QString::fromLatin1(key)));
        auto *item = page.value<QQuickItem *>();
        return item ? item->property("content").value<QQuickItem *>() : nullptr;
    }
    QStringList tabs() const {
        QStringList out;
        for (const QVariant &v : d.item(QStringLiteral("PageTabs"))->property("entries").toList())
            out << v.toMap().value(QStringLiteral("tab")).toString();
        return out;
    }
    QWindow *window(const char *name) const {
        for (QWindow *w : QGuiApplication::topLevelWindows())
            if (w->objectName() == QLatin1String(name) && w->isVisible()) return w;
        return nullptr;
    }
    bool dialog(const char *name) const {
        QObject *o = d.object(QString::fromLatin1(name));
        return o && o->property("opened").toBool();
    }
};

bool switchTo(Pages &p, const QString &layout, const char *shellName) {
    p.layout->setLayout(layout);
    QObject *loader = p.d.object(QStringLiteral("MainWindow.shell"));
    const bool ok = p.d.waitFor(3000, [&] {
        QObject *now = loader ? loader->property("item").value<QObject *>() : nullptr;
        return now && now->objectName() == QLatin1String(shellName);
    });
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    p.settle();
    return ok;
}

struct Page {
    const char *section, *tab, *item;
};
const Page kPages[] = {
    { "measure", "measure", "Measure.tabs.none" }, { "measure", "results", "" }, { "measure", "live", "" }, { "measure", "tune", "" },
    { "measure", "documents", "" }, { "measure", "units", "" },
    { "library", "presets", "LibraryPresets.list" }, { "library", "snapshots", "SnapshotsPanel" }, { "library", "history", "HistoryPanel" },
    { "library", "files", "LibraryFiles.body" }, { "library", "autoeq", "AutoEqPanel" },
    { "system", "status", "" }, { "system", "clock", "" }, { "system", "installation", "" }, { "system", "events", "EventsPanel" },
    { "devices", "devices", "DevicesPopup.demo" }, { "devices", "hubs", "DevicesPopup.connect" }, { "devices", "firmware", "FirmwarePanel" },
    { "settings", "application", "Settings.row.about" }, { "settings", "display", "Settings.row.graphing" },
    { "settings", "device", "Settings.row.overview" }, { "settings", "control", "" },
};

void waysPart(Pages &p) {
    const QHash<QString, QStringList> all{
        { QStringLiteral("measure"), { "measure", "results", "live", "tune", "documents", "units" } },
        { QStringLiteral("library"), { "presets", "snapshots", "history", "files", "autoeq" } },
        { QStringLiteral("system"), { "status", "clock", "installation", "events" } },
        { QStringLiteral("devices"), { "devices", "hubs", "firmware" } },
        { QStringLiteral("settings"), { "application", "display", "device", "control" } },
    };
    for (auto it = all.begin(); it != all.end(); ++it) {
        p.go(qPrintable(it.key()));
        p.check(it.key() + QStringLiteral(" has its tabs, in order"), p.tabs() == it.value());
        if (p.tabs() != it.value()) std::printf("smoke: lay5b: %s: %s\n", qPrintable(it.key()), qPrintable(p.tabs().join(QLatin1Char(' '))));
    }
    for (const Page &page : kPages) {
        const QString key = QStringLiteral("%1/%2").arg(QLatin1String(page.section), QLatin1String(page.tab));
        const QString item = QLatin1String(page.item).contains(QLatin1String(".none")) ? QString() : QString::fromLatin1(page.item);
        const auto here = [&] { return p.at() == key && (item.isEmpty() || p.shown(item)); };
        p.go("overview");
        const bool rail = p.click(QStringLiteral("NavRail.") + QLatin1String(page.section));
        p.check(key + QStringLiteral(" opens from the rail and its tab"),
                rail && (p.at() == key || p.click(QStringLiteral("PageTabs.") + QLatin1String(page.tab))) && here());
        p.go("overview");
        p.go(page.section, page.tab);
        p.check(key + QStringLiteral(" opens as the palette opens it"), here());
        QQuickItem *tab = p.shown(QStringLiteral("PageTabs.") + QLatin1String(page.tab));
        p.check(key + QStringLiteral(": its tab is the one marked"), tab && tab->property("checked").toBool());
    }
    const std::pair<const char *, const char *> actions[] = {
        { "tools.measure", "measure" }, { "device.snapshots", "library/snapshots" }, { "edit.history", "library/history" },
        { "autoeq.browse", "library/autoeq" }, { "window.system", "system/" }, { "device.events", "system/events" },
        { "device.firmwareUpdate", "devices/firmware" }, { "edit.settings", "settings/" } };
    for (const auto &a : actions) {
        p.go("overview");
        p.shell->trigger(QString::fromLatin1(a.first));
        p.settle();
        p.check(QStringLiteral("%1 opens its page").arg(QLatin1String(a.first)),
                p.at().startsWith(QLatin1String(a.second)) && p.shell->pageAction() == QLatin1String(a.first)
                    && !p.shell->shownWindows().contains(QLatin1String(a.first)));
    }
    // The System action with Events last: its own panel, not the section's tab
    p.go("system", "clock");
    p.go("system", "events");
    p.shell->trigger(QStringLiteral("window.system"));
    p.settle();
    p.check("System… is its panel though Events was the section's tab", p.at() == QLatin1String("system/status"));
    // Old places still lead there (a panel's own tab)
    p.go("settings", "settings");
    p.check("a panel's own tab goes to one of its parts", p.at().startsWith(QLatin1String("settings/")) && p.shown("SettingsPanel"));
}

// ---- Measure -----------------------------------------------------------------------------

bool simulated(Pages &p) {
    const auto ready = [&] { return p.mc->simAvailable() && p.mc->micChoice() == QLatin1String("sim") && !p.mc->setupOutputs().isEmpty(); };
    if (!p.d.waitFor(3000, ready)) p.mc->setMicChoice(QStringLiteral("sim"));   // an earlier section chose another
    if (!p.d.waitFor(5000, ready)) return false;
    bool one = false;
    for (const QVariant &v : p.mc->setupOutputs()) {
        const QVariantMap m = v.toMap();
        const bool on = !one;
        one = true;
        p.mc->setOutputOn(m.value(QStringLiteral("output")).toInt(), on);
    }
    p.mc->setPlacement(QStringLiteral("listeningPosition"));
    p.mc->setSweepMs(1000);
    p.mc->setPasses(3);
    return true;
}

int measureThreads(const MeasureController *mc) {
    int n = 0;
    for (const QThread *t : mc->findChildren<QThread *>(Qt::FindDirectChildrenOnly)) n += t->isRunning();
    return n;
}

void measurePart(Pages &p) {
    auto *registry = DeviceRegistry::instance();
    auto *sc = Registrar::instance<SpeakersController>();
    p.go("measure", "measure");
    QPointer<QQuickItem> panel = p.content("measure/measure");
    p.check("Measure's page has no tabs of its own in it", panel && p.shown("MeasurePanel") && !p.shown("Measure.tabs"));
    QPointer<QQuickItem> setup = panel;
    p.go("measure", "live");
    p.check("its parts are one panel", p.content("measure/measure") == panel && panel->property("part").toString() == QLatin1String("live")
                && panel->property("index").toInt() == 2);
    p.check("the simulated microphone, one output", simulated(p));
    // A run goes on over the tabs, with its Stop on each
    p.go("measure", "measure");
    p.mc->start();
    p.check("a measurement starts", p.d.waitFor(10000, [&] { return p.mc->measuring(); }));
    QQuickItem *dot = p.shown("PageTabs.measure.dot");
    p.check("its tab has a dot", dot && dot->property("on").toBool());
    p.go("measure", "documents");
    p.check("it goes on over a tab, whose page has its Stop", p.mc->measuring() && p.shown("Measure.stop"));
    p.go("overview");
    p.check("and over a section", p.mc->measuring());
    // A window of it starts on the part, and Show Here moves it
    p.go("measure", "results");
    p.click("PageTabs.openWindow");
    QWindow *w = nullptr;
    p.check("Open in Window", p.d.waitFor(3000, [&] { return (w = p.window("MeasureWindow")) != nullptr; }));
    QObject *inner = w ? w->property("content").value<QObject *>() : nullptr;
    p.check("it starts on the page's part, with tabs of its own", inner && inner->property("part").toString().isEmpty()
                && inner->property("index").toInt() == 1);
    p.check("the page says the panel's name", p.main->isVisible() && p.shown("PageHost.page.measure/measure.inWindow"));
    p.go("measure", "measure");
    p.click("PageHost.page.measure/measure.bringBack");
    p.check("Show Here closes it", p.d.waitFor(3000, [&] { return !p.window("MeasureWindow") && panel->property("shown").toBool(); }));
    p.check("and the run goes on", p.mc->measuring());
    // The window's close stops what runs on the device shown, not another's
    registry->openDemo();
    p.check("a second demo", p.d.waitFor(10000, [&] { return registry->opening() == 0 && registry->sessions().size() == 2; }));
    DeviceSession *a = registry->mainSession(), *b = registry->sessions().value(1);
    if (!b) return;
    registry->activate(b);
    p.settle();
    QPointer<MeasureSession> runs;
    for (MeasureSession *m : p.mc->sessions())
        if (m->session() == a) runs = m;
    p.check("the run goes on beside it", runs && runs->measuring() && !p.mc->measuring());
    b->model()->write(QStringLiteral("lay5b volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
    p.check("B's own simulated microphone", simulated(p));
    p.mc->start();
    p.check("B measures too", p.d.waitFor(10000, [&] { return p.mc->measuring(); }) && runs && runs->measuring());
    p.shell->openWindow(QStringLiteral("tools.measure"));
    p.d.waitFor(3000, [&] { return (w = p.window("MeasureWindow")) != nullptr; });
    if (w) w->close();
    p.check("the window closed by hand stops B's", p.d.waitFor(10000, [&] { return !p.mc->running(); }));
    p.check("A's goes on", runs && runs->measuring());
    if (runs) runs->stop();
    p.check("A's is stopped", p.d.waitFor(10000, [&] { return runs && !runs->running(); }));
    p.mc->clearError();
    p.d.waitFor(3000, [&] { return !p.shell->shownWindows().contains(QStringLiteral("tools.measure")); });

    // A device that is only looked at has no measurement thread
    p.go("overview");
    const int before = measureThreads(p.mc);
    registry->openDemo();
    p.d.waitFor(10000, [&] { return registry->opening() == 0 && registry->sessions().size() == 3; });
    DeviceSession *c = registry->sessions().value(2);
    if (c) registry->activate(c);
    p.settle();
    p.check("a device that is shown has no measurement thread", c && p.mc->sessions().size() == 3 && measureThreads(p.mc) == before);
    p.check("nor when a property is read", p.mc->phaseText().size() >= 0 && !p.mc->connected() == false && measureThreads(p.mc) == before);
    p.go("measure", "measure");
    p.check("Measure shown on it, it has one, and its microphone and outputs",
            measureThreads(p.mc) == before + 1 && p.d.waitFor(8000, [&] { return p.mc->simAvailable(); }));
    for (DeviceSession *s : { b, c })
        if (s) registry->close(s->id());
    p.d.waitFor(10000, [&] { return registry->sessions().size() == 1; });
    p.settle();

    // A Confirm's words are not under the Measure table
    if (sc) {
        sc->reload();
        p.d.waitFor(5000, [&] { return sc->loaded(); });
        sc->confirm();
        p.check("a layout is confirmed", p.d.waitFor(5000, [&] { return sc->status() == QLatin1String("confirmed") && sc->said().startsWith(QLatin1String("Confirmed:")); }));
        p.settle();   // its answer, not an earlier section's
        p.check("its words are not for under Measure's table", sc->saidMeasure().isEmpty());
        const int out = p.mc->setupOutputs().value(0).toMap().value(QStringLiteral("output")).toInt();
        const QString was = sc->roleOf(out);
        p.mc->setOutputRole(out, was == QLatin1String("sub") ? QStringLiteral("full") : QStringLiteral("sub"));
        p.check("a Measure edit's are", p.d.waitFor(5000, [&] { return !sc->saidMeasure().isEmpty() && sc->saidMeasure() == sc->said(); }));
        if (!was.isEmpty()) p.mc->setOutputRole(out, was);
        p.check("and kept", p.d.waitFor(5000, [&] { return was.isEmpty() || sc->roleOf(out) == was; }));
        p.settle();
    }
    // A connection that may not play: what is left, and the page leaves the part that went
    p.go("measure", "live");
    DeviceRegistry::instance()->close(DeviceRegistry::instance()->activeId());
    p.check("with no device Measure has what needs none", p.d.waitFor(5000, [&] {
        return p.tabs() == QStringList({ QStringLiteral("results"), QStringLiteral("documents"), QStringLiteral("passive") });
    }));
    p.settle();
    p.check("and the page is on one of them", p.at() == QLatin1String("measure/passive") && p.content("measure/measure") == panel);
    p.d.app().device->openDemo();
    p.d.waitFor(10000, [&] { return p.d.app().device->connected(); });
    p.settle();
    p.check("back on the device, all of it", p.tabs().size() == 6 && p.at() == QLatin1String("measure/results"));
}

// ---- Settings ----------------------------------------------------------------------------

void settingsPart(Pages &p) {
    p.settings->open(QStringLiteral("about"));
    p.go("settings");
    p.check("Settings is on its page's group", p.at() == QLatin1String("settings/application") && p.shown("Settings.row.about"));
    p.check("the list is the group's alone", !p.shown("Settings.row.graphing") && !p.shown("Settings.row.overview"));
    p.check("and has no heading: the tab is it", [&] {
        const QVariantList rows = p.content("settings/settings")->property("rows").toList();
        for (const QVariant &v : rows)
            if (v.toMap().contains(QStringLiteral("heading"))) return false;
        return !rows.isEmpty();
    }());
    p.click("Settings.button.updates");
    p.click("PageTabs.display");
    p.check("another tab shows its group's first page", p.settings->currentPage() == QLatin1String("graphing") && p.shown("Settings.row.graphing"));
    p.click("PageTabs.application");
    p.check("back, the page it was on", p.settings->currentPage() == QLatin1String("updates"));
    // -- WP DL7 -- Modern has no back and forward buttons (the page list is the
    // navigation); the controller's history stays
    p.settings->back();
    p.settle();
    p.check("Back goes to the page before, and its tab", p.settings->currentPage() == QLatin1String("graphing") && p.at() == QLatin1String("settings/display"));
    // A link into another group, from elsewhere
    p.go("overview");
    p.settings->showPage(QStringLiteral("global"));
    p.settle();
    p.check("a page opened from elsewhere brings its tab", p.at() == QLatin1String("settings/device") && p.settings->currentPage() == QLatin1String("global"));
    p.settings->showPage(QStringLiteral("advanced"));
    p.settle();
    p.check("and from another group's", p.at() == QLatin1String("settings/application"));
    p.settings->open(QStringLiteral("coreVoltage"));
    p.settle();
    p.check("a hidden page keeps the tab it is opened from", p.at() == QLatin1String("settings/application"));
    p.settings->open(QStringLiteral("about"));
    // The search is over every group
    QQuickItem *search = p.shown("Settings.search");
    if (search) search->setProperty("text", QStringLiteral("phase"));
    p.settle();
    p.check("the search finds pages of every group", p.shown("Settings.row.graphing") != nullptr);
    p.press(QStringLiteral("Settings.button.graphing"));
    p.check("and goes to the one chosen, with its tab", p.settings->currentPage() == QLatin1String("graphing") && p.at() == QLatin1String("settings/display"));
    if (search) search->setProperty("text", QString());
    p.settle();
    // A window has it meanwhile: its page is its own, and the tab follows after
    p.shell->openWindow(QStringLiteral("edit.settings"));
    p.check("Settings in a window", p.d.waitFor(3000, [&] { return p.window("SettingsWindow") != nullptr; }));
    p.settings->open(QStringLiteral("updates"));
    p.settle();
    p.check("the page's tabs do not follow it", p.at() == QLatin1String("settings/display"));
    if (QWindow *w = p.window("SettingsWindow")) w->close();
    p.check("closed, the page has its page and that one's tab", p.d.waitFor(3000, [&] {
        return !p.window("SettingsWindow") && p.at() == QLatin1String("settings/application") && p.shown("Settings.row.updates");
    }) && p.settings->currentPage() == QLatin1String("updates"));
    p.activate();
    // The device goes: its groups too
    p.go("settings", "device");
    p.check("a device page", p.settings->groupOf(p.settings->currentPage()) == QLatin1String("device"));
    DeviceRegistry::instance()->close(DeviceRegistry::instance()->activeId());
    p.check("without a device two groups", p.d.waitFor(5000, [&] {
        return p.tabs() == QStringList({ QStringLiteral("application"), QStringLiteral("display") });
    }));
    p.settle();
    p.check("and the page is on one of them", p.at() == QLatin1String("settings/application") && p.settings->currentPage() == QLatin1String("about"));
    p.d.app().device->openDemo();
    p.d.waitFor(10000, [&] { return p.d.app().device->connected(); });
    p.settle();
}

// ---- -- WP DL7 -- Settings in the design language ----------------------------------------

void dl7Part(Pages &p) {
    const auto chk = [&](const char *what, bool ok) { p.d.check(QStringLiteral("dl7: ") + QString::fromUtf8(what), ok); };
    const auto pic = [&](const char *name) { p.d.grab(p.main, QStringLiteral("gallery/dl7-%1.png").arg(QLatin1String(name))); };
    p.activate();
    p.settings->open(QStringLiteral("about"));
    p.go("settings", "application");
    chk("no back and forward in Modern, the title stays",
        p.shown("Settings.sidebar") && !p.shown("Settings.back") && !p.shown("Settings.forward") && p.shown("Settings.title"));
    // The page's column: at the left margin, no wider than 760
    QQuickItem *loader = p.shown("Settings.page");
    auto *flick = loader ? loader->property("item").value<QQuickItem *>() : nullptr;
    auto *inner = flick ? flick->property("contentItem").value<QQuickItem *>() : nullptr;
    QQuickItem *column = inner && !inner->childItems().isEmpty() ? inner->childItems().first() : nullptr;
    chk("the page's column is left-aligned at 760",
        column && flick->width() > 800 && qAbs(column->x() - 16) < 0.5 && qAbs(column->width() - 760) < 0.5);
    pic("about");

    // Graphing: a value field beside each slider, the value not in the label
    p.settings->open(QStringLiteral("graphing"));
    p.settle();
    auto *graph = Registrar::instance<GraphController>();
    QQuickItem *field = p.shown("Graphing.lineWidth.field");
    const double width = graph ? graph->graphLineWidth() : 0;
    chk("a slider row has its field and title", field && p.shown("Graphing.lineWidth.slider") && p.text("Graphing.lineWidth.title") == QLatin1String("Line Width")
        && p.shown("Graphing.gridOpacity.field") && p.shown("Graphing.dbCenter.field"));
    if (field && graph) {
        QMetaObject::invokeMethod(field, "committed", Q_ARG(double, 2.7));
        p.settle();
        chk("a typed value is held to the slider's step", graph->graphLineWidth() == 2.5);
        graph->setGraphLineWidth(width);
    }
    pic("graphing");

    // AI Agents: one Limits button, content-sized segments, ROLES' switch, the activity
    p.settings->open(QStringLiteral("ai"));
    p.settle();
    chk("one Limits button", (p.shown("AiAgents.limitsSettings") != nullptr) + (p.shown("AiAgents.limitsOffer.action") != nullptr) == 1);
    QQuickItem *segments = p.shown("AiAgents.segments");
    chk("the agent picker is grey segments sized to their content",
        segments && !p.shown("AiAgents.tabs") && segments->width() < 600 && p.shown("AiAgents.segments.1"));
    if (segments) {
        QMetaObject::invokeMethod(segments, "chosen", Q_ARG(int, 1));
        p.settle();
        QQuickItem *tabs = p.d.item(QStringLiteral("AiAgents.tabs"));
        chk("and choose the agent", tabs && tabs->property("currentIndex").toInt() == 1 && segments->property("current").toInt() == 1);
        QMetaObject::invokeMethod(segments, "chosen", Q_ARG(int, 0));
    }
    chk("the roles switch is there, in the section", p.shown("AiAgents.roles.switch") && p.shown("AiAgents.detect.switch"));
    chk("the activity: its list or the empty state, never both",
        (p.shown("AiAgents.log.list") != nullptr) != (p.shown("AiAgents.logEmpty") != nullptr));
    pic("ai-page");

    // The AI activity page: a quiet Stop, an empty state
    p.go("settings", "aiAgents");
    QQuickItem *stop = p.d.item(QStringLiteral("AiAgents.stop"));
    chk("Stop is no red fill", stop && !stop->property("destructive").toBool());
    chk("the activity page: its list or the empty state",
        (p.shown("AiActivity.list") != nullptr) != (p.shown("AiAgentsPanel.empty") != nullptr));
    pic("ai-activity");

    // What's New: a page, no Done
    p.go("settings", "whatsNew");
    QQuickItem *release = p.shown("WhatsNew.release.0");
    chk("What's New has no Done in its page", p.shown("WhatsNew.list") && !p.shown("WhatsNew.done"));
    chk("and its column stops at 760", release && release->width() <= 760.5);
    pic("whats-new");

    // Speakers: a section, one state line
    p.go("settings", "device");
    p.settings->open(QStringLiteral("speakers"));
    p.settle();
    QQuickItem *layout = p.shown("Speakers.layout");
    QQuickItem *confirm = p.shown("SpeakersPage.confirm");
    chk("Speakers is a section, not a card", layout && layout->property("flat").toBool());
    chk("one state line", !p.shown("SpeakersPage.stamp") && !p.shown("SpeakersPage.said") && p.shown("SpeakersPage.state"));
    chk("Confirm is blue only while it can act", confirm && confirm->property("primary").toBool() == confirm->isEnabled());
    pic("speakers");
    p.settings->open(QStringLiteral("about"));
    p.settle();
}

// ---- Library -----------------------------------------------------------------------------

void libraryPart(Pages &p) {
    auto *sidebar = Registrar::instance<SidebarController>();
    DeviceModel *device = p.d.app().device;
    p.activate();
    p.go("library", "presets");
    const int active = device->presets()->activeSlot();
    const auto slot = [](int n, const char *what = "") { return QStringLiteral("LibraryPresets.slot.%1%2").arg(n).arg(QLatin1String(what)); };
    p.check("every slot has a row", [&] {
        for (int i = 0; i < 4; ++i)
            if (!p.shown(slot(i))) return false;
        return p.shown("LibraryPresets.list")->property("count").toInt() == device->presets()->rowCount();
    }());
    p.check("the active one says so, with Save and no Recall", p.shown(slot(active, ".save")) && !p.shown(slot(active, ".recall")));
    p.check("what loads at power-on is said", !p.text("LibraryPresets.startup").isEmpty());
    const int other = active == 1 ? 2 : 1;
    sidebar->requestPresetCopy(other);
    p.check("the active one copied there", p.d.waitFor(6000, [&] { return sidebar->presetOccupied(other) && !sidebar->presetBusy(); }));
    p.settle();
    p.check("Recall", p.click(slot(other, ".recall")));
    p.check("recalls it", p.d.waitFor(8000, [&] { return device->presets()->activeSlot() == other; }));
    p.settle();
    p.check("and the row moves", p.shown(slot(other, ".save")) && p.shown(slot(active, ".recall")));
    // LAY3b's singleton: not in the window's tree
    QObject *menus = p.d.app().engine->singletonInstance<QObject *>("UConsole", "ContextMenus");
    QObject *menu = menus ? menus->property("slotMenu").value<QObject *>() : nullptr;
    const auto open = [&] { return menu && p.d.waitFor(1500, [&] { return menu->property("opened").toBool(); }); };
    p.check("a slot's button opens its menu", p.click(slot(0, ".menu")) && open());
    const QVariant target = menu ? menu->property("target") : QVariant();
    const QVariantMap t = target.canConvert<QJSValue>() ? target.value<QJSValue>().toVariant().toMap() : target.toMap();
    p.check("for that slot, with the shell's dialogs", t.value(QStringLiteral("slot"), -1).toInt() == 0
                && t.value(QStringLiteral("dialogs")).value<QObject *>() != nullptr);
    p.d.grab(p.main, QStringLiteral("gallery/lay5b-slot-menu.png"));
    if (menu) QMetaObject::invokeMethod(menu, "close");
    p.d.waitFor(1500, [&] { return menu && !menu->property("visible").toBool(); });
    p.check("so does a right-click", p.click(slot(3), Qt::RightButton) && open());
    if (menu) QMetaObject::invokeMethod(menu, "close");
    p.settle();
    sidebar->requestPresetSwitch(active);
    p.d.waitFor(8000, [&] { return device->presets()->activeSlot() == active && !sidebar->presetBusy(); });
    sidebar->clearPreset(other);
    p.d.waitFor(6000, [&] { return !sidebar->presetOccupied(other) && !sidebar->presetBusy(); });
    p.settle();

    auto *files = Registrar::instance<FilesController>();
    p.go("library", "files");
    p.check("the files' rows are the menu's", p.shown("LibraryFiles.file.importFilters") && p.shown("LibraryFiles.file.exportConfiguration")
                && p.text("LibraryFiles.file.openFile") == QStringLiteral("Open File…"));
    p.check("Save File needs a file", !p.shown("LibraryFiles.file.saveFile")->isEnabled());
    QQuickItem *body = p.content("library/files");
    p.check("the templates are listed, with no picker", p.d.waitFor(5000, [&] { return body && !body->property("templates").toList().isEmpty(); })
                && !p.dialog("TemplatePicker"));
    p.check("one opens", p.press(QStringLiteral("LibraryFiles.template.0")) && p.d.waitFor(10000, [&] { return files->offline(); }));
    p.settle();
    p.check("the page says which file is open", p.text("LibraryFiles.open").startsWith(QLatin1String("Open now")));
    p.check("and Save File is offered", p.shown("LibraryFiles.file.saveFile") && p.shown("LibraryFiles.file.saveFile")->isEnabled());
}

// The demo alone again, whatever the file opened on
void demoAlone(Pages &p) {
    auto *registry = DeviceRegistry::instance();
    for (DeviceSession *s : registry->sessions())
        if (!s->isMain()) registry->close(s->id());
    p.d.waitFor(10000, [&] { return registry->sessions().size() == 1; });
    if (!registry->mainSession()->model()->demo()) registry->mainSession()->model()->openDemo();
    p.d.waitFor(10000, [&] { return p.d.app().device->connected() && p.d.app().device->demo(); });
    p.settle();
}

// ---- Devices and System ------------------------------------------------------------------

class AlinkHub {
public:
    bool start(const QString &store, bool pairing) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        QDir().mkpath(store);
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList{ "--demo", "hub", "--alink", "--loopback", "--json", "--no-keep-awake", "--forget-clients", "--store",
                                          store, "--port", "0", pairing ? "--pair" : "--open" });
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
                const QString event = e.value(QLatin1String("event")).toString();
                if (event == QLatin1String("status")) port = e.value(QLatin1String("port")).toInt();
                if (event == QLatin1String("pairing")) pin = e.value(QLatin1String("pin")).toString();
            }
            if (port > 0 && (!pairing || !pin.isEmpty())) return true;
        }
        return false;
    }
    ~AlinkHub() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    QString url() const { return QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(port); }
    int port = 0;
    QString pin;

private:
    QProcess m_proc;
};

void devicesPart(Pages &p) {
    auto *registry = DeviceRegistry::instance();
    auto *link = Registrar::instance<LinkController>();
    auto *list = Registrar::instance<DevicesController>();
    p.check("DeviceRegistry.close is not QML's: closeOpen asks and heeds a pin",
            registry->metaObject()->indexOfMethod("close(QString)") < 0 && list->metaObject()->indexOfMethod("closeOpen(QString)") >= 0);
    p.activate();
    p.go("devices", "devices");
    p.check("This Computer lists what is here", p.shown("DevicesPopup.demo") && !p.shown("DevicesPopup.connect") && list->listShown());
    p.check("with what is done to the device shown", p.shown("DevicesSection.device.identify") && p.shown("DevicesSection.device.firmwareUpdate"));
    p.click("DevicesSection.hubs");
    p.check("its button goes to the hubs, live too", p.at() == QLatin1String("devices/hubs") && list->listShown() && p.shown("DevicesPopup.lookForHubs")
                && !p.shown("DevicesPopup.demo"));
    AlinkHub hub;
    p.check("a PIN hub runs (DSPI_SMOKE_UCONSOLE)", hub.start(p.d.outDir() + QStringLiteral("/lay5b-pin-hub"), true));
    if (!hub.port) return;
    const QString demo = registry->activeId();
    QQuickItem *address = p.shown("DevicesPopup.address");
    if (address) address->setProperty("text", QStringLiteral("127.0.0.1:%1").arg(hub.port));
    p.check("Connect to it", p.press(QStringLiteral("DevicesPopup.connect")));
    p.check("it asks for its PIN", p.d.waitFor(30000, [&] { return !link->busy() && link->phase() == QLatin1String("needs_pairing"); }));
    p.settle();
    p.check("on the page, and no dialog opens", p.shown("DevicesHubs.pair") && p.shown("DevicesHubs.pair.pin") && !p.dialog("PairingDialog"));
    p.check("in the core's words", p.text("DevicesHubs.pair.text") == link->phaseText());
    p.check("the PIN has the focus", p.shown("DevicesHubs.pair.pin")->hasActiveFocus());
    p.check("Pair waits for six digits", !p.shown("DevicesHubs.pair.accept")->isEnabled());
    p.d.grab(p.main, QStringLiteral("gallery/lay5b-pair.png"));
    // Left, the dialog asks; back on the page, the page does
    p.go("overview");
    p.check("elsewhere the dialog asks", p.d.waitFor(2000, [&] { return p.dialog("PairingDialog"); }));
    p.go("devices", "hubs");
    p.check("back here, the page", p.d.waitFor(2000, [&] { return !p.dialog("PairingDialog"); }) && p.shown("DevicesHubs.pair")
                && link->phase() == QLatin1String("needs_pairing"));
    QQuickItem *pin = p.shown("DevicesHubs.pair.pin");
    const QString wrong = hub.pin == QLatin1String("111111") ? QStringLiteral("222222") : QStringLiteral("111111");
    if (pin) pin->setProperty("text", wrong);
    p.check("a wrong PIN", p.press(QStringLiteral("DevicesHubs.pair.accept"))
                && p.d.waitFor(15000, [&] { return !link->busy() && !link->errorText().isEmpty(); }));
    p.settle();
    p.check("is said under the field, which empties", p.text("DevicesHubs.pair.error") == link->errorText() && pin->property("text").toString().isEmpty());
    pin->setProperty("text", hub.pin);
    p.check("the right one pairs", p.press(QStringLiteral("DevicesHubs.pair.accept"))
                && p.d.waitFor(15000, [&] { return !link->busy() && link->phase() == QLatin1String("choose_device"); }));
    p.settle();
    p.check("the card goes", !p.shown("DevicesHubs.pair"));
    p.check("the hub's name, role and security beside it", p.text("DevicesHubs.hub.name") == link->hubName()
                && !p.text("DevicesHubs.hub.role").isEmpty() && !p.text("DevicesHubs.hub.security").isEmpty());
    p.check("its units are listed", p.shown("DevicesPopup.hubDevice.DEMO-ALINK-A"));
    p.d.grab(p.main, QStringLiteral("gallery/lay5b-hub.png"));
    p.check("one opens beside the demo", p.press(QStringLiteral("DevicesPopup.hubDevice.DEMO-ALINK-A"))
                && p.d.waitFor(20000, [&] { return registry->activeId() != demo && p.d.app().device->remote(); }));
    p.settle();
    p.go("devices", "devices");
    p.check("This Computer lists the open devices", p.shown("DevicesPopup.openHeading") != nullptr);
    demoAlone(p);
    link->closeSession();
    p.settle();
}

// A second client of the same hub, which renames a room: the other installer
bool elsewhere(const QString &url, const QByteArray &edits) {
    FfiCore *other = dspi_core_new();
    if (!other) return false;
    QByteArray buf(256 * 1024, '\0');
    const auto reply = [&](const std::function<uint32_t(char *, uint32_t)> &call) {
        buf.fill('\0');
        call(buf.data(), uint32_t(buf.size()));
        const QJsonObject o = QJsonDocument::fromJson(buf.constData()).object();
        return o.value(QLatin1String("ok")).toBool() ? o : QJsonObject();
    };
    const QByteArray u = url.toUtf8();
    bool ok = !reply([&](char *b, uint32_t n) { return dspi_link_connect(other, u.constData(), "lay5b-other", "0", "", "", b, n); }).isEmpty();
    if (ok) {
        const QJsonObject view = reply([&](char *b, uint32_t n) { return dspi_installation_view(other, b, n); });
        QByteArray draft = QJsonDocument(view.value(QLatin1String("draft")).toObject()).toJson(QJsonDocument::Compact);
        const QJsonObject edited = reply([&](char *b, uint32_t n) { return dspi_installation_draft_edit(other, draft.constData(), edits.constData(), b, n); });
        draft = QJsonDocument(edited).toJson(QJsonDocument::Compact);
        ok = !edited.isEmpty() && !reply([&](char *b, uint32_t n) { return dspi_installation_save(other, draft.constData(), b, n); }).isEmpty();
    }
    dspi_link_close(other);
    dspi_core_free(other);
    return ok;
}

void systemPart(Pages &p) {
    auto *registry = DeviceRegistry::instance();
    auto *link = Registrar::instance<LinkController>();
    auto *list = Registrar::instance<DevicesController>();
    auto *sys = Registrar::instance<SystemController>();
    auto *inst = Registrar::instance<InstallationController>();
    auto *modes = Registrar::instance<ModeController>();
    if (!link || !list || !sys || !inst || !modes) return p.check("the system's controllers", false);
    p.go("system", "installation");
    p.check("not on a hub it says so", p.shown("SystemWindow.none") && !inst->shown());
    const QString demo = registry->activeId();
    AlinkHub hub;
    p.check("uconsole --demo hub --alink runs", hub.start(p.d.outDir() + QStringLiteral("/lay5b-hub"), false));
    if (!hub.port) return;
    link->connectToHub(hub.url());
    p.check("the hub's units are listed", p.d.waitFor(30000, [&] {
        return !link->busy() && link->phase() == QLatin1String("choose_device") && !link->devices().isEmpty();
    }));
    int handle = -1;
    for (const QVariant &v : link->devices())
        if (v.toMap().value(QStringLiteral("serial")).toString() == QLatin1String("DEMO-ALINK-A"))
            handle = v.toMap().value(QStringLiteral("handle")).toInt();
    list->chooseHubDevice(handle, QStringLiteral("DEMO-ALINK-A"));
    p.check("its unit opens beside the demo", p.d.waitFor(20000, [&] { return registry->activeId() != demo && p.d.app().device->remote(); }));
    if (registry->activeId() == demo) return;
    p.check("the installation is read", p.d.waitFor(20000, [&] { return sys->available() && inst->available() && !inst->unitIds().isEmpty(); }));
    p.settle();

    // ---- The parts of the System panel
    p.check("System > Installation hosts InstallationPanel", p.at() == QLatin1String("system/installation") && p.shown("InstallationPanel")
                && p.shown("Installation.unit.A") && !p.shown("SystemWindow.units") && !p.shown("SystemWindow.tabs"));
    p.d.grab(p.main, QStringLiteral("gallery/lay5b-installation.png"));
    p.go("system", "status");
    p.check("Units has the units and the scenes", p.shown("SystemWindow.units") && !p.shown("SystemWindow.editor") && !p.shown("InstallationPanel"));
    p.go("system", "clock");
    p.check("Clock and Links has the editor", p.shown("SystemWindow.editor") && p.shown("SystemWindow.save") && !p.shown("SystemWindow.units")
                && p.shown("SystemWindow.summary"));
    modes->setProperty("mode", QStringLiteral("simple"));
    p.settle();
    if (modes->property("mode").toString() == QLatin1String("simple")) {
        p.check("Simple has the status only", p.tabs() == QStringList({ QStringLiteral("status"), QStringLiteral("events") }) && p.at() == QLatin1String("system/status"));
        modes->setProperty("mode", QStringLiteral("advanced"));
        p.settle();
        p.check("Advanced has all again", p.tabs().size() == 4);
    }

    // ---- Open in tab, by the serial as the hub gives it
    p.go("system", "installation");
    p.d.waitFor(5000, [&] { return inst->available(); });
    const auto tabButton = [](const char *unit) { return QStringLiteral("Installation.openInTab.") + QLatin1String(unit); };
    QString unitB, unitC;
    for (const QString &id : inst->unitIds()) {
        const QString serial = inst->element(id).value(QStringLiteral("hubSerial")).toString();
        if (serial == QLatin1String("DEMO-ALINK-B")) unitB = id;
        if (serial == QLatin1String("DEMO-ALINK-C")) unitC = id;
    }
    p.check("every unit but the one shown has Open in tab", !p.shown(tabButton("A")) && p.shown(tabButton(qPrintable(unitB)))
                && p.shown(tabButton(qPrintable(unitC))));
    const QString a = registry->activeId();
    p.check("Open in tab", p.press(tabButton(qPrintable(unitB))));
    p.check("opens B's tab and shows it", p.d.waitFor(20000, [&] {
        return registry->holder(QStringLiteral("DEMO-ALINK-B")) && registry->active() == registry->holder(QStringLiteral("DEMO-ALINK-B"));
    }));
    p.check("B's page has the installation, with Show tab for A", p.d.waitFor(20000, [&] {
        QQuickItem *b = p.shown(tabButton("A"));
        return inst->available() && b && b->property("text").toString() == QLatin1String("Show tab");
    }) && !p.shown(tabButton(qPrintable(unitB))));
    p.check("which shows it", p.press(tabButton("A")) && p.d.waitFor(5000, [&] { return registry->activeId() == a; }));
    p.d.waitFor(20000, [&] { return inst->available() && !inst->unitIds().isEmpty(); });
    p.settle();

    // ---- A refused edit's words stay while they are shown
    p.check("an edit is refused, in words", !inst->addRoom(QString()) && !inst->issueOf(QStringLiteral("new:room"), QString()).isEmpty());
    p.d.spin(6000);   // past the 5 s poll
    p.check("the words are kept over the polls", !inst->issueOf(QStringLiteral("new:room"), QString()).isEmpty());

    // ---- Two panels at once: one poll, until both are gone
    p.click("PageTabs.openWindow");
    QWindow *w = nullptr;
    p.check("System in a window, on its Installation tab", p.d.waitFor(3000, [&] { return (w = p.window("SystemWindow")) != nullptr; })
                && w->property("content").value<QObject *>() && w->property("content").value<QObject *>()->property("onInstallation").toBool());
    p.check("is read there", inst->shown());
    {
        inst->panelShown(true);
        inst->panelShown(true);
        inst->panelShown(false);
        p.check("one of two panels gone, it is still read", inst->shown());
        inst->panelShown(false);
        p.check("and with the window's alone", inst->shown());
    }
    if (w) w->close();
    p.check("closed, the page reads it again", p.d.waitFor(3000, [&] { return !p.window("SystemWindow") && p.shown("InstallationPanel") && inst->shown(); }));
    p.go("overview");
    p.check("left, nothing reads it", !inst->shown());
    p.go("system", "installation");
    p.d.waitFor(5000, [&] { return inst->available(); });

    // ---- Changed elsewhere: the edits are kept aside, and made again
    p.check("an edit", inst->addRoom(QStringLiteral("Cellar")) && inst->dirty());
    p.check("another installer saves meanwhile", elsewhere(hub.url(), R"([{"op":"renameElement","id":"main","name":"Hall"}])"));
    p.check("it is seen", p.d.waitFor(15000, [&] { return inst->stale(); }));
    p.settle();
    p.check("Reload, and nothing to make again yet", p.shown("Installation.reload") && !p.shown("Installation.lost"));
    p.press(QStringLiteral("Installation.reload"));
    p.check("reloaded, the edits are offered again, in the core's sentences", !inst->stale() && !inst->dirty() && !inst->lost().isEmpty()
                && inst->lost().join(QLatin1Char(' ')).contains(QLatin1String("Cellar")) && p.shown("Installation.lost") && p.shown("Installation.remake"));
    p.d.grab(p.main, QStringLiteral("gallery/lay5b-make-again.png"));
    p.check("made again", p.press(QStringLiteral("Installation.remake")) && inst->dirty() && inst->lost().isEmpty()
                && inst->roomIds().contains(QStringLiteral("Cellar")) && !p.shown("Installation.lost"));
    p.check("on the installation as it is now", inst->element(QStringLiteral("main")).value(QStringLiteral("name")).toString() == QLatin1String("Hall"));
    inst->save();
    p.check("and saved, with the other's change kept", p.d.waitFor(15000, [&] { return !inst->busy() && !inst->dirty(); }) && !inst->messageFailed()
                && inst->element(QStringLiteral("main")).value(QStringLiteral("name")).toString() == QLatin1String("Hall")
                && inst->roomIds().contains(QStringLiteral("Cellar")));
    // One that cannot be made again says why, and stays offered
    p.d.waitFor(8000, [&] { return inst->editable(); });
    p.check("an edit of something", inst->rename(QStringLiteral("Cellar"), QStringLiteral("Wine cellar")));
    p.check("another installer removes it", elsewhere(hub.url(), R"([{"op":"removeElement","id":"Cellar"}])"));
    p.check("that is seen", p.d.waitFor(15000, [&] { return inst->stale(); }));
    inst->reload();
    p.check("it cannot be made again, which is said", !inst->remake() && inst->messageFailed() && !inst->message().isEmpty() && !inst->lost().isEmpty());
    p.settle();
    p.check("Leave Them", p.press(QStringLiteral("Installation.dropLost")) && inst->lost().isEmpty());

    demoAlone(p);
    link->closeSession();
    p.settle();
}

// ---- Pictures ----------------------------------------------------------------------------

void galleryPart(Pages &p) {
    const Qt::ColorScheme before = p.shell->colorSchemeOverride();
    const QSize was = p.main->size();
    for (const QSize &size : { QSize(1280, 800), QSize(1000, 660) }) {
        p.main->resize(size);
        p.d.waitFor(1500, [&] { return p.main->contentItem()->width() == size.width(); });
        for (const Page &page : kPages) {
            p.go(page.section, page.tab);
            for (const bool dark : { true, false }) {
                if (size.width() == 1000 && !dark) continue;
                p.shell->setColorSchemeOverride(dark ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
                p.settle();
                const QString name = QStringLiteral("gallery/lay5b-%1-%2-%3-%4.png")
                                         .arg(size.width()).arg(QLatin1String(page.section), QLatin1String(page.tab), dark ? QStringLiteral("dark") : QStringLiteral("light"));
                p.check(QStringLiteral("screenshot %1").arg(name), p.d.grab(p.main, name));
            }
            QQuickItem *scroll = p.shown(QStringLiteral("PageHost.page.%1.scroll").arg(p.host()->property("pageKey").toString()));
            p.check(QStringLiteral("%1/%2 at %3 px needs no sideways scrolling").arg(QLatin1String(page.section), QLatin1String(page.tab)).arg(size.width()),
                    scroll && scroll->property("contentWidth").toReal() <= scroll->width() + 0.5);
        }
    }
    p.shell->setColorSchemeOverride(before);
    p.main->resize(was);
    p.settle();
}

void pagesSection(SmokeDriver &d) {
    d.allowSectionTime(420);
    Pages p{ d, d.app().shell, Registrar::instance<LayoutController>(), Registrar::instance<MeasureController>(),
             Registrar::instance<SettingsController>(), d.mainWindow() };
    p.check("the controllers", p.shell && p.layout && p.mc && p.settings && p.main && DeviceRegistry::instance());
    if (!p.shell || !p.layout || !p.mc || !p.settings || !p.main) return;
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    p.check("on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    for (const QString &action : p.shell->shownWindows()) p.shell->closeWindow(action);
    p.activate();
    const QSize was = p.main->size();
    const QString page = p.settings->currentPage();
    p.main->resize(1280, 800);
    device->write(QStringLiteral("lay5b volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
    p.settle();
    QDir(d.outDir()).mkpath(QStringLiteral("gallery"));
    p.check("Modern", switchTo(p, QStringLiteral("modern"), "ModernShell") && p.host());
    if (!p.host()) return;

    waysPart(p);
    measurePart(p);
    settingsPart(p);
    dl7Part(p);   // -- WP DL7 --
    libraryPart(p);
    demoAlone(p);
    devicesPart(p);
    systemPart(p);
    galleryPart(p);

    device->write(QStringLiteral("lay5b volume"), [](FfiCore *c) { return dspi_set_user_volume(c, 0.0f); });
    p.settings->open(page);
    p.check("back in Classic", switchTo(p, QStringLiteral("classic"), "ClassicShell"));
    QWindow *w = p.shell->openWindow(QStringLiteral("tools.measure"));
    p.check("Classic's Measure window has its tabs", w && d.waitFor(3000, [&] { return w->isVisible(); })
                && d.item(QStringLiteral("Measure.tabs"), qobject_cast<QQuickWindow *>(w)->contentItem())->isVisible());
    if (w) w->close();
    // As it was found: the System window on its Status
    if ((w = p.shell->openWindow(QStringLiteral("window.system")))) {
        if (QObject *system = w->property("content").value<QObject *>())
            QMetaObject::invokeMethod(system, "showPart", Q_ARG(QVariant, QStringLiteral("status")));
        w->close();
    }
    p.main->resize(was);
    d.spin(200);
}

const SmokeDriver::Section pagesSmoke("lay5b-pages", 898, &pagesSection);

} // namespace
