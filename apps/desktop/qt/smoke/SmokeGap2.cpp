// -- WP GAP2 -- "Solo this output", in Modern on the demo device (gap2-solo):
// the channel menu's Solo row solos the output under the pointer (the core says
// so, the state keeps the user's mutes: none), the row then says End Solo, the
// status strip's banner names the output and ends it; the Routing inspector's
// Solo switch shows it, moves it to another output and ends it; a solo the core
// ended by itself (a load, a save) leaves the controller and the banner at the
// next snapshot. Screenshots, dark and light, go to <out>/gap2/.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"
#include "Snapshot.h"
#include "SoloController.h"

#include <QDir>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>

namespace {

// The shown item named `name` under `from`
QQuickItem *shownItem(QQuickItem *from, const QString &name) {
    if (!from || !from->isVisible()) return nullptr;
    if (from->objectName() == name) return from;
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = shownItem(child, name)) return found;
    return nullptr;
}

// The core's own answer, on the active device's worker
int coreSolo(SmokeDriver &d) {
    int out = -2;
    bool done = false;
    QObject ctx;
    d.app().core->worker()->call(&ctx, [](FfiCore *c) { return dspi_solo_state(c); }, [&](int o) {
        out = o;
        done = true;
    });
    d.waitFor(5000, [&] { return done; });
    return out;
}

