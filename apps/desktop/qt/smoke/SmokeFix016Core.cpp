// -- FIX016 -- The 0.16 review's core fixes as Qt shows them, on the demo:
// - "fix016-locksrc" (LOCKSRC): under the installer's lock Home's source (and
//   the dock's) is a working control (the core lets a locked user change the source), and a
//   gain is still refused.
// - "fix016-ab" (R3): on side A of "as saved" against "now" B's edit is
//   unsaved: Save to Device counts it, a preset switch asks (and a discard
//   drops B as asked), a Save on A saves B.
// Each section leaves the layout, the mode, the lock and the device as it
// found them.
#include "SmokeDriver.h"

#include "Confirm.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "FilesController.h"
#include "LimitsController.h"
#include "PresetModel.h"
#include "ModeController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"
#include "SnapshotsController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QQuickItem>
#include <QSignalSpy>
#include <QQuickWindow>
#include <QTest>
#include <cmath>
#include <functional>

namespace {

const QString kFixPin = QStringLiteral("2468");

QQuickItem *fixShown(QQuickItem *from, const QString &name) {
    if (!from || !from->isVisible()) return nullptr;
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *f = fixShown(c, name)) return f;
    return nullptr;
}

struct Fix {
    SmokeDriver &d;
    const char *tag;
    void check(const QString &what, bool ok) const { d.check(QStringLiteral("fix016: %1: %2").arg(QLatin1String(tag), what), ok); }
    void check(const char *what, bool ok) const { check(QString::fromUtf8(what), ok); }
    void settle() const {
        d.spin(40);
        d.waitIdle();
        d.spin(120);
    }
    // A write on the shown device; whether it went through
    bool write(const char *what, std::function<bool(FfiCore *)> job) const {
        bool done = false, ok = false;
        d.app().device->write(QString::fromLatin1(what), std::move(job), [&](bool r) { done = true; ok = r; });
        return d.waitFor(5000, [&] { return done; }) && d.waitIdle() && ok;
    }
    bool click(QQuickWindow *w, QQuickItem *i) const {
        if (!w || !i || i->width() <= 0) return false;
        QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint());
        settle();
        return true;
    }
};

bool fixSettled(SmokeDriver &d, LimitsController *lc) {
    d.waitIdle();
    const bool ok = d.waitFor(8000, [&] { return !lc->busy(); });
    d.waitIdle();
    d.spin(60);
    return ok;
}

// The installer's lock on the shown device: a PIN, the covers, Lock Now
bool fixLock(SmokeDriver &d, LimitsController *lc) {
    lc->setPin(kFixPin, kFixPin);
    fixSettled(d, lc);
    lc->dismissRecovery();
    lc->setCovers(true);
    fixSettled(d, lc);
    lc->lockNow();
    fixSettled(d, lc);
    return lc->locked();
}

// And off again, as before it: no PIN, no limits
bool fixUnlock(SmokeDriver &d, LimitsController *lc) {
    lc->unlock(kFixPin);
    fixSettled(d, lc);
    lc->clearPin();
    fixSettled(d, lc);
    lc->clearLimits();
    fixSettled(d, lc);
    return !lc->locked() && !lc->pinSet();
}

