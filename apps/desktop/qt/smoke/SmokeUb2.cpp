// -- WP UB2 -- (UXP B2, D4) One remote home for Simple mode and the locked user.
// On the demo, in Modern at 1600 x 1000:
// - "ub2-home": Advanced has no Home tab and the dock has User/Master and Bypass
//   EQ; Simple opens on Overview › Home (the remote: the device's name, one big
//   User volume that writes, Mute, the source as segments, the stored presets
//   as a Recall list), the dock without Master and Bypass EQ; Window › Mode…
//   is offered again. The installer's lock (PIN, covers, Lock Now) opens Home
//   in Advanced too: the source is words there, Save and undo are not in the top
//   bar, and another page (Equaliser › PEQ) is read-only: its controls off, its
//   scroll on, its tools off, one notice that says why. Dark and light pictures
//   of each. Unlocked and back in Advanced, all is as before.
// Mode and lock go through their controllers, so the section also builds on a
// tree without the change (the before pictures; its checks fail there).
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "LimitsController.h"
#include "ModeController.h"
#include "PresetModel.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>

#include <algorithm>
#include <cmath>

namespace {

const QString kPin = QStringLiteral("2468");

QQuickItem *shownItem(QQuickItem *from, const QString &name) {
    if (!from) return nullptr;
    if (from->objectName() == name && from->isVisible()) return from;
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = shownItem(child, name)) return found;
    return nullptr;
}

