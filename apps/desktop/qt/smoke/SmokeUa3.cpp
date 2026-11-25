// -- WP UA3 -- Save state per device (UXP A3, C1, C2), in Modern with three
// demos open: a tab carries a dot and "N unsaved" while its device has unsaved
// changes; Save ▾ offers Save All with one question and no Factory Reset;
// Save to Device's question names the device, the slot and the preset and lists
// the changes; the count explains itself; a click on the active tab does
// nothing and its ▾ opens the device menu; a tab's tooltip only shows what is
// cut off; A/B shows only while a compare runs, saying what A and B are; a
// Keep that leaves the preset unsaved says so and offers Save to Device;
// Factory Reset is on Devices › This Computer; with no device, Save to
// Device is greyed and Ctrl+S does nothing.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "FilesController.h"
#include "LayoutController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"
#include "SidebarController.h"
#include "SnapshotsController.h"
#include "UnsavedGuard.h"

extern "C" {
#include "dspi_core.h"
}

#include <QCoreApplication>
#include <QPointer>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QSignalSpy>
#include <QTest>

namespace {

struct Ua3 {
    SmokeDriver &d;
    QQuickWindow *main;
    DeviceRegistry *r;
    // The tabs' place: the top bar's with one device, the row of several (ModernShell)
    QString tabs = QStringLiteral("DeviceTabs");

