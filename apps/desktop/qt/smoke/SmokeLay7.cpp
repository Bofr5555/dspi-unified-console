// -- WP LAY7 -- Both layouts: the layout switched with three devices open (each
// keeps its own state, the active one stays, nothing is lost whichever way),
// narrow windows with three device tabs in each arrangement, the quit question
// for a device a job pins, a second path while a question is open, menus and
// the command palette that follow what changes under them, a tool window that
// starts at a remembered size; and the contrast of every text on Modern's pages
// with what it is drawn on (the tokens' pairs are tools/contrast.py's): 4.5:1,
// and 3:1 for large text and, as contrast.py has it, for what is dimmed because
// it is off (an output, a band).
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "DevicesController.h"
#include "FilesController.h"
#include "FirmwareController.h"
#include "LayoutController.h"
#include "NoticeCenter.h"
#include "PaletteController.h"
#include "PrefsMap.h"
#include "PresetModel.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"
#include "SidebarController.h"
#include "Snapshot.h"
#include "UnsavedGuard.h"

#include <QJSValue>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSemaphore>
#include <QSettings>
#include <QSignalSpy>
#include <QTest>
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <functional>
#include <memory>

namespace {

// ---- Contrast --------------------------------------------------------------------------

double channel(double v) { return v <= 0.03928 ? v / 12.92 : std::pow((v + 0.055) / 1.055, 2.4); }
double luminance(const QColor &c) { return 0.2126 * channel(c.redF()) + 0.7152 * channel(c.greenF()) + 0.0722 * channel(c.blueF()); }
double ratio(const QColor &a, const QColor &b) {
    const double x = luminance(a), y = luminance(b);
    return (std::max(x, y) + 0.05) / (std::min(x, y) + 0.05);
}
QColor over(const QColor &top, const QColor &under) {
    const double a = top.alphaF();
    return QColor::fromRgbF(float(top.redF() * a + under.redF() * (1 - a)), float(top.greenF() * a + under.greenF() * (1 - a)),
                            float(top.blueF() * a + under.blueF() * (1 - a)));
}

// What `text` is drawn on: the rectangles under its middle, nearest first, down
// to an opaque one; the window's colour below them all
QColor behind(QQuickItem *text, QQuickWindow *window) {
    QList<QColor> layers;
    const QPointF at = text->mapToScene(QPointF(std::min(text->width(), text->implicitWidth()) / 2, text->height() / 2));
    const auto paints = [&](QQuickItem *it) {
        if (!it->isVisible() || it->opacity() <= 0 || !it->inherits("QQuickRectangle") || !it->contains(it->mapFromScene(at))) return false;
        QColor c = it->property("color").value<QColor>();
        c.setAlphaF(float(c.alphaF() * it->opacity()));
        if (c.alphaF() > 0.004) layers << c;
        return c.alphaF() > 0.996;
    };
    bool opaque = false;
    for (QQuickItem *it = text; it && !opaque; it = it->parentItem()) {
        QQuickItem *parent = it->parentItem();
        if (!parent) break;
        const QList<QQuickItem *> siblings = parent->childItems();
        const qsizetype own = siblings.indexOf(it);
        QList<QQuickItem *> below;
        for (qsizetype i = 0; i < siblings.size(); ++i)
            if (siblings[i] != it && (siblings[i]->z() < it->z() || (siblings[i]->z() == it->z() && i < own))) below << siblings[i];
        std::stable_sort(below.begin(), below.end(), [](QQuickItem *a, QQuickItem *b) { return a->z() < b->z(); });
        for (qsizetype i = below.size() - 1; i >= 0 && !opaque; --i) opaque = paints(below[i]);
        if (!opaque) opaque = paints(parent);
    }
    QColor c = window->color();
    for (qsizetype i = layers.size() - 1; i >= 0; --i) c = over(layers[i], c);
    return c;
}

struct Contrast {
    QColor disabled;   // Theme.textDisabled
    int texts = 0;
    QStringList bad;
};

void contrastOf(const QString &where, QQuickItem *root, QQuickWindow *window, Contrast &out, double opacity = 1) {
    if (!root || !root->isVisible()) return;
    opacity *= root->opacity();
    if (opacity <= 0.02) return;
    const bool input = root->inherits("QQuickTextInput");
    if (root->inherits("QQuickText") || input) {
        const QString text = root->property("text").toString();
        // What cannot be used is exempt (WCAG 1.4.3); its token is contrast.py's
        if (!text.trimmed().isEmpty() && root->width() > 0 && root->height() > 0 && root->isEnabled()) {
            QColor ink = root->property("color").value<QColor>();
            const QColor paper = behind(root, window);
            ink.setAlphaF(float(ink.alphaF() * opacity));
            const QFont font = root->property("font").value<QFont>();
            const int px = font.pixelSize() > 0 ? font.pixelSize() : int(font.pointSizeF() * 96 / 72);
            const bool large = px >= 24 || (px >= 19 && font.weight() >= QFont::Bold);
            const double r = ratio(over(ink, paper), paper);
            ++out.texts;
            const bool dimmed = opacity < 0.99 || root->property("color").value<QColor>() == out.disabled;
            if (r < (large || dimmed ? 3.0 : 4.5)) {
                QString name;
                for (QQuickItem *i = root; i && name.isEmpty(); i = i->parentItem()) name = i->objectName();
                out.bad << QStringLiteral("%1: '%2' in %3: %4 on %5 = %6").arg(where, text.left(40), name, over(ink, paper).name(), paper.name())
                               .arg(r, 0, 'f', 2);
            }
        }
    }
    for (QQuickItem *child : root->childItems()) contrastOf(where, child, window, out, opacity);
}

void contrastSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    auto *layout = Registrar::instance<LayoutController>();
    Shell *shell = d.app().shell;
    QQuickWindow *main = d.mainWindow();
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check("lay7: on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    for (const QString &action : shell->shownWindows()) shell->closeWindow(action);
    const bool in = layout && main && d.switchLayout("modern");
    d.check("lay7: in Modern", in);
    if (!in) return;
    const QSize was = main->size();
    const Qt::ColorScheme before = shell->colorSchemeOverride();
    const QVariant tone = layout->arrangement()->property("tone");
    main->resize(1366, 800);
    Registrar::instance<SidebarController>()->select(0);   // the inspector and the strip have something to say
    const QStringList keys = d.modernTabs();
    for (const char *t : { "grey", "darker" }) {
        layout->arrangement()->setProperty("tone", QString::fromLatin1(t));
        for (const Qt::ColorScheme scheme : { Qt::ColorScheme::Dark, Qt::ColorScheme::Light }) {
            shell->setColorSchemeOverride(scheme);
            d.spin(300);
            QObject *theme = d.app().engine->singletonInstance<QObject *>("UConsole", "Theme");
            Contrast found;
            if (theme) found.disabled = theme->property("textDisabled").value<QColor>();
            d.check("lay7: the theme's tokens", found.disabled.isValid());
            for (const QString &key : keys) {
                d.goTo(key, 60);
                contrastOf(key, main->contentItem(), main, found);
            }
            found.bad.removeDuplicates();
            const QString name = QStringLiteral("%1 %2").arg(QLatin1String(t), scheme == Qt::ColorScheme::Dark ? QStringLiteral("dark") : QStringLiteral("light"));
            for (const QString &line : std::as_const(found.bad).mid(0, 60)) std::printf("smoke: lay7: %s: %s\n", qPrintable(name), qPrintable(line));
            std::printf("smoke: lay7: %s: %d texts on %d pages, %d below AA\n", qPrintable(name), found.texts, int(keys.size()), int(found.bad.size()));
            d.check(QStringLiteral("lay7: every text on Modern's pages has AA contrast (%1)").arg(name), found.texts > 1000 && found.bad.isEmpty());
        }
    }
    layout->arrangement()->setProperty("tone", tone);
    shell->setColorSchemeOverride(before);
    main->resize(was);
    Registrar::instance<SidebarController>()->select(-1);
    d.goTo(QStringLiteral("overview/overview"));
    d.check("lay7: back in Classic", d.switchLayout("classic"));
}

// ---- Three devices -----------------------------------------------------------------------

struct Three {
    SmokeDriver &d;
    DeviceRegistry *r;
    QQuickWindow *main;
    QList<QPointer<DeviceSession>> s;

