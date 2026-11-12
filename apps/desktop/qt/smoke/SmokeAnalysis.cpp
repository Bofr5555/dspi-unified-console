// -- WP QP9 -- The analysis tools on the demo device: the signal generator
// (signals, outputs, level, Space, live edits, playing on with the window
// closed), the Interrupt Monitor (lines, pause, Copy, Clear), Stats for nerbs
// (sections, history, Reset Watermarks, what stays hidden), and the spectrum
// analyser (the request, the graph overlay and its options, the bar strip,
// the window, channel pages, and following another console over a fake hub).
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "GraphController.h"
#include "LinkController.h"
#include "LinkFakeHub.h"
#include "MonitorController.h"
#include "Poller.h"
#include "PrefsMap.h"
#include "RtaController.h"
#include "Shell.h"
#include "SidebarController.h"
#include "SiggenController.h"
#include "StatsController.h"
#include "TokenStore.h"

#include <QClipboard>
#include <QDir>
#include <QGuiApplication>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QTest>
#include <functional>
#include <memory>

namespace {

// A window opened by its menu row, its items found by name
struct Qp9Window {
    SmokeDriver &d;
    QQuickWindow *w = nullptr;
    Qp9Window(SmokeDriver &driver, const char *action) : d(driver) {
        w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QString::fromLatin1(action)));
        d.spin(150);
    }
    QQuickItem *item(const QString &name) const { return w ? d.item(name, w->contentItem()) : nullptr; }
    bool shows(const QString &name) const {
        QQuickItem *it = item(name);
        if (!it) return false;
        for (QQuickItem *p = it; p; p = p->parentItem())
            if (!p->isVisible()) return false;
        return true;
    }
    void click(const QString &name) const {
        QQuickItem *it = item(name);
        if (!it || !w) return;
        QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, it->mapToScene(QPointF(it->width() / 2, it->height() / 2)).toPoint());
        d.spin(80);
    }
    // Keys go to this window only once it is the focus window
    bool focus() const {
        if (!w) return false;
        w->requestActivate();
        return d.waitFor(3000, [this] { return QGuiApplication::focusWindow() == w; });
    }
    QString shownName(const QString &name) const {
        QQuickItem *it = item(name);
        return it ? it->property("shownName").toString() : QString();
    }
};

// Runs `job` on the core's worker and waits for it
template <class T> T qp9OnWorker(SmokeDriver &d, std::function<T(FfiCore *)> job) {
    auto out = std::make_shared<T>();
    auto done = std::make_shared<std::atomic<bool>>(false);
    d.app().core->worker()->post([job, out, done](FfiCore *c) {
        *out = job(c);
        done->store(true);
    });
    d.waitFor(5000, [done] { return done->load(); });
    return *out;
}

// ---------------------------------------------------------------- generator

