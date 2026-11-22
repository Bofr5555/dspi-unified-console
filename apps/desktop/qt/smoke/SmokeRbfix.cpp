// -- WP RBFIX -- Review RB's fixes: a name with markup in it is shown as it is
// written (the shared controls, and a channel's name on every page of both
// layouts); the Link banner's Disconnect on a hub's unit opened beside the
// others asks about its unsaved changes and is refused while a job pins it;
// the Modern layout is not left while a measurement runs on a device.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "DevicesController.h"
#include "FilesController.h"
#include "FirmwareController.h"
#include "LayoutController.h"
#include "LinkController.h"
#include "LinkFakeHub.h"
#include "MeasureController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "SettingsController.h"
#include "Shell.h"
#include "UnsavedGuard.h"

extern "C" {
#include "dspi_core.h"
}

#include <QCoreApplication>
#include <QGuiApplication>
#include <QPointer>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <cstdio>
#include <functional>
#include <memory>

namespace {

const QString kMarkup = QStringLiteral("<b>Rm</b>");

// Every Text below `item`
void texts(QQuickItem *item, QList<QQuickItem *> &out) {
    if (!item) return;
    if (item->inherits("QQuickText")) out << item;
    for (QQuickItem *c : item->childItems()) texts(c, out);
}

bool plain(const QQuickItem *text) { return text->property("textFormat").toInt() == Qt::PlainText; }

// The Texts showing `what`, in every window; those that would read markup in it are printed
struct Shown {
    int all = 0, rich = 0;
};
Shown showing(const QString &what) {
    Shown s;
    for (QWindow *w : QGuiApplication::topLevelWindows()) {
        auto *window = qobject_cast<QQuickWindow *>(w);
        if (!window || !window->isVisible()) continue;
        QList<QQuickItem *> found;
        texts(window->contentItem(), found);
        for (const QQuickItem *t : std::as_const(found)) {
            if (!t->property("text").toString().contains(what)) continue;
            ++s.all;
            if (plain(t)) continue;
            ++s.rich;
            const QQmlContext *context = qmlContext(t);
            std::printf("smoke: rbfix: markup read in %s\n", context ? qPrintable(context->baseUrl().fileName()) : "");
        }
    }
    return s;
}

bool switchTo(SmokeDriver &d, LayoutController *layout, const char *name, const char *shellName) {
    layout->setLayout(QString::fromLatin1(name));
    QObject *loader = d.object(QStringLiteral("MainWindow.shell"));
    const bool ok = d.waitFor(3000, [&] {
        const QObject *now = loader ? loader->property("item").value<QObject *>() : nullptr;
        return now && now->objectName() == QLatin1String(shellName);
    });
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    d.waitIdle();
    d.spin(200);
    return ok;
}

// ---- Names are plain text ----------------------------------------------------------------

void sharedControls(SmokeDriver &d) {
    const struct {
        const char *what, *qml;
    } controls[] = {
        { "AppButton", "AppButton { text: name }" },
        { "AppCheckBox", "AppCheckBox { text: name }" },
        { "AppComboBox", "AppComboBox { model: [name]; currentIndex: 0 }" },
        { "AppSwitch", "AppSwitch { text: name }" },
        { "AppTabButton", "AppTabButton { text: name }" },
        { "Pill", "Pill { text: name }" },
        { "SectionHeading", "SectionHeading { text: name }" },
        { "Banner", "Banner { text: name }" },
        { "SettingsPickerRow", "SettingsPickerRow { label: name; texts: [name]; values: [1]; value: 1 }" },
        { "SettingsToggle", "SettingsToggle { title: name }" },
    };
    for (const auto &c : controls) {
        QQmlComponent component(d.app().engine);
        component.setData(QByteArray("import QtQuick\nimport UConsole\nItem { id: host; property string name: \"x\"; property alias control: "
                                     "control; ") +
                              QByteArray(c.qml).replace("{ ", "{ id: control; ").replace(": name", ": host.name").replace("[name]", "[host.name]") + " }",
                          QUrl(QStringLiteral("rbfix.qml")));
        std::unique_ptr<QObject> made(component.create());
        auto *host = qobject_cast<QQuickItem *>(made.get());
        if (!host) std::printf("smoke: rbfix: %s\n", qPrintable(component.errorString()));
        int found = 0, wrong = 0;
        if (host) {
            host->setParentItem(d.mainWindow()->contentItem());
            host->setProperty("name", kMarkup);
            d.spin(30);
            QList<QQuickItem *> all;
            texts(host, all);
            for (const QQuickItem *t : std::as_const(all)) {
                if (t->property("text").toString() != kMarkup) continue;
                ++found;
                if (!plain(t)) ++wrong;
            }
            host->setParentItem(nullptr);
        }
        d.check(QStringLiteral("rbfix: %1 shows a name as it is written").arg(QLatin1String(c.what)), found > 0 && wrong == 0);
    }
    // The list of a combo box and a tooltip are popups of their own
    QQmlComponent component(d.app().engine);
    component.setData("import QtQuick\nimport UConsole\nItem { width: 200; height: 40; property string name\n"
                      "function pop() { box.popup.open(); tip.visible = true }\n"
                      "function drop() { box.popup.close(); tip.visible = false }\n"
                      "AppComboBox { id: box; model: [parent.name]; HelpTip { id: tip; text: parent.parent.name; delay: 0 } } }",
                      QUrl(QStringLiteral("rbfix.qml")));
    std::unique_ptr<QObject> made(component.create());
    auto *host = qobject_cast<QQuickItem *>(made.get());
    if (!host) std::printf("smoke: rbfix: %s\n", qPrintable(component.errorString()));
    Shown s;
    if (host) {
        host->setParentItem(d.mainWindow()->contentItem());
        host->setProperty("name", kMarkup);
        QMetaObject::invokeMethod(host, "pop");
        // The box's own text, its list's row and the tooltip's
        d.waitFor(3000, [&] { s = showing(kMarkup); return s.all >= 3; });
        QMetaObject::invokeMethod(host, "drop");
        d.spin(200);
        host->setParentItem(nullptr);
    }
    d.check("rbfix: a combo box's list and a tooltip too", s.all >= 3 && s.rich == 0);
}

void namesPart(SmokeDriver &d, LayoutController *layout) {
    sharedControls(d);
    DeviceModel *device = d.app().device;
    const int ch = device->state() ? device->state()->num_input_channels : 0;   // output 1
    device->write(QStringLiteral("rbfix name"), [ch](FfiCore *c) { return dspi_set_channel_name(c, uint8_t(ch), "<b>Rm</b>"); });
    d.waitIdle();
    Shown s;
    d.check("rbfix: a channel named with markup is listed in Classic",
            d.waitFor(5000, [&] { s = showing(kMarkup); return s.all > 0; }) && s.rich == 0);
    d.check("rbfix: Modern", switchTo(d, layout, "modern", "ModernShell"));
    auto *host = qobject_cast<QQuickItem *>(d.app().shell->pageHost());
    int all = 0, rich = 0, pages = 0;
    for (const QVariant &v : d.app().shell->panels()) {
        d.app().shell->trigger(v.toMap().value(QStringLiteral("action")).toString());
        d.waitIdle();
        d.spin(120);
        s = showing(kMarkup);
        all += s.all;
        rich += s.rich;
        ++pages;
    }
    if (host)
        for (const char *section : { "overview", "equaliser", "devices" }) {
            QVariant ok;
            QMetaObject::invokeMethod(host, "go", Q_RETURN_ARG(QVariant, ok), Q_ARG(QVariant, QString::fromLatin1(section)),
                                      Q_ARG(QVariant, QString()));
            d.waitIdle();
            d.spin(120);
            s = showing(kMarkup);
            all += s.all;
            rich += s.rich;
            ++pages;
        }
    std::printf("smoke: rbfix: %d pages, the name shown %d times\n", pages, all);
    d.check("rbfix: every page of Modern shows it as it is written", pages > 10 && all > pages && rich == 0);
    device->write(QStringLiteral("rbfix name"), [ch](FfiCore *c) { return dspi_set_channel_name(c, uint8_t(ch), ""); });
    d.waitIdle();
}

// ---- The layout while a measurement runs (Modern is on) ------------------------------------

void layoutPart(SmokeDriver &d, LayoutController *layout, MeasureController *mc, DeviceRegistry *r) {
    auto *notices = Registrar::instance<NoticeCenter>();
    const auto idle = [&](DeviceSession *s) {
        return d.waitFor(8000, [&] { return !s->model()->busy() && s->worker()->pending() == 0; });
    };
    DeviceSession *a = r->mainSession();
    r->activate(a);
    r->setKeepOpen(true);
    r->openDemo();
    d.check("rbfix: a second demo", d.waitFor(10000, [&] { return r->opening() == 0 && r->sessions().size() == 2; }));
    if (r->sessions().size() != 2) return;
    DeviceSession *b = r->sessions().at(1);
    r->activate(a);
    a->model()->write(QStringLiteral("rbfix volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
    idle(a);
    d.app().shell->trigger(QStringLiteral("tools.measure"));
    // An earlier section may have left another microphone chosen
    d.waitFor(5000, [&] { return mc->simAvailable(); });
    mc->setMicChoice(QStringLiteral("sim"));
    d.waitFor(5000, [&] { return mc->simAvailable() && mc->micChoice() == QLatin1String("sim") && !mc->setupOutputs().isEmpty(); });
    d.check("rbfix: Measure is a page, nothing runs, the layout may be switched",
            d.app().shell->pageAction() == QLatin1String("tools.measure") && layout->switchBlocker().isEmpty());
    bool one = false;
    for (const QVariant &v : mc->setupOutputs()) {
        const QVariantMap m = v.toMap();
        const bool on = m.value(QStringLiteral("on")).toBool() && !one;
        one = one || on;
        mc->setOutputOn(m.value(QStringLiteral("output")).toInt(), on);
    }
    mc->setPlacement(QStringLiteral("listeningPosition"));
    mc->setSweepMs(1000);
    mc->setPasses(3);
    QSignalSpy blocked(layout, &LayoutController::switchBlockerChanged);
    mc->start();
    d.check("rbfix: a measurement runs on the first", d.waitFor(8000, [&] { return mc->measuring(); }));
    if (!mc->measuring())
        std::printf("smoke: rbfix: no measurement: %s %s %s %s\n", qPrintable(mc->error()), qPrintable(mc->setupProblem()),
                    qPrintable(mc->playBlocker()), qPrintable(mc->issueText()));
    // Another device and another page are shown
    r->activate(b);
    d.app().shell->trigger(QStringLiteral("edit.settings"));
    Registrar::instance<SettingsController>()->open(QStringLiteral("appearance"));
    d.spin(200);
    d.check("rbfix: it keeps the layout, named with its device",
            !blocked.isEmpty() && layout->switchBlocker().contains(a->title()) && !layout->switchBlocker().contains(b->title()));
    QQuickItem *row = d.item(QStringLiteral("Appearance.layout"));
    d.check("rbfix: Settings > Appearance does not offer it", row && !row->isEnabled());
    const int before = notices->serial();
    layout->setLayout(QStringLiteral("classic"));   // the app menu's row
    d.spin(100);
    d.check("rbfix: the switch is refused, with why",
            layout->modern() && notices->serial() > before && notices->last().value(QStringLiteral("text")).toString() == layout->switchBlocker());
    r->activate(a);
    d.check("rbfix: the measurement goes on", d.waitFor(2000, [&] { return mc->measuring(); }));
    mc->stop();
    d.check("rbfix: stopped, the layout may be switched",
            d.waitFor(8000, [&] { return !mc->running() && layout->switchBlocker().isEmpty(); }) && d.waitFor(1000, [&] { return row && row->isEnabled(); }));
    mc->clearError();
    d.check("rbfix: and is", switchTo(d, layout, "classic", "ClassicShell"));
    r->setKeepOpen(true);   // leaving Modern turned it off
    QPointer<DeviceSession> gone(b);
    r->close(b->id());
    d.waitFor(10000, [&] { return gone.isNull(); });
    r->setKeepOpen(false);
    a->model()->write(QStringLiteral("rbfix volume"), [](FfiCore *c) { return dspi_set_user_volume(c, 0.0f); });
    idle(a);
    notices->clear();
}

// ---- Disconnect on a hub's unit beside the others -------------------------------------------

void disconnectPart(SmokeDriver &d, DeviceRegistry *r) {
    auto *list = Registrar::instance<DevicesController>();
    auto *link = Registrar::instance<LinkController>();
    auto *guard = Registrar::instance<UnsavedGuard>();
    auto *files = Registrar::instance<FilesController>();
    auto *firmware = Registrar::instance<FirmwareController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    linksmoke::FakeHubProcess hub;
    const bool up = list && link && guard && files && firmware && notices && hub.start({ "--auth", "none" });
    d.check("rbfix: the fake hub runs (DSPI_SMOKE_FAKEHUB)", up);
    if (!up) return;
    DeviceSession *a = r->mainSession();
    r->setKeepOpen(true);
    list->connectAddress(hub.address);
    d.check("rbfix: the hub's unit is listed", d.waitFor(30000, [&] {
        return !link->busy() && link->phase() == QLatin1String("choose_device") && link->devices().size() == 1;
    }));
    const QVariantMap unit = link->devices().value(0).toMap();
    list->chooseHubDevice(unit.value(QStringLiteral("handle")).toInt(), unit.value(QStringLiteral("serial")).toString());
    d.check("rbfix: it opens beside the demo",
            d.waitFor(20000, [&] { return r->opening() == 0 && r->sessions().size() == 2 && r->active() != a && link->phase() == QLatin1String("bound"); }));
    if (r->active() == a) {
        r->setKeepOpen(false);
        return;
    }
    QPointer<DeviceSession> u = r->active();
    const QString id = u->id();
    d.waitIdle();

    // Pinned: refused, with a notice
    firmware->pretendRunForTest(u);
    int before = notices->serial();
    link->closeSession();
    d.spin(300);
    d.check("rbfix: Disconnect is refused while a firmware update runs on it",
            r->session(id) == u && notices->serial() > before && link->phase() == QLatin1String("bound"));
    firmware->pretendRunForTest(nullptr);

    // Unsaved changes: asked
    bool done = false;
    const float gain = u->model()->state() ? u->model()->state()->output_gain_db[0] : 0.0f;
    d.app().device->write(QStringLiteral("rbfix gain"), [gain](FfiCore *c) { return dspi_set_output_gain(c, 0, gain - 2.0f); },
                          [&done](bool) { done = true; });
    d.waitFor(5000, [&] { return done; });
    d.waitIdle();
    QSignalSpy question(guard, &UnsavedGuard::question);
    QObject *unsaved = files->host() ? files->host()->findChild<QObject *>(QStringLiteral("FilesHost.unsavedQuestion")) : nullptr;
    const auto opened = [&] { return unsaved && unsaved->property("opened").toBool(); };
    const auto finish = [&](const char *answer) {
        if (unsaved) QMetaObject::invokeMethod(unsaved, "finish", Q_ARG(QVariant, QString::fromLatin1(answer)));
    };
    link->closeSession();
    d.check("rbfix: Disconnect asks about its unsaved changes", d.waitFor(5000, [&] { return question.size() == 1 && opened(); }) &&
                                                                   guard->deviceTitle() == u->title() && r->session(id) == u);
    finish("reject");
    d.spin(300);
    d.check("rbfix: Cancel keeps it open", r->session(id) == u && link->phase() == QLatin1String("bound"));
    link->closeSession();
    d.waitFor(5000, [&] { return question.size() == 2 && opened(); });
    finish("alternative");
    d.check("rbfix: Discard closes it", d.waitFor(10000, [&] { return u.isNull(); }) && r->sessions().size() == 1 && r->active() == a &&
                                           a->model()->connected());
    if (u) r->close(id);
    d.waitFor(10000, [&] { return u.isNull(); });
    r->setKeepOpen(false);
    notices->clear();
    hub.kill();
    d.waitIdle();
}

void rbfixSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    auto *layout = Registrar::instance<LayoutController>();
    auto *mc = Registrar::instance<MeasureController>();
    DeviceRegistry *r = DeviceRegistry::instance();
    d.check("rbfix: the controllers", layout && mc && r);
    if (!layout || !mc || !r) return;
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check("rbfix: on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    for (const QString &action : d.app().shell->shownWindows()) d.app().shell->closeWindow(action);
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (w->isVisible() && w != d.mainWindow() && w->inherits("QQuickWindow")) w->close();
    const QString settingsPage = Registrar::instance<SettingsController>()->currentPage();
    namesPart(d, layout);
    layoutPart(d, layout, mc, r);
    if (layout->modern()) switchTo(d, layout, "classic", "ClassicShell");
    disconnectPart(d, r);
    Registrar::instance<SettingsController>()->open(settingsPage);
}

const SmokeDriver::Section rbfix("rbfix", 970, &rbfixSection);

} // namespace