    void check(const char *what, bool ok) const { d.check(QStringLiteral("ua3: ") + QString::fromUtf8(what), ok); }
    void settle() const {
        d.spin(40);
        d.waitIdle();
        d.spin(120);
    }
    QQuickItem *shown(const QString &name, QQuickItem *from = nullptr) const {
        QQuickItem *at = from ? from : main->contentItem();
        if (!at->isVisible()) return nullptr;
        if (at->objectName() == name) return at;
        for (QQuickItem *c : at->childItems())
            if (QQuickItem *f = shown(name, c)) return f;
        return nullptr;
    }
    QPoint point(QQuickItem *i, qreal fx = 0.5, qreal fy = 0.5) const {
        return i->mapToScene(QPointF(i->width() * fx, i->height() * fy)).toPoint();
    }
    bool click(QQuickItem *i, qreal fx = 0.5) const {
        if (!i || i->width() <= 0) return false;
        QTest::mouseClick(main, Qt::LeftButton, Qt::NoModifier, point(i, fx));
        settle();
        return true;
    }
    bool click(const QString &name, qreal fx = 0.5) const { return click(shown(name), fx); }
    static bool opened(QObject *popup) { return popup && popup->property("opened").toBool(); }
    bool waitOpen(QObject *popup) const { return popup && d.waitFor(2000, [&] { return opened(popup); }); }
    void finish(QObject *popup, const char *answer) const {
        if (popup) QMetaObject::invokeMethod(popup, "finish", Q_ARG(QVariant, QString::fromLatin1(answer)));
        d.waitFor(1500, [&] { return !popup || !popup->property("visible").toBool(); });
        settle();
    }
    void close(QObject *popup) const {
        if (popup) QMetaObject::invokeMethod(popup, "close");
        d.waitFor(1500, [&] { return !popup || !popup->property("visible").toBool(); });
        settle();
    }
    QVariantMap row(DeviceSession *s) const {
        for (const QVariant &v : r->devices())
            if (v.toMap().value(QStringLiteral("id")).toString() == s->id()) return v.toMap();
        return {};
    }
    bool dirty(DeviceSession *s) const { return row(s).value(QStringLiteral("dirty")).toBool(); }
    // Output 1's gain on `s`, made the active device: a preset change
    bool setGain(DeviceSession *s, float db) const {
        r->activate(s);
        bool done = false;
        d.app().device->write(QStringLiteral("ua3 gain"), [db](FfiCore *c) { return dspi_set_output_gain(c, 0, db); },
                              [&done](bool ok) { done = ok; });
        return d.waitFor(5000, [&] { return done; }) && d.waitIdle();
    }
    float gainOf(DeviceSession *s) const {
        const DspState *st = s->model()->state();
        return st ? st->output_gain_db[0] : 0.0f;
    }
};

// The window in the dark and the light scheme: <name>-dark.png, <name>-light.png
void shots(SmokeDriver &d, QQuickWindow *main, const char *name) {
    auto *layout = Registrar::instance<LayoutController>();
    const QString was = layout->theme();
    for (const char *scheme : { "dark", "light" }) {
        layout->setTheme(QString::fromLatin1(scheme));
        d.spin(250);
        d.grab(main, QStringLiteral("ua3-%1-%2.png").arg(QLatin1String(name), QLatin1String(scheme)));
    }
    layout->setTheme(was);
    d.spin(150);
}

bool switchTo(SmokeDriver &d, LayoutController *layout, const QString &to, const char *shellName) {
    layout->setLayout(to);
    QObject *loader = d.object(QStringLiteral("MainWindow.shell"));
    const bool ok = d.waitFor(3000, [&] {
        QObject *now = loader ? loader->property("item").value<QObject *>() : nullptr;
        return now && now->objectName() == QLatin1String(shellName);
    });
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    d.waitIdle();
    d.spin(200);
    return ok;
}

QString idNamed(const SnapshotsController *snaps, const QString &name) {
    for (const QVariant &v : snaps->snapshots())
        if (v.toMap().value(QStringLiteral("name")).toString() == name) return v.toMap().value(QStringLiteral("id")).toString();
    return {};
}

void saveStateSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    auto *layout = Registrar::instance<LayoutController>();
    auto *files = Registrar::instance<FilesController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    auto *snaps = Registrar::instance<SnapshotsController>();
    auto *sidebar = Registrar::instance<SidebarController>();
    Ua3 t{ d, d.mainWindow(), DeviceRegistry::instance() };
    t.check("the controllers", layout && files && notices && snaps && sidebar && t.main && t.r);
    if (!layout || !files || !notices || !snaps || !sidebar || !t.main || !t.r) return;
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    t.check("on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    const QString wasLayout = layout->layout();
    const QSize wasSize = t.main->size();
    t.main->resize(1440, 900);
    t.check("Modern", switchTo(d, layout, QStringLiteral("modern"), "ModernShell"));
    DeviceRegistry *r = t.r;
    DeviceSession *a = r->active();
    r->openDemo();
    d.waitFor(10000, [&] { return r->opening() == 0 && r->sessions().size() == 2; });
    r->openDemo();
    t.check("three demos open", d.waitFor(10000, [&] { return r->opening() == 0 && r->sessions().size() == 3; }));
    if (r->sessions().size() != 3) return;
    DeviceSession *b = r->sessions().at(1);
    DeviceSession *c = r->sessions().at(2);
    t.settle();
    if (t.shown(QStringLiteral("DeviceRow"))) t.tabs = QStringLiteral("DeviceRow");
    QObject *deviceQuestion = files->host() ? files->host()->findChild<QObject *>(QStringLiteral("FilesHost.deviceQuestion")) : nullptr;
    QObject *filesNotice = files->host() ? files->host()->findChild<QObject *>(QStringLiteral("FilesHost.notice")) : nullptr;

    // ---- Edit two: two dots, "N unsaved" under them; the third stays clean
    const float gainA = t.gainOf(a), gainB = t.gainOf(b);
    t.check("a change on the first and the second", t.setGain(a, gainA - 2.0f) && t.setGain(b, gainB - 2.0f));
    t.check("the registry knows both, the third clean",
            d.waitFor(3000, [&] { return t.dirty(a) && t.dirty(b) && !t.dirty(c); }) && t.row(a).value(QStringLiteral("unsaved")).toInt() > 0);
    t.settle();
    const auto dot = [&](DeviceSession *s) { return t.shown(t.tabs + QStringLiteral(".dirty.") + s->id()) != nullptr; };
    t.check("two tabs carry the unsaved dot", dot(a) && dot(b) && !dot(c));
    QQuickItem *tabB = t.shown(t.tabs + QStringLiteral(".tab.") + b->id());
    const int unsavedB = t.row(b).value(QStringLiteral("unsaved")).toInt();
    t.check("and say how many under the name", tabB && tabB->property("detail").toString().startsWith(QStringLiteral("%1 unsaved").arg(unsavedB)));
    shots(d, t.main, "tabs-unsaved");

    // ---- The count explains itself
    QQuickItem *save = t.shown(QStringLiteral("TopBar.save"));
    const QStringList lines = sidebar->unsavedChanges();
    const QString tip = save ? save->property("countToolTip").toString() : QString();
    t.check("Save to Device's count has the unsaved list as its tooltip",
            !lines.isEmpty() && tip.contains(QStringLiteral("unsaved change")) && tip.contains(QStringLiteral("• ") + lines.first()));
    if (QQuickItem *button = t.shown(QStringLiteral("TopBar.save.button"))) {
        QTest::mouseMove(t.main, t.point(button));
        d.spin(900);
        shots(d, t.main, "save-count");
        QTest::mouseMove(t.main, QPoint(t.main->width() / 2, t.main->height() / 2));
        t.settle();
    }

    // ---- Save ▾: Save All, no Factory Reset
    QObject *menu = d.object(QStringLiteral("TopBar.save.menu"));
    t.check("Save ▾ opens", t.click(QStringLiteral("TopBar.save.more")) && t.waitOpen(menu));
    QQuickItem *saveAll = t.shown(QStringLiteral("TopBar.save.menu.saveAll"));
    if (!saveAll && menu) saveAll = menu->findChild<QQuickItem *>(QStringLiteral("TopBar.save.menu.saveAll"));
    t.check("with Save All (2 devices)…, enabled", saveAll && saveAll->property("text").toString() == QStringLiteral("Save All (2 devices)…")
                                                       && saveAll->isEnabled());
    QObject *reset = menu ? menu->findChild<QObject *>(QStringLiteral("TopBar.save.menu.device.factoryReset")) : nullptr;
    t.check("and without Factory Reset", menu && !reset && menu->findChild<QObject *>(QStringLiteral("TopBar.save.menu.device.revert")));
    shots(d, t.main, "save-menu");
    QObject *allQuestion = d.object(QStringLiteral("TopBar.saveAllQuestion"));
    if (saveAll) QTest::mouseClick(t.main, Qt::LeftButton, Qt::NoModifier, t.point(saveAll));
    t.check("Save All asks once", t.waitOpen(allQuestion));
    const QString allText = allQuestion ? allQuestion->property("text").toString() : QString();
    t.check("naming both devices and their presets, not the clean one",
            allText.startsWith(QStringLiteral("Save the unsaved changes of 2 devices?")) && allText.count(QStringLiteral("• ")) == 2
                && allText.count(QStringLiteral(", preset ")) == 2);
    shots(d, t.main, "save-all");
    const int noticesBefore = notices->serial();
    t.finish(allQuestion, "accept");
    t.check("and saves both", d.waitFor(6000, [&] { return !t.dirty(a) && !t.dirty(b); }) && !Registrar::instance<UnsavedGuard>()->asking());
    t.check("said so", d.waitFor(3000, [&] {
        return notices->serial() > noticesBefore && notices->last().value(QStringLiteral("text")).toString() == QStringLiteral("Saved on 2 devices.");
    }));
    t.settle();
    t.check("the dots are gone", !dot(a) && !dot(b));

    // ---- Save to Device names the device, the slot and the preset, and lists the changes
    t.check("another change on the first", t.setGain(a, gainA - 4.0f) && d.waitFor(3000, [&] { return t.dirty(a) && sidebar->unsaved(); }));
    shell->trigger(QStringLiteral("device.commit"));
    t.check("Save to Device asks", t.waitOpen(deviceQuestion));
    const QString commitText = deviceQuestion ? deviceQuestion->property("text").toString() : QString();
    const QRegularExpression head(QStringLiteral("^Save (1 change|\\d+ changes) to %1, preset \\d+( “[^”]*”)?\\?\\n\\n• ")
                                      .arg(QRegularExpression::escape(a->title())));
    t.check("in the device's, the slot's and the preset's words, with the list",
            deviceQuestion && deviceQuestion->property("title").toString() == QStringLiteral("Save to Device") && head.match(commitText).hasMatch()
                && commitText.contains(sidebar->unsavedChanges().value(0)));
    shots(d, t.main, "save-question");
    t.finish(deviceQuestion, "reject");
    t.check("a Cancel saves nothing", t.dirty(a));

    // ---- The active tab: a click does nothing, its ▾ opens the device menu
    QObject *switcher = d.object(QStringLiteral("DeviceSwitcher"));
    QQuickItem *tabA = t.shown(t.tabs + QStringLiteral(".tab.") + a->id());
    t.click(tabA, 0.3);
    d.spin(300);
    t.check("a click on the active tab opens nothing and keeps it active", tabA && r->active() == a && !Ua3::opened(switcher));
    QObject *tabMenu = tabA ? tabA->findChild<QObject *>(t.tabs + QStringLiteral(".menu.") + a->id()) : nullptr;
    t.check("its ▾ opens the device menu", t.click(t.tabs + QStringLiteral(".more.") + a->id()) && t.waitOpen(tabMenu));
    t.close(tabMenu);
    t.check("only the active tab has a ▾", !t.shown(t.tabs + QStringLiteral(".more.") + b->id()));
    t.click(t.tabs + QStringLiteral(".tab.") + b->id(), 0.3);
    t.check("a click on another tab still makes it active", d.waitFor(3000, [&] { return r->active() == b; }));
    r->activate(a);
    t.settle();

    // ---- A tab's tooltip only when something is cut off
    QQuickItem *tabC = t.shown(t.tabs + QStringLiteral(".tab.") + c->id());
    QObject *tipC = nullptr;
    if (tabC)
        for (QObject *o : tabC->findChildren<QObject *>(Qt::FindDirectChildrenOnly))   // its own, not a button's
            if (o->inherits("QQuickToolTip")) tipC = o;
    t.check("the tab's tooltip is there", tipC != nullptr);
    if (Ua3::opened(switcher)) t.close(switcher);
    if (tabC) QTest::mouseMove(t.main, t.point(tabC, 0.3));
    d.spin(900);
    t.check("a tab that shows all of it has no tooltip", tabC && tipC && tabC->property("hovered").toBool()
                                                             && !tabC->property("elided").toBool() && !tipC->property("visible").toBool());
    QTest::mouseMove(t.main, QPoint(t.main->width() / 2, t.main->height() / 2));
    t.settle();

    // ---- A/B: hidden with no compare, then says what A and B are
    t.check("A/B is not shown with no compare", !snaps->abActive() && !t.shown(QStringLiteral("TopBar.ab")));
    QSignalSpy stored(snaps, &SnapshotsController::stored);
    const auto capture = [&](const QString &name) {
        stored.clear();
        snaps->capture(name, QString());
        return d.waitFor(5000, [&] { return !stored.isEmpty() && !idNamed(snaps, name).isEmpty(); });
    };
    t.check("two snapshots", capture(QStringLiteral("UA3 A")) && t.setGain(a, gainA - 6.0f) && capture(QStringLiteral("UA3 B")));
    const QString idA = idNamed(snaps, QStringLiteral("UA3 A")), idB = idNamed(snaps, QStringLiteral("UA3 B"));
    snaps->abStart(idA, idB);
    t.check("a compare runs", d.waitFor(5000, [&] { return snaps->abActive(); }));
    t.settle();
    QQuickItem *ab = t.shown(QStringLiteral("TopBar.ab"));
    t.check("A/B shows while it runs, its tooltip naming A and B",
            ab && ab->property("toolTip").toString().contains(QStringLiteral("“UA3 A”")) && ab->property("toolTip").toString().contains(QStringLiteral("“UA3 B”")));
    if (ab) {
        QTest::mouseMove(t.main, t.point(ab, 0.25));
        d.spin(900);
        shots(d, t.main, "ab");
        QTest::mouseMove(t.main, QPoint(t.main->width() / 2, t.main->height() / 2));
    }
    snaps->abKeep(QStringLiteral("b"));
    t.check("the compare ends, A/B goes", d.waitFor(5000, [&] { return !snaps->abActive(); }) && t.d.waitIdle() && !t.shown(QStringLiteral("TopBar.ab")));

    // ---- A Keep that leaves the preset unsaved says so, and offers Save to Device
    snaps->setRollback(true);
    snaps->recall(idA);
    t.check("a recall with rollback waits", d.waitFor(8000, [&] { return snaps->guardWaiting(); }));
    snaps->keep();
    t.check("Kept: the notice says it is not saved, with Save to Device", d.waitFor(5000, [&] {
        const QVariantMap n = notices->last();
        return n.value(QStringLiteral("tag")).toString() == QStringLiteral("ua3.kept")
               && n.value(QStringLiteral("text")).toString().contains(QStringLiteral("not saved on the device yet"))
               && n.value(QStringLiteral("action")).toString() == QStringLiteral("Save to Device…");
    }));
    // The notices before it go first (a notice shows for a few seconds each)
    d.waitFor(15000, [&] {
        if (notices->tag() == QStringLiteral("ua3.kept")) return true;
        notices->dismiss();
        return false;
    });
    shots(d, t.main, "kept");
    notices->act();
    t.check("its button asks Save to Device's question", t.waitOpen(deviceQuestion)
                                                            && deviceQuestion->property("title").toString() == QStringLiteral("Save to Device"));
    t.finish(deviceQuestion, "reject");
    snaps->setRollback(false);
    for (const QString &id : { idA, idB }) {
        stored.clear();
        snaps->remove(id);
        d.waitFor(3000, [&] { return !stored.isEmpty(); });
    }

    // ---- Factory Reset is on Devices › This Computer
    QObject *host = shell->pageHost();
    if (host) QMetaObject::invokeMethod(host, "go", Q_ARG(QVariant, QStringLiteral("devices")), Q_ARG(QVariant, QStringLiteral("devices")));
    t.settle();
    t.check("Factory Reset is on Devices", t.click(QStringLiteral("DevicesSection.device.factoryReset")) && t.waitOpen(deviceQuestion)
                                               && deviceQuestion->property("title").toString() == QStringLiteral("Factory Reset"));
    shots(d, t.main, "devices");
    t.finish(deviceQuestion, "reject");

    // ---- One device: no Save All
    QPointer<DeviceSession> goneB(b), goneC(c);
    r->close(b->id());
    r->close(c->id());
    d.waitFor(10000, [&] { return goneB.isNull() && goneC.isNull(); });
    t.settle();
    t.check("Save ▾ opens with one device", t.click(QStringLiteral("TopBar.save.more")) && t.waitOpen(menu));
    QObject *lone = menu ? menu->findChild<QObject *>(QStringLiteral("TopBar.save.menu.saveAll")) : nullptr;
    t.check("without Save All", menu && (!lone || !qobject_cast<QQuickItem *>(lone)->isVisible()));
    t.close(menu);

    // ---- No device: Save is greyed, Ctrl+S does nothing
    r->close(r->active()->id());   // the main session's: it stays, with no device
    t.check("no device", d.waitFor(5000, [&] { return !device->connected(); }));
    t.settle();
    QQuickItem *button = t.shown(QStringLiteral("TopBar.save.button"));
    t.check("Save to Device is greyed", button && !button->isEnabled());
    const int noticesNow = notices->serial();
    shell->trigger(QStringLiteral("device.commit"));
    d.spin(500);
    t.check("Ctrl+S does nothing: no question, no alert, no notice",
            !Ua3::opened(deviceQuestion) && !Ua3::opened(filesNotice) && notices->serial() == noticesNow);
    shots(d, t.main, "no-device");

    device->openDemo();
    d.waitFor(10000, [&] { return device->connected() && device->demo(); });
    d.waitIdle();
    switchTo(d, layout, wasLayout, wasLayout == QLatin1String("modern") ? "ModernShell" : "ClassicShell");
    t.main->resize(wasSize);
    d.spin(200);
}

const SmokeDriver::Section saveState("ua3-savestate", 899, &saveStateSection);
} // namespace