void siggenSection(SmokeDriver &d) {
    auto *gen = Registrar::instance<SiggenController>();
    d.check("qp9: the generator's controller", gen != nullptr);
    if (!gen) return;
    Qp9Window qp9Gen(d, "tools.signalGenerator");
    d.check("qp9: Signal Generator... opens its window", qp9Gen.w && qp9Gen.w->isVisible()
                                                              && qp9Gen.w->title() == QLatin1String("Signal Generator"));
    if (!qp9Gen.w) return;
    d.check("qp9: the demo has a generator", d.waitFor(5000, [gen] { return gen->available() && gen->controlsEnabled(); }));
    QQuickItem *qp9LastTile = qp9Gen.item("SiggenWindow.signal.14");
    d.check("qp9: the tiles share the grid's width", qp9LastTile && qp9LastTile->width() > 40 && qp9LastTile->isVisible());
    d.check("qp9: fifteen signals, a tile each", gen->types().size() == 15 && qp9Gen.item("SiggenWindow.signal.14")
                                                     && qp9Gen.item("SiggenWindow.signal.0"));
    d.check("qp9: an output chip per output", gen->outputCount() > 0
                                                  && qp9Gen.item(QStringLiteral("SiggenWindow.output.%1").arg(gen->outputCount() - 1)));
    d.check("qp9: the unsupported notice is hidden", !qp9Gen.shows("SiggenWindow.unsupported"));
    QQuickItem *qp9Level = qp9Gen.item("SiggenWindow.level");
    d.check("qp9: the two columns share the width", qp9Level && qp9Level->width() > qp9Gen.w->width() / 2 - 80);

    // A tile: the log sweep, with its defaults and timing
    qp9Gen.click("SiggenWindow.signal.4");
    d.check("qp9: a tile chooses its signal", gen->signalType() == SiggenController::SweepLog);
    d.check("qp9: with the type's defaults", gen->params().value(0).toDouble() == 20 && gen->params().value(1).toDouble() == 20000
                                                  && gen->durationSeconds() == 5.0 && gen->timing() == SiggenController::Sweep);
    d.check("qp9: the sweep's timing rows", qp9Gen.shows("SiggenWindow.duration") && qp9Gen.shows("SiggenWindow.gap"));
    gen->setDurationSeconds(0, 0.01);
    d.check("qp9: a sweep is at least 10 ms", gen->durationSeconds() == 0.01);

    // Outputs: off, on, inverted, off
    gen->noOutputs();
    d.check("qp9: None blocks Start", gen->startBlocker() == QLatin1String("Select at least one output")
                                          && !qp9Gen.item("SiggenWindow.start")->isEnabled());
    qp9Gen.click("SiggenWindow.output.1");
    const bool on = gen->channelMask() == 2 && gen->invertMask() == 0;
    qp9Gen.click("SiggenWindow.output.1");
    const bool inverted = gen->channelMask() == 2 && gen->invertMask() == 2
                          && qp9Gen.shownName("SiggenWindow.output.1").endsWith(QStringLiteral(" ø"));
    qp9Gen.click("SiggenWindow.output.1");
    d.check("qp9: a chip goes on, inverted, off", on && inverted && gen->channelMask() == 0 && gen->invertMask() == 0);
    gen->allOutputs();
    d.check("qp9: All selects the device's outputs", gen->channelMask() != 0);

    // Level: the field to -120, the slider to -80
    gen->setLevel(-150, false);
    const bool field = gen->levelDb() == -120;
    gen->setLevel(-95, true);
    d.check("qp9: the level's limits", field && gen->levelDb() == -80);

    // Sine on output 1, started with Space
    gen->selectType(SiggenController::Sine);
    gen->noOutputs();
    gen->stepOutput(0);
    gen->setLevel(-30);
    d.check("qp9: ready, in the Mac's words", gen->transportTitle() == QStringLiteral("Ready · Sine")
                                                   && gen->transportDetail() == QStringLiteral("1 output · peak -30.0 dBFS"));
    d.check("qp9: the window has the keyboard", qp9Gen.focus());
    QTest::keyClick(qp9Gen.w, Qt::Key_Space);
    d.check("qp9: Space starts it", d.waitFor(3000, [gen] { return gen->playing() && gen->status().signal_type == 0; }));
    d.check("qp9: the device plays the draft", [&] {
        const SiggenConfig cfg = qp9OnWorker<SiggenConfig>(d, [](FfiCore *c) {
            SiggenConfig out{};
            dspi_siggen_get_config(c, &out);
            return out;
        });
        return cfg.signal_type == 0 && cfg.channel_mask == 1 && std::fabs(cfg.level_db + 30) < 0.01;
    }());
    d.check("qp9: the title names the state and signal", gen->transportTitle().endsWith(QStringLiteral(" · Sine"))
                                                             && gen->transportDetail().contains(QStringLiteral("edits apply live")));
    const int polledBefore = Poller::instance()->counts(gen->statusPoll()).completed;
    d.spin(800);
    d.check("qp9: the status is read while it plays", Poller::instance()->counts(gen->statusPoll()).completed > polledBefore);
    d.grab(qp9Gen.w, QStringLiteral("window-siggen.png"));

    // A live edit reaches the device after the debounce
    gen->setParameter(0, 440);
    d.check("qp9: an edit while playing is sent", d.waitFor(3000, [&] {
        const SiggenConfig cfg = qp9OnWorker<SiggenConfig>(d, [](FfiCore *c) {
            SiggenConfig out{};
            dspi_siggen_get_config(c, &out);
            return out;
        });
        return std::fabs(cfg.p[0] - 440) < 0.01;
    }));
    d.check("qp9: and it keeps playing", gen->playing());

    // Closing the window leaves it playing
    qp9Gen.w->close();
    d.spin(700);
    d.check("qp9: the window closed, it plays on", gen->playing());
    Qp9Window again(d, "tools.signalGenerator");
    d.check("qp9: Space stops it (faded)", again.focus() && [&] {
        QTest::keyClick(again.w, Qt::Key_Space);
        return d.waitFor(3000, [gen] { return !gen->playing(); });
    }());
    d.check("qp9: stopped by the host", d.waitFor(2000, [gen] { return gen->transportDetail() == QLatin1String("Stopped"); }));
    gen->play();
    d.check("qp9: Start again", d.waitFor(3000, [gen] { return gen->playing(); }));
    again.click("SiggenWindow.stopNow");
    d.check("qp9: the hard stop", d.waitFor(3000, [gen] { return !gen->playing(); }));

    // Formats
    d.check("qp9: the elapsed and frequency formats",
            SiggenController::formatElapsed(75300) == QLatin1String("1:15.3") && SiggenController::formatElapsed(4200) == QLatin1String("4.2 s")
                && SiggenController::formatFrequency(1500) == QLatin1String("1.50 kHz")
                && SiggenController::formatFrequency(440) == QLatin1String("440 Hz"));
    // The tone pair's presets
    gen->selectType(SiggenController::TonePair);
    gen->applyPreset(QStringLiteral("ccif"));
    d.check("qp9: the CCIF preset", gen->params().value(0).toDouble() == 19000 && gen->params().value(1).toDouble() == 20000
                                        && gen->params().value(2).toDouble() == 1);
    gen->selectType(SiggenController::Sine);
    gen->setLevel(SiggenController::kDefaultLevel);
    again.w->close();
}