    void check(const char *what, bool ok) const { d.check(QStringLiteral("lay7: ") + QString::fromUtf8(what), ok); }
    bool idle(DeviceSession *on) const {
        return d.waitFor(8000, [&] { return !on->model()->busy() && on->worker()->pending() == 0; });
    }
    // Output 1's gain, as the session's snapshot and as its core has it
    float shown(int i) const { return s[i]->model()->state() ? s[i]->model()->state()->output_gain_db[0] : 99.0f; }
    void set(int i, float db) const {
        s[i]->model()->write(QStringLiteral("lay7 gain"), [db](FfiCore *c) { return dspi_set_output_gain(c, 0, db); });
        idle(s[i]);
    }
    bool values(std::initializer_list<float> want) const {
        int i = 0;
        bool ok = true;
        for (float w : want) ok = ok && std::abs(shown(i++) - w) < 0.01f;
        return ok;
    }
    bool visible(const QString &name) const {
        QQuickItem *it = d.item(name);
        return it && it->isVisible();
    }
    bool tab(const char *place, int i, bool whole = false) const {
        QQuickItem *it = d.item(QLatin1String(place) + QStringLiteral(".tab.") + s[i]->id());
        if (!it || !it->isVisible()) return false;
        if (!whole) return true;
        const qreal left = it->mapToScene(QPointF(0, 0)).x(), right = it->mapToScene(QPointF(it->width(), 0)).x();
        return it->width() >= 60 && left >= -1 && right <= main->width() + 1;
    }
    bool click(const QString &name) const {
        QQuickItem *it = d.item(name);
        if (!it || !it->isVisible() || it->width() <= 0) return false;
        QTest::mouseClick(main, Qt::LeftButton, Qt::NoModifier, it->mapToScene(QPointF(it->width() / 2, it->height() / 2)).toPoint());
        d.spin(80);
        return true;
    }
};

void switchingPart(Three &t) {
    SmokeDriver &d = t.d;
    auto *layout = Registrar::instance<LayoutController>();
    Shell *shell = d.app().shell;
    t.check("three tabs, the third active", t.tab("DeviceRow", 0) && t.tab("DeviceRow", 1) && t.tab("DeviceRow", 2) && t.r->active() == t.s[2]);
    t.check("a tab makes the second active", t.click(QStringLiteral("DeviceRow.tab.") + t.s[1]->id()) && d.waitFor(3000, [&] { return t.r->active() == t.s[1]; }));
    d.waitIdle();
    d.goTo(QStringLiteral("equaliser/delay"));
    d.grab(t.main, QStringLiteral("lay7-three-modern.png"));
    QSignalSpy moved(t.r, &DeviceRegistry::activeChanged);

    // ---- Modern -> Classic: the active one stays, the others stay open behind it
    t.check("to Classic", d.switchLayout("classic"));
    t.check("the second is still the active one, all three are open and connected",
            t.r->active() == t.s[1] && moved.isEmpty() && t.r->sessions().size() == 3 && t.s[0]->model()->connected() && t.s[1]->model()->connected()
                && t.s[2]->model()->connected());
    QObject *status = d.object(QStringLiteral("DeviceStatus.button"));
    t.check("Classic shows it", status && status->property("shownName").toString() == t.s[1]->title());
    t.check("and no device tab", !d.item(QStringLiteral("DeviceRow")) && !t.r->keepOpen());
    auto *devices = Registrar::instance<DevicesController>();
    QQuickItem *host = devices->host();
    QObject *popup = host ? host->findChild<QObject *>(QStringLiteral("DevicesPopup")) : nullptr;
    shell->trigger(QStringLiteral("device.devices"));
    QQuickItem *content = popup && d.waitFor(2000, [&] { return popup->property("opened").toBool(); }) ? popup->property("contentItem").value<QQuickItem *>() : nullptr;
    bool listed = content != nullptr;
    for (const auto &s : t.s) listed = listed && d.item(QStringLiteral("DevicesPopup.open.") + s->id(), content);
    t.check("its device list has the three open ones", listed);
    d.grab(t.main, QStringLiteral("lay7-three-classic.png"));
    if (popup) QMetaObject::invokeMethod(popup, "close");
    d.spin(300);
    // An edit in Classic lands on the one it shows
    d.app().device->write(QStringLiteral("lay7 gain"), [](FfiCore *c) { return dspi_set_output_gain(c, 0, -5.0f); });
    t.idle(t.s[1]);
    t.check("an edit there is its own", t.values({ -1.0f, -5.0f, -3.0f }));
    // Another one is made active from Classic's list
    devices->chooseOpen(t.s[0]->id());
    t.check("Classic shows another of them", d.waitFor(3000, [&] { return t.r->active() == t.s[0]; }) && d.waitIdle()
                && status->property("shownName").toString() == t.s[0]->title());
    // A tool window open over the switch shows the active device, in both
    QWindow *stats = shell->openWindow(QStringLiteral("tools.matrix"));
    t.check("a tool window", stats && d.waitFor(2000, [&] { return stats->isVisible(); }));

    // ---- Classic -> Modern
    t.check("back to Modern", d.switchLayout("modern"));
    t.check("three tabs again, that one active", t.tab("DeviceRow", 0) && t.tab("DeviceRow", 1) && t.tab("DeviceRow", 2) && t.r->active() == t.s[0]
                && t.r->keepOpen() && d.item(QStringLiteral("DeviceRow.tab.") + t.s[0]->id())->property("checked").toBool());
    t.check("the tool window is still there, and its page says so", stats && stats->isVisible() && shell->shownWindows().contains(QStringLiteral("tools.matrix")));
    t.check("no device lost a value", t.values({ -1.0f, -5.0f, -3.0f }));
    t.r->activate(t.s[2]);
    d.waitIdle();
    t.check("a write is the active one's, whichever layout it was opened in", [&] {
        d.app().device->write(QStringLiteral("lay7 gain"), [](FfiCore *c) { return dspi_set_output_gain(c, 0, -7.0f); });
        t.idle(t.s[2]);
        return t.values({ -1.0f, -5.0f, -7.0f });
    }());
    shell->closeWindow(QStringLiteral("tools.matrix"));
    d.spin(200);

    // ---- There and back at once, as fast as a person can
    for (int i = 0; i < 6; ++i) {
        layout->setLayout(i % 2 ? QStringLiteral("modern") : QStringLiteral("classic"));
        d.spin(20);
        t.r->activate(t.s[i % 3]);
    }
    d.spin(300);
    for (const auto &s : t.s) t.idle(s);
    t.check("switched back and forth at once: the last layout, three devices, their own values",
            layout->modern() && shell->pageHost() && t.r->sessions().size() == 3 && t.r->active() == t.s[2] && t.values({ -1.0f, -5.0f, -7.0f }));
    t.check("and three tabs", d.waitFor(2000, [&] { return t.tab("DeviceRow", 0) && t.tab("DeviceRow", 1) && t.tab("DeviceRow", 2); }));
}

void narrowPart(Three &t) {
    SmokeDriver &d = t.d;
    PrefsMap *arrangement = Registrar::instance<LayoutController>()->arrangement();
    const QSize was = t.main->size();
    t.main->resize(1000, 660);
    d.waitFor(1000, [&] { return t.main->contentItem()->width() == 1000; });
    for (const char *navigation : { "rail", "tabs" })
        for (const char *place : { "topBar", "rail" }) {
            arrangement->setProperty("navigation", QString::fromLatin1(navigation));
            arrangement->setProperty("deviceTabs", QString::fromLatin1(place));
            d.spin(250);
            const bool rail = !qstrcmp(navigation, "rail") && !qstrcmp(place, "rail");
            const char *tabs = rail ? "DeviceRail" : "DeviceRow";
            const QString how = QStringLiteral("sections %1, devices %2").arg(QLatin1String(navigation), QLatin1String(place));
            d.check(QStringLiteral("lay7: three device tabs are whole at 1000x660 (%1)").arg(how),
                    t.tab(tabs, 0, true) && t.tab(tabs, 1, true) && t.tab(tabs, 2, true));
            bool bars = SmokeDriver::fits(d.item(QStringLiteral("TopBar"))) && SmokeDriver::fits(d.item(QStringLiteral("Dock")));
            QQuickItem *nav = d.item(!qstrcmp(navigation, "rail") ? QStringLiteral("NavRail") : QStringLiteral("NavTabs"));
            bars = bars && nav && nav->isVisible();
            d.check(QStringLiteral("lay7: the bars fit (%1)").arg(how), bars);
            // The last section can be reached: by scrolling where there is no room
            QQuickItem *last = d.item(nav->objectName() + QStringLiteral(".settings"));
            if (QQuickItem *first = d.item(nav->objectName() + QStringLiteral(".overview"))) first->forceActiveFocus(Qt::TabFocusReason);
            if (last) last->forceActiveFocus(Qt::TabFocusReason);
            d.spin(60);
            const QPointF end = last ? last->mapToItem(nav, QPointF(last->width(), last->height())) : QPointF(1e9, 1e9);
            d.check(QStringLiteral("lay7: the last section comes into sight with the keyboard (%1)").arg(how),
                    last && end.x() <= nav->width() + 1 && end.y() <= nav->height() + 1);
            d.grab(t.main, QStringLiteral("lay7-narrow-%1-%2.png").arg(QLatin1String(navigation), QLatin1String(place)));
        }
    arrangement->setProperty("navigation", QStringLiteral("rail"));
    arrangement->setProperty("deviceTabs", QStringLiteral("topBar"));
    t.main->resize(was);
    d.spin(250);
}

void quitPart(Three &t) {
    SmokeDriver &d = t.d;
    auto *firmware = Registrar::instance<FirmwareController>();
    auto *guard = Registrar::instance<UnsavedGuard>();
    auto *files = Registrar::instance<FilesController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    QObject *busy = files->host() ? files->host()->findChild<QObject *>(QStringLiteral("FilesHost.busyQuestion")) : nullptr;
    QObject *unsaved = files->host() ? files->host()->findChild<QObject *>(QStringLiteral("FilesHost.unsavedQuestion")) : nullptr;
    t.check("the quit questions are there", firmware && guard && notices && busy && unsaved);
    if (!firmware || !guard || !notices || !busy || !unsaved) return;
    const auto opened = [&](QObject *q) { return d.waitFor(3000, [q] { return q->property("opened").toBool(); }); };
    const auto closed = [&](QObject *q) { return d.waitFor(3000, [q] { return !q->property("visible").toBool(); }); };
    const auto finish = [&](QObject *q, const char *answer) { QMetaObject::invokeMethod(q, "finish", Q_ARG(QVariant, QString::fromLatin1(answer))); };

    // ---- A firmware update on a device that is not shown
    t.r->activate(t.s[0]);
    d.waitIdle();
    firmware->pretendRunForTest(t.s[2]);
    d.app().shell->trigger(QStringLiteral("file.quit"));
    t.check("quitting asks first while a firmware update runs on another device, named",
            opened(busy) && busy->property("text").toString().contains(t.s[2]->title()) && !busy->property("text").toString().contains(t.s[0]->title())
                && guard->asking());
    d.grab(t.main, QStringLiteral("lay7-quit-busy.png"));
    // ---- A second path while a question is open
    QStringList said;
    const QMetaObject::Connection heard = QObject::connect(notices, &NoticeCenter::changed, notices, [&] {
        const QString text = notices->last().value(QStringLiteral("text")).toString();
        if (said.isEmpty() || said.last() != text) said << text;
    });
    const auto refused = [&] { return said.contains(QStringLiteral("Answer the open question first.")) || (std::printf("smoke: lay7: said: %s\n", qPrintable(said.join(QStringLiteral(" | ")))), false); };
    notices->clear();   // the same notice again only restarts its time
    said.clear();
    d.app().shell->trigger(QStringLiteral("file.quit"));
    d.spin(100);
    t.check("a second path while a question is open says why nothing happens", refused() && busy->property("opened").toBool());
    notices->clear();
    said.clear();
    Registrar::instance<DevicesController>()->closeOpen(t.s[1]->id());
    d.spin(100);
    t.check("closing a tab too, and it stays open", refused() && t.r->sessions().size() == 3);
    QObject::disconnect(heard);
    finish(busy, "reject");
    t.check("Cancel stays", closed(busy) && d.waitFor(1000, [&] { return !guard->asking(); }) && t.r->sessions().size() == 3);
    t.main->close();
    t.check("closing the window asks the same", opened(busy) && t.main->isVisible());
    finish(busy, "accept");
    t.check("Quit goes on to the unsaved changes", closed(busy) && opened(unsaved) && guard->asking());
    finish(unsaved, "reject");
    t.check("and Cancel there stays", closed(unsaved) && d.waitFor(1000, [&] { return !guard->asking(); }) && t.main->isVisible());
    firmware->pretendRunForTest(nullptr);
    d.app().shell->trigger(QStringLiteral("file.quit"));
    t.check("with nothing running only the unsaved changes are asked about", opened(unsaved) && !busy->property("visible").toBool());
    finish(unsaved, "reject");
    closed(unsaved);
    d.waitFor(1000, [&] { return !guard->asking(); });
    notices->clear();
    d.spin(100);
}

void menusPart(Three &t) {
    SmokeDriver &d = t.d;
    auto *firmware = Registrar::instance<FirmwareController>();
    // ---- A pin that begins while the tab's menu is open
    const QString id = t.s[1]->id();
    QQuickItem *tab = d.item(QStringLiteral("DeviceRow.tab.") + id);
    const QString name = QStringLiteral("DeviceRow.menu.") + id;
    if (tab) QTest::mouseClick(t.main, Qt::RightButton, Qt::NoModifier, tab->mapToScene(QPointF(tab->width() / 2, tab->height() / 2)).toPoint());
    d.spin(200);
    QObject *menu = tab ? tab->findChild<QObject *>(name) : nullptr;
    if (!menu) menu = d.object(name);
    const auto part = [&](const char *suffix) { return qobject_cast<QQuickItem *>(menu ? menu->findChild<QObject *>(name + QLatin1String(suffix)) : nullptr); };
    QQuickItem *close = part(".device.close");
    if (!close) close = d.item(name + QStringLiteral(".device.close"), t.main->contentItem()->parentItem());
    QQuickItem *why = part(".device.close.why");
    if (!why) why = d.item(name + QStringLiteral(".device.close.why"), t.main->contentItem()->parentItem());
    t.check("a tab's menu opens, Close can be chosen and gives no reason",
            menu && menu->property("opened").toBool() && close && close->isEnabled() && why && !why->isVisible() && why->height() < 1);
    firmware->pretendRunForTest(t.s[1]);
    d.spin(100);
    t.check("a firmware update that begins greys it while the menu is open, with why",
            close && !close->isEnabled() && why && why->isVisible() && why->height() > 10 && why->property("text").toString().contains(t.s[1]->title()));
    d.grab(t.main, QStringLiteral("lay7-menu-pinned.png"));
    firmware->pretendRunForTest(nullptr);
    d.spin(100);
    t.check("and its end gives it back", close && close->isEnabled() && why && !why->isVisible());
    if (menu) QMetaObject::invokeMethod(menu, "close");
    d.spin(200);

    // ---- The preset slot's submenu
    QObject *menus = d.app().engine->singletonInstance<QObject *>("UConsole", "ContextMenus");
    QObject *slot = menus ? menus->findChild<QObject *>(QStringLiteral("ContextMenus.slot")) : nullptr;
    QQuickItem *at = d.item(QStringLiteral("TopBar.preset"));
    const int active = d.app().device->presets()->activeSlot();
    if (menus && at)
        QMetaObject::invokeMethod(menus, "slot", Q_ARG(QVariant, active), Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QVariant::fromValue(at)), Q_ARG(QVariant, 4),
                                  Q_ARG(QVariant, 4));
    d.waitFor(1000, [&] { return slot && slot->property("opened").toBool(); });
    QObject *copy = slot ? slot->findChild<QObject *>(QStringLiteral("ContextMenus.slot.slot.copyTo")) : nullptr;
    const auto rows = [&] {
        QList<QQuickItem *> out;
        const int n = copy ? copy->property("count").toInt() : 0;
        for (int i = 0; i < n; ++i) {
            QQuickItem *it = nullptr;
            QMetaObject::invokeMethod(copy, "itemAt", Q_RETURN_ARG(QQuickItem *, it), Q_ARG(int, i));
            if (it) out << it;
        }
        return out;
    };
    // The submenu's own row in the menu
    const auto parentRow = [&]() -> QQuickItem * {
        const int n = slot ? slot->property("count").toInt() : 0;
        for (int i = 0; i < n; ++i) {
            QQuickItem *it = nullptr;
            QMetaObject::invokeMethod(slot, "itemAt", Q_RETURN_ARG(QQuickItem *, it), Q_ARG(int, i));
            if (it && it->property("subMenu").value<QObject *>() == copy) return it;
        }
        return nullptr;
    };
    const auto all = [&](bool on) {
        const QList<QQuickItem *> list = rows();
        return !list.isEmpty() && std::all_of(list.begin(), list.end(), [on](QQuickItem *i) { return i->property("enabled").toBool() == on; });
    };
    t.check("a slot's menu: Copy to… and its rows can be chosen", slot && copy && parentRow() && parentRow()->property("enabled").toBool() && all(true));
    // A save begins under it, and waits in the device's queue
    auto gate = std::make_shared<QSemaphore>();
    t.r->active()->worker()->post([gate](FfiCore *) { gate->tryAcquire(1, 10000); });
    Registrar::instance<SidebarController>()->savePreset();
    d.spin(60);
    t.check("a save that begins while it is open greys Copy to… and every row of it", parentRow() && !parentRow()->property("enabled").toBool() && all(false));
    gate->release();
    t.idle(t.r->active());
    t.check("and its end gives them back", d.waitFor(2000, [&] { return parentRow() && parentRow()->property("enabled").toBool() && all(true); }));
    if (slot) QMetaObject::invokeMethod(slot, "close");
    d.spin(200);

