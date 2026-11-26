// -- WP UB34 -- (UXP B3, owner D7) The top bar's A | B from the first edit, in
// Modern on the demo: hidden with nothing unsaved; after an edit it shows with
// B (now) chosen and says what A and B are; a click on A recalls the preset as
// saved on the device with the listening volume where it was; the B key goes
// back to the edit; a Save on B ends the compare and A | B goes; the B key
// with nothing compared starts it too. A compare of two snapshots still names
// them (ua3-savestate).
// -- WP UB34 -- (UXP B4, owner D6) The dock's All devices link, with three
// demos open: the chip shows, off; on, a User volume move moves every device
// keeping their differences (back from the floor too), Mute mutes all, a
// preset recall asks once, names each device and recalls on all; Measure ›
// Several units says why it still needs the hub; off and in Classic nothing
// is linked.
#include "SmokeDriver.h"

#include "Confirm.h"
#include "DeviceLinkController.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "FilesController.h"
#include "LayoutController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"
#include "SessionModel.h"
#include "ShortcutController.h"
#include "SnapshotsController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QCoreApplication>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <cmath>
#include <functional>

namespace {

struct Ub34 {
    SmokeDriver &d;
    QQuickWindow *main;
    const char *tag;

    void check(const char *what, bool ok) const {
        d.check(QStringLiteral("ub34: %1: %2").arg(QLatin1String(tag), QString::fromUtf8(what)), ok);
    }
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
    bool click(QQuickItem *i) const {
        if (!i || i->width() <= 0) return false;
        QTest::mouseClick(main, Qt::LeftButton, Qt::NoModifier, point(i));
        settle();
        return true;
    }
    // A write on the shown device
    bool write(const char *what, std::function<bool(FfiCore *)> job) const {
        bool done = false;
        d.app().device->write(QString::fromLatin1(what), std::move(job), [&done](bool ok) { done = ok; });
        return d.waitFor(5000, [&] { return done; }) && d.waitIdle();
    }
    const DspState *state() const { return d.app().device->state(); }
    float gain() const { return state() ? state()->output_gain_db[0] : 0.0f; }
    float volume() const { return state() ? state()->user_volume_db : 0.0f; }
    void shots(const char *name) const {
        auto *layout = Registrar::instance<LayoutController>();
        const QString was = layout->theme();
        for (const char *scheme : { "dark", "light" }) {
            layout->setTheme(QString::fromLatin1(scheme));
            d.spin(250);
            d.grab(main, QStringLiteral("ub34-%1-%2.png").arg(QLatin1String(name), QLatin1String(scheme)));
        }
        layout->setTheme(was);
        d.spin(150);
    }
    void hoverShots(QQuickItem *at, const char *name) const {
        if (!at) return;
        QTest::mouseMove(main, point(at));
        d.spin(900);
        shots(name);
        QTest::mouseMove(main, QPoint(main->width() / 2, main->height() / 2));
        settle();
    }
};

bool modern(SmokeDriver &d, LayoutController *layout, const QString &to, const char *shellName) {
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

void abSection(SmokeDriver &d) {
    d.allowSectionTime(120);
    auto *layout = Registrar::instance<LayoutController>();
    auto *snaps = Registrar::instance<SnapshotsController>();
    auto *sidebar = Registrar::instance<SidebarController>();
    auto *files = Registrar::instance<FilesController>();
    Ub34 t{ d, d.mainWindow(), "ab" };
    t.check("the controllers", layout && snaps && sidebar && files && t.main);
    if (!layout || !snaps || !sidebar || !files || !t.main) return;
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    DeviceRegistry *r = DeviceRegistry::instance();
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    t.check("on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    // One device: the others an earlier section left are closed
    while (r && r->sessions().size() > 1) {
        DeviceSession *other = r->sessions().last();
        r->close(other->id());
        d.waitFor(5000, [&] { return !r->sessions().contains(other); });
    }
    const QString wasLayout = layout->layout();
    const QSize wasSize = t.main->size();
    t.main->resize(1440, 900);
    t.check("Modern", modern(d, layout, QStringLiteral("modern"), "ModernShell"));
    if (snaps->abActive()) snaps->abKeep(QStringLiteral("b"));
    // Nothing unsaved: the saved preset loaded again
    if (sidebar->unsaved()) t.write("ub34 revert", [](FfiCore *c) { return dspi_load_params(c) == 0; });
    t.check("nothing unsaved", d.waitFor(5000, [&] { return !sidebar->unsaved() && !snaps->abActive(); }));
    t.settle();
    t.check("A/B is hidden with nothing to compare", !t.shown(QStringLiteral("TopBar.ab")) && !snaps->canStartSaved());
    t.shots("before-edit");

    // ---- The first edit: A | B shows, B (now) chosen, saying what A and B are
    const float savedGain = t.gain();
    t.check("a listening volume", t.write("ub34 volume", [](FfiCore *c) { return dspi_set_user_volume(c, -18.0f); }));
    t.check("an edit", t.write("ub34 gain", [g = savedGain](FfiCore *c) { return dspi_set_output_gain(c, 0, g - 5.0f); }));
    const float editGain = t.gain();
    t.check("is unsaved", d.waitFor(5000, [&] { return sidebar->unsaved() && snaps->canStartSaved(); }));
    t.settle();
    t.shots("edited");   // before UB34: no A | B here
    QQuickItem *ab = t.shown(QStringLiteral("TopBar.ab"));
    t.check("A/B shows from the first edit", ab != nullptr);
    if (!ab) return;
    t.check("with B, now, chosen", ab->property("current").toInt() == 1);
    t.check("its tooltip says A is as saved, B is now",
            ab->property("toolTip").toString().contains(QStringLiteral("A is the preset as saved on the device, B is now")));
    t.hoverShots(t.shown(QStringLiteral("TopBar.ab.1")), "offered");

    // ---- A: the preset as saved, the volume kept
    t.check("a click on A", t.click(t.shown(QStringLiteral("TopBar.ab.0"))));
    t.check("starts as saved against now, on A",
            d.waitFor(5000, [&] { return snaps->abActive() && snaps->abSaved() && snaps->abSide() == QLatin1String("a"); }) && d.waitIdle());
    t.settle();
    t.check("the device has the saved gain", std::fabs(t.gain() - savedGain) < 0.01f);
    t.check("and the listening volume where it was", std::fabs(t.volume() + 18.0f) < 0.01f);
    // -- FIX016 -- (REVG R3) B's edit still counts as unsaved on A
    t.check("B's edit is unsaved on A", d.waitFor(3000, [&] { return sidebar->unsaved(); }));
    t.check("A chosen, in this app's words", ab->property("current").toInt() == 0 && snaps->abAName() == QStringLiteral("As saved")
                                                  && snaps->abBName() == QStringLiteral("Now"));
    t.check("the tooltip says A is heard", ab->property("toolTip").toString().contains(QStringLiteral("you hear A")));
    t.hoverShots(t.shown(QStringLiteral("TopBar.ab.0")), "on-a");

    // ---- The B key: back to the edit
    shell->trigger(QStringLiteral("device.abToggle"));
    t.check("the B key goes back to now", d.waitFor(5000, [&] { return snaps->abSide() == QLatin1String("b"); }) && d.waitIdle());
    t.settle();
    t.check("with the edit", std::fabs(t.gain() - editGain) < 0.01f && std::fabs(t.volume() + 18.0f) < 0.01f);

    // ---- A Save on B: it is what is saved now, the compare ends, A | B goes
    QObject *question = files->host() ? files->host()->findChild<QObject *>(QStringLiteral("FilesHost.deviceQuestion")) : nullptr;
    shell->trigger(QStringLiteral("device.commit"));
    t.check("Save to Device asks", question && d.waitFor(2000, [&] { return question->property("opened").toBool(); }));
    if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    t.check("a save on B ends the compare", d.waitFor(6000, [&] { return !snaps->abActive() && !sidebar->unsaved(); }));
    t.settle();
    t.check("and A/B goes, nothing being unsaved", !t.shown(QStringLiteral("TopBar.ab")));
    t.check("the edit is what is saved", std::fabs(t.gain() - editGain) < 0.01f);

    // ---- The B key with nothing compared starts it
    t.check("another edit", t.write("ub34 gain 2", [g = editGain](FfiCore *c) { return dspi_set_output_gain(c, 0, g + 2.0f); })
                                && d.waitFor(5000, [&] { return snaps->canStartSaved(); }));
    shell->trigger(QStringLiteral("device.abToggle"));
    t.check("the B key starts as saved against now", d.waitFor(5000, [&] { return snaps->abSaved() && snaps->abSide() == QLatin1String("a"); }));
    t.check("on the last save", d.waitIdle() && std::fabs(t.gain() - editGain) < 0.01f);
    snaps->abKeep(QStringLiteral("b"));
    d.waitFor(5000, [&] { return !snaps->abActive(); });
    // The saved gain back, for the sections after
    t.write("ub34 revert", [](FfiCore *c) { return dspi_load_params(c) == 0; });
    t.write("ub34 gain back", [g = savedGain](FfiCore *c) { return dspi_set_output_gain(c, 0, g); });

    modern(d, layout, wasLayout, wasLayout == QLatin1String("modern") ? "ModernShell" : "ClassicShell");
    t.main->resize(wasSize);
    d.spin(200);
}

// A write on `s`, whichever device is shown
bool writeOn(SmokeDriver &d, DeviceSession *s, std::function<bool(FfiCore *)> job) {
    bool done = false, ok = false;
    s->model()->write(QStringLiteral("ub34 write"), std::move(job), [&](bool r) { done = true; ok = r; });
    return d.waitFor(5000, [&] { return done; }) && ok && d.waitIdle();
}
float volumeOf(DeviceSession *s) { return s->model()->state() ? s->model()->state()->user_volume_db : 99.0f; }
bool mutedOf(DeviceSession *s) { return s->model()->state() && s->model()->state()->user_mute; }
int slotOf(DeviceSession *s) { return s->model()->state() ? s->model()->state()->active_preset_slot : -1; }

void dockSection(SmokeDriver &d) {
    d.allowSectionTime(180);
    auto *layout = Registrar::instance<LayoutController>();
    auto *link = Registrar::instance<DeviceLinkController>();
    auto *sidebar = Registrar::instance<SidebarController>();
    Ub34 t{ d, d.mainWindow(), "dock" };
    t.check("the controllers", layout && link && sidebar && t.main);
    if (!layout || !link || !sidebar || !t.main) return;
    DeviceModel *device = d.app().device;
    DeviceRegistry *r = DeviceRegistry::instance();
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    t.check("on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    const QString wasLayout = layout->layout();
    const QSize wasSize = t.main->size();
    t.main->resize(1440, 900);
    t.check("Modern", modern(d, layout, QStringLiteral("modern"), "ModernShell"));
    t.settle();
    t.check("one device: no chip", r->sessions().size() > 1 || (!t.shown(QStringLiteral("Dock.link")) && !link->available()));
    while (r->sessions().size() < 3) {
        const int before = int(r->sessions().size());
        r->openDemo();
        if (!d.waitFor(10000, [&] { return r->opening() == 0 && r->sessions().size() > before; })) break;
    }
    t.check("three demos open", r->sessions().size() == 3);
    if (r->sessions().size() != 3) return;
    DeviceSession *a = r->sessions().at(0), *b = r->sessions().at(1), *c = r->sessions().at(2);
    r->activate(a);
    t.settle();
    QQuickItem *chip = t.shown(QStringLiteral("Dock.link"));
    t.check("the chip shows, off", chip && !chip->property("checked").toBool() && link->available() && !link->linked());
    t.check("naming the devices", link->names().size() == 3 && link->help().contains(b->title()));

    // ---- Off: a move is the shown device's only
    t.check("volumes -20, -30, -10", writeOn(d, a, [](FfiCore *x) { return dspi_set_user_volume(x, -20.0f); })
                                         && writeOn(d, b, [](FfiCore *x) { return dspi_set_user_volume(x, -30.0f); })
                                         && writeOn(d, c, [](FfiCore *x) { return dspi_set_user_volume(x, -10.0f); }));
    link->setUserVolume(-22.0);
    d.waitIdle();
    t.check("unlinked, the others stay", d.waitFor(3000, [&] { return std::fabs(volumeOf(a) + 22.0f) < 0.05f; })
                                             && std::fabs(volumeOf(b) + 30.0f) < 0.05f && std::fabs(volumeOf(c) + 10.0f) < 0.05f);
    t.hoverShots(chip, "chip");

    // ---- On: the differences kept, back from the floor too
    t.check("a click links", t.click(chip) && link->linked() && chip->property("checked").toBool());
    link->setUserVolume(-27.0);
    t.check("a move moves all by as much", d.waitFor(3000, [&] {
        return std::fabs(volumeOf(a) + 27.0f) < 0.05f && std::fabs(volumeOf(b) + 35.0f) < 0.05f && std::fabs(volumeOf(c) + 15.0f) < 0.05f;
    }));
    link->setUserVolume(-55.0);
    t.check("one held at the floor", d.waitFor(3000, [&] { return std::fabs(volumeOf(b) + 60.0f) < 0.05f && std::fabs(volumeOf(c) + 43.0f) < 0.05f; }));
    link->setUserVolume(-22.0);
    t.check("and its difference back when the volume comes back", d.waitFor(3000, [&] {
        return std::fabs(volumeOf(a) + 22.0f) < 0.05f && std::fabs(volumeOf(b) + 30.0f) < 0.05f && std::fabs(volumeOf(c) + 10.0f) < 0.05f;
    }));
    t.shots("linked");

    // ---- Mute mutes all
    t.check("Mute mutes all", t.click(t.shown(QStringLiteral("Dock.mute")))
                                  && d.waitFor(3000, [&] { return mutedOf(a) && mutedOf(b) && mutedOf(c); }));
    d.app().shell->trigger(QStringLiteral("device.mute"));
    t.check("and Ctrl+M unmutes all", d.waitFor(3000, [&] { return !mutedOf(a) && !mutedOf(b) && !mutedOf(c); }));

    // ---- A preset on all: one question naming each device
    const int to = (slotOf(a) + 1) % MAX_PRESETS;
    for (DeviceSession *s : { a, b, c })
        if (s->model()->state() && !((s->model()->state()->preset_occupied >> to) & 1u)) {
            const int back = slotOf(s);
            writeOn(d, s, [to, back](FfiCore *x) { return Confirm::savePreset(x, uint8_t(to)) == 0 && Confirm::loadPreset(x, uint8_t(back)) == 0; });
        }
    t.check("each has the preset", d.waitFor(3000, [&] {
        for (DeviceSession *s : { a, b, c })
            if (!s->model()->state() || !((s->model()->state()->preset_occupied >> to) & 1u) || slotOf(s) == to) return false;
        return true;
    }));
    t.check("an edit on the second", writeOn(d, b, [](FfiCore *x) { return dspi_set_output_gain(x, 0, -3.0f); }));
    d.waitFor(3000, [&] { return r->dirty(b); });
    QObject *question = d.object(QStringLiteral("Dock.recallQuestion"));
    sidebar->requestPresetSwitch(to);
    t.check("a recall asks once", question && d.waitFor(2000, [&] { return question->property("opened").toBool(); }));
    const QString text = question ? question->property("text").toString() : QString();
    t.check("naming each device and the changes it drops",
            question && question->property("title").toString() == QStringLiteral("Recall preset %1 on all devices?").arg(to + 1)
                && text.count(QStringLiteral("• ")) == 3 && text.contains(b->title() + QStringLiteral(": ")) && text.contains(QStringLiteral("unsaved change")));
    t.shots("recall-question");
    if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    t.check("and recalls on all", d.waitFor(8000, [&] { return slotOf(a) == to && slotOf(b) == to && slotOf(c) == to; }));

    // ---- Several units: why it still needs the hub
    t.check("Several units", d.goTo(QStringLiteral("measure/units"), 400));
    QQuickItem *notice = t.shown(QStringLiteral("MeasureUnits.localNotice"));
    t.check("says each keeps its own clock, and points at the link",
            notice && notice->property("text").toString().contains(QStringLiteral("shared time base"))
                && notice->property("text").toString().contains(QStringLiteral("All devices")));
    t.shots("units");
    d.goTo(QStringLiteral("overview/overview"));

    // ---- Off, and Classic: nothing linked
    t.check("a click unlinks", t.click(t.shown(QStringLiteral("Dock.link"))) && !link->linked());
    link->setUserVolume(-24.0);
    t.check("a move is the shown device's again", d.waitFor(3000, [&] { return std::fabs(volumeOf(a) + 24.0f) < 0.05f; })
                                                      && std::fabs(volumeOf(b) + 30.0f) < 0.05f);
    link->setLinked(true);
    t.check("Classic", modern(d, layout, QStringLiteral("classic"), "ClassicShell"));
    t.check("unlinks and offers nothing", !link->linked() && !link->available());

    modern(d, layout, wasLayout, wasLayout == QLatin1String("modern") ? "ModernShell" : "ClassicShell");
    for (DeviceSession *s : { c, b })
        if (r->sessions().contains(s)) r->close(s->id());
    d.waitFor(5000, [&] { return r->sessions().size() == 1; });
    t.main->resize(wasSize);
    d.spin(200);
}

const SmokeDriver::Section ab("ub34-ab", 899, &abSection);
const SmokeDriver::Section dock("ub34-dock", 899, &dockSection);
} // namespace