// ---------------------------------------------------------------- monitor

void monitorSection(SmokeDriver &d) {
    auto *mon = Registrar::instance<MonitorController>();
    d.check("qp9: the monitor's controller", mon != nullptr);
    if (!mon) return;
    const auto renameElsewhere = [&](const QByteArray &name) {
        d.app().core->worker()->post([name](FfiCore *c) { dspi_demo_rename_elsewhere(c, 0xFF, name.constData()); });
    };
    const auto hasLine = [mon](const QString &text) {
        for (const QString &l : mon->lines()->lines())
            if (l.contains(text)) return true;
        return false;
    };
    Qp9Window qp9Mon(d, "tools.interruptMonitor");
    d.check("qp9: Interrupt Monitor... opens its window", qp9Mon.w && qp9Mon.w->title() == QLatin1String("Interrupt Monitor"));
    if (!qp9Mon.w) return;
    d.check("qp9: listening", mon->status() == QLatin1String("listening")
                                  && qp9Mon.shownName("MonitorWindow.status") == QLatin1String("Listening"));
    renameElsewhere("Qp9 Monitor A");
    d.check("qp9: a notification shows as a line", d.waitFor(3000, [&] { return hasLine(QStringLiteral("Qp9 Monitor A")); }));
    const QRegularExpression timed(QStringLiteral("^\\d\\d:\\d\\d:\\d\\d\\.\\d{3}  \\S"));
    d.check("qp9: with its time in front", !mon->lines()->lines().isEmpty() && timed.match(mon->lines()->lines().last()).hasMatch());
    const int count = mon->lines()->rowCount();
    d.check("qp9: the count's words", qp9Mon.shownName("MonitorWindow.count")
                                          == (count == 1 ? QStringLiteral("1 event") : QStringLiteral("%1 events").arg(count)));

    // Pause drops what comes meanwhile; Resume goes on from now
    qp9Mon.click("MonitorWindow.pause");
    d.check("qp9: paused", mon->paused() && qp9Mon.shownName("MonitorWindow.status") == QLatin1String("Paused"));
    renameElsewhere("Qp9 Monitor B");
    d.spin(500);
    d.check("qp9: nothing arrives while paused", mon->lines()->rowCount() == count && !hasLine(QStringLiteral("Qp9 Monitor B")));
    qp9Mon.click("MonitorWindow.pause");
    d.spin(400);
    renameElsewhere("Qp9 Monitor C");
    d.check("qp9: after Resume new lines come", d.waitFor(3000, [&] { return hasLine(QStringLiteral("Qp9 Monitor C")); }));
    d.check("qp9: and the paused ones stay dropped", !hasLine(QStringLiteral("Qp9 Monitor B")));

    // Copy and Clear
    qp9Mon.click("MonitorWindow.copy");
    QClipboard *clip = QGuiApplication::clipboard();
    d.check("qp9: Copy puts every line on the clipboard", clip && clip->text() == mon->text()
                                                              && mon->text().count(QLatin1Char('\n')) == mon->lines()->rowCount());
    d.grab(qp9Mon.w, QStringLiteral("window-monitor.png"));
    d.check("qp9: the window has the keyboard", qp9Mon.focus());
    QTest::keyClick(qp9Mon.w, Qt::Key_K, Qt::ControlModifier);
    d.check("qp9: Ctrl+K clears", mon->lines()->rowCount() == 0);
    renameElsewhere("Qp9 Monitor D");
    d.check("qp9: after Clear only new lines", d.waitFor(3000, [&] { return hasLine(QStringLiteral("Qp9 Monitor D")); })
                                                   && !hasLine(QStringLiteral("Qp9 Monitor C")));
    d.check("qp9: the time format", MonitorController::shown(QStringLiteral("0\tx")).endsWith(QStringLiteral("  x"))
                                        && MonitorController::shown(QStringLiteral("no tab")) == QLatin1String("no tab"));
    renameElsewhere("");
    qp9Mon.w->close();
    d.spin(100);
    d.check("qp9: the monitor stops reading with its window", !mon->windowShown());
}

