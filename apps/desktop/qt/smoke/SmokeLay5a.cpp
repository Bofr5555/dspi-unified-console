// -- WP LAY5a -- Modern's Overview, Routing, Effects and Analyse on the demo
// device: every page and tab opens from the rail, from its tab, as the palette
// will (PageHost.go) and by its action; the effects' dots, the signal chain and
// its switches, the Overview's chips, facts and cards, the matrix's selection
// with the inspector beside it, the generator's dot and Stop. Pictures of the
// pages go to gallery/lay5a-*.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DevicesController.h"
#include "EffectsController.h"
#include "LayoutController.h"
#include "LinkController.h"
#include "PrefsMap.h"
#include "Registrar.h"
#include "RtaController.h"
#include "Shell.h"
#include "SidebarController.h"
#include "SiggenController.h"
#include "SystemController.h"
#include "ToolsController.h"

#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QGuiApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPointer>
#include <QProcess>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <algorithm>
#include <cmath>

namespace {

struct Pages {
    SmokeDriver &d;
    Shell *shell;
    LayoutController *layout;
    ChannelController *cc;
    SidebarController *sc;
    EffectsController *fx;
    ToolsController *tc;
    QQuickWindow *main;

    void check(const char *what, bool ok) const { d.check(QStringLiteral("lay5a: ") + QString::fromUtf8(what), ok); }
    void check(const QString &what, bool ok) const { d.check(QStringLiteral("lay5a: ") + what, ok); }
    const DspState *state() const { return d.app().device->state(); }
    QQuickItem *host() const { return qobject_cast<QQuickItem *>(shell->pageHost()); }
    QString at() const {
        return host() ? host()->property("section").toString() + QLatin1Char('/') + host()->property("tab").toString() : QString();
    }
    void go(const QString &section, const QString &tab = QString()) const {
        QVariant ok;
        QMetaObject::invokeMethod(host(), "go", Q_RETURN_ARG(QVariant, ok), Q_ARG(QVariant, section), Q_ARG(QVariant, tab));
        settle();
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
    static QPoint centre(QQuickItem *i) { return i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint(); }
    bool click(const QString &name, Qt::MouseButton button = Qt::LeftButton) const {
        QQuickItem *i = shown(name);
        if (!i || i->width() <= 0) return false;
        QTest::mouseClick(main, button, Qt::NoModifier, centre(i));
        settle();
        return true;
    }
    bool click(const char *name) const { return click(QString::fromLatin1(name)); }
    QString fact(const char *name) const {
        QQuickItem *i = shown(QStringLiteral("OverviewSummary.") + QLatin1String(name));
        return i ? i->property("shownName").toString() : QString();
    }
    // A dot on a tab: 1 on, 0 off, -1 none
    int dot(const char *tab) const {
        QQuickItem *i = shown(QStringLiteral("PageTabs.%1.dot").arg(QLatin1String(tab)));
        return i ? int(i->property("on").toBool()) : -1;
    }
    void resize(int w, int h) const {
        main->resize(w, h);
        d.waitFor(1500, [&] { return main->contentItem()->width() == w && main->contentItem()->height() == h; });
        settle();
    }
    // `i` whole inside `outer`
    static bool inside(QQuickItem *i, QQuickItem *outer) {
        if (!i || !outer) return false;
        const QRectF r = i->mapRectToItem(outer, QRectF(0, 0, i->width(), i->height()));
        return r.left() >= -0.5 && r.top() >= -0.5 && r.right() <= outer->width() + 0.5 && r.bottom() <= outer->height() + 0.5;
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
    const char *section, *tab, *action, *item; // an item of the panel's
};
const Page kPages[] = {
    { "routing", "matrix", "tools.matrix", "Matrix.table" },
    { "effects", "crossfeed", "tools.crossfeed", "CrossfeedWindow.header" },
    { "effects", "loudness", "tools.loudness", "LoudnessWindow.header" },
    { "effects", "leveller", "tools.leveller", "LevellerWindow.header" },
    { "effects", "psybass", "tools.psybass", "PsybassWindow.header" },
    { "effects", "subharmonic", "tools.subharmonic", "SubharmonicWindow.header" },
    { "effects", "tube", "tools.tube", "TubeWindow.header" },
    { "effects", "upmixer", "tools.upmixer", "UpmixerWindow.header" },
    { "analyse", "spectrum", "tools.spectrum", "SpectrumPanel" },
    { "analyse", "signalGenerator", "tools.signalGenerator", "SiggenPanel" },
    { "analyse", "stats", "tools.stats", "StatsPanel" },
    { "analyse", "interruptMonitor", "tools.interruptMonitor", "MonitorPanel" },
};
const char *const kEffects[] = { "crossfeed", "loudness", "leveller", "psybass", "subharmonic", "tube", "upmixer" };

// ---- Every page and tab, three ways ----------------------------------------------

void waysPart(Pages &p) {
    for (const Page &page : kPages) {
        const QString key = QStringLiteral("%1/%2").arg(QLatin1String(page.section), QLatin1String(page.tab));
        // The action (a menu, a shortcut, a button)
        p.go(QStringLiteral("overview"));
        p.shell->trigger(QString::fromLatin1(page.action));
        p.settle();
        p.check(key + QStringLiteral(" opens by its action, on its page and in no window"),
                p.at() == key && p.shown(page.item) && !p.shell->shownWindows().contains(QLatin1String(page.action)));
        // The rail, then its tab
        p.go(QStringLiteral("overview"));
        const bool rail = p.click(QStringLiteral("NavRail.") + QLatin1String(page.section));
        p.check(key + QStringLiteral(": the rail shows its section"), rail && p.at().startsWith(QLatin1String(page.section)));
        if (p.shown(QStringLiteral("PageTabs.") + QLatin1String(page.tab))) p.click(QStringLiteral("PageTabs.") + QLatin1String(page.tab));
        p.check(key + QStringLiteral(" opens from the rail and its tab"), p.at() == key && p.shown(page.item));
        // As the palette goes there
        p.go(QStringLiteral("overview"));
        p.go(QLatin1String(page.section), QLatin1String(page.tab));
        p.check(key + QStringLiteral(" opens as the palette opens it"), p.at() == key && p.shown(page.item));
    }
    p.click("NavRail.overview");
    p.check("the Overview from the rail", p.at() == QLatin1String("overview/overview") && p.shown("OverviewSummary"));
    // What stands beside and over the panels
    p.go(QStringLiteral("routing"));
    p.check("the matrix has its inspector and its button", p.shown("RoutingInspector.sheet") && p.shown("RoutingTools.inspector"));
    p.go(QStringLiteral("effects"));
    p.check("the effects have the signal chain and their buttons", p.shown("EffectsChain.sheet") && p.shown("EffectsTools.inspector") && p.shown("EffectsTools.limiters"));
    p.go(QStringLiteral("analyse"));
    p.check("the analysis pages have no inspector", !p.shown("EffectsChain.sheet") && !p.shown("RoutingInspector.sheet") && p.shown("AnalyseTools"));
    p.go(QStringLiteral("library"));
    p.check("nor has another section any of these", !p.shown("AnalyseTools") && !p.shown("EffectsTools") && !p.shown("RoutingTools"));
}

// ---- Effects ---------------------------------------------------------------------

bool isOn(const Pages &p, const QString &id) {
    const DspState *s = p.state();
    if (!s) return false;
    if (id == QLatin1String("crossfeed")) return p.fx->property("crossfeedOn").toBool();
    if (id == QLatin1String("loudness")) return p.fx->property("loudnessOn").toBool();
    if (id == QLatin1String("leveller")) return p.fx->property("levellerOn").toBool();
    if (id == QLatin1String("psybass")) return p.fx->property("psybassOn").toBool();
    if (id == QLatin1String("subharmonic")) return p.tc->subharm(QStringLiteral("enabled")) != 0;
    if (id == QLatin1String("tube")) return p.tc->tube(QStringLiteral("enabled")) != 0;
    return p.tc->upmix(QStringLiteral("enabled")) != 0;
}

void effectsPart(Pages &p) {
    p.go(QStringLiteral("effects"), QStringLiteral("crossfeed"));
    int on = 0;
    for (const char *id : kEffects) {
        p.check(QStringLiteral("%1 has a dot, as the device says").arg(QLatin1String(id)), p.dot(id) == int(isOn(p, QLatin1String(id))));
        on += isOn(p, QLatin1String(id));
    }
    const auto summary = [&] {
        QQuickItem *i = p.shown("EffectsChain.summary");
        return i ? i->property("text").toString() : QString();
    };
    p.check("none is on at first", on == 0 && summary().contains(QLatin1String("No effect")));
    // The signal chain, in the firmware's order: the demo has the output masks
    QStringList order;
    QList<std::pair<qreal, QString>> found;
    for (const char *id : { "input", "leveller", "upmixer", "matrix", "crossfeed", "subharmonic", "psybass", "tube", "crossover", "output",
                            "gain", "loudness", "limiter", "delay" })
        if (QQuickItem *i = p.shown(QStringLiteral("EffectsChain.") + QLatin1String(id))) found.append({ Pages::centre(i).y(), QLatin1String(id) });
    std::stable_sort(found.begin(), found.end(), [](const auto &a, const auto &b) { return a.first < b.first; });
    for (const auto &f : found) order << f.second;
    p.check(QStringLiteral("the signal chain is the firmware's: %1").arg(order.join(QLatin1Char(' '))),
            p.fx->property("crossfeedMaskSupported").toBool() && p.fx->property("loudnessMaskSupported").toBool()
                && order == QStringList({ QStringLiteral("input"), QStringLiteral("leveller"), QStringLiteral("upmixer"), QStringLiteral("matrix"),
                                          QStringLiteral("crossfeed"), QStringLiteral("subharmonic"), QStringLiteral("psybass"),
                                          QStringLiteral("tube"), QStringLiteral("crossover"), QStringLiteral("output"), QStringLiteral("gain"),
                                          QStringLiteral("loudness"), QStringLiteral("limiter"), QStringLiteral("delay") }));
    // Each effect's switch in the chain: the device, the dot and the panel follow
    for (const char *id : kEffects) {
        const QString name = QStringLiteral("EffectsChain.%1.on").arg(QLatin1String(id));
        QPointer<QQuickItem> toggle = p.shown(name);
        const bool clicked = p.click(name);
        p.check(QStringLiteral("%1: its switch in the chain switches it on").arg(QLatin1String(id)),
                clicked && p.d.waitFor(2000, [&] { return isOn(p, QLatin1String(id)); }));
        p.settle();
        p.check(QStringLiteral("%1: and its tab's dot, the other dots as they were").arg(QLatin1String(id)), p.dot(id) == 1);
        p.check(QStringLiteral("%1: the chain keeps its items").arg(QLatin1String(id)),
                toggle && toggle == p.shown(name) && toggle->property("checked").toBool());
        int others = 0;
        for (const char *other : kEffects) others += qstrcmp(other, id) != 0 && p.dot(other) == 1;
        p.check(QStringLiteral("%1: no other is").arg(QLatin1String(id)), others == 0);
        p.check(QStringLiteral("%1: the chain counts it").arg(QLatin1String(id)), summary().contains(QLatin1String("1 effect is on")));
        p.check(QStringLiteral("%1: its stage opens its tab").arg(QLatin1String(id)),
                p.click(QStringLiteral("EffectsChain.%1").arg(QLatin1String(id))) && p.at() == QStringLiteral("effects/%1").arg(QLatin1String(id)));
        for (const char *h : { "CrossfeedWindow", "LoudnessWindow", "LevellerWindow", "PsybassWindow", "SubharmonicWindow", "TubeWindow", "UpmixerWindow" })
            if (QQuickItem *sw = p.shown(QStringLiteral("%1.header.switch").arg(QLatin1String(h))))
                p.check(QStringLiteral("%1: its panel's switch is on").arg(QLatin1String(id)), sw->property("checked").toBool());
        p.click(name);
        p.check(QStringLiteral("%1: and off again").arg(QLatin1String(id)),
                p.d.waitFor(2000, [&] { return !isOn(p, QLatin1String(id)); }) && p.d.waitFor(500, [&] { return p.dot(id) == 0; }));
    }
    // The panel's own switch, and Classic's quick strip's
    p.go(QStringLiteral("effects"), QStringLiteral("loudness"));
    p.click("LoudnessWindow.header.switch");
    p.check("the panel's switch moves the dot and the chain", p.d.waitFor(1000, [&] { return p.dot("loudness") == 1; })
            && p.shown("EffectsChain.loudness.on") && p.shown("EffectsChain.loudness.on")->property("checked").toBool());
    p.sc->setEffect(QStringLiteral("loudness"), false);
    p.settle();
    p.check("so does a change from elsewhere", p.dot("loudness") == 0 && !p.shown("EffectsChain.loudness.on")->property("checked").toBool());
    // The stages that are no effect
    const std::pair<const char *, const char *> stages[] = { { "matrix", "routing/matrix" }, { "crossover", "equaliser/crossover" },
                                                             { "output", "equaliser/peq" }, { "limiter", "equaliser/delay" } };
    for (const auto &s : stages) {
        p.go(QStringLiteral("effects"), QStringLiteral("crossfeed"));
        p.check(QStringLiteral("the chain's %1 opens %2").arg(QLatin1String(s.first), QLatin1String(s.second)),
                p.click(QStringLiteral("EffectsChain.%1").arg(QLatin1String(s.first))) && p.at() == QLatin1String(s.second));
    }
    p.go(QStringLiteral("effects"), QStringLiteral("crossfeed"));
    p.sc->setEffect(QStringLiteral("bypass"), true);
    p.settle();
    p.check("Bypass EQ shows on the inputs' stage", p.shown("EffectsChain.input.bypassed") != nullptr);
    p.sc->setEffect(QStringLiteral("bypass"), false);
    p.settle();
    p.check("and goes", !p.shown("EffectsChain.input.bypassed"));
    p.click("EffectsTools.limiters");
    p.check("Output Limiters opens Delay & Gain", p.at() == QLatin1String("equaliser/delay"));
    // Solo ends with its tab, the chain beside it or not
    p.go(QStringLiteral("effects"), QStringLiteral("subharmonic"));
    p.tc->setSubharm(QStringLiteral("enabled"), 1);
    p.settle();
    p.tc->setSubharmSolo(true);
    p.settle();
    p.check("Solo on the Subharmonic page", p.tc->property("subharmSolo").toBool());
    p.click("EffectsChain.tube");
    p.check("leaving the page ends it", p.at() == QLatin1String("effects/tube") && p.d.waitFor(1500, [&] { return !p.tc->property("subharmSolo").toBool(); }));
    p.tc->setSubharm(QStringLiteral("enabled"), 0);
    p.settle();
    // The inspector hides and shows, and the panel takes its room
    p.go(QStringLiteral("effects"), QStringLiteral("crossfeed"));
    QQuickItem *scroll = p.shown("PageHost.page.effects/crossfeed.scroll");
    const qreal beside = scroll ? scroll->width() : 0;
    p.click("EffectsTools.inspector");
    p.check("hidden, the panel has the whole page", !p.shown("EffectsChain.sheet") && scroll && scroll->width() >= beside + 300);
    p.click("EffectsTools.inspector");
    p.check("shown, it is beside the panel", p.shown("EffectsChain.sheet") && scroll && std::abs(scroll->width() - beside) < 1);
    // A panel in its own window takes the chain away with it
    p.shell->openWindow(QStringLiteral("tools.crossfeed"));
    p.settle();
    p.check("in its own window, the page says so without the chain", p.shown("PageHost.page.effects/crossfeed.inWindow") && !p.shown("EffectsChain.sheet"));
    p.check("the dots stay", p.dot("crossfeed") == 0);
    p.shell->closeWindow(QStringLiteral("tools.crossfeed"));
    p.d.waitFor(2000, [&] { return !p.shell->shownWindows().contains(QLatin1String("tools.crossfeed")); });
    p.settle();
    p.main->requestActivate();
    p.d.spin(60);
    p.check("and both are back", p.shown("CrossfeedWindow.header") && p.shown("EffectsChain.sheet"));
}

// ---- Overview --------------------------------------------------------------------

void overviewPart(Pages &p) {
    p.go(QStringLiteral("overview"));
    p.check("the device's facts", !p.fact("name").isEmpty() && !p.fact("board").isEmpty() && p.fact("firmware").contains(QLatin1Char('.')) && !p.fact("preset").isEmpty());
    p.check("a demo says so, and has no hub", p.fact("transport") == QLatin1String("Demo") && !p.shown("OverviewSummary.hub")
                && !p.shown("OverviewSummary.security") && !p.shown("OverviewSummary.alarms"));
    for (const char *id : kEffects) {
        QQuickItem *chip = p.shown(QStringLiteral("OverviewSummary.effect.") + QLatin1String(id));
        p.check(QStringLiteral("a chip for %1, off").arg(QLatin1String(id)), chip && !chip->property("on").toBool());
    }
    p.sc->setEffect(QStringLiteral("crossfeed"), true);
    p.settle();
    QPointer<QQuickItem> chip = p.shown("OverviewSummary.effect.crossfeed");
    p.check("it lights with its effect", chip && chip->property("on").toBool());
    p.check("unsaved changes are marked on the preset", p.d.waitFor(2000, [&] { return p.sc->property("unsaved").toBool() && p.fact("preset").endsWith(QLatin1Char('*')); }));
    p.click("OverviewSummary.effect.tube");
    p.check("a chip opens its page", p.at() == QLatin1String("effects/tube"));
    p.go(QStringLiteral("overview"));
    p.sc->setEffect(QStringLiteral("crossfeed"), false);
    p.settle();
    p.check("the chips are kept over a change", chip && chip == p.shown("OverviewSummary.effect.crossfeed") && !chip->property("on").toBool());
    // A card opens its channel
    const int out = p.cc->outputChannel(0);
    for (const int ch : { 0, out }) {
        p.go(QStringLiteral("overview"));
        p.check(QStringLiteral("nothing is selected on the Overview (%1)").arg(ch), p.sc->selectedChannel() == -1);
        // -- WP UA4 -- the setup list may stand over the cards: the card in view first
        if (QQuickItem *c = p.shown(QStringLiteral("Dashboard.open.%1").arg(ch)), *f = p.shown(QStringLiteral("Dashboard")); c && f) {
            const qreal y = c->mapToItem(f->property("contentItem").value<QQuickItem *>(), 0, 0).y();
            const qreal most = std::max<qreal>(0, f->property("contentHeight").toReal() - f->height());
            f->setProperty("contentY", std::clamp<qreal>(y - 12, 0, most));
            p.settle();
        }
        p.check(QStringLiteral("the card of channel %1 opens it on the Equaliser").arg(ch),
                p.click(QStringLiteral("Dashboard.open.%1").arg(ch)) && p.at() == QLatin1String("equaliser/peq") && p.sc->selectedChannel() == ch);
    }
    p.go(QStringLiteral("overview"));
    QQuickItem *card = p.shown(QStringLiteral("Dashboard.open.%1").arg(out));
    if (card) card->forceActiveFocus(Qt::TabFocusReason);
    QTest::keyClick(p.main, Qt::Key_Space);
    p.settle();
    p.check("by the keyboard too", p.at() == QLatin1String("equaliser/peq") && p.sc->selectedChannel() == out);
    // -- WP DL6 -- No cards-per-row choice: the channel cards fit the width
    p.go(QStringLiteral("overview"));
    p.check("no cards-per-row choice", !p.shown("OverviewTools") && !p.shown("OverviewTools.perRow"));
    p.check("a card per output, with its name and its feeds chip",
            p.shown(QStringLiteral("OverviewChannelCard.name.%1").arg(out)) != nullptr
                && p.shown(QStringLiteral("OverviewChannelCard.chip.%1.feeds").arg(out)) != nullptr);
    QQuickItem *gear = p.d.item(QStringLiteral("Dashboard.gear"));
    QTest::mouseMove(p.main, Pages::centre(p.shown(QStringLiteral("Dashboard.open.%1").arg(out))));
    p.d.spin(300);
    p.check("no layout gear on the cards", !gear || gear->opacity() == 0);
    p.check("Manage Devices", p.click("OverviewSummary.devices") && p.at() == QLatin1String("devices/devices"));
}

// ---- Routing ---------------------------------------------------------------------

void routingPart(Pages &p) {
    const int out = p.cc->outputChannel(1);
    p.sc->select(-1);
    p.go(QStringLiteral("routing"));
    p.check("with no channel selected the inspector says what to do", p.shown("RoutingInspector.none") != nullptr);
    p.check("an output's header selects it", p.click("Matrix.header.1.select") && p.sc->selectedChannel() == out);
    QQuickItem *title = p.shown("ChannelInspector.title");
    QQuickItem *header = p.shown("Matrix.header.1.select");
    p.check("the inspector shows it, and its header is marked", title && header && header->property("current").toBool() && !p.shown("RoutingInspector.none"));
    for (const char *s : { "Inspector.crossover", "Inspector.limiter", "Inspector.linkwitz" })
        p.check(QStringLiteral("with %1").arg(QLatin1String(s)), p.shown(s) != nullptr);
    p.check("without a way to the page it is on", !p.shown("Inspector.editOutputRouting"));
    if (QQuickItem *section = p.shown("Inspector.limiter"); section && !section->property("expanded").toBool()) p.click("Inspector.limiter.heading");
    p.check("its limiter can be set there", p.shown("LimiterSettings.threshold") != nullptr);
    p.click("Inspector.limiter.heading");
    // Beside the label's trim, which keeps its clicks
    QQuickItem *label = p.shown("Matrix.row.0.select");
    if (label) QTest::mouseClick(p.main, Qt::LeftButton, Qt::NoModifier, label->mapToScene(QPointF(label->width() / 2, 8)).toPoint());
    p.settle();
    p.check("an input's label selects it", label && p.sc->selectedChannel() == 0 && p.shown("Inspector.feeds"));
    p.check("an input's has no way to it either", !p.shown("Inspector.editRouting"));
    p.check("what it feeds selects that output", p.click("Inspector.feeds.0") && p.sc->selectedChannel() == p.cc->outputChannel(0));
    p.check("Open in Equaliser", p.click("RoutingInspector.equaliser") && p.at() == QLatin1String("equaliser/peq") && p.sc->selectedChannel() == p.cc->outputChannel(0));
    p.check("where the inspector does have the way back", p.shown("Inspector.editOutputRouting") != nullptr);
    p.click("Inspector.editOutputRouting");
    p.check("to the matrix", p.at() == QLatin1String("routing/matrix"));
    // The matrix's own menu and cells work as before
    QQuickItem *cell = p.shown("Matrix.header.2.menu");
    p.check("the header keeps its menu", cell && Pages::inside(cell, p.shown("Matrix.header.2")));
    p.click("Matrix.header.2.menu");
    QObject *menu = p.d.object(QStringLiteral("Matrix.columnMenu"));
    if (!menu) {
        for (QQuickItem *i : { p.shown("MatrixPanel") })
            if (i) menu = i->findChild<QObject *>(QStringLiteral("Matrix.columnMenu"));
    }
    p.check("and it opens", menu && menu->property("opened").toBool());
    if (menu) QMetaObject::invokeMethod(menu, "close");
    p.d.spin(100);
    p.sc->select(-1);
    // Classic's window has none of this
    p.shell->openWindow(QStringLiteral("tools.matrix"));
    p.settle();
    QWindow *window = nullptr;
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (w != p.main && w->isVisible() && w->objectName().contains(QLatin1String("Matrix"))) window = w;
    auto *quick = qobject_cast<QQuickWindow *>(window);
    QQuickItem *button = quick ? p.shown(QStringLiteral("Matrix.header.1.select"), quick->contentItem()) : nullptr;
    p.check("the matrix's window has no selection", quick && p.shown(QStringLiteral("Matrix.header.1"), quick->contentItem()) && !button);
    p.shell->closeWindow(QStringLiteral("tools.matrix"));
    p.d.waitFor(2000, [&] { return !p.shell->shownWindows().contains(QLatin1String("tools.matrix")); });
    p.main->requestActivate();
    p.settle();
}

// ---- Analyse ---------------------------------------------------------------------

void analysePart(Pages &p) {
    auto *sg = Registrar::instance<SiggenController>();
    auto *rta = Registrar::instance<RtaController>();
    p.go(QStringLiteral("analyse"), QStringLiteral("spectrum"));
    p.check("the Spectrum tab's name is short", p.shown("PageTabs.spectrum") && p.shown("PageTabs.spectrum")->property("text").toString() == QLatin1String("Spectrum"));
    p.check("no dot while the generator is silent", p.dot("signalGenerator") == -1 && p.dot("spectrum") == -1 && !p.shown("AnalyseTools.stop"));
    if (rta && rta->property("available").toBool()) {
        const bool on = rta->property("showGraph").toBool();
        p.check("the spectrum on the Equaliser's graph", p.click("AnalyseTools.graph") && rta->property("showGraph").toBool() != on);
        p.click("AnalyseTools.graph");
        p.check("and back", rta->property("showGraph").toBool() == on);
    }
    p.go(QStringLiteral("analyse"), QStringLiteral("stats"));
    p.check("only where it belongs", !p.shown("AnalyseTools.graph"));
    if (!sg) return;
    sg->selectType(SiggenController::Sine);
    sg->setLevel(-60);
    if (sg->property("channelMask").toInt() == 0) sg->stepOutput(0);
    p.settle();
    sg->play();
    p.check("the generator plays", p.d.waitFor(3000, [&] { return sg->property("playing").toBool(); }));
    p.settle();
    p.check("its tab has a dot, from another tab", p.at() == QLatin1String("analyse/stats") && p.dot("signalGenerator") == 1);
    p.check("and a Stop", p.shown("AnalyseTools.stop") && p.click("AnalyseTools.stop"));
    p.check("which stops it", p.d.waitFor(4000, [&] { return !sg->property("playing").toBool(); }));
    p.settle();
    p.check("then neither", p.dot("signalGenerator") == -1 && !p.shown("AnalyseTools.stop"));
}

// ---- Another device, and none ------------------------------------------------------

void devicesPart(Pages &p) {
    auto *registry = DeviceRegistry::instance();
    const QString first = registry->activeId();
    p.go(QStringLiteral("effects"), QStringLiteral("crossfeed"));
    p.sc->setEffect(QStringLiteral("crossfeed"), true);
    p.settle();
    p.check("crossfeed is on, on the first device", p.dot("crossfeed") == 1);
    registry->openDemo();
    p.check("a second device beside it", p.d.waitFor(15000, [&] { return registry->devices().size() == 2 && registry->activeId() != first; }));
    const QString second = registry->activeId();
    p.settle();
    p.check("the dots, the chain and the panel are the second's", p.d.waitFor(2000, [&] { return p.dot("crossfeed") == 0; })
                && p.shown("EffectsChain.crossfeed.on") && !p.shown("EffectsChain.crossfeed.on")->property("checked").toBool()
                && p.shown("CrossfeedWindow.header.switch") && !p.shown("CrossfeedWindow.header.switch")->property("checked").toBool());
    p.click("EffectsChain.tube.on");
    p.check("a switch in the chain is the second's alone", p.d.waitFor(2000, [&] { return p.dot("tube") == 1; }));
    registry->activate(first);
    p.d.waitFor(3000, [&] { return registry->activeId() == first; });
    p.settle();
    p.check("and back they are the first's", p.d.waitFor(2000, [&] { return p.dot("crossfeed") == 1 && p.dot("tube") == 0; }));
    p.sc->setEffect(QStringLiteral("crossfeed"), false);
    p.settle();
    registry->close(second);
    p.check("the second closes", p.d.waitFor(8000, [&] { return registry->devices().size() == 1; }));
    p.settle();
    // None
    registry->close(first);
    p.check("the last device closes", p.d.waitFor(5000, [&] { return !p.d.app().device->connected() && registry->devices().isEmpty(); }));
    p.settle();
    bool dots = false;
    for (const char *id : kEffects) dots = dots || p.dot(id) != -1;
    p.check("without a device there are no dots and no chain", !dots && !p.shown("EffectsChain.crossfeed"));
    p.go(QStringLiteral("overview"));
    p.check("the Overview has no facts then", !p.shown("OverviewTools"));
    p.check("and a way to a device", p.click("OverviewSection.empty.action") && p.at() == QLatin1String("devices/devices"));   // -- WP DL6 --
    p.d.app().device->openDemo();
    p.check("the demo again", p.d.waitFor(10000, [&] { return p.d.app().device->connected(); }));
    p.settle();
    p.go(QStringLiteral("effects"), QStringLiteral("crossfeed"));
    p.check("with its dots", p.dot("crossfeed") == 0 && p.shown("EffectsChain.crossfeed"));
}

// ---- A hub's unit, with the hub's audio link ----------------------------------------

// `uconsole --demo hub --alink`, as mdqt2b-rebind runs it
class AlinkHub {
public:
    bool start(const QString &store) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList{ "--demo", "hub", "--alink", "--loopback", "--json", "--no-keep-awake", "--forget-clients",
                                          "--store", store, "--port", "0", "--open" });
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
                if (e.value(QLatin1String("event")).toString() == QLatin1String("status")) {
                    url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(e.value(QLatin1String("port")).toInt());
                    return true;
                }
            }
        }
        return false;
    }
    ~AlinkHub() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    QString url;

