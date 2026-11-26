// -- WP LAY2 -- The popovers' content split from their hosts: Classic's
// popovers host the content files (LimiterSettingsContent, LinkwitzContent,
// GraphOptionsContent, DashboardLayoutContent, DevicesContent), and the same
// content works in an InspectorSection (SmokeLay2Window.qml) with the same
// object names: the Linkwitz draft and Apply, the limiter's link groups, the
// graph options' two pages, the dashboard tiles and the Devices list's Done.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "DashboardController.h"
#include "DeviceModel.h"
#include "Registrar.h"
#include "Shell.h"
#include "ToolsController.h"

#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <cmath>

namespace {

const Registrar::Window lay2Window("smoke.lay2", "SmokeLay2Window");

constexpr int kPeaking = 0;      // FILTER_PEAKING
constexpr int kLinkwitz = 11;    // FILTER_LINKWITZ_TRANSFORM

bool near(double a, double b, double tol = 1e-3) { return std::abs(a - b) <= tol; }

// The first object named `name` under `scope`, its QObject children then its items
QObject *find(QObject *scope, const QString &name) {
    if (!scope) return nullptr;
    if (QObject *o = scope->findChild<QObject *>(name)) return o;
    std::function<QQuickItem *(QQuickItem *)> walk = [&](QQuickItem *i) -> QQuickItem * {
        if (!i) return nullptr;
        if (i->objectName() == name) return i;
        for (QQuickItem *c : i->childItems())
            if (QQuickItem *f = walk(c)) return f;
        return nullptr;
    };
    if (auto *w = qobject_cast<QQuickWindow *>(scope)) return walk(w->contentItem());
    return walk(qobject_cast<QQuickItem *>(scope));
}

// A popover's content item is the content file's type ("LimiterSettingsContent_QMLTYPE_12")
bool hosts(QObject *popover, const char *type) {
    auto *content = popover ? popover->property("contentItem").value<QObject *>() : nullptr;
    return content && QString::fromLatin1(content->metaObject()->className()).startsWith(QLatin1String(type));
}

QString textOf(QObject *o) { return o ? o->property("text").toString() : QString(); }

// Types into a ChannelValueField and presses Enter
void type(SmokeDriver &d, QObject *scope, const QString &field, const QString &text) {
    QObject *o = find(scope, field + QStringLiteral(".field"));
    d.check(QStringLiteral("lay2: %1 is found in the inspector").arg(field), o != nullptr);
    if (!o) return;
    o->setProperty("text", text);
    QMetaObject::invokeMethod(o, "textEdited"); // -- WP INTE -- (REVE C1) as typed: Enter writes only typed text
    QMetaObject::invokeMethod(o, "accepted");
    d.waitIdle();
}

void click(SmokeDriver &d, QObject *scope, const QString &name) {
    QObject *o = find(scope, name);
    d.check(QStringLiteral("lay2: %1 is found in the inspector").arg(name), o != nullptr);
    if (o) QMetaObject::invokeMethod(o, "click");
    d.waitIdle();
}

// The Classic hosts: each popover, made apart from the window that holds it,
// carries its content file
void classicHosts(SmokeDriver &d) {
    struct Host {
        const char *type;
        const char *popover;   // the PopoverPanel's name inside it, or "" when it is one
        const char *content;
    };
    const Host all[] = {
        { "LimiterSettings", "", "LimiterSettingsContent" },
        { "Linkwitz", "", "LinkwitzContent" },
        { "GraphOptions", "GraphHost.options.popover", "GraphOptionsContent" },
        { "DashboardLayoutPopup", "", "DashboardLayoutContent" },
        { "DevicesPopup", "", "DevicesContent" },
    };
    for (const Host &h : all) {
        QQmlComponent c(d.app().engine, QStringLiteral("UConsole"), QString::fromLatin1(h.type));
        QObject *o = c.createWithInitialProperties({ { QStringLiteral("parent"), QVariant::fromValue(d.mainWindow()->contentItem()) } });
        if (!o) std::printf("smoke: lay2: %s did not load: %s\n", h.type, qPrintable(c.errorString()));
        QObject *popover = o && *h.popover ? o->findChild<QObject *>(QLatin1String(h.popover)) : o;
        d.check(QStringLiteral("lay2: Classic's %1 is a popover over %2").arg(QLatin1String(h.type), QLatin1String(h.content)),
                popover && popover->inherits("QQuickPopup") && hosts(popover, h.content));
        delete o;
    }
    QObject *graph = d.object(QStringLiteral("GraphHost.options.popover"));
    d.check(QStringLiteral("lay2: the graph's popover still says which page shows"), graph && graph->property("setup").isValid());
}

void limiter(SmokeDriver &d, QQuickWindow *w) {
    auto *t = Registrar::instance<ToolsController>();
    QObject *section = find(w, QStringLiteral("Lay2.limiter"));
    QObject *content = find(w, QStringLiteral("Lay2.limiter.content"));
    d.check(QStringLiteral("lay2: the limiter's content is in an inspector section"), section && content && t);
    if (!section || !content || !t) return;
    for (const char *name : { "LimiterSettings.enabled", "LimiterSettings.threshold", "LimiterSettings.release", "LimiterSettings.group",
                              "LimiterSettings.groupSummary", "LimiterSettings.copyToAll", "LimiterSettings.allOutputs" })
        d.check(QStringLiteral("lay2: %1 is found in the inspector").arg(QLatin1String(name)), find(content, QLatin1String(name)) != nullptr);
    d.check(QStringLiteral("lay2: the limiter's content polls while its section shows"), content->property("active").toBool());
    section->setProperty("expanded", false);
    d.check(QStringLiteral("lay2: folded, it stops"), !content->property("active").toBool() && !section->property("shown").toBool());
    section->setProperty("expanded", true);
    d.check(QStringLiteral("lay2: unfolded, it polls again"), content->property("active").toBool());

    // The link groups work the same from here: unlink all, then join group 3
    // (the settings take edits while the limiter is on)
    const QVariantMap before = t->limiter(0);
    t->setLimiterEnabled(0, true);
    d.waitIdle();
    QMetaObject::invokeMethod(find(content, QStringLiteral("LimiterSettings.unlinkAll")), "triggered");
    d.waitIdle();
    d.check(QStringLiteral("lay2: Unlink all outputs, from the inspector"),
            d.waitFor(1000, [&] { return textOf(find(content, QStringLiteral("LimiterSettings.groupSummary"))) == QLatin1String("Not linked."); }));
    click(d, content, QStringLiteral("LimiterSettings.group.3"));
    d.check(QStringLiteral("lay2: the inspector's group choice reaches the output"),
            d.waitFor(1000, [&] { return t->limiter(0).value(QStringLiteral("group")).toInt() == 3; }));
    d.check(QStringLiteral("lay2: and its summary follows"),
            textOf(find(content, QStringLiteral("LimiterSettings.groupSummary"))) == QLatin1String("No other outputs in group 3."));
    t->setLimiterGroup(0, before.value(QStringLiteral("group")).toInt());
    t->setLimiterEnabled(0, before.value(QStringLiteral("enabled")).toBool());
    d.waitIdle();
}

void linkwitz(SmokeDriver &d, QQuickWindow *w, ChannelController *cc, DeviceModel *device) {
    QObject *section = find(w, QStringLiteral("Lay2.linkwitz"));
    QObject *content = find(w, QStringLiteral("Lay2.linkwitz.content"));
    d.check(QStringLiteral("lay2: the Linkwitz content is in an inspector section"), section && content);
    if (!section || !content) return;
    const int ch = cc->outputChannel(0);
    const auto gain = [&] { return device->state() ? double(device->state()->filters[ch][0].gain) : NAN; };
    const QVariantMap was = cc->band(ch, 0, false);

    // A band made a flat transform at 40 Hz, then shown in the inspector
    cc->setBandType(ch, 0, false, kPeaking);
    d.waitIdle();
    cc->setBandValue(ch, 0, false, QStringLiteral("freq"), 40);
    d.waitIdle();
    cc->setBandType(ch, 0, false, kLinkwitz);
    d.waitIdle();
    d.check(QStringLiteral("lay2: the band is a flat transform (fp = f0)"), near(gain(), 40, 0.06));
    w->setProperty("channel", ch);
    w->setProperty("band", 0);
    d.spin(100);
    d.check(QStringLiteral("lay2: shown, the draft starts from the band as applied"),
            content->property("active").toBool() && near(content->property("fp").toDouble(), 40, 0.06));

    // The draft stays a draft until Apply
    type(d, content, QStringLiteral("Linkwitz.fp"), QStringLiteral("20"));
    QObject *state = find(content, QStringLiteral("Linkwitz.state"));
    QObject *boost = find(content, QStringLiteral("Linkwitz.boost"));
    d.check(QStringLiteral("lay2: a draft in the inspector is not sent"), near(gain(), 40, 0.06));
    d.check(QStringLiteral("lay2: the inspector says the draft is not applied"), textOf(state) == QLatin1String("Not applied yet"));
    d.check(QStringLiteral("lay2: and shows the implied boost"), textOf(boost) == QLatin1String("+12.0 dB"));
    d.check(QStringLiteral("lay2: screenshot of the inspector sections"), d.grab(w, QStringLiteral("window-lay2-inspector.png")));

    // Folding drops the draft, as closing the popover does
    section->setProperty("expanded", false);
    section->setProperty("expanded", true);
    d.check(QStringLiteral("lay2: folded and unfolded, the draft starts again from the band"),
            near(content->property("fp").toDouble(), 40, 0.06) && textOf(state) == QLatin1String("Applied"));

    type(d, content, QStringLiteral("Linkwitz.fp"), QStringLiteral("20"));
    click(d, content, QStringLiteral("Linkwitz.apply"));
    d.check(QStringLiteral("lay2: Apply in the inspector sends the transform"), near(gain(), 20, 0.06));
    d.check(QStringLiteral("lay2: and the inspector says so"), textOf(state) == QLatin1String("Applied"));

    // Revert goes back to the band as applied
    type(d, content, QStringLiteral("Linkwitz.fp"), QStringLiteral("30"));
    click(d, content, QStringLiteral("Linkwitz.revert"));
    d.check(QStringLiteral("lay2: Revert in the inspector goes back to the band"),
            near(content->property("fp").toDouble(), 20, 0.06) && near(gain(), 20, 0.06));

    // The band as it was
    w->setProperty("channel", -1);
    cc->setBandType(ch, 0, false, kPeaking);
    d.waitIdle();
    cc->setBandType(ch, 0, false, was.value(QStringLiteral("type")).toInt());
    d.waitIdle();
    for (const char *key : { "freq", "gain", "q" }) {
        cc->setBandValue(ch, 0, false, QLatin1String(key), was.value(QLatin1String(key)).toDouble());
        d.waitIdle();
    }
}

void graphOptions(SmokeDriver &d, QQuickWindow *w) {
    QObject *content = find(w, QStringLiteral("Lay2.graph.content"));
    d.check(QStringLiteral("lay2: the graph options are in an inspector section"), content != nullptr);
    if (!content) return;
    for (const char *name : { "Lay2.graphOptions.setup", "Lay2.graphOptions.popOut", "Lay2.graphOptions.back", "Lay2.graphOptions.glow" })
        d.check(QStringLiteral("lay2: %1 is found in the inspector").arg(QLatin1String(name)), find(content, QLatin1String(name)) != nullptr);
    QObject *setup = find(content, QStringLiteral("Lay2.graphOptions.setup"));
    if (setup) QMetaObject::invokeMethod(setup, "activated");
    d.check(QStringLiteral("lay2: Graph Setup opens its page in the inspector"), content->property("setup").toBool());
    QObject *back = find(content, QStringLiteral("Lay2.graphOptions.back"));
    if (back) QMetaObject::invokeMethod(back, "activated");
    d.check(QStringLiteral("lay2: Back returns to the main page"), !content->property("setup").toBool());
    d.check(QStringLiteral("lay2: nothing asked the inspector to put the options away"), w->property("graphDone").toInt() == 0);
}

void dashboard(SmokeDriver &d, QQuickWindow *w) {
    auto *dc = Registrar::instance<DashboardController>();
    QObject *content = find(w, QStringLiteral("Lay2.dashboard.content"));
    d.check(QStringLiteral("lay2: the dashboard layout is in an inspector section"), content && dc);
    if (!content || !dc) return;
    const int before = dc->cardsPerRow();
    QObject *two = find(content, QStringLiteral("DashboardLayoutPopup.tile.2"));
    d.check(QStringLiteral("lay2: DashboardLayoutPopup.tile.2 is found in the inspector"), two != nullptr);
    if (two) QMetaObject::invokeMethod(two, "clicked");
    d.check(QStringLiteral("lay2: the inspector's tile chooses two per row"), dc->cardsPerRow() == 2);
    d.check(QStringLiteral("lay2: its summary follows"),
            textOf(find(content, QStringLiteral("DashboardLayoutPopup.summary"))) == QLatin1String("Up to 2 cards per row."));
    dc->setCardsPerRow(before);
}

void devices(SmokeDriver &d, QQuickWindow *w) {
    QObject *content = find(w, QStringLiteral("Lay2.devices.content"));
    d.check(QStringLiteral("lay2: the device list is in an inspector section"), content != nullptr);
    if (!content) return;
    for (const char *name : { "DevicesPopup.done", "DevicesPopup.demo", "DevicesPopup.phaseText", "DevicesPopup.lookForHubs", "DevicesPopup.address" })
        d.check(QStringLiteral("lay2: %1 is found in the inspector").arg(QLatin1String(name)), find(content, QLatin1String(name)) != nullptr);
    QObject *demo = find(content, QStringLiteral("DevicesPopup.demo"));
    d.check(QStringLiteral("lay2: the inspector's list marks the demo as current"), demo && demo->property("current").toBool());
    click(d, content, QStringLiteral("DevicesPopup.done"));
    d.check(QStringLiteral("lay2: Done tells the inspector's host"), w->property("devicesDone").toInt() == 1);
}

void lay2Section(SmokeDriver &d) {
    auto *cc = Registrar::instance<ChannelController>();
    DeviceModel *device = d.app().device;
    d.check(QStringLiteral("lay2: on the demo"), cc && device->connected() && device->demo());
    if (!cc || !device->connected()) return;
    classicHosts(d);

    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("smoke.lay2")));
    d.check(QStringLiteral("lay2: the inspector window opens"), w && d.waitFor(3000, [&] { return w->isVisible(); }));
    if (!w) return;
    d.spin(200);
    limiter(d, w);
    linkwitz(d, w, cc, device);
    graphOptions(d, w);
    dashboard(d, w);
    devices(d, w);
    w->close();
    d.spin(100);
}

const SmokeDriver::Section section("lay2-content", 950, &lay2Section);

} // namespace