// ---------------------------------------------------------------- stats

void statsSection(SmokeDriver &d) {
    auto *stats = Registrar::instance<StatsController>();
    d.check("qp9: the stats controller", stats != nullptr);
    if (!stats) return;
    Qp9Window qp9Stats(d, "tools.stats");
    d.check("qp9: Stats for nerbs opens System Statistics", qp9Stats.w && qp9Stats.w->title() == QLatin1String("System Statistics"));
    if (!qp9Stats.w) return;
    Poller *poller = Poller::instance();
    d.check("qp9: the 2 s read arrives", d.waitFor(5000, [&] { return poller->counts(stats->slowPoll()).completed > 0; }));
    d.check("qp9: the 60 ms read arrives", d.waitFor(3000, [&] { return poller->counts(stats->fastPoll()).completed > 2; }));
    const QVariantMap device = stats->device();
    d.check("qp9: the device's facts", device.value(QStringLiteral("platform")).toString() != QLatin1String("-")
                                           && device.value(QStringLiteral("firmware")).toString().startsWith(QLatin1Char('v'))
                                           && qp9Stats.shownName("StatsWindow.platform") == device.value(QStringLiteral("platform")).toString());
    d.check("qp9: the system's numbers", stats->system().value(QStringLiteral("clock")).toString().endsWith(QStringLiteral(" MHz"))
                                             && stats->system().value(QStringLiteral("temperature")).toString().endsWith(QStringLiteral(" °C")));
    const QVariantList ring = stats->ringHealth();
    std::printf("smoke: qp9 stats: ring health %d rows, %d fill rows, clock diagnostics %d, interfaces s%d l%d a%d i%d\n",
                int(ring.size()), int(stats->fillRows().size()), int(stats->clockDiagnostics().size()),
                int(stats->spdifInput().size()), int(stats->lgSoundSync().size()), int(stats->adatOutput().size()),
                int(stats->i2sSlave().size()));
    d.check("qp9: ring health shows when the device reports it",
            ring.isEmpty() ? !qp9Stats.shows("StatsWindow.ringHeading")
                           : ring.size() == 7 && qp9Stats.shows("StatsWindow.ringHeading")
                                 && ring.first().toMap().value(QStringLiteral("text")).toString().contains(QStringLiteral(" frames (")));
    d.check("qp9: clock diagnostics only when built", stats->clockDiagnostics().isEmpty() == !qp9Stats.shows("StatsWindow.clockDiagnostics"));
    d.check("qp9: no USB error section without the export", stats->usbErrors().isEmpty() && !qp9Stats.shows("StatsWindow.usbErrors"));
    d.check("qp9: the connection footer", qp9Stats.shownName("StatsWindow.connection") == QLatin1String("Connected"));
    d.spin(400);
    const QVariantList rows = stats->fillRows();
    d.check("qp9: fill rows carry their history", rows.isEmpty()
                                                      || rows.first().toMap().value(QStringLiteral("history")).toList().size() == StatsController::kHistory);
    d.grab(qp9Stats.w, QStringLiteral("window-stats.png"));

    const int setters = d.app().device->settersRun();
    qp9Stats.click("StatsWindow.reset");
    d.check("qp9: Reset Watermarks writes once", d.waitFor(3000, [&] { return d.app().device->settersRun() == setters + 1; }));

    d.check("qp9: the elapsed format", StatsController::formatElapsed(7000) == QLatin1String("0m 07s")
                                           && StatsController::formatElapsed(3723000) == QLatin1String("1h 02m 03s"));
    d.check("qp9: the gap and latency formats", StatsController::gapText(950) == QStringLiteral("950 µs")
                                                    && StatsController::gapText(12500) == QLatin1String("12.5 ms")
                                                    && StatsController::latencyText(4000) == QLatin1String("4 ms")
                                                    && StatsController::latencyText(667) == QLatin1String("0.67 ms"));
    d.check("qp9: the fill bands", StatsController::spdifBand(50) == QLatin1String("good") && StatsController::spdifBand(10) == QLatin1String("warn")
                                       && StatsController::spdifBand(100) == QLatin1String("bad")
                                       && StatsController::pdmDmaBand(20) == QLatin1String("good") && StatsController::pdmDmaBand(60) == QLatin1String("bad")
                                       && StatsController::pdmRingBand(30) == QLatin1String("warn")
                                       && StatsController::ringBand(25, 128, 48) == QLatin1String("good")
                                       && StatsController::ringBand(95, 128, 48) == QLatin1String("bad"));
    qp9Stats.w->close();
    d.spin(100);
    const int slowAfter = poller->counts(stats->slowPoll()).posted;
    const int fastAfter = poller->counts(stats->fastPoll()).posted;
    d.spin(400);
    d.check("qp9: closed, it reads nothing", poller->counts(stats->fastPoll()).posted == fastAfter
                                                 && poller->counts(stats->slowPoll()).posted == slowAfter);
}