private:
    QProcess m_proc;
};

void hubPart(Pages &p) {
    auto *registry = DeviceRegistry::instance();
    auto *link = Registrar::instance<LinkController>();
    auto *list = Registrar::instance<DevicesController>();
    auto *sys = Registrar::instance<SystemController>();
    if (!link || !list || !sys) return p.check("the hub's controllers", false);
    const QString demo = registry->activeId();
    const QString store = p.d.outDir() + QStringLiteral("/lay5a-hub");
    QDir().mkpath(store);
    AlinkHub hub;
    p.check("uconsole --demo hub --alink runs (DSPI_SMOKE_UCONSOLE)", hub.start(store));
    if (hub.url.isEmpty()) return;
    link->connectToHub(hub.url);
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
    p.go(QStringLiteral("overview"));
    p.check("the hub's audio link is read", p.d.waitFor(20000, [&] { return sys->available() && sys->installed(); }));
    p.settle();
    p.check("the connection is the hub's", p.fact("transport") == QLatin1String("DSPi Link") && !p.fact("hub").isEmpty());
    p.check("its security in words", p.fact("security") == QLatin1String("This computer") || p.fact("security") == QLatin1String("Encrypted"));
    p.check("its audio link's state", !sys->summary().isEmpty() && p.fact("alink") == sys->summary());
    p.check("its alarms, when there are any", p.fact("alarms") == (sys->property("alarms").toList().isEmpty() ? QString() : QString::number(sys->property("alarms").toList().size())));
    QQuickItem *box = p.shown("OverviewSummary.connection");
    p.check("in its box", box && Pages::inside(p.shown("OverviewSummary.devices"), box));
    p.d.grab(p.main, QStringLiteral("gallery/lay5a-1280-overview-hub.png"));
    p.check("which opens the System page", p.click("OverviewSummary.alink") && p.at() == QLatin1String("system/status"));
    p.go(QStringLiteral("overview"));
    const QString unit = registry->activeId();
    registry->activate(demo);
    p.d.waitFor(5000, [&] { return registry->activeId() == demo; });
    p.settle();
    p.check("back on the demo, none of the hub's", p.fact("transport") == QLatin1String("Demo") && !p.shown("OverviewSummary.hub")
                && !p.shown("OverviewSummary.security") && !p.shown("OverviewSummary.alink"));
    registry->close(unit);
    p.check("the unit closes", p.d.waitFor(10000, [&] { return registry->devices().size() == 1; }));
    link->closeSession();
    p.settle();
}

