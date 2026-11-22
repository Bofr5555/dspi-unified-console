// -- WP LAY1a -- The matrix mixer and the seven effects as panels, on the demo
// device: each opens by its action in a PanelWindow named, titled and sized as
// before, holding its panel with its old item names; the upmixer's most size
// and the matrix's fit to its table are the window's; the panels know when they
// show and stop their polls while minimised; each loads in a host that is no
// window (a page), and leaves its polls off when that goes. And ToolWindow's
// fit: a window resized while hidden still fits its content when it shows.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "Registrar.h"
#include "Shell.h"
#include "ToolsController.h"

#include <QPointer>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QScreen>
#include <QWindow>
#include <algorithm>
#include <cmath>
#include <cstdio>

namespace {

struct Tool {
    const char *action;
    const char *type;      // the window's name, as registered
    const char *panel;
    const char *item;      // one of its items, named as before
    const char *title;
    const char *section;
    const char *windowId;  // where its place was remembered before it was a panel
    QSize size, least, most;
};

const QSize kNoMost(16777215, 16777215);   // QWindow's

const Tool kTools[] = {
    { "tools.matrix", "MatrixWindow", "MatrixPanel", "Matrix.table", "Matrix Mixer", "routing", "matrix", { 960, 660 }, { 480, 320 }, kNoMost },
    { "tools.crossfeed", "CrossfeedWindow", "CrossfeedPanel", "CrossfeedWindow.pairMap", "Crossfeed", "effects", "crossfeed", { 780, 470 }, { 740, 460 }, kNoMost },
    { "tools.loudness", "LoudnessWindow", "LoudnessPanel", "LoudnessWindow.graph", "Loudness Compensation", "effects", "loudness", { 780, 400 }, { 740, 390 }, kNoMost },
    { "tools.leveller", "LevellerWindow", "LevellerPanel", "LevellerWindow.amount", "Volume Leveller", "effects", "leveller", { 380, 540 }, { 380, 400 }, kNoMost },
    { "tools.psybass", "PsybassWindow", "PsybassPanel", "PsybassWindow.cutoff", "Psychoacoustic Bass", "effects", "psybass", { 780, 500 }, { 740, 496 }, kNoMost },
    { "tools.subharmonic", "SubharmonicWindow", "SubharmonicPanel", "SubharmonicWindow.solo", "Subharmonic Synthesizer", "effects", "subharmonic", { 780, 620 }, { 740, 612 }, kNoMost },
    { "tools.tube", "TubeWindow", "TubePanel", "TubeWindow.mode.0", "Tube Modeller", "effects", "tube", { 780, 664 }, { 740, 400 }, kNoMost },
    { "tools.upmixer", "UpmixerWindow", "UpmixerPanel", "UpmixerWindow.body", "Stereo Upmixer", "effects", "upmixer", { 400, 720 }, { 400, 400 }, { 400, 1200 } },
};

QQuickWindow *open(SmokeDriver &d, const QString &action) {
    QWindow *window = nullptr;
    d.waitFor(3000, [&] {
        window = d.app().shell->openWindow(action);
        return window && window->isVisible();
    });
    return qobject_cast<QQuickWindow *>(window);
}

QQuickItem *panelOf(QQuickWindow *w) { return w ? w->property("content").value<QQuickItem *>() : nullptr; }

void windowPart(SmokeDriver &d, const Tool &t) {
    Shell *shell = d.app().shell;
    const QString action = QString::fromLatin1(t.action), name = QString::fromLatin1(t.panel);
    const QVariantMap row = shell->panel(action);
    shell->trigger(action);
    QQuickWindow *w = open(d, action);
    QQuickItem *panel = panelOf(w);
    d.check(QStringLiteral("lay1a: %1 opens %2 in a PanelWindow named %3").arg(action, name, QLatin1String(t.type)),
            w && panel && panel->objectName() == name && w->objectName() == QLatin1String(t.type)
                && QString::fromLatin1(w->metaObject()->className()).startsWith(QLatin1String("PanelWindow")));
    if (!w || !panel) return;
    d.waitIdle();
    d.spin(200);
    QQuickItem *item = d.item(QLatin1String(t.item), w->contentItem());
    d.check(QStringLiteral("lay1a: %1 keeps its item names (%2)").arg(name, QLatin1String(t.item)),
            item && panel->isAncestorOf(item));
    d.check(QStringLiteral("lay1a: %1 has its place: section %2, its row's tab and icon").arg(name, QLatin1String(t.section)),
            row.value(QStringLiteral("section")).toString() == QLatin1String(t.section)
                && row.value(QStringLiteral("panel")).toString() == name);
    d.check(QStringLiteral("lay1a: %1's window has its title, sizes and remembered place as before").arg(name),
            w->title() == QLatin1String(t.title) && w->property("windowId").toString() == QLatin1String(t.windowId)
                && w->property("defaultWidth").toInt() == t.size.width() && w->property("defaultHeight").toInt() == t.size.height()
                && w->minimumSize() == t.least && w->maximumSize() == t.most);
    d.check(QStringLiteral("lay1a: %1 fills its window").arg(name),
            std::abs(panel->width() - w->width()) < 1 && std::abs(panel->height() - w->height()) < 1);
    d.check(QStringLiteral("lay1a: %1 knows it shows, and its action").arg(name),
            panel->property("shown").toBool() && panel->property("onScreen").toBool()
                && panel->property("actionId").toString() == action && !panel->property("locked").toBool());
    if (panel->property("polling").isValid()) {
        w->showMinimized();
        d.check(QStringLiteral("lay1a: %1 stops its polls while minimised").arg(name),
                d.waitFor(2000, [&] { return !panel->property("polling").toBool(); }) && panel->property("shown").toBool());
        w->showNormal();
        d.check(QStringLiteral("lay1a: and starts them again"),
                d.waitFor(2000, [&] { return panel->property("polling").toBool(); }));
    }
    w->close();
    d.check(QStringLiteral("lay1a: %1 knows it no longer shows").arg(name),
            d.waitFor(2000, [&] { return !w->isVisible(); }) && !panel->property("shown").toBool()
                && !panel->property("onScreen").toBool());
}

// Opened again, each showing starts from the panel's own rule
void againPart(SmokeDriver &d) {
    // The matrix: fitted to its table on every show, whatever size it had
    QQuickWindow *w = open(d, QStringLiteral("tools.matrix"));
    d.waitIdle();
    d.spin(300);
    QQuickItem *table = w ? d.item(QStringLiteral("Matrix.table"), w->contentItem()) : nullptr;
    d.check("lay1a: the matrix's table is there", table != nullptr);
    if (w && table) {
        w->close();
        w->resize(600, 400);
        d.spin(100);
        w = open(d, QStringLiteral("tools.matrix"));
        d.spin(300);
        const QSize avail = w->screen() ? w->screen()->availableVirtualSize() : QSize();
        const int wantW = qMin(int(table->width()) + 12, avail.width()), wantH = qMin(int(table->height()) + 12, avail.height());
        d.check(QStringLiteral("lay1a: shown again, the matrix fits its table (%1x%2, table %3x%4, screen %5x%6)")
                    .arg(w->width()).arg(w->height()).arg(table->width()).arg(table->height()).arg(avail.width()).arg(avail.height()),
                w->width() >= wantW && w->height() >= wantH && w->width() <= qMax(w->minimumWidth(), int(table->width()) + 60)
                    && w->height() <= qMax(w->minimumHeight(), int(table->height()) + 60));
        w->close();
        d.spin(100);
    }
    // The scrolled columns start at the top again
    for (const char *action : { "tools.crossfeed", "tools.psybass" }) {
        QQuickWindow *tool = open(d, QString::fromLatin1(action));
        QQuickItem *panel = panelOf(tool);
        if (!panel) continue;
        d.spin(200);
        const QVariantList bodies = panel->property("fitBodies").toList();
        auto *body = bodies.isEmpty() ? nullptr : bodies.first().value<QQuickItem *>();
        d.check(QStringLiteral("lay1a: %1 says what scrolls").arg(QLatin1String(action)), body != nullptr);
        if (!body) continue;
        body->setProperty("contentY", 20);
        tool->close();
        d.waitFor(1000, [&] { return !tool->isVisible(); });
        open(d, QString::fromLatin1(action));
        d.spin(100);
        d.check(QStringLiteral("lay1a: %1 opened again starts at the top").arg(QLatin1String(action)),
                std::abs(body->property("contentY").toReal()) < 1);
        tool->close();
    }
}

// Solo takes the program off the outputs: it goes with the panel, in any host
void soloPart(SmokeDriver &d, QQuickItem *panel, const char *host) {
    auto *t = Registrar::instance<ToolsController>();
    if (!t || !t->subharmSupported() || !t->subharmExtended()) {
        d.notRun(QStringLiteral("lay1a: solo in %1").arg(QLatin1String(host)), QStringLiteral("no Solo on this device"));
        return;
    }
    t->setSubharmSolo(true);
    d.check(QStringLiteral("lay1a: Solo on in %1").arg(QLatin1String(host)), d.waitFor(2000, [&] { return t->subharmSolo(); }));
    panel->setProperty("shown", false);
    d.check(QStringLiteral("lay1a: Solo goes when the panel stops showing in %1").arg(QLatin1String(host)),
            d.waitFor(2000, [&] { return !t->subharmSolo(); }));
    d.waitIdle();
    panel->setProperty("shown", true);
    d.spin(100);
}

// A page: an item of a window that is not the panel's own
void pagePart(SmokeDriver &d) {
    QQuickWindow page;
    page.setObjectName(QStringLiteral("lay1a.page"));
    page.resize(1000, 760);
    page.show();
    d.waitFor(2000, [&] { return page.isVisible(); });
    for (const Tool &t : kTools) {
        const QString name = QString::fromLatin1(t.panel);
        QQmlComponent component(d.app().engine);
        component.loadFromModule("UConsole", t.panel);
        QPointer<QQuickItem> panel = qobject_cast<QQuickItem *>(component.create());
        d.check(QStringLiteral("lay1a: %1 loads outside a window of its own").arg(name), panel != nullptr);
        if (!panel) {
            std::printf("smoke: lay1a: %s\n", qPrintable(component.errorString()));
            continue;
        }
        panel->setParentItem(page.contentItem());
        panel->setSize(QSizeF(page.width(), page.height()));
        panel->setProperty("shown", true);
        d.waitIdle();
        d.spin(250);
        QQuickItem *item = d.item(QLatin1String(t.item), panel);
        d.check(QStringLiteral("lay1a: %1 fills a page with its items").arg(name),
                item && item->width() > 0 && panel->property("onScreen").toBool()
                    && (!panel->property("polling").isValid() || panel->property("polling").toBool()));
        d.grab(&page, QStringLiteral("lay1a-page-%1.png").arg(name));
        if (t.panel == QLatin1String("SubharmonicPanel")) soloPart(d, panel, "a page");
        panel->setProperty("shown", false);
        d.spin(50);
        delete panel.data();
        d.spin(50);
    }
    page.close();
}

// How much more height the visible bodies of `w` want than they have
qreal extraHeight(QQuickWindow *w) {
    qreal extra = -1e9;
    for (const QVariant &v : w->property("fitBodies").toList()) {
        auto *b = v.value<QQuickItem *>();
        if (b && b->isVisible()) extra = std::max(extra, b->property("contentHeight").toReal() - b->height());
    }
    return extra;
}

// A window resized while hidden: its first frame comes before its layouts have
// followed, and the showing must fit it all the same (ToolWindow.fitBodySize)
void refitPart(SmokeDriver &d) {
    for (const char *action : { "tools.leveller", "tools.loudness", "tools.stats" }) {
        QQuickWindow *w = open(d, QString::fromLatin1(action));
        d.check(QStringLiteral("lay1a: %1 opens").arg(QLatin1String(action)), w != nullptr);
        if (!w) continue;
        w->close();
        w->resize(w->property("defaultWidth").toInt(), w->property("defaultHeight").toInt());
        open(d, QString::fromLatin1(action));
        d.waitIdle();
        d.spin(400);
        const QSize fitted = w->size();
        const int cap = d.app().shell->fitCap(w).height();
        for (const int by : { -120, 90 }) {
            w->close();
            d.spin(100);
            const int hidden = qMax(w->minimumHeight(), fitted.height() + by);
            w->resize(fitted.width(), hidden);
            open(d, QString::fromLatin1(action));
            d.waitIdle();
            d.spin(400);
            // Above the cap already (a hidden size may be), it keeps what its content needs of that
            d.check(QStringLiteral("lay1a: %1 resized while hidden to %2 fits again when shown: %3 (was %4, cap %5, wants %6 more)")
                        .arg(QLatin1String(action)).arg(hidden).arg(w->height()).arg(fitted.height()).arg(cap).arg(extraHeight(w)),
                    by < 0 ? w->height() == fitted.height()
                           : w->height() == fitted.height() || (fitted.height() >= cap && extraHeight(w) >= -1 && w->height() <= hidden));
        }
        // A size the person gives it while it shows still sticks
        const int hand = qMax(w->minimumHeight(), w->height() - 40);
        w->resize(w->width(), hand);
        d.spin(400);
        d.check(QStringLiteral("lay1a: %1 resized by hand to %2 stays (is %3)").arg(QLatin1String(action)).arg(hand).arg(w->height()),
                w->height() == hand);
        w->resize(fitted);
        w->close();
        d.spin(100);
    }
}

void panelsSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check("lay1a: on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    for (const Tool &t : kTools) windowPart(d, t);
    againPart(d);
    pagePart(d);
    refitPart(d);
}

const SmokeDriver::Section panelsSmoke("lay1a-panels", 885, &panelsSection);

} // namespace
