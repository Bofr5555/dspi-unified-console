// -- WP P1QT2 -- Settings > Installer, the lock, tokens, the AutoEQ preview,
// notices and undo groups (PLAN-CLIENTS 5.9), on the demo device. Each
// section leaves the demo as it found it: no limits, no PIN. The new page,
// banner, dialog and the AutoEQ preview are shot in the dark scheme and the
// light (<out>/p1qt2/<name>-<scheme>.png).
#include "Qp18Notices.h"
#include "SmokeDriver.h"

#include "AutoEqController.h"
#include "ChannelController.h"
#include "Confirm.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "HistoryController.h"
#include "LimitsController.h"
#include "MatrixController.h"
#include "NoticeCenter.h"
#include "PresetModel.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SidebarController.h"
#include "Snapshot.h"
#include "ToolsController.h"

#include "CoreApi.h"

#include <QDir>
#include <QJsonDocument>
#include <QJsonObject>
#include <cstdio>
#include <QGuiApplication>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QWindow>
#include <atomic>
#include <cmath>
#include <functional>
#include <memory>

namespace {

constexpr const char *kPin = "2468";

struct Scheme {
    Qt::ColorScheme scheme;
    const char *name;
};
constexpr Scheme kSchemes[] = { { Qt::ColorScheme::Dark, "dark" }, { Qt::ColorScheme::Light, "light" } };

// Shoots `window` in both schemes as p1qt2/<name>-<scheme>.png
void shootBoth(SmokeDriver &d, QQuickWindow *window, const QString &name) {
    Shell *shell = d.app().shell;
    const Qt::ColorScheme before = shell->colorSchemeOverride();
    QDir(d.outDir()).mkpath(QStringLiteral("p1qt2"));
    for (const Scheme &s : kSchemes) {
        shell->setColorSchemeOverride(s.scheme);
        d.spin(300);
        d.check(QStringLiteral("p1qt2: screenshot %1, %2").arg(name, QLatin1String(s.name)),
                d.grab(window, QStringLiteral("p1qt2/%1-%2.png").arg(name, QLatin1String(s.name))));
    }
    shell->setColorSchemeOverride(before);
    d.spin(100);
}

bool near(double a, double b) { return std::fabs(a - b) < 0.05; }

bool ensureDemo(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    return d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle();
}

bool settled(SmokeDriver &d, LimitsController *lc) {
    d.waitIdle();
    const bool ok = d.waitFor(8000, [&] { return !lc->busy(); });
    d.waitIdle();
    d.spin(60);
    return ok;
}

// A write through the app's one write path; whether the core took it
bool write(SmokeDriver &d, const QString &what, std::function<bool(FfiCore *)> setter) {
    auto ok = std::make_shared<int>(-1);
    d.app().device->write(what, std::move(setter), [ok](bool done) { *ok = done ? 1 : 0; });
    d.waitFor(5000, [&] { return *ok >= 0; });
    d.waitIdle();
    return *ok == 1;
}

// A job on the core's worker, waited for
template <class T>
T onWorker(SmokeDriver &d, std::function<T(FfiCore *)> job) {
    auto out = std::make_shared<T>();
    auto done = std::make_shared<bool>(false);
    d.app().core->worker()->call(d.app().device, [job](FfiCore *c) { return job(c); }, [out, done](T v) {
        *out = v;
        *done = true;
    });
    d.waitFor(8000, [&] { return *done; });
    return *out;
}

const DspState &st(SmokeDriver &d) { return *d.app().device->state(); }

QString lastNotice() {
    NoticeCenter *n = Registrar::instance<NoticeCenter>();
    return n ? n->last().value(QStringLiteral("text")).toString() : QString();
}

// A new notice since the mark, whose text starts with `prefix`
bool noticeStarts(SmokeDriver &d, const QString &prefix) {
    return d.waitFor(3000, [&] { return qp18NoticeNew() && lastNotice().startsWith(prefix); });
}

QWindow *topWindow(const QString &name) {
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (w->objectName() == name) return w;
    return nullptr;
}

bool shown(QQuickItem *item) {
    for (QQuickItem *i = item; i; i = i->parentItem())
        if (!i->isVisible()) return false;
    return item != nullptr;
}

// Leaves the demo with no limits and no PIN, unlocked
void cleanUp(SmokeDriver &d, LimitsController *lc) {
    if (lc->locked()) {
        lc->unlock(QString::fromLatin1(kPin));
        settled(d, lc);
    }
    if (lc->pinSet()) {
        lc->clearPin();
        settled(d, lc);
    }
    if (lc->active()) {
        lc->clearLimits();
        settled(d, lc);
    }
}

} // namespace