void grabBoth(SmokeDriver &d, QQuickWindow *w, const QString &name) {
    Shell *shell = d.app().shell;
    const Qt::ColorScheme before = shell->colorSchemeOverride();
    QDir(d.outDir()).mkpath(QStringLiteral("ub2"));
    for (const auto &[scheme, word] : { std::pair{ Qt::ColorScheme::Dark, "dark" }, std::pair{ Qt::ColorScheme::Light, "light" } }) {
        shell->setColorSchemeOverride(scheme);
        d.spin(250);
        d.grab(w, QStringLiteral("ub2/%1-%2.png").arg(name, QLatin1String(word)));
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
    QString where() const {
        QObject *h = d.app().shell->pageHost();
        return h ? h->property("section").toString() + QLatin1Char('/') + h->property("tab").toString() : QString();
    }
};

bool settled(SmokeDriver &d, LimitsController *lc) {
    d.waitIdle();
    const bool ok = d.waitFor(8000, [&] { return !lc->busy(); });
    d.waitIdle();
    d.spin(60);
    return ok;
}

void homeSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check(QStringLiteral("ub2: the demo is open"), d.waitFor(10000, [&] { return device->connected(); }) && d.waitIdle());
    auto *mode = Registrar::instance<ModeController>();
    auto *lc = Registrar::instance<LimitsController>();
    auto *sidebar = Registrar::instance<SidebarController>();
    mode->choose(QStringLiteral("advanced"), true);
    Modern m(d);
    d.check(QStringLiteral("ub2: Modern"), m.ok);
    if (!m.ok) return;
    QQuickItem *root = m.w->contentItem();
    d.goTo(QStringLiteral("overview/overview"), 200);

    // ---- Advanced: no Home, the dock as it was
    d.check(QStringLiteral("ub2: Advanced has no Home tab"), !d.goTo(QStringLiteral("overview/home")) || m.where() != QLatin1String("overview/home"));
    d.goTo(QStringLiteral("overview/overview"), 200);
    d.check(QStringLiteral("ub2: Advanced's dock has User/Master and Bypass EQ"),
            shownItem(root, QStringLiteral("Dock.mode")) && shownItem(root, QStringLiteral("Dock.bypass")));
    grabBoth(d, m.w, QStringLiteral("advanced-overview"));

    // ---- Simple: Home first, a remote
    mode->choose(QStringLiteral("simple"), true);
    d.spin(300);
    d.check(QStringLiteral("ub2: Simple opens on Home (%1)").arg(m.where()), m.where() == QLatin1String("overview/home"));
    QQuickItem *home = shownItem(root, QStringLiteral("HomeSection"));
    d.check(QStringLiteral("ub2: Home is shown"), home != nullptr);
    d.check(QStringLiteral("ub2: Home names the device"), home && shownItem(home, QStringLiteral("HomeSection.name"))
            && !shownItem(home, QStringLiteral("HomeSection.name"))->property("text").toString().isEmpty());
    QQuickItem *volume = home ? shownItem(home, QStringLiteral("HomeSection.volume")) : nullptr;
    d.check(QStringLiteral("ub2: one big User volume"), volume && volume->isEnabled());
    if (volume) {
        const double before = sidebar->userVolumeDb();
        QMetaObject::invokeMethod(volume, "committed", Q_ARG(qreal, sidebar->userDbToPosition(-20.0)));
        d.check(QStringLiteral("ub2: the volume writes the User volume (was %1)").arg(before),
                d.waitFor(3000, [&] { return std::abs(sidebar->userVolumeDb() + 20.0) < 0.6; }));
    }
    d.check(QStringLiteral("ub2: Mute on Home"), home && shownItem(home, QStringLiteral("HomeSection.mute")));
    d.check(QStringLiteral("ub2: the source as segments"), home && shownItem(home, QStringLiteral("HomeSection.source"))
            && !shownItem(home, QStringLiteral("HomeSection.sourceText")));
    QQuickItem *active = home ? shownItem(home, QStringLiteral("HomeSection.preset.%1").arg(device->presets()->property("activeSlot").toInt())) : nullptr;
    d.check(QStringLiteral("ub2: the preset that plays is chosen in the Recall list"), active && active->property("checked").toBool());
    d.check(QStringLiteral("ub2: Simple's dock: no Master, no Bypass EQ, Mute stays"),
            !shownItem(root, QStringLiteral("Dock.mode")) && !shownItem(root, QStringLiteral("Dock.bypass")) && shownItem(root, QStringLiteral("Dock.mute")));
    QObject *table = d.app().engine->singletonInstance<QObject *>("UConsole", "MenuTable");
    QVariant exported;
    if (table) QMetaObject::invokeMethod(table, "exported", Q_RETURN_ARG(QVariant, exported));
    const QJsonArray actions = QJsonDocument::fromJson(exported.toString().toUtf8()).object().value(QLatin1String("actions")).toArray();
    const bool modeRow = std::any_of(actions.begin(), actions.end(), [](const QJsonValue &a) {
        return a.toObject().value(QLatin1String("id")).toString() == QLatin1String("window.mode")
               && a.toObject().value(QLatin1String("places")).toArray().contains(QLatin1String("appMenu.settings"));
    });
    d.check(QStringLiteral("ub2: Mode… is in the app menu again"), modeRow);
    grabBoth(d, m.w, QStringLiteral("simple-home"));

    // ---- The installer's lock: Home in Advanced too, the rest read-only
    mode->choose(QStringLiteral("advanced"), true);
    d.goTo(QStringLiteral("overview/overview"), 200);
    lc->setText(QStringLiteral("note"), QStringLiteral("Bar: volume and presets only, please."));
    lc->setText(QStringLiteral("contact"), QStringLiteral("Installer service desk, ext. 200"));
    lc->setPin(kPin, kPin);
    settled(d, lc);
    lc->dismissRecovery();
    lc->setCovers(true);
    settled(d, lc);
    lc->lockNow();
    settled(d, lc);
    d.check(QStringLiteral("ub2: locked"), lc->locked());
    d.spin(300);
    d.check(QStringLiteral("ub2: the lock opens on Home (%1)").arg(m.where()), m.where() == QLatin1String("overview/home"));
    home = shownItem(root, QStringLiteral("HomeSection"));
    d.check(QStringLiteral("ub2: locked: the source in words, no segments"), home && shownItem(home, QStringLiteral("HomeSection.sourceText"))
            && !shownItem(home, QStringLiteral("HomeSection.source")));
    d.check(QStringLiteral("ub2: locked: no Save, no undo, no redo in the top bar"),
            !shownItem(root, QStringLiteral("TopBar.save")) && !shownItem(root, QStringLiteral("TopBar.undo")) && !shownItem(root, QStringLiteral("TopBar.redo")));
    d.check(QStringLiteral("ub2: locked: the dock has no Master, no Bypass EQ; the source is off"),
            !shownItem(root, QStringLiteral("Dock.mode")) && !shownItem(root, QStringLiteral("Dock.bypass"))
                && (!shownItem(root, QStringLiteral("Dock.source")) || !shownItem(root, QStringLiteral("Dock.source"))->isEnabled()));
    d.check(QStringLiteral("ub2: locked: Home's volume and presets are on"), home && shownItem(home, QStringLiteral("HomeSection.volume"))
            && shownItem(home, QStringLiteral("HomeSection.volume"))->isEnabled());
    d.check(QStringLiteral("ub2: Home says nothing about read-only"), !shownItem(root, QStringLiteral("ModernShell.lockRead")));
    grabBoth(d, m.w, QStringLiteral("locked-home"));

    d.goTo(QStringLiteral("equaliser/peq"), 300);
    QQuickItem *peq = d.item(QStringLiteral("EqualiserPeq"));
    QQuickItem *scroll = shownItem(root, QStringLiteral("PageHost.page.equaliser/peq.scroll"));
    d.check(QStringLiteral("ub2: locked: PEQ's controls are off"), peq && !peq->isEnabled());
    d.check(QStringLiteral("ub2: locked: PEQ still scrolls"), scroll && scroll->isEnabled());
    QQuickItem *tools = shownItem(root, QStringLiteral("PageTabs.tools"));
    d.check(QStringLiteral("ub2: locked: the page's tools are off"), !tools || !tools->isEnabled());
    QQuickItem *notice = shownItem(root, QStringLiteral("ModernShell.lockRead"));
    d.check(QStringLiteral("ub2: locked: one notice says the page is read-only"), notice != nullptr);
    grabBoth(d, m.w, QStringLiteral("locked-peq"));
    if (notice) {
        SmokeDriver::press(notice->findChild<QObject *>(QStringLiteral("ModernShell.lockRead.action")));
        d.spin(200);
        d.check(QStringLiteral("ub2: Go to Home goes there (%1)").arg(m.where()), m.where() == QLatin1String("overview/home"));
    }
    d.goTo(QStringLiteral("settings/application"), 200);
    d.check(QStringLiteral("ub2: locked: Settings stay as they are (its Installer page unlocks)"), !shownItem(root, QStringLiteral("ModernShell.lockRead")));

    // ---- Unlocked: as before
    lc->unlock(kPin);
    settled(d, lc);
    lc->clearPin();
    settled(d, lc);
    lc->clearLimits();
    settled(d, lc);
    d.check(QStringLiteral("ub2: unlocked"), !lc->locked());
    d.goTo(QStringLiteral("equaliser/peq"), 300);
    peq = d.item(QStringLiteral("EqualiserPeq"));
    d.check(QStringLiteral("ub2: unlocked: PEQ's controls are on again"), peq && peq->isEnabled());
    d.check(QStringLiteral("ub2: unlocked: Save and undo are back"), shownItem(root, QStringLiteral("TopBar.save")) && shownItem(root, QStringLiteral("TopBar.undo")));
    d.check(QStringLiteral("ub2: unlocked in Advanced: Bypass EQ is back"), shownItem(root, QStringLiteral("Dock.bypass")) != nullptr);
    d.goTo(QStringLiteral("overview/overview"), 200);
}

const SmokeDriver::Section home("ub2-home", 993, &homeSection);

} // namespace