    // ---- The words: Modern has no Edit menu
    d.goTo(QStringLiteral("equaliser/peq"));
    QObject *clear = d.object(QStringLiteral("EqualiserPeq.bands.clearQuestion"));
    const QString words = clear ? clear->property("text").toString() : QString();
    t.check("Clear All's question names no menu", words.contains(QLatin1String("Undo puts them back")) && !words.contains(QLatin1String("Edit >")));

    // ---- The command palette follows
    auto *palette = Registrar::instance<PaletteController>();
    t.main->requestActivate();
    d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == t.main; });
    palette->open();
    QObject *popup = d.object(QStringLiteral("CommandPalette"));
    t.check("the palette opens", popup && d.waitFor(1500, [&] { return popup->property("opened").toBool(); }));
    const auto entry = [&](const char *id) {
        const QVariant v = popup ? popup->property("entries") : QVariant();
        for (const QVariant &e : v.canConvert<QJSValue>() ? v.value<QJSValue>().toVariant().toList() : v.toList())
            if (e.toMap().value(QStringLiteral("id")).toString() == QLatin1String(id)) return e.toMap();
        return QVariantMap();
    };
    t.check("Redo is grey: nothing was undone", !entry("edit.redo").isEmpty() && !entry("edit.redo").value(QStringLiteral("enabled")).toBool());
    bool undone = false;
    d.app().device->write(QStringLiteral("lay7 gain"), [](FfiCore *c) { return dspi_set_output_gain(c, 0, -9.0f); });
    d.waitIdle();
    d.app().device->write(QStringLiteral("lay7 undo"), [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_undo(c, b, n); }, 1 << 14).size() > 0; },
                          [&undone](bool ok) { undone = ok; });
    t.check("an undo while it is open lets Redo be chosen", d.waitFor(5000, [&] { return undone && entry("edit.redo").value(QStringLiteral("enabled")).toBool(); }));
    if (popup) QMetaObject::invokeMethod(popup, "close");
    d.spin(200);
    d.waitIdle();
}