// ---- Narrow windows ----------------------------------------------------------------

void narrowPart(Pages &p) {
    p.go(QStringLiteral("effects"), QStringLiteral("crossfeed"));
    p.resize(1000, 660);
    p.check("below 1280 px the chain is folded away", !p.shown("EffectsChain.sheet"));
    QQuickItem *scroll = p.shown("PageHost.page.effects/crossfeed.scroll");
    const qreal all = scroll ? scroll->width() : 0;
    p.click("EffectsTools.inspector");
    QQuickItem *sheet = p.shown("EffectsChain.sheet");
    p.check("and opens over the page", sheet && scroll && scroll->width() == all && Pages::inside(sheet, p.shown("PageHost")));
    // Its clicks are its own
    p.check("its stages work there", p.click("EffectsChain.leveller") && p.at() == QLatin1String("effects/leveller"));
    p.check("the sheet stays over the next tab", p.shown("EffectsChain.sheet") != nullptr);
    p.click("EffectsChain.close");
    p.check("its close hides it", !p.shown("EffectsChain.sheet"));
    QQuickItem *tools = p.shown("EffectsTools");
    p.check("the row over the effects fits", tools && Pages::inside(tools, p.shown("PageTabs")) && !p.shown("EffectsTools.limiters"));
    // Every tab can be reached: the bar scrolls to the current one
    p.go(QStringLiteral("effects"), QStringLiteral("upmixer"));
    QQuickItem *tab = p.shown("PageTabs.upmixer");
    p.check("the last tab is in sight and clear of the buttons",
            tab && tools && tab->mapToScene(QPointF(tab->width(), 0)).x() <= p.shown("EffectsTools")->mapToScene(QPointF(0, 0)).x() + 0.5);
    p.go(QStringLiteral("overview"));
    p.check("the Overview's three boxes stand side by side", p.shown("OverviewSummary") && p.shown("OverviewSummary")->property("columns").toInt() == 3);
    for (const char *box : { "device", "effects", "connection" }) {
        QQuickItem *b = p.shown(QStringLiteral("OverviewSummary.") + QLatin1String(box));
        bool fits = b != nullptr;
        if (b)
            for (QQuickItem *col : b->childItems())
                for (QQuickItem *c : col->childItems())
                    if (c->isVisible() && c->width() > 0 && !Pages::inside(c, b)) fits = false;
        p.check(QStringLiteral("its %1 box holds what is in it").arg(QLatin1String(box)), fits);
    }
    p.resize(1280, 800);
}

