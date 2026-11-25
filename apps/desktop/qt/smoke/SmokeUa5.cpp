// -- WP UA5 -- (UXP A5, C6, C12) Settings opens where the work is; keyboard
// paths between the Modern shell's areas; the dock's words. On the demo, in
// Modern at 1600 x 1000:
// - "ua5-settings": with no page chosen Settings opens on Device › Speakers; a
//   page that goes falls back there and is not remembered; About while no
//   device was open gives way to Speakers when Settings opens again; a page
//   chosen stays; the Device pages in the order Speakers, Outputs, Inputs,
//   Global, I2S, Overview; the palette's "Settings › Device"; the protocol rows
//   and Raw Request… on Advanced, not on About.
// - "ua5-keys": the rail and the tab row are one Tab stop each; F6 and
//   Shift+F6 go through the areas; the arrows, Home and End move in the rail
//   and the tab row; Ctrl+PageDown/PageUp through the tabs; a click leaves no
//   focus ring; Keyboard Shortcuts (help.shortcuts, Ctrl+/) lists the menu
//   table's keys; the dock's User/Master help and its one DSP readout.
// Settings and the landing use invokeMethod by name, so the section also
// builds on a tree without the change (the before pictures; its checks fail).
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"

#include <QDir>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTest>

namespace {

const char *const kLastPage = "settingsWindow/lastPage";
// A page of no group's list, to open and take away
const SettingsRegistry::Page probePage{ "ua5probe", SettingsRegistry::Hidden, 99, "UA5 Probe", "info",
                                        "SmokeSettingsPage", {} };

QQuickItem *shownItem(QQuickItem *from, const QString &name) {
    if (!from) return nullptr;
    if (from->objectName() == name && from->isVisible()) return from;
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = shownItem(child, name)) return found;
    return nullptr;
}

// Whether `item` is `area` or inside it
bool inside(QQuickItem *item, QQuickItem *area) {
    for (QQuickItem *i = item; i; i = i->parentItem())
        if (i == area) return true;
    return false;
}

// The items under `from` whose name starts with `prefix` and that take the focus by Tab
int tabStops(QQuickItem *from, const QString &prefix) {
    if (!from || !from->isVisible()) return 0;
    int n = from->objectName().startsWith(prefix) && from->activeFocusOnTab() ? 1 : 0;
    for (QQuickItem *child : from->childItems()) n += tabStops(child, prefix);
    return n;
}

// The items under `from` whose name starts with `prefix` that show a focus ring
int ringed(QQuickItem *from, const QString &prefix) {
    if (!from || !from->isVisible()) return 0;
    int n = from->objectName().startsWith(prefix) && from->property("visualFocus").toBool() ? 1 : 0;
    for (QQuickItem *child : from->childItems()) n += ringed(child, prefix);
    return n;
}

QString call(QObject *o, const char *method) {
    QString out;
    QMetaObject::invokeMethod(o, method, Q_RETURN_ARG(QString, out));
    return out;
}

void grabBoth(SmokeDriver &d, QQuickWindow *w, const QString &name) {
    Shell *shell = d.app().shell;
    const Qt::ColorScheme before = shell->colorSchemeOverride();
    QDir(d.outDir()).mkpath(QStringLiteral("ua5"));
    for (const auto &[scheme, word] : { std::pair{ Qt::ColorScheme::Dark, "dark" }, std::pair{ Qt::ColorScheme::Light, "light" } }) {
        shell->setColorSchemeOverride(scheme);
        d.spin(250);
        d.grab(w, QStringLiteral("ua5/%1-%2.png").arg(name, QLatin1String(word)));
    }
    shell->setColorSchemeOverride(before);
    d.spin(100);
}

struct Modern {
    SmokeDriver &d;
    QQuickWindow *w;
    QSize original;
    bool ok;
    explicit Modern(SmokeDriver &driver) : d(driver), w(driver.mainWindow()), original(w ? w->size() : QSize()) {
        ok = w && d.switchLayout("modern");
        if (ok) {
            w->resize(QSize(1600, 1000));
            d.spin(300);
        }
    }
    ~Modern() {
        if (!ok) return;
        w->resize(original);
        d.switchLayout("classic");
    }
    QObject *host() const { return d.app().shell->pageHost(); }
    QString where() const {
        QObject *h = host();
        return h ? h->property("section").toString() + QLatin1Char('/') + h->property("tab").toString() : QString();
    }
};

void settingsSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    auto *s = Registrar::instance<SettingsController>();
    d.check("ua5: the controller", s != nullptr);
    if (!s) return;
    d.check("ua5: on the demo", d.waitFor(10000, [&] { return device->connected(); }) && d.waitIdle());
    Modern m(d);
    d.check("ua5: in Modern", m.ok);
    if (!m.ok) return;
    QQuickWindow *w = m.w;

    // ---- No page chosen yet: Device › Speakers --------------------------------------
    m.d.goTo(QStringLiteral("overview/overview"), 200);
    QSettings().remove(QLatin1String(kLastPage));
    d.check(QStringLiteral("ua5: with no page chosen the landing page is Speakers (%1)").arg(call(s, "landingPage")),
            call(s, "landingPage") == QLatin1String("speakers"));
    // A page the person opens is remembered; when it goes, Settings shows the
    // landing page and still remembers the page chosen
    s->open(QStringLiteral("ua5probe"));
    d.check("ua5: a page opened is remembered", QSettings().value(QLatin1String(kLastPage)).toString() == QLatin1String("ua5probe"));
    s->setPageAvailable(QStringLiteral("ua5probe"), false);
    d.spin(100);
    d.check(QStringLiteral("ua5: a page that goes falls back to Speakers (%1)").arg(s->currentPage()),
            s->currentPage() == QLatin1String("speakers"));
    d.check(QStringLiteral("ua5: the fallback is not remembered (%1)").arg(QSettings().value(QLatin1String(kLastPage)).toString()),
            QSettings().value(QLatin1String(kLastPage)).toString() == QLatin1String("ua5probe"));
    s->setPageAvailable(QStringLiteral("ua5probe"), true);
    QSettings().remove(QLatin1String(kLastPage));

    // The rail's Settings: the Device tab on Speakers
    QQuickItem *settingsButton = shownItem(w->contentItem(), QStringLiteral("NavRail.settings"));
    d.check("ua5: the rail's Settings", settingsButton != nullptr);
    if (settingsButton) SmokeDriver::press(settingsButton);
    d.waitIdle();
    d.spin(400);
    d.check(QStringLiteral("ua5: Settings opens on Device › Speakers (%1, %2)").arg(m.where(), s->currentPage()),
            m.where() == QLatin1String("settings/device") && s->currentPage() == QLatin1String("speakers"));
    grabBoth(d, w, QStringLiteral("settings-landing"));

    // ---- The Device pages in the order of the work ------------------------------------
    QStringList device_;
    for (const QVariant &row : s->sidebarRows(QString())) {
        const QVariantMap r = row.toMap();
        if (r.value(QStringLiteral("group")).toString() == QLatin1String("device")) device_ << r.value(QStringLiteral("id")).toString();
    }
    const QStringList want{ QStringLiteral("speakers"), QStringLiteral("outputs"), QStringLiteral("inputs"),
                            QStringLiteral("global"), QStringLiteral("i2s"), QStringLiteral("overview") };
    d.check(QStringLiteral("ua5: the Device pages are Speakers, Outputs, Inputs, Global, I2S, Overview (%1)").arg(device_.join(QLatin1Char(','))),
            device_.mid(0, want.size()) == want);
    QString breadcrumb;
    for (const QVariant &e : s->searchEntries())
        if (e.toMap().value(QStringLiteral("id")).toString() == QLatin1String("outputs"))
            breadcrumb = e.toMap().value(QStringLiteral("group")).toString();
    d.check(QStringLiteral("ua5: the palette says Settings › Device for a Device page (%1)").arg(breadcrumb),
            breadcrumb == QLatin1String("Device"));

    // ---- About is the app's; the protocol is on Advanced --------------------------------
    s->open(QStringLiteral("about"));
    d.waitIdle();
    d.spin(400);
    d.check("ua5: About has no protocol rows or Raw Request…",
            shownItem(w->contentItem(), QStringLiteral("AboutPage")) && !shownItem(w->contentItem(), QStringLiteral("About.raw"))
                && !shownItem(w->contentItem(), QStringLiteral("DeviceInfoRows")));
    grabBoth(d, w, QStringLiteral("settings-about"));
    s->open(QStringLiteral("advanced"));
    d.waitIdle();
    d.spin(400);
    d.check("ua5: Advanced has the protocol rows and Raw Request…",
            shownItem(w->contentItem(), QStringLiteral("Advanced.raw")) && shownItem(w->contentItem(), QStringLiteral("DeviceInfoRows")));
    grabBoth(d, w, QStringLiteral("settings-advanced"));

    // ---- A page the person chose stays; one the app chose gives way --------------------
    s->open(QStringLiteral("graphing"));
    QMetaObject::invokeMethod(s, "land");
    d.check(QStringLiteral("ua5: a page chosen stays when Settings opens (%1)").arg(s->currentPage()),
            s->currentPage() == QLatin1String("graphing"));
    s->open(QStringLiteral("speakers"));
    QSettings().remove(QLatin1String(kLastPage));
    m.d.goTo(QStringLiteral("overview/overview"), 200);
    // No device: Speakers goes, the app shows About
    d.app().core->worker()->post([](FfiCore *c) { dspi_disconnect(c); });
    device->refresh();
    d.check("ua5: the device closes", d.waitFor(3000, [&] { return !device->connected(); }));
    d.spin(200);
    d.check(QStringLiteral("ua5: without a device the app shows About (%1)").arg(s->currentPage()), s->currentPage() == QLatin1String("about"));
    device->openDemo();
    d.check("ua5: the demo opens again", d.waitFor(8000, [&] { return device->connected(); }));
    d.waitIdle();
    d.spin(300);
    if (settingsButton) SmokeDriver::press(settingsButton);
    d.waitIdle();
    d.spin(400);
    d.check(QStringLiteral("ua5: with the device back, Settings opens on Speakers again (%1, %2)").arg(m.where(), s->currentPage()),
            s->currentPage() == QLatin1String("speakers") && m.where() == QLatin1String("settings/device"));
    m.d.goTo(QStringLiteral("overview/overview"), 200);
}

void keysSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    d.check("ua5-keys: on the demo", d.waitFor(10000, [&] { return device->connected(); }) && d.waitIdle());
    Modern m(d);
    d.check("ua5-keys: in Modern", m.ok);
    if (!m.ok) return;
    QQuickWindow *w = m.w;
    w->requestActivate();
    d.spin(100);
    QQuickItem *rail = shownItem(w->contentItem(), QStringLiteral("NavRail"));
    QQuickItem *tabs = shownItem(w->contentItem(), QStringLiteral("PageTabs"));
    QQuickItem *top = shownItem(w->contentItem(), QStringLiteral("TopBar"));
    QQuickItem *dock = shownItem(w->contentItem(), QStringLiteral("Dock"));
    d.check("ua5-keys: the rail, the tab row, the top bar and the dock", rail && tabs && top && dock);
    if (!rail || !tabs || !top || !dock) return;

    d.goTo(QStringLiteral("equaliser/peq"), 300);
    d.check(QStringLiteral("ua5-keys: the rail is one Tab stop (%1)").arg(tabStops(rail, QStringLiteral("NavRail."))),
            tabStops(rail, QStringLiteral("NavRail.")) == 1);
    d.check(QStringLiteral("ua5-keys: the tab row is one Tab stop (%1)").arg(tabStops(tabs, QStringLiteral("PageTabs."))),
            tabStops(tabs, QStringLiteral("PageTabs.")) == 1);

    // ---- F6 through the areas ----------------------------------------------------------
    w->contentItem()->forceActiveFocus();
    d.spin(50);
    QTest::keyClick(w, Qt::Key_F6);
    d.spin(100);
    d.check(QStringLiteral("ua5-keys: F6 goes to the top bar (%1)").arg(w->activeFocusItem() ? w->activeFocusItem()->objectName() : QString()),
            inside(w->activeFocusItem(), top));
    QTest::keyClick(w, Qt::Key_F6);
    d.spin(100);
    QQuickItem *peqButton = shownItem(rail, QStringLiteral("NavRail.equaliser"));
    d.check(QStringLiteral("ua5-keys: F6 again: the section on show in the rail (%1)").arg(w->activeFocusItem() ? w->activeFocusItem()->objectName() : QString()),
            peqButton && w->activeFocusItem() == peqButton && peqButton->property("visualFocus").toBool());
    grabBoth(d, w, QStringLiteral("keys-rail-focus"));

    // ---- Arrows in the rail -----------------------------------------------------------------
    QTest::keyClick(w, Qt::Key_Down);
    d.waitIdle();
    d.spin(200);
    d.check(QStringLiteral("ua5-keys: Down shows the next section (%1)").arg(m.where()), m.where().startsWith(QLatin1String("routing/")));
    d.check("ua5-keys: and its button has the focus",
            w->activeFocusItem() && w->activeFocusItem()->objectName() == QLatin1String("NavRail.routing"));
    QTest::keyClick(w, Qt::Key_Right);
    d.waitIdle();
    d.spin(200);
    d.check(QStringLiteral("ua5-keys: Right shows the one after (%1)").arg(m.where()), m.where().startsWith(QLatin1String("effects/")));
    QTest::keyClick(w, Qt::Key_Up);
    QTest::keyClick(w, Qt::Key_Left);
    d.waitIdle();
    d.spin(200);
    d.check(QStringLiteral("ua5-keys: Up and Left go back (%1)").arg(m.where()), m.where().startsWith(QLatin1String("equaliser/")));
    QTest::keyClick(w, Qt::Key_End);
    d.waitIdle();
    d.spin(200);
    d.check(QStringLiteral("ua5-keys: End shows the last section (%1)").arg(m.where()), m.where().startsWith(QLatin1String("settings/")));
    QTest::keyClick(w, Qt::Key_Home);
    d.waitIdle();
    d.spin(200);
    d.check(QStringLiteral("ua5-keys: Home shows the first (%1)").arg(m.where()), m.where().startsWith(QLatin1String("overview/")));
    d.goTo(QStringLiteral("equaliser/peq"), 300);
    QTest::keyClick(w, Qt::Key_Tab);   // the focus was on the rail's Overview; from the rail to the tab row
    d.spin(100);

    // ---- The tab row -------------------------------------------------------------------------
    peqButton->forceActiveFocus(Qt::TabFocusReason);
    QTest::keyClick(w, Qt::Key_F6);
    d.spin(100);
    QQuickItem *peqTab = shownItem(tabs, QStringLiteral("PageTabs.peq"));
    d.check(QStringLiteral("ua5-keys: F6 from the rail: the tab on show (%1)").arg(w->activeFocusItem() ? w->activeFocusItem()->objectName() : QString()),
            peqTab && w->activeFocusItem() == peqTab);
    QTest::keyClick(w, Qt::Key_Right);
    d.waitIdle();
    d.spin(200);
    const QString afterRight = m.where();
    d.check(QStringLiteral("ua5-keys: Right in the tab row shows the next tab (%1)").arg(afterRight),
            afterRight.startsWith(QLatin1String("equaliser/")) && afterRight != QLatin1String("equaliser/peq"));
    QTest::keyClick(w, Qt::Key_Left);
    d.waitIdle();
    d.spin(200);
    d.check(QStringLiteral("ua5-keys: Left goes back (%1)").arg(m.where()), m.where() == QLatin1String("equaliser/peq"));
    w->contentItem()->forceActiveFocus();
    QTest::keyClick(w, Qt::Key_PageDown, Qt::ControlModifier);
    d.waitIdle();
    d.spin(200);
    d.check(QStringLiteral("ua5-keys: Ctrl+PageDown shows the next tab from anywhere (%1)").arg(m.where()), m.where() == afterRight);
    QTest::keyClick(w, Qt::Key_PageUp, Qt::ControlModifier);
    d.waitIdle();
    d.spin(200);
    d.check(QStringLiteral("ua5-keys: Ctrl+PageUp the one before (%1)").arg(m.where()), m.where() == QLatin1String("equaliser/peq"));

    // ---- Shift+F6 back; F6 on to the dock ---------------------------------------------------
    peqTab = shownItem(tabs, QStringLiteral("PageTabs.peq"));
    if (peqTab) peqTab->forceActiveFocus(Qt::TabFocusReason);
    QTest::keyClick(w, Qt::Key_F6, Qt::ShiftModifier);
    d.spin(100);
    d.check("ua5-keys: Shift+F6 from the page: the rail", w->activeFocusItem() == shownItem(rail, QStringLiteral("NavRail.equaliser")));
    bool docked = false;
    for (int i = 0; i < 4 && !docked; ++i) {
        QTest::keyClick(w, Qt::Key_F6);
        d.spin(100);
        docked = inside(w->activeFocusItem(), dock);
    }
    d.check("ua5-keys: F6 reaches the dock", docked);

    // ---- A click leaves no ring in the rail -------------------------------------------------
    QQuickItem *equaliser = shownItem(rail, QStringLiteral("NavRail.equaliser"));
    QQuickItem *routing = shownItem(rail, QStringLiteral("NavRail.routing"));
    if (equaliser) equaliser->forceActiveFocus(Qt::TabFocusReason);
    d.spin(50);
    if (routing) {
        const QPointF at = routing->mapToScene(QPointF(routing->width() / 2, routing->height() / 2));
        QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, at.toPoint());
    }
    d.waitIdle();
    d.spin(200);
    d.check(QStringLiteral("ua5-keys: a click in the rail leaves no focus ring (%1 ringed, at %2)").arg(ringed(rail, QStringLiteral("NavRail."))).arg(m.where()),
            m.where().startsWith(QLatin1String("routing/")) && ringed(rail, QStringLiteral("NavRail.")) == 0);
    grabBoth(d, w, QStringLiteral("keys-after-click"));

    // ---- Keyboard Shortcuts -------------------------------------------------------------------
    d.goTo(QStringLiteral("equaliser/peq"), 200);
    d.app().shell->trigger(QStringLiteral("help.shortcuts"));
    d.spin(400);
    QObject *dialog = d.object(QStringLiteral("ShortcutsDialog"));
    d.check("ua5-keys: help.shortcuts opens Keyboard Shortcuts", dialog && dialog->property("opened").toBool());
    QObject *save = d.object(QStringLiteral("ShortcutsDialog.key.device.commit"));
    d.check(QStringLiteral("ua5-keys: it lists Save's Ctrl+S (%1)").arg(save ? save->property("text").toString() : QString()),
            save && save->property("text").toString() == QLatin1String("Ctrl+S"));
    QObject *presets = d.object(QStringLiteral("ShortcutsDialog.key.device.preset1"));
    d.check(QStringLiteral("ua5-keys: the presets are one line (%1)").arg(presets ? presets->property("text").toString() : QString()),
            presets && presets->property("text").toString() == QLatin1String("Ctrl+1 … Ctrl+9"));
    d.check("ua5-keys: and the keys for moving around", d.object(QStringLiteral("ShortcutsDialog.row.area")) != nullptr);
    grabBoth(d, w, QStringLiteral("keys-shortcuts"));
    QTest::keyClick(w, Qt::Key_Escape);
    d.check("ua5-keys: Escape closes it", dialog && d.waitFor(2000, [&] { return !dialog->property("visible").toBool(); }));
    QTest::keyClick(w, Qt::Key_Slash, Qt::ControlModifier);
    d.check("ua5-keys: Ctrl+/ opens it", dialog && d.waitFor(2000, [&] { return dialog->property("opened").toBool(); }));
    if (dialog) QMetaObject::invokeMethod(dialog, "close");
    d.spin(200);

    // ---- The dock's words -------------------------------------------------------------------
    QQuickItem *mode = shownItem(dock, QStringLiteral("Dock.mode"));
    const QString help = mode ? mode->property("help").toString() : QString();
    d.check(QStringLiteral("ua5-keys: the dock says what User and Master are (%1)").arg(help),
            help.contains(QStringLiteral("User: the everyday volume")) && help.contains(QStringLiteral("Master: the installer's ceiling")));
    QQuickItem *dsp = shownItem(dock, QStringLiteral("Dock.dsp"));
    d.check(QStringLiteral("ua5-keys: one DSP load readout (%1)").arg(dsp ? dsp->property("text").toString() : QString()),
            dsp && dsp->property("text").toString().startsWith(QLatin1String("DSP ")) && !shownItem(dock, QStringLiteral("Dock.cpu0"))
                && !shownItem(dock, QStringLiteral("Dock.cpu1")));
    if (mode) {
        const QPointF at = mode->mapToScene(QPointF(mode->width() / 2, mode->height() / 2));
        QTest::mouseMove(w, at.toPoint());
        d.spin(1000);
    }
    grabBoth(d, w, QStringLiteral("dock-mode-tip"));
    if (dsp) {
        const QPointF at = dsp->mapToScene(QPointF(dsp->width() / 2, dsp->height() / 2));
        QTest::mouseMove(w, at.toPoint());
        d.spin(1000);
    }
    grabBoth(d, w, QStringLiteral("dock-dsp-tip"));
    QTest::mouseMove(w, QPoint(800, 500));
    d.goTo(QStringLiteral("overview/overview"), 200);
}

const SmokeDriver::Section settings("ua5-settings", 991, &settingsSection);
const SmokeDriver::Section keys("ua5-keys", 992, &keysSection);

} // namespace