// ---- Settings > Installer: limits, the PIN, the lock, tokens -------------------------

static void installerSection(SmokeDriver &d) {
    qp18NoticeMark();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *lc = Registrar::instance<LimitsController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    auto *settings = Registrar::instance<SettingsController>();
    auto *channels = Registrar::instance<ChannelController>();
    auto *sidebar = Registrar::instance<SidebarController>();
    d.check("p1qt2: the limits controller is made", lc && notices && settings && channels && sidebar);
    if (!lc || !notices || !settings || !channels || !sidebar) return;
    d.check("p1qt2: on the demo", ensureDemo(d));
    settled(d, lc);
    cleanUp(d, lc);

    // ---- Tokens are required by every core (P1TK, P1TZ), and the app's dangerous calls carry them ----
    const int slot = device->presets()->activeSlot();
    d.check("p1qt2: a preset save without a token is refused, nothing written",
            slot >= 0 && onWorker<bool>(d, [slot](FfiCore *c) {
                char buf[512] = {};
                dspi_save_preset_confirmed(c, uint8_t(slot), nullptr, buf, sizeof buf);
                return QByteArray(buf).contains("\"confirmation_required\"");
            }));
    QSignalSpy presetFailed(sidebar, &SidebarController::presetFailed);
    sidebar->savePreset();
    d.waitFor(5000, [&] { return !sidebar->presetBusy(); });
    d.waitIdle();
    d.check("p1qt2: File > Save Preset mints its token and saves", presetFailed.isEmpty());
    d.check("p1qt2: a gated call with a token passes",
            onWorker<int>(d, [slot](FfiCore *c) { return int(Confirm::savePreset(c, uint8_t(slot))); }) == PRESET_OK);

    // ---- No PIN: nothing is locked and nothing is asked ----
    d.check("p1qt2: no PIN, nothing locked", !lc->pinSet() && !lc->locked() && lc->installerOpen());
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check("p1qt2: the settings window opens", w && d.waitFor(3000, [&] { return w->isVisible(); }));
    if (!w) return;
    settings->open(QStringLiteral("installer"));
    d.spin(300);
    auto item = [&](const char *name) { return d.item(QString::fromLatin1(name), w->contentItem()); };
    d.check("p1qt2: Settings > Installer is a page of its own",
            settings->currentPage() == QLatin1String("installer") && item("InstallerPage") != nullptr);
    d.check("p1qt2: with no PIN the page is open, no PIN asked",
            shown(item("Installer.limits")) && !shown(item("Installer.unlockGroup")) && shown(item("Installer.pinGroup")));
    QObject *banner = d.object(QStringLiteral("LockBanner"));
    d.check("p1qt2: no lock banner without a PIN", banner && !banner->property("visible").toBool());
    const float gainBefore = st(d).output_gain_db[0];
    d.check("p1qt2: with no PIN every write passes",
            write(d, QStringLiteral("p1qt2 gain"), [](FfiCore *c) { return dspi_set_output_gain(c, 0, 3.0f); })
                && near(st(d).output_gain_db[0], 3.0));
    shootBoth(d, w, QStringLiteral("installer-no-pin"));

    // ---- A limit: narrowing it clamps and reports; a refused write shows its notice ----
    qp18NoticeMark();
    lc->setOutputLimit(0, QStringLiteral("maxGainDb"), -3.0);
    settled(d, lc);
    d.check("p1qt2: the profile is in force", lc->active() && lc->profileState() == QLatin1String("ok"));
    d.check("p1qt2: narrowing the limit below the gain clamps it", near(st(d).output_gain_db[0], -3.0));
    d.check("p1qt2: and reports it in the import's words", qp18NoticeNew() && lastNotice().contains(QLatin1String("out.gain")));
    qp18NoticeMark();
    const QString outside = notices->row(QStringLiteral("limit_exceeded")).value(QStringLiteral("title")).toString();
    channels->setOutputGain(0, 6.0);
    d.waitIdle();
    d.check("p1qt2: a write past the limit is refused", near(st(d).output_gain_db[0], -3.0));
    d.check("p1qt2: and shows the core's notice", !outside.isEmpty() && noticeStarts(d, outside));
    lc->setLimit(QStringLiteral("masterVolumeMaxDb"), -10.0);
    lc->setText(QStringLiteral("note"), QStringLiteral("Tweeters on outputs 3 and 4"));
    lc->setText(QStringLiteral("contact"), QStringLiteral("installer@example.org"));
    settled(d, lc);
    // -- WP P1EFIX -- the editor's ranges are the core's one table
    {
        const QVariantMap r = lc->ranges();
        const QVariantMap gain = r.value(QStringLiteral("maxGainDb")).toMap();
        QQuickItem *gainField = item("Installer.output.0.maxGain");
        QQuickItem *masterField = item("Installer.masterVolumeMax");
        d.check("p1efix: the limits ranges come from the core",
                r.size() == 8 && gain.value(QStringLiteral("max")).toDouble() == 20.0
                    && r.value(QStringLiteral("masterVolumeMaxDb")).toMap().value(QStringLiteral("min")).toDouble() == -128.0);
        d.check("p1efix: the Installer page's fields use them",
                gainField && masterField && gainField->property("to").toDouble() == 20.0
                    && gainField->property("from").toDouble() == -60.0 && masterField->property("from").toDouble() == -128.0
                    && gainField->property("unit").toString() == QLatin1String("dB"));
    }
    d.check("p1qt2: the page shows the limits",
            lc->limits().value(QStringLiteral("masterVolumeMaxDb")).toDouble() == -10.0
                && lc->outputs().value(0).toMap().value(QStringLiteral("maxGainDb")).toDouble() == -3.0);
    d.spin(200);
    shootBoth(d, w, QStringLiteral("installer-limits"));

    // ---- A PIN, the recovery code, the lock ----
    lc->setPin(QString::fromLatin1(kPin), QStringLiteral("2469"));
    d.check("p1qt2: two different PINs are not set", !lc->pinSet() && !lc->pinError().isEmpty());
    lc->setPin(QString::fromLatin1(kPin), QString::fromLatin1(kPin));
    settled(d, lc);
    d.check("p1qt2: the PIN is set, the console unlocked", lc->pinSet() && !lc->locked() && lc->installerOpen());
    d.check("p1qt2: the recovery code is shown once", lc->recoveryCode().count(QLatin1Char('-')) == 3 && shown(item("Installer.recovery")));
    d.spin(200);
    if (QQuickItem *page = item("InstallerPage")) page->setProperty("contentY", qMax(0.0, page->property("contentHeight").toReal() - page->height()));
    shootBoth(d, w, QStringLiteral("installer-pin-set"));
    lc->dismissRecovery();
    d.check("p1qt2: and gone once written down", lc->recoveryCode().isEmpty());
    lc->setCovers(true);
    settled(d, lc);
    d.check("p1qt2: the lock covers the settings", lc->covers());
    lc->lockNow();
    settled(d, lc);
    d.check("p1qt2: Lock Now locks this console", lc->locked() && !lc->installerOpen());
    d.check("p1qt2: the locked page asks for the PIN first",
            shown(item("Installer.unlockGroup")) && !shown(item("Installer.limits")) && !item("Installer.scope")->property("text").toString().isEmpty());
    if (QQuickItem *page = item("InstallerPage")) page->setProperty("contentY", 0);
    shootBoth(d, w, QStringLiteral("installer-locked"));

    // Only volume, mute and presets work; everything else is refused with its notice
    d.check("p1qt2: locked: master volume works",
            write(d, QStringLiteral("p1qt2 master"), [](FfiCore *c) { return dspi_set_master_volume(c, -20.0f); }));
    d.check("p1qt2: locked: mute works",
            write(d, QStringLiteral("p1qt2 mute"), [](FfiCore *c) { return dspi_set_output_mute(c, 1, true); })
                && write(d, QStringLiteral("p1qt2 unmute"), [](FfiCore *c) { return dspi_set_output_mute(c, 1, false); }));
    d.check("p1qt2: locked: a preset loads",
            slot >= 0 && write(d, QStringLiteral("p1qt2 preset"), [slot](FfiCore *c) { return dspi_load_preset(c, uint8_t(slot)) == PRESET_OK; }));
    qp18NoticeMark();
    const QString lockTitle = notices->row(QStringLiteral("installer_lock.locked")).value(QStringLiteral("title")).toString();
    const float delay = st(d).output_delay_ms[1];
    d.check("p1qt2: locked: an output's delay is refused",
            !write(d, QStringLiteral("p1qt2 delay"), [](FfiCore *c) { return dspi_set_output_delay(c, 1, 2.5f); })
                && near(st(d).output_delay_ms[1], delay));
    d.check("p1qt2: with the lock's notice", !lockTitle.isEmpty() && noticeStarts(d, lockTitle));
    qp18NoticeMark();
    sidebar->savePreset();
    d.waitFor(5000, [&] { return !sidebar->presetBusy(); });
    d.waitIdle();
    d.check("p1qt2: locked: a preset save is refused, with the lock's notice", noticeStarts(d, lockTitle));

    // The locked-user banner and the one prompt
    d.check("p1qt2: the lock banner shows", banner && d.waitFor(2000, [&] { return banner->property("visible").toBool(); }));
    // -- WP P1EFIX -- its line is the core's row, the same in every client
    const QString bannerLine = notices->row(QStringLiteral("installer_lock.banner")).value(QStringLiteral("reason")).toString();
    d.check("p1efix: the lock banner's line is the core's installer_lock.banner row",
            banner && !bannerLine.isEmpty() && banner->property("text").toString().startsWith(bannerLine));
    w->close();
    QQuickWindow *main = d.mainWindow();
    d.spin(200);
    shootBoth(d, main, QStringLiteral("lock-banner"));
    QObject *dialog = banner ? banner->findChild<QObject *>(QStringLiteral("UnlockDialog")) : nullptr;
    QObject *unlockButton = banner ? banner->findChild<QObject *>(QStringLiteral("LockBanner.unlock")) : nullptr;
    if (unlockButton) QMetaObject::invokeMethod(unlockButton, "clicked");
    d.check("p1qt2: Unlock… opens the prompt", dialog && d.waitFor(2000, [&] { return dialog->property("opened").toBool(); }));
    lc->unlock(QStringLiteral("0000"));
    settled(d, lc);
    d.check("p1qt2: a wrong PIN is refused in the core's words", lc->locked() && !lc->pinError().isEmpty());
    d.spin(200);
    shootBoth(d, main, QStringLiteral("unlock-dialog"));
    lc->unlock(QString::fromLatin1(kPin));
    settled(d, lc);
    d.check("p1qt2: the PIN unlocks", !lc->locked() && lc->installerOpen() && lc->pinError().isEmpty());
    d.check("p1qt2: and closes the prompt", dialog && d.waitFor(2000, [&] { return !dialog->property("opened").toBool(); }));
    d.check("p1qt2: unlocked, every write passes again",
            write(d, QStringLiteral("p1qt2 delay"), [delay](FfiCore *c) { return dspi_set_output_delay(c, 1, delay); }));

    // ---- Back as it was: no PIN, no limits ----
    lc->clearPin();
    settled(d, lc);
    d.check("p1qt2: Remove PIN removes it", !lc->pinSet() && !lc->locked());
    lc->clearLimits();
    settled(d, lc);
    d.check("p1qt2: Remove All Limits removes the profile", !lc->active());
    write(d, QStringLiteral("p1qt2 gain"), [gainBefore](FfiCore *c) { return dspi_set_output_gain(c, 0, gainBefore); });
    write(d, QStringLiteral("p1qt2 master"), [](FfiCore *c) { return dspi_set_master_volume(c, 0.0f); });
    cleanUp(d, lc);
    notices->clear();
}