// ---------------------------------------------------------------- analyser

void spectrumSection(SmokeDriver &d) {
    auto *rta = Registrar::instance<RtaController>();
    auto *graph = Registrar::instance<GraphController>();
    auto *sidebar = Registrar::instance<SidebarController>();
    d.check("qp9: the analyser's controller", rta && graph && sidebar);
    if (!rta || !graph || !sidebar) return;
    QQuickWindow *main = d.mainWindow();
    d.check("qp9: the demo's analyser probed", d.waitFor(5000, [rta] { return rta->available(); }));
    sidebar->select(-1);
    d.spin(100);
    // A clean dashboard: its own selection, the graph on, the bars off
    rta->prefs()->insert(QStringLiteral("dashboardSource"), QString());
    rta->setShowGraph(true);
    rta->setShowBars(false);
    d.spin(100);
    d.check("qp9: the dashboard starts on the first output", rta->onDashboard() && rta->ownTap() == RTA_TAP_OUTPUT
                                                                 && rta->ownChannels() == QList<int>({ 0 }));
    d.check("qp9: the graph's overlay asks for it with the bins",
            d.waitFor(2000, [rta] { return rta->viewActive(QStringLiteral("graph")) && rta->request().mask == 1
                                           && rta->request().wants_bins && rta->request().tap == RTA_TAP_OUTPUT && rta->watching(); }));
    QQuickItem *surface = d.item(QStringLiteral("GraphHost.spectrum.surface"));
    d.check("qp9: the overlay is in the graph's slot", surface != nullptr);
    d.check("qp9: a picture arrives", d.waitFor(8000, [rta] { return rta->snapshot().count > 0 && rta->analysing(); }));
    d.check("qp9: and the overlay draws it", surface && d.waitFor(5000, [surface] { return surface->property("drawnCount").toInt() == 1; }));
    d.check("qp9: the settings reach the request", rta->request().fft_order == 10 && rta->request().avg_ms == 300
                                                       && rta->request().peak_decay_db_s == 12);

    // The options: chips, side, Clear, the switches
    QQuickItem *gear = d.item(QStringLiteral("GraphHost.options"));
    if (gear) QTest::mouseClick(main, Qt::LeftButton, Qt::NoModifier, gear->mapToScene(QPointF(gear->width() / 2, gear->height() / 2)).toPoint());
    d.spin(250);
    d.check("qp9: the gear's popover has the spectrum part", d.waitFor(1000, [&] {
        QObject *o = main->findChild<QObject *>(QStringLiteral("GraphHost.options.rta"));
        auto *it = qobject_cast<QQuickItem *>(o);
        return it && it->isVisible();
    }));
    d.grab(main, QStringLiteral("window-main-spectrum-options.png"));
    rta->toggleChannel(1);
    d.check("qp9: a chip adds a channel, stored", rta->ownChannels() == QList<int>({ 0, 1 })
                                                      && rta->prefs()->value(QStringLiteral("dashboardSource")).toString() == QLatin1String("out:0,1")
                                                      && rta->request().mask == 3);
    rta->switchSide(RTA_TAP_INPUT);
    d.check("qp9: the other side starts empty and asks for nothing", rta->ownTap() == RTA_TAP_INPUT && rta->ownChannels().isEmpty()
                                                                         && rta->request().mask == 0 && !rta->watching());
    rta->toggleChannel(0);
    rta->switchSide(RTA_TAP_OUTPUT);
    d.check("qp9: back on the outputs, the side remembered", rta->ownChannels() == QList<int>({ 0, 1 })
                                                                 && rta->prefs()->value(QStringLiteral("dashboardOtherSide")).toString() == QLatin1String("in:0"));
    if (gear) QTest::keyClick(main, Qt::Key_Escape);
    d.spin(150);

    // The bar strip
    rta->setShowBars(true);
    QQuickItem *strip = d.item(QStringLiteral("SpectrumStrip.dashboard"));
    d.check("qp9: RTA Bars shows the strip and registers it", strip && d.waitFor(2000, [&] {
        return strip->isVisible() && rta->viewActive(QStringLiteral("strip.dashboard"));
    }));
    QQuickItem *bars = d.item(QStringLiteral("SpectrumStrip.dashboard.bars.0"));
    d.check("qp9: the strip draws bars", bars && d.waitFor(5000, [bars] { return bars->property("drawnBars").toInt() > 3; }));
    rta->prefs()->insert(QStringLiteral("barColumns"), 2);
    rta->prefs()->insert(QStringLiteral("barHeight"), 96);
    d.check("qp9: the strip's layout", rta->barColumnsFor(1) == 1 && rta->barColumnsFor(3) == 2 && rta->barCellHeight(1) == 96
                                           && rta->barCellHeight(2) == 72 && rta->draggedBarHeight(96, 24, 1) == 120
                                           && rta->draggedBarHeight(96, 1000, 2) == 240 && rta->draggedBarHeight(96, -1000, 1) == 64
                                           && rta->draggedBarHeight(96, 15, 2) == 116);
    rta->prefs()->insert(QStringLiteral("barColumns"), 1);
    d.spin(100);
    d.grab(main, QStringLiteral("window-main-spectrum.png"));

    // The window: the page's spectrum, the mode from the page, the status bar
    Qp9Window qp9Rta(d, "tools.spectrum");
    d.check("qp9: Spectrum Analyser... opens its window", qp9Rta.w && qp9Rta.w->title() == QLatin1String("Spectrum Analyser"));
    if (qp9Rta.w) {
        d.check("qp9: both views, as the page shows both", qp9Rta.w->property("mode").toInt() == 2
                                                               && d.waitFor(2000, [rta] { return rta->viewActive(QStringLiteral("window.curves"))
                                                                                                 && rta->viewActive(QStringLiteral("window.bars")); }));
        d.check("qp9: the page's title", qp9Rta.shownName("SpectrumWindow.page") == QLatin1String("Dashboard"));
        d.check("qp9: the status bar runs", d.waitFor(3000, [&] {
            return qp9Rta.shownName("SpectrumWindow.running") == QLatin1String("Running")
                   && rta->statusParts().value(0).contains(QStringLiteral("each refreshed every"));
        }));
        qp9Rta.click("SpectrumWindow.legend.1");
        d.check("qp9: the legend hides a channel here only", qp9Rta.w->property("visibleChannels").toList().size() == 1
                                                                 && rta->ownChannels().size() == 2);
        d.check("qp9: the window's curves draw", d.waitFor(3000, [&] {
            QQuickItem *c = qp9Rta.item("SpectrumWindow.curves");
            return c && c->property("drawnCount").toInt() == 1;
        }));
        QQuickItem *qp9Curves = qp9Rta.item("SpectrumWindow.curves");
        QQuickItem *qp9Status = qp9Rta.item("SpectrumWindow.status");
        d.check("qp9: curves and bars share the height, the status bar fits", qp9Curves && qp9Status
                    && qp9Curves->height() > qp9Rta.w->height() / 4
                    && qp9Status->mapToScene(QPointF(qp9Status->width(), 0)).x() <= qp9Rta.w->width());
        d.grab(qp9Rta.w, QStringLiteral("window-spectrum.png"));
        qp9Rta.w->close();
        d.spin(150);
        d.check("qp9: closed, the window asks for nothing", !rta->viewActive(QStringLiteral("window.curves"))
                                                                && !rta->viewActive(QStringLiteral("window.bars")));
    }

    // A channel page: its own channel, its own switches
    const QVariantList qp9Outs = rta->channels(RTA_TAP_OUTPUT);
    const QVariantMap qp9Page = qp9Outs.value(qp9Outs.size() > 1 ? 1 : 0).toMap();
    const int qp9PageCh = qp9Page.value(QStringLiteral("channel")).toInt();
    sidebar->select(qp9Page.value(QStringLiteral("wire")).toInt());
    d.check("qp9: an output page starts on its channel", d.waitFor(2000, [rta, qp9PageCh] {
        return !rta->onDashboard() && rta->ownTap() == RTA_TAP_OUTPUT && rta->ownChannels() == QList<int>({ qp9PageCh });
    }));
    d.check("qp9: the page's request", d.waitFor(2000, [rta, qp9PageCh] { return rta->request().mask == (1 << qp9PageCh); }));
    rta->setShowGraph(false);
    rta->setShowBars(false);
    d.check("qp9: nothing watching, the analyser is released", d.waitFor(2000, [rta] { return rta->request().mask == 0 && !rta->watching(); }));
    d.check("qp9: and the device stops it", d.waitFor(8000, [rta] { return !rta->analysing(); }));
    rta->setShowGraph(true);
    rta->setShowBars(false);
    sidebar->select(-1);
    d.check("qp9: back on the dashboard, its selection again", d.waitFor(2000, [rta] {
        return rta->onDashboard() && rta->ownChannels() == QList<int>({ 0, 1 }) && rta->request().mask == 3;
    }));
    d.check("qp9: the short frequency labels", RtaController::shortHz(1000) == QLatin1String("1k")
                                                   && RtaController::shortHz(200) == QLatin1String("200"));
    RtaController::Selection parsed;
    d.check("qp9: selection keys", RtaController::Selection::parse(QStringLiteral("in:2,0"), &parsed)
                                       && parsed.tap == RTA_TAP_INPUT && parsed.channels == QList<int>({ 0, 2 })
                                       && !RtaController::Selection::parse(QStringLiteral("x:1"), nullptr)
                                       && !RtaController::Selection::parse(QStringLiteral("out:16"), nullptr));
    rta->setShowBars(false);
    d.spin(100);
}