void galleryPart(Pages &p) {
    const Qt::ColorScheme before = p.shell->colorSchemeOverride();
    p.sc->setEffect(QStringLiteral("crossfeed"), true);
    p.tc->setTube(QStringLiteral("enabled"), 1);
    p.settle();
    const std::pair<const char *, const char *> pages[] = {
        { "overview", "overview" }, { "routing", "matrix" }, { "effects", "crossfeed" }, { "effects", "leveller" }, { "effects", "tube" },
        { "analyse", "spectrum" }, { "analyse", "signalGenerator" }, { "analyse", "stats" } };
    QDir(p.d.outDir()).mkpath(QStringLiteral("gallery"));
    for (const QSize &size : { QSize(1920, 1008), QSize(1280, 800), QSize(1000, 660) }) {
        p.resize(size.width(), size.height());
        for (const auto &page : pages) {
            p.go(QLatin1String(page.first), QLatin1String(page.second));
            if (QLatin1String(page.first) == QLatin1String("routing")) p.sc->select(p.cc->outputChannel(0));
            for (const bool dark : { true, false }) {
                p.shell->setColorSchemeOverride(dark ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
                p.settle();
                p.d.spin(120);
                const QString name = QStringLiteral("gallery/lay5a-%1-%2-%3-%4.png")
                                         .arg(size.width()).arg(QLatin1String(page.first), QLatin1String(page.second), dark ? QStringLiteral("dark") : QStringLiteral("light"));
                p.check(QStringLiteral("screenshot %1").arg(name), p.d.grab(p.main, name));
            }
            const QString key = QStringLiteral("%1/%2").arg(QLatin1String(page.first), QLatin1String(page.second));
            QQuickItem *scroll = p.shown(QStringLiteral("PageHost.page.%1.scroll").arg(key));
            p.check(QStringLiteral("%1 at %2 px needs no sideways scrolling").arg(key).arg(size.width()),
                    scroll && scroll->property("contentWidth").toReal() <= scroll->width() + 0.5);
        }
    }
    p.shell->setColorSchemeOverride(before);
    p.sc->setEffect(QStringLiteral("crossfeed"), false);
    p.tc->setTube(QStringLiteral("enabled"), 0);
    p.sc->select(-1);
    p.resize(1280, 800);
}

void pagesSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    Pages p{ d, d.app().shell, Registrar::instance<LayoutController>(), Registrar::instance<ChannelController>(),
             Registrar::instance<SidebarController>(), Registrar::instance<EffectsController>(), Registrar::instance<ToolsController>(),
             d.mainWindow() };
    p.check("the controllers", p.shell && p.layout && p.cc && p.sc && p.fx && p.tc && p.main);
    if (!p.shell || !p.layout || !p.cc || !p.sc || !p.fx || !p.tc || !p.main) return;
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    p.check("on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    for (const QString &action : p.shell->shownWindows()) p.shell->closeWindow(action);
    p.main->requestActivate();
    d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == p.main; });
    const QSize was = p.main->size();
    p.main->resize(1280, 800);
    p.sc->select(-1);
    // The effects off, whatever earlier sections left on
    for (const char *e : { "crossfeed", "loudness", "leveller", "psybass", "bypass" }) p.sc->setEffect(QLatin1String(e), false);
    p.tc->setSubharm(QStringLiteral("enabled"), 0);
    p.tc->setTube(QStringLiteral("enabled"), 0);
    p.tc->setUpmix(QStringLiteral("enabled"), 0);
    p.settle();
    p.check("Modern", switchTo(p, QStringLiteral("modern"), "ModernShell") && p.host());
    if (!p.host()) return;
    p.layout->arrangement()->setProperty("inspector", QStringLiteral("shown"));
    p.resize(1280, 800);

    waysPart(p);
    effectsPart(p);
    overviewPart(p);
    routingPart(p);
    analysePart(p);
    narrowPart(p);
    galleryPart(p);
    devicesPart(p);
    hubPart(p);

    p.sc->select(-1);
    p.check("back in Classic, with none of these", switchTo(p, QStringLiteral("classic"), "ClassicShell") && !p.shown("OverviewSummary")
                && !p.shown("EffectsChain.sheet"));
    p.settle();
    QQuickItem *dashboard = p.shown("Dashboard");
    p.check("Classic's dashboard has its cards only", dashboard && !p.shown("Dashboard.head") && !p.shown(QStringLiteral("Dashboard.open.0")));
    p.main->resize(was);
    d.spin(200);
}

const SmokeDriver::Section pagesSmoke("lay5a-pages", 896, &pagesSection);

} // namespace