// ---- Undo groups: Clear All, Direct 1:1, a linked pair's preamp, Protect all ---------------

static void groupsSection(SmokeDriver &d) {
    qp18NoticeMark();
    auto *channels = Registrar::instance<ChannelController>();
    auto *history = Registrar::instance<HistoryController>();
    auto *matrix = Registrar::instance<MatrixController>();
    auto *tools = Registrar::instance<ToolsController>();
    d.check("p1qt2: the controllers are made", channels && history && matrix && tools);
    if (!channels || !history || !matrix || !tools) return;
    d.check("p1qt2: on the demo", ensureDemo(d));
    auto undo = [&] {
        history->undo();
        d.waitIdle();
        d.spin(100);
        d.waitIdle();
    };

    // Clear All: every band back with one undo
    const int ch = channels->outputChannel(0);
    const int bands = 4;
    for (int b = 0; b < bands; ++b) {
        channels->setBandType(ch, b, false, FILTER_PEAKING);
        channels->setBandValue(ch, b, false, QStringLiteral("freq"), 200.0 * (b + 1));
        channels->setBandValue(ch, b, false, QStringLiteral("gain"), 2.0 + b);
    }
    d.waitIdle();
    auto bandsSet = [&] {
        for (int b = 0; b < bands; ++b)
            if (st(d).filters[ch][b].filter_type != FILTER_PEAKING || !near(st(d).filters[ch][b].gain, 2.0 + b)) return false;
        return true;
    };
    d.check("p1qt2: four bands set", bandsSet());
    channels->clearBands(ch, false);
    d.waitIdle();
    bool flat = true;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) flat = flat && st(d).filters[ch][b].filter_type == FILTER_FLAT;
    d.check("p1qt2: Clear All clears every band", flat);
    d.waitFor(2000, [&] { return !history->entries().isEmpty(); });
    d.check("p1qt2: the history names the group",
            history->entries().value(0).toMap().value(QStringLiteral("title")).toString().startsWith(QLatin1String("Clear All")));
    undo();
    d.check("p1qt2: Clear All is one undo: every band back", bandsSet());
    channels->clearBands(ch, false);
    d.waitIdle();

    // Direct 1:1: one undo puts the routing back. Its outputs on first: an
    // output switched on is never undone, and stays outside the group
    const int n = qMin(int(st(d).num_input_channels), d.app().device->numOutputs() - 1);
    for (int o = 0; o < n; ++o)
        if (!st(d).output_enabled[o])
            write(d, QStringLiteral("p1qt2 enable %1").arg(o), [o](FfiCore *c) { return dspi_set_output_enable(c, uint8_t(o), true) == 1; });
    matrix->setConnected(0, 1, true);
    matrix->setConnected(1, 1, true);
    d.waitIdle();
    const DspState before = st(d);
    matrix->directRouting();
    d.waitIdle();
    d.check("p1qt2: Direct 1:1 routes each input to its output",
            st(d).matrix_routing[0][0] && !st(d).matrix_routing[0][1] && st(d).matrix_routing[1][1]);
    undo();
    bool routesBack = true;
    for (int i = 0; i < MAX_INPUTS; ++i)
        for (int o = 0; o < MAX_OUTPUTS; ++o) routesBack = routesBack && st(d).matrix_routing[i][o] == before.matrix_routing[i][o];
    d.check("p1qt2: Direct 1:1 is one undo: every crosspoint back", routesBack);
    matrix->setConnected(0, 1, false);
    d.waitIdle();

    // A linked input pair's preamp: both inputs, one undo
    if (!channels->isLinked(0)) channels->linkPair(0, 0);
    d.waitIdle();
    const float pre0 = st(d).input_preamp_db[0], pre1 = st(d).input_preamp_db[1];
    channels->setPreamp(0, -4.0);
    d.waitIdle();
    d.check("p1qt2: a linked pair's preamp moves both", near(st(d).input_preamp_db[0], -4) && near(st(d).input_preamp_db[1], -4));
    undo();
    d.check("p1qt2: and one undo puts both back", near(st(d).input_preamp_db[0], pre0) && near(st(d).input_preamp_db[1], pre1));

    // PRO1 S2: the warning row, and Protect all outputs (P1PR) as one undo
    if (tools->limiterSupported()) {
        const DspState lim = st(d);
        for (int o = 0; o < tools->numOutputs(); ++o) tools->setLimiterEnabled(o, false);
        write(d, QStringLiteral("p1qt2 enable"), [](FfiCore *c) { return dspi_set_output_enable(c, 0, true) == 1; });
        write(d, QStringLiteral("p1qt2 gain"), [](FfiCore *c) { return dspi_set_output_gain(c, 0, 0.0f); });
        d.check("p1qt2: an output above -10 dB without a limiter is warned of",
                d.waitFor(3000, [&] { return tools->unprotectedOutputs().contains(0) && !tools->unprotectedText().isEmpty(); }));
        const DspState pre = st(d);
        tools->protectAllOutputs();
        d.waitIdle();
        d.check("p1qt2: Protect all outputs leaves no warning",
                d.waitFor(3000, [&] { return tools->unprotectedOutputs().isEmpty(); }));
        bool allOn = true;
        for (int o = 0; o < tools->numOutputs(); ++o) allOn = allOn && st(d).limiter_enabled[o] && near(st(d).limiter_threshold_db[o], -1);
        d.check("p1qt2: every limiter on at -1 dBFS", allOn);
        // One undo puts every threshold and link group back; the limiters stay
        // on (switching one is audible: the registry never undoes it)
        qp18NoticeMark();
        undo();
        // -- WP P1EFIX -- and says so: the core's history note
        d.check("p1efix: undoing Protect all says the limiters stay on",
                noticeStarts(d, QStringLiteral("The limiters stay on")));
        bool back = true;
        for (int o = 0; o < tools->numOutputs(); ++o)
            back = back && near(st(d).limiter_threshold_db[o], pre.limiter_threshold_db[o])
                   && st(d).limiter_link_group[o] == pre.limiter_link_group[o] && st(d).limiter_enabled[o];
        d.check("p1qt2: Protect all outputs is one undo (the limiters stay on)", back);
        for (int o = 0; o < tools->numOutputs(); ++o) {
            tools->setLimiterEnabled(o, lim.limiter_enabled[o]);
            tools->setLimiterThreshold(o, lim.limiter_threshold_db[o]);
        }
        write(d, QStringLiteral("p1qt2 gain"), [g = lim.output_gain_db[0]](FfiCore *c) { return dspi_set_output_gain(c, 0, g); });
        d.waitIdle();
    } else {
        d.notRun(QStringLiteral("p1qt2: protect all outputs"), QStringLiteral("no output limiter on this demo"));
    }
}