// ---------------------------------------------------------------- following

void followSection(SmokeDriver &d) {
    using namespace linksmoke;
    auto *rta = Registrar::instance<RtaController>();
    auto *link = Registrar::instance<LinkController>();
    d.check("qp9: the analyser and Link controllers", rta && link);
    if (!rta || !link) return;
    FakeHubProcess hub;
    const bool up = hub.start({ "--rta", "--auth", "none" });
    d.check("qp9: the fake hub runs (DSPI_SMOKE_FAKEHUB)", up);
    if (!up) return;
    const auto settled = [&](LinkController &l) { return d.waitFor(30000, [&] { return !l.busy(); }); };
    const auto bind = [&](LinkController &l) {
        l.connectToHub(hub.url);
        if (!settled(l) || !d.waitFor(5000, [&] { return l.phase() == QLatin1String("choose_device") && !l.devices().isEmpty(); }))
            return false;
        const QVariantMap dev = l.devices().value(0).toMap();
        l.bindDevice(dev.value(QStringLiteral("handle")).toInt(), dev.value(QStringLiteral("serial")).toString());
        return settled(l) && d.waitFor(8000, [&] { return l.phase() == QLatin1String("bound"); });
    };
    d.check("qp9: the app binds the hub's device", bind(*link) && d.waitFor(5000, [&] { return d.app().device->remote(); }));
    d.check("qp9: its analyser probed", d.waitFor(10000, [rta] { return rta->available(); }));
    rta->setShowGraph(true);
    const QVariant qp9Source = rta->prefs()->value(QStringLiteral("dashboardSource"));
    const QVariant qp9OtherSide = rta->prefs()->value(QStringLiteral("dashboardOtherSide"));
    d.check("qp9: driving at first", d.waitFor(10000, [rta] { return rta->watching() && rta->analysing() && !rta->following(); }));
    const int ownTap = rta->ownTap();

    // Another console asks for the other side: it drives, the app follows
    const QString scratch = d.outDir() + QStringLiteral("/qp9-follow");
    QDir().mkpath(scratch);
    FfiCore *qp9Other = dspi_core_new();
    auto otherLink = std::make_unique<LinkController>(qp9Other, std::make_unique<TokenStore>(nullptr, scratch + QStringLiteral("/tokens.json")), nullptr);
    d.check("qp9: another console binds the device", bind(*otherLink));
    const int otherTap = ownTap == RTA_TAP_INPUT ? RTA_TAP_OUTPUT : RTA_TAP_INPUT;
    RtaRequest theirs{};
    theirs.tap = uint8_t(otherTap);
    theirs.mask = 0b11;
    theirs.fft_order = 10;
    theirs.avg_ms = 300;
    theirs.peak_decay_db_s = 12;
    dspi_rta_set_request(qp9Other, &theirs);
    dspi_rta_start(qp9Other);
    const auto otherFollows = [&] {
        RtaOwnership o{};
        dspi_rta_get_ownership(qp9Other, &o);
        return o.following;
    };
    d.check("qp9: the app follows the other console", d.waitFor(15000, [rta] { return rta->following(); }));
    d.check("qp9: and draws the device's selection", rta->drawnTap() == otherTap && rta->drawnChannels() == QList<int>({ 0, 1 })
                                                         && rta->ownTap() == ownTap);
    Qp9Window qp9Win(d, "tools.spectrum");
    d.check("qp9: the window says so", qp9Win.w && d.waitFor(3000, [&] {
        return qp9Win.shows("SpectrumWindow.following") && qp9Win.shows("SpectrumWindow.takeControl")
               && qp9Win.shownName("SpectrumWindow.following") == QLatin1String("Following another console");
    }));
    d.spin(1500);
    d.check("qp9: showing it takes nothing back", rta->following() && !otherFollows());
    if (qp9Win.w) d.grab(qp9Win.w, QStringLiteral("window-spectrum-following.png"));
    qp9Win.click("SpectrumWindow.takeControl");
    d.check("qp9: Take control drives again", d.waitFor(10000, [rta] { return !rta->following(); }));
    d.check("qp9: and the other console follows", d.waitFor(15000, otherFollows));
    d.check("qp9: the window's own selection again", rta->drawnTap() == ownTap);
    // A choice in the options takes it back too
    dspi_rta_set_request(qp9Other, &theirs);
    theirs.mask = 0b1;
    dspi_rta_set_request(qp9Other, &theirs);
    d.check("qp9: following once more", d.waitFor(15000, [rta] { return rta->following(); }));
    rta->switchSide(ownTap);
    d.check("qp9: a choice while following takes control", d.waitFor(10000, [rta] { return !rta->following(); }));

    if (qp9Win.w) qp9Win.w->close();
    // The dashboard's choice as it was
    RtaController::Selection qp9Back;
    if (RtaController::Selection::parse(qp9Source.toString(), &qp9Back)) {
        if (rta->ownTap() != qp9Back.tap) rta->switchSide(qp9Back.tap);
        for (int c : qp9Back.channels)
            if (!rta->ownChannels().contains(c)) rta->toggleChannel(c);
    }
    rta->prefs()->insert(QStringLiteral("dashboardOtherSide"), qp9OtherSide);
    otherLink->closeSession();
    otherLink.reset();
    dspi_core_free(qp9Other);
    link->closeSession();
    d.check("qp9: the app leaves the hub", d.waitFor(5000, [&] { return link->phase() == QLatin1String("idle") || link->phase() == QLatin1String("closed"); }));
    d.check("qp9: back on the demo", d.waitFor(8000, [&] { return d.app().device->connected() && !d.app().device->remote(); }));
}

} // namespace

static const SmokeDriver::Section qp9Siggen("qp9-siggen", 90, &siggenSection);
static const SmokeDriver::Section qp9Monitor("qp9-monitor", 91, &monitorSection);
static const SmokeDriver::Section qp9Stats("qp9-stats", 92, &statsSection);
static const SmokeDriver::Section qp9Spectrum("qp9-spectrum", 93, &spectrumSection);
static const SmokeDriver::Section qp9Follow("qp9-follow", 94, &followSection);