// ---- A remembered size --------------------------------------------------------------------

void restoredPart(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    for (const char *action : { "tools.loudness", "tools.leveller" }) {
        auto *real = qobject_cast<QQuickWindow *>(shell->openWindow(QString::fromLatin1(action)));
        d.waitFor(2000, [&] { return real && real->isVisible(); });
        if (real) {
            real->close();
            real->resize(real->property("defaultWidth").toInt(), real->property("defaultHeight").toInt());
            shell->openWindow(QString::fromLatin1(action));
        }
        d.waitIdle();
        d.spin(400);
        const QSize fitted = real ? real->size() : QSize();
        shell->closeWindow(QString::fromLatin1(action));
        QVariantMap row;
        for (const QVariant &v : shell->panels())
            if (v.toMap().value(QStringLiteral("action")).toString() == QLatin1String(action)) row = v.toMap();
        for (const int by : { -150, 110 }) {
            const QString id = QStringLiteral("lay7%1%2").arg(QLatin1String(action).mid(6)).arg(by < 0 ? QStringLiteral("small") : QStringLiteral("tall"));
            row.insert(QStringLiteral("windowId"), id);
            {
                QSettings settings;
                settings.setValue(QStringLiteral("windows/%1/width").arg(id), fitted.width());
                settings.setValue(QStringLiteral("windows/%1/height").arg(id), fitted.height() + by);
                settings.sync();
            }
            QQmlComponent component(d.app().engine);
            component.setData("import QtQuick\nimport UConsole\nPanelWindow { visible: false }", QUrl(QStringLiteral("lay7.qml")));
            std::unique_ptr<QObject> made(component.createWithInitialProperties({ { QStringLiteral("panel"), row } }));
            auto *w = qobject_cast<QQuickWindow *>(made.get());
            if (!w) std::printf("smoke: lay7: %s\n", qPrintable(component.errorString()));
            d.check(QStringLiteral("lay7: %1 starts at its remembered height (%2)").arg(QLatin1String(action)).arg(w ? w->height() : -1),
                    w && (w->height() == fitted.height() + by || w->height() == w->minimumHeight()));
            if (!w) continue;
            w->show();
            d.waitIdle();
            d.spin(500);
            d.check(QStringLiteral("lay7: %1 remembered at %2 fits its content when it shows: %3 (fitted %4)")
                        .arg(QLatin1String(action)).arg(fitted.height() + by).arg(w->height()).arg(fitted.height()),
                    // (above the cap already, it keeps what its content needs of that: lay1a)
                    w->height() == fitted.height() || (by > 0 && fitted.height() >= shell->fitCap(w).height() && w->height() > fitted.height() && w->height() < fitted.height() + by));
            w->close();
            d.spin(100);
            made.reset();
            QSettings().remove(QStringLiteral("windows/") + id);
        }
    }
}

void devicesSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    auto *layout = Registrar::instance<LayoutController>();
    DeviceRegistry *r = DeviceRegistry::instance();
    DeviceModel *device = d.app().device;
    Three t{ d, r, d.mainWindow(), {} };
    t.check("the controllers", layout && r && t.main);
    if (!layout || !r || !t.main) return;
    for (const QString &action : d.app().shell->shownWindows()) d.app().shell->closeWindow(action);
    restoredPart(d);
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    t.check("on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    const float was = device->state() ? device->state()->output_gain_db[0] : 0.0f;
    const QSize size = t.main->size();
    t.main->resize(1280, 800);
    t.check("in Modern", d.switchLayout("modern"));
    t.s << r->mainSession();
    for (int i = 1; i < 3; ++i) {
        r->openDemo();
        const bool open = d.waitFor(15000, [&] { return r->opening() == 0 && r->sessions().size() == i + 1 && r->active() == r->sessions().last(); });
        if (open) t.s << r->sessions().last();
    }
    t.check("three devices are open", t.s.size() == 3);
    d.waitIdle();
    d.spin(300);
    if (t.s.size() == 3) {
        for (int i = 0; i < 3; ++i) t.set(i, -1.0f - float(i));
        t.check("each has a value of its own", t.values({ -1.0f, -2.0f, -3.0f }));
        t.check("and a name of its own", d.waitFor(3000, [&] {
            return t.s[0]->title() != t.s[1]->title() && t.s[1]->title() != t.s[2]->title() && t.s[0]->title() != t.s[2]->title();
        }));
        switchingPart(t);
        narrowPart(t);
        quitPart(t);
        menusPart(t);
    }
    if (layout->modern()) d.switchLayout("classic");
    r->activate(r->mainSession());
    for (DeviceSession *s : r->sessions())
        if (!s->isMain()) r->close(s->id());
    d.waitFor(10000, [&] { return r->sessions().size() == 1; });
    t.check("one device again", r->sessions().size() == 1 && device->connected());
    device->write(QStringLiteral("lay7 gain"), [was](FfiCore *c) { return dspi_set_output_gain(c, 0, was); });
    d.waitIdle();
    t.main->resize(size);
    d.spin(200);
}

const SmokeDriver::Section contrast("lay7-contrast", 901, &contrastSection);
const SmokeDriver::Section devicesSmoke("lay7-devices", 968, &devicesSection);

} // namespace