// ---- AutoEQ: the preview and the channel pair before Apply (P1AQ) ------------------------

static void autoEqPreviewSection(SmokeDriver &d) {
    qp18NoticeMark();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *aeq = Registrar::instance<AutoEqController>();
    auto *channels = Registrar::instance<ChannelController>();
    auto *history = Registrar::instance<HistoryController>();
    d.check("p1qt2: the AutoEQ controller is made", aeq && channels && history);
    if (!aeq || !channels || !history) return;
    d.check("p1qt2: on the demo", ensureDemo(d));
    d.check("p1qt2: the database is loaded", d.waitFor(15000, [&] { return !aeq->loading() && aeq->profileCount() > 0; }));

    // The pairs: inputs first, then outputs
    const QVariantList pairs = aeq->pairs();
    int outPair = -1;
    for (int i = 0; i < pairs.size(); ++i)
        if (outPair < 0 && pairs.at(i).toMap().value(QStringLiteral("outputs")).toBool()) outPair = i;
    d.check("p1qt2: inputs 1 and 2 come first, outputs after",
            !pairs.isEmpty() && !pairs.value(0).toMap().value(QStringLiteral("outputs")).toBool() && outPair > 0);
    if (outPair < 0) return;

    shell->trigger(QStringLiteral("autoeq.browse"));
    auto *browser = qobject_cast<QQuickWindow *>(topWindow(QStringLiteral("AutoEqWindow")));
    d.check("p1qt2: the browser opens", browser && d.waitFor(3000, [&] { return browser->isVisible(); }));
    if (!browser) return;
    // -- WP LAY1b -- the browser is a panel in a PanelWindow: its properties are the panel's
    QObject *browserPanel = browser->property("content").value<QObject *>();
    if (!browserPanel) return;
    aeq->profiles()->setSearchText(QStringLiteral("HD 600"));
    const QString id = aeq->profiles()->idAt(0);
    aeq->setPair(outPair);
    const int sent = device->settersRun();
    const DspState before = st(d);
    browserPanel->setProperty("selectedId", id);
    d.check("p1qt2: the preview comes from the core",
            d.waitFor(5000, [&] {
                const QVariantMap p = aeq->preview();
                return p.value(QStringLiteral("profileId")).toString() == id && !p.value(QStringLiteral("curves")).toList().isEmpty();
            }));
    d.waitIdle();
    d.check("p1qt2: the preview writes nothing", device->settersRun() == sent);
    const int out0 = channels->outputChannel(pairs.at(outPair).toMap().value(QStringLiteral("left")).toInt() - device->state()->num_input_channels);
    bool same = true;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b)
        same = same && st(d).filters[out0][b].filter_type == before.filters[out0][b].filter_type
               && st(d).filters[out0][b].gain == before.filters[out0][b].gain;
    d.check("p1qt2: nothing changed on the device", same);
    d.spin(300);
    shootBoth(d, browser, QStringLiteral("autoeq-preview"));

    // Apply: only the chosen outputs change, one undo puts them back
    QMetaObject::invokeMethod(browserPanel, "applySelected");
    d.check("p1qt2: Apply closes the browser", d.waitFor(3000, [&] { return !browser->isVisible(); }));
    d.waitIdle();
    bool outChanged = false;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) outChanged = outChanged || st(d).filters[out0][b].filter_type != before.filters[out0][b].filter_type;
    bool inputsSame = true;
    for (int ch = 0; ch < 2; ++ch)
        for (int b = 0; b < BANDS_PER_CHANNEL; ++b)
            inputsSame = inputsSame && st(d).filters[ch][b].filter_type == before.filters[ch][b].filter_type
                         && st(d).filters[ch][b].gain == before.filters[ch][b].gain;
    d.check("p1qt2: the profile is on the chosen outputs", outChanged);
    d.check("p1qt2: and the inputs are left alone", inputsSame);
    d.check("p1qt2: the outputs' gain is left alone",
            near(st(d).output_gain_db[0], before.output_gain_db[0]) && near(st(d).output_gain_db[1], before.output_gain_db[1]));
    history->undo();
    d.waitIdle();
    d.spin(100);
    d.waitIdle();
    bool back = true;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b)
        back = back && st(d).filters[out0][b].filter_type == before.filters[out0][b].filter_type
               && st(d).filters[out0][b].gain == before.filters[out0][b].gain;
    d.check("p1qt2: the apply is one undo", back);
    aeq->setPair(0);
    aeq->profiles()->setSearchText(QString());
    Registrar::instance<NoticeCenter>()->clear();
}

static const SmokeDriver::Section installer("p1qt2-installer", 158, &installerSection);
static const SmokeDriver::Section groups("p1qt2-groups", 159, &groupsSection);
static const SmokeDriver::Section autoEqPreview("p1qt2-autoeq", 160, &autoEqPreviewSection);