// ---- LOCKSRC ------------------------------------------------------------------------------------
void lockSourceSection(SmokeDriver &d) {
    d.allowSectionTime(90);
    Fix t{ d, "locksrc" };
    DeviceModel *device = d.app().device;
    auto *mode = Registrar::instance<ModeController>();
    auto *lc = Registrar::instance<LimitsController>();
    auto *sidebar = Registrar::instance<SidebarController>();
    QQuickWindow *w = d.mainWindow();
    t.check("the controllers", mode && lc && sidebar && w);
    if (!mode || !lc || !sidebar || !w) return;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    t.check("the demo is open", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    const QString wasMode = mode->mode();
    const bool wasModern = d.app().shell->pageHost() != nullptr;
    const QSize wasSize = w->size();
    t.check("Modern", d.switchLayout("modern"));
    w->resize(QSize(1600, 1000));
    mode->choose(QStringLiteral("advanced"), true);
    t.settle();
    const QVariantList sources = sidebar->sources();
    const int was = sidebar->source();
    t.check(QStringLiteral("the demo has sources to choose (%1)").arg(sources.size()), sidebar->sourceSupported() && sources.size() > 1);

    t.check("locked", fixLock(d, lc));
    d.goTo(QStringLiteral("overview/home"), 300);
    QQuickItem *home = fixShown(w->contentItem(), QStringLiteral("HomeSection"));
    QQuickItem *segments = home ? fixShown(home, QStringLiteral("HomeSection.source")) : nullptr;
    t.check("Home's source is segments under the lock, not words",
            segments && !fixShown(home, QStringLiteral("HomeSection.sourceText")));
    t.check("and they are on", segments && segments->isEnabled());
    QQuickItem *dockSource = fixShown(w->contentItem(), QStringLiteral("Dock.source"));
    t.check("the dock's source is on under the lock too", dockSource && dockSource->isEnabled());
    // Another source, by a click on its segment
    int to = -1;
    for (int i = 0; i < sources.size() && to < 0; ++i)
        if (sources.at(i).toMap().value(QStringLiteral("value")).toInt() != was) to = i;
    const int value = to >= 0 ? sources.at(to).toMap().value(QStringLiteral("value")).toInt() : -1;
    QQuickItem *segment = segments && to >= 0 ? fixShown(segments, QStringLiteral("HomeSection.source.%1").arg(to)) : nullptr;
    t.check("a click on another source", t.click(w, segment));
    t.check(QStringLiteral("changes the source while locked (%1 to %2)").arg(was).arg(sidebar->source()),
            d.waitFor(3000, [&] { return sidebar->source() == value; }) && lc->locked());
    // The rest stays locked: a gain is refused
    const float gain = device->state() ? device->state()->output_gain_db[0] : 0.0f;
    t.check("a gain is still refused", !t.write("fix016 gain", [g = gain](FfiCore *c) { return dspi_set_output_gain(c, 0, g - 3.0f); }));
    t.check("and nothing changed", device->state() && std::fabs(device->state()->output_gain_db[0] - gain) < 0.01f);
    d.grab(w, QStringLiteral("fix016-locksrc-home.png"));

    // ---- As found
    sidebar->setSource(was);
    d.waitFor(3000, [&] { return sidebar->source() == was; });
    t.check("unlocked", fixUnlock(d, lc));
    mode->choose(wasMode.isEmpty() ? QStringLiteral("advanced") : wasMode, true);
    w->resize(wasSize);
    d.switchLayout(wasModern ? "modern" : "classic");
    d.spin(200);
}

// ---- R3 ------------------------------------------------------------------------------------------
void abSection(SmokeDriver &d) {
    d.allowSectionTime(120);
    Fix t{ d, "ab" };
    DeviceModel *device = d.app().device;
    auto *snaps = Registrar::instance<SnapshotsController>();
    auto *sidebar = Registrar::instance<SidebarController>();
    auto *files = Registrar::instance<FilesController>();
    DeviceRegistry *r = DeviceRegistry::instance();
    QQuickWindow *w = d.mainWindow();
    t.check("the controllers", snaps && sidebar && files && r && w);
    if (!snaps || !sidebar || !files || !r || !w) return;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    t.check("the demo is open", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    while (r->sessions().size() > 1) {
        DeviceSession *other = r->sessions().last();
        r->close(other->id());
        d.waitFor(5000, [&] { return !r->sessions().contains(other); });
    }
    const bool wasModern = d.app().shell->pageHost() != nullptr;
    t.check("Modern", d.switchLayout("modern"));
    if (snaps->abActive()) snaps->abKeep(QStringLiteral("b"));
    if (sidebar->unsaved()) t.write("fix016 revert", [](FfiCore *c) { return dspi_load_params(c) == 0; });
    t.check("nothing unsaved", d.waitFor(5000, [&] { return !sidebar->unsaved() && !snaps->abActive(); }));
    const int first = device->presets()->property("activeSlot").toInt();
    const int other = (first + 1) % MAX_PRESETS;
    // The other slot holds a preset to switch to
    t.check("another preset", t.write("fix016 other", [first, other](FfiCore *c) {
        return Confirm::savePreset(c, uint8_t(other)) == 0 && Confirm::loadPreset(c, uint8_t(first)) == 0;
    }));
    const float saved = device->state() ? device->state()->output_gain_db[0] : 0.0f;

    // ---- An edit, then A: B's edit is unsaved there
    t.check("an edit", t.write("fix016 edit", [g = saved](FfiCore *c) { return dspi_set_output_gain(c, 0, g - 7.0f); }));
    const float edited = saved - 7.0f;
    t.check("is unsaved", d.waitFor(5000, [&] { return sidebar->unsaved() && snaps->canStartSaved(); }));
    const int onB = sidebar->unsavedChanges().size();
    snaps->abStartSaved();
    t.check("on A of as saved against now", d.waitFor(5000, [&] { return snaps->abActive() && snaps->abSaved() && snaps->abSide() == QLatin1String("a"); })
                                                 && d.waitIdle());
    t.settle();
    t.check("the device plays A", device->state() && std::fabs(device->state()->output_gain_db[0] - saved) < 0.01f);
    t.check(QStringLiteral("B's edit is unsaved on A (%1 change(s), %2 on B)").arg(sidebar->unsavedChanges().size()).arg(onB),
            d.waitFor(3000, [&] { return sidebar->unsaved() && sidebar->unsavedChanges().size() >= onB; }));
    t.check("the device counts as unsaved (Save All, the link, the close and quit guard)",
            d.waitFor(3000, [&] { return r->dirty(r->active()); }));
    QQuickItem *save = fixShown(w->contentItem(), QStringLiteral("TopBar.save"));
    t.check(QStringLiteral("Save to Device shows the count (%1)").arg(save ? save->property("count").toInt() : -1),
            save && save->property("count").toInt() == sidebar->unsavedChanges().size() && save->property("count").toInt() > 0);
    d.grab(w, QStringLiteral("fix016-ab-on-a.png"));

    // ---- A preset switch on A asks; "discard" drops B as asked
    QSignalSpy asked(sidebar, &SidebarController::unsavedQuestion);
    sidebar->requestPresetSwitch(other);
    t.check("a preset switch on A asks", d.waitFor(3000, [&] { return !asked.isEmpty(); })
                                             && asked.first().at(0).toString() == QLatin1String("switch"));
    t.check("nothing loaded before the answer", device->presets()->property("activeSlot").toInt() == first && snaps->abActive());
    sidebar->resolveUnsaved(QStringLiteral("switch"), other, QStringLiteral("alternative"));
    t.check("the discard loads the other preset and ends the compare",
            d.waitFor(8000, [&] { return device->presets()->property("activeSlot").toInt() == other && !snaps->abActive(); }) && d.waitIdle());
    sidebar->requestPresetSwitch(first);
    t.check("back on the first, with nothing to ask", d.waitFor(8000, [&] { return device->presets()->property("activeSlot").toInt() == first; })
                                                          && d.waitIdle());
    t.check("B is gone, as asked", device->state() && std::fabs(device->state()->output_gain_db[0] - saved) < 0.01f && !sidebar->unsaved());

    // ---- A Save on A saves B
    t.check("the edit again", t.write("fix016 edit 2", [g = saved](FfiCore *c) { return dspi_set_output_gain(c, 0, g - 7.0f); })
                                  && d.waitFor(5000, [&] { return snaps->canStartSaved(); }));
    snaps->abStartSaved();
    t.check("on A again", d.waitFor(5000, [&] { return snaps->abSaved() && snaps->abSide() == QLatin1String("a"); }) && d.waitIdle());
    QObject *question = files->host() ? files->host()->findChild<QObject *>(QStringLiteral("FilesHost.deviceQuestion")) : nullptr;
    d.app().shell->trigger(QStringLiteral("device.commit"));
    t.check("Save to Device asks", question && d.waitFor(2000, [&] { return question->property("opened").toBool(); }));
    if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    t.check("the save ends the compare with nothing unsaved", d.waitFor(8000, [&] { return !snaps->abActive() && !sidebar->unsaved(); }) && d.waitIdle());
    t.check("and B is what plays", device->state() && std::fabs(device->state()->output_gain_db[0] - edited) < 0.01f);
    t.write("fix016 revert 2", [](FfiCore *c) { return dspi_load_params(c) == 0; });
    t.check("and what is saved", device->state() && std::fabs(device->state()->output_gain_db[0] - edited) < 0.01f);

    // ---- As found: the saved gain saved again
    t.write("fix016 gain back", [first, g = saved](FfiCore *c) {
        return dspi_set_output_gain(c, 0, g) && Confirm::savePreset(c, uint8_t(first)) == 0;
    });
    d.waitFor(5000, [&] { return !sidebar->unsaved(); });
    d.switchLayout(wasModern ? "modern" : "classic");
    d.spin(200);
}

} // namespace

const SmokeDriver::Section fix016Ab("fix016-ab", 1001, &abSection);
const SmokeDriver::Section fix016LockSource("fix016-locksrc", 1001, &lockSourceSection);
