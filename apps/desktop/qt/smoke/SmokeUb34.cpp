// -- WP UB34 -- (UXP B3, owner D7) The top bar's A | B from the first edit, in
// Modern on the demo: hidden with nothing unsaved; after an edit it shows with
// B (now) chosen and says what A and B are; a click on A recalls the preset as
// saved on the device with the listening volume where it was; the B key goes
// back to the edit; a Save on B ends the compare and A | B goes; the B key
// with nothing compared starts it too. A compare of two snapshots still names
// them (ua3-savestate).
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "FilesController.h"
#include "LayoutController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"
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
    t.check("nothing unsaved on A", d.waitFor(3000, [&] { return !sidebar->unsaved(); }));
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

const SmokeDriver::Section ab("ub34-ab", 899, &abSection);
} // namespace