void soloSection(SmokeDriver &d) {
    d.allowSectionTime(150);
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *cc = Registrar::instance<ChannelController>();
    auto *sc = Registrar::instance<SidebarController>();
    auto *solo = Registrar::instance<SoloController>();
    QObject *menus = d.app().engine->singletonInstance<QObject *>("UConsole", "ContextMenus");
    QQuickWindow *w = d.mainWindow();
    d.check("gap2: the controllers and the window", cc && sc && solo && menus && w);
    if (!cc || !sc || !solo || !menus || !w) return;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check("gap2: on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    d.check("gap2: no solo to begin with", solo->output() == -1 && coreSolo(d) == -1);
    const int outputs = cc->numOutputs();
    QVector<int> on;
    for (int o = 0; o < outputs; ++o)
        if (cc->output(o).value(QStringLiteral("enabled")).toBool()) on << o;
    d.check(QStringLiteral("gap2: two outputs are on (%1)").arg(on.size()), on.size() >= 2);
    if (on.size() < 2) return;
    const int first = on[0], second = on[1];
    const auto mutes = [&] {
        int n = 0;
        if (const DspState *s = device->state())
            for (int o = 0; o < outputs; ++o) n += s->output_muted[o] ? 1 : 0;
        return n;
    };
    const int mutedBefore = mutes();

    const QSize original = w->size();
    const Qt::ColorScheme scheme = shell->colorSchemeOverride();
    if (!d.switchLayout("modern")) {
        d.check("gap2: in Modern", false);
        return;
    }
    if (auto *notices = Registrar::instance<NoticeCenter>()) notices->clear();
    w->resize(QSize(1600, 1000));
    d.waitFor(1000, [&] { return w->contentItem()->width() == 1600; });
    QDir(d.outDir()).mkpath(QStringLiteral("gap2"));

    QObject *menu = menus->findChild<QObject *>(QStringLiteral("ContextMenus.channel"));
    const auto row = [&](const char *id) {
        return menu ? menu->findChild<QObject *>(QStringLiteral("ContextMenus.channel.") + QLatin1String(id)) : nullptr;
    };
    const auto settle = [&] {
        d.spin(40);
        d.waitIdle();
        d.spin(120);
    };
    // A right-click on the output's row in the Equaliser's channel list
    const auto openMenu = [&](int output) {
        d.goTo(QStringLiteral("equaliser/peq"), 200);
        QQuickItem *it = shownItem(w->contentItem(), QStringLiteral("Sidebar.channel.%1").arg(cc->outputChannel(output)));
        if (!it || !menu) return false;
        QTest::mouseClick(w, Qt::RightButton, Qt::NoModifier, it->mapToScene(QPointF(it->width() * 0.3, it->height() / 2)).toPoint());
        return d.waitFor(1500, [&] { return menu->property("opened").toBool(); });
    };
    const auto choose = [&](const char *id) {
        auto *r = qobject_cast<QQuickItem *>(row(id));
        if (!r || !r->isVisible() || !r->isEnabled()) return false;
        QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, r->mapToScene(QPointF(r->width() / 2, r->height() / 2)).toPoint());
        const bool closed = d.waitFor(1500, [&] { return !menu->property("visible").toBool(); });
        settle();
        return closed;
    };
    const auto closeMenu = [&] {
        if (menu) QMetaObject::invokeMethod(menu, "close");
        d.waitFor(1500, [&] { return !menu || !menu->property("visible").toBool(); });
        settle();
    };
    const auto banner = [&] { return shownItem(w->contentItem(), QStringLiteral("SoloBanner")); };
    const auto bannerText = [&] {
        QQuickItem *b = banner();
        return b ? b->property("text").toString() : QString();
    };
    const auto inspectorSwitch = [&](int output) {
        d.goTo(QStringLiteral("routing/matrix"), 200);
        sc->select(cc->outputChannel(output));
        settle();
        return shownItem(w->contentItem(), QStringLiteral("RoutingInspector.solo"));
    };
    // A click, as the pointer makes it (a switch answers toggled, not clicked)
    const auto click = [&](QQuickItem *it) {
        if (!it || !it->isEnabled()) return false;
        QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, it->mapToScene(QPointF(it->width() / 2, it->height() / 2)).toPoint());
        settle();
        return true;
    };
    const auto shoot = [&](const QString &name, const char *schemeName) {
        if (QQuickItem *focused = w->activeFocusItem(); focused && focused != w->contentItem()) focused->setFocus(false);
        d.spin(250);
        d.check(QStringLiteral("gap2: screenshot %1-%2").arg(name, QLatin1String(schemeName)),
                d.grab(w, QStringLiteral("gap2/%1-%2.png").arg(name, QLatin1String(schemeName))));
    };

    for (const char *schemeName : { "dark", "light" }) {
        shell->setColorSchemeOverride(QLatin1String(schemeName) == QLatin1String("dark") ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
        d.spin(300);

        // The channel menu: Solo, then End Solo
        d.check("gap2: an output's menu opens", openMenu(first));
        QObject *soloRow = row("channel.solo");
        d.check(QStringLiteral("gap2: it has Solo (%1)").arg(soloRow ? soloRow->property("text").toString() : QString()),
                soloRow && soloRow->property("text").toString() == QLatin1String("Solo"));
        shoot(QStringLiteral("menu-solo"), schemeName);
        d.check("gap2: Solo solos it", choose("channel.solo") && d.waitFor(3000, [&] { return solo->output() == first; }));
        d.check("gap2: the core says so", coreSolo(d) == first);
        d.check(QStringLiteral("gap2: the state keeps the user's mutes (%1, was %2)").arg(mutes()).arg(mutedBefore), mutes() == mutedBefore);
        d.check(QStringLiteral("gap2: the banner names it (%1)").arg(bannerText()),
                banner() && bannerText().startsWith(QLatin1String("Solo: only ")) && bannerText().contains(QString::number(first + 1)));
        d.check("gap2: and offers End Solo", shownItem(w->contentItem(), QStringLiteral("SoloBanner.end")) != nullptr);
        d.check("gap2: the row now says End Solo", openMenu(first) && row("channel.solo")
                && row("channel.solo")->property("text").toString() == QLatin1String("End Solo"));
        shoot(QStringLiteral("menu-end-solo"), schemeName);
        closeMenu();

        // The Routing inspector: the switch shows it, moves it, ends it
        QQuickItem *sw = inspectorSwitch(first);
        d.check("gap2: the inspector's Solo is on", sw && sw->property("checked").toBool());
        shoot(QStringLiteral("routing-solo"), schemeName);
        sw = inspectorSwitch(second);
        d.check("gap2: another output's Solo is off", sw && !sw->property("checked").toBool());
        d.check("gap2: switching it on moves the solo", click(sw)
                && d.waitFor(3000, [&] { return solo->output() == second; }) && coreSolo(d) == second);
        sw = inspectorSwitch(second);
        d.check("gap2: switching it off ends the solo", sw && sw->property("checked").toBool() && click(sw)
                && d.waitFor(3000, [&] { return solo->output() == -1; }) && coreSolo(d) == -1);
        d.check("gap2: the banner goes", d.waitFor(2000, [&] { return !banner(); }));
        shoot(QStringLiteral("routing-no-solo"), schemeName);

        // The banner's End Solo
        solo->solo(second);
        d.check("gap2: soloed again", d.waitFor(3000, [&] { return solo->output() == second; }));
        QQuickItem *end = shownItem(w->contentItem(), QStringLiteral("SoloBanner.end"));
        d.check("gap2: the banner's End Solo ends it", end && SmokeDriver::press(end)
                && d.waitFor(3000, [&] { return solo->output() == -1 && !banner(); }));
    }

    // A solo the core ended by itself (as a load or a save does) leaves at the
    // next snapshot
    solo->solo(first);
    d.check("gap2: soloed for the core's end", d.waitFor(3000, [&] { return solo->output() == first; }));
    bool ended = false;
    QObject ctx;
    d.app().core->worker()->call(&ctx, [](FfiCore *c) { return dspi_solo_end(c); }, [&](bool) { ended = true; });
    d.waitFor(3000, [&] { return ended; });
    device->refresh();
    d.check("gap2: the controller follows the core", d.waitFor(4000, [&] { return solo->output() == -1; }));
    d.check(QStringLiteral("gap2: the user's mutes as they were (%1)").arg(mutes()), mutes() == mutedBefore);

    shell->setColorSchemeOverride(scheme);
    sc->select(-1);
    w->resize(original);
    d.spin(200);
}

const SmokeDriver::Section soloSmoke("gap2-solo", 992, &soloSection);

} // namespace
