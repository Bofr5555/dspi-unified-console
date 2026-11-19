// -- WP P5QUI -- Tools > Measure… on the demo DSPi with its simulated
// microphone (P5DEMO: the core's arithmetic room; nothing is played or
// captured by the host): a walked measurement from the setup to its results
// and its document, Stop stopping the generator; the live view with "cut this
// peak" applied as one undo step and taken back by Undo; the auto-tune
// proposed, applied guarded and reverted to the "before" state, then applied
// again and kept through the second confirmation the core asks for after a
// verify that did not pass, and the A/B compare. Each section leaves the demo
// as it found it (the volume back, every write undone or reverted).
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "HistoryController.h"
#include "MeasureController.h"
#include "Registrar.h"
#include "Shell.h"
#include "TuneController.h"

#include "CoreApi.h"

#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <cmath>
#include <cstring>
#include <utility>

namespace {

// What a tune or an adjustment may change, to compare before and after
struct Chain {
    QByteArray filters, xover, gains, delays, invert;
    bool operator==(const Chain &o) const {
        return filters == o.filters && xover == o.xover && gains == o.gains && delays == o.delays && invert == o.invert;
    }
};

Chain chainOf(DeviceModel *device) {
    Chain c;
    const DspState *s = device->state();
    if (!s) return c;
    c.filters = QByteArray(reinterpret_cast<const char *>(&s->filters), sizeof s->filters);
    c.xover = QByteArray(reinterpret_cast<const char *>(&s->xover), sizeof s->xover);
    c.gains = QByteArray(reinterpret_cast<const char *>(&s->output_gain_db), sizeof s->output_gain_db);
    c.delays = QByteArray(reinterpret_cast<const char *>(&s->output_delay_ms), sizeof s->output_delay_ms);
    c.invert = QByteArray(reinterpret_cast<const char *>(&s->matrix_invert), sizeof s->matrix_invert);
    return c;
}

// Equal but for the last digits a snapshot's recall may round (the A/B
// recalls a stored document, not the guard's image)
bool nearChain(const Chain &a, const Chain &b) {
    if (a.invert != b.invert || a.filters.size() != b.filters.size() || a.xover.size() != b.xover.size()) return false;
    const auto floats = [](const QByteArray &x, const QByteArray &y) {
        if (x.size() != y.size()) return false;
        for (int i = 0; i + 4 <= x.size(); i += 4) {
            float u, v;
            std::memcpy(&u, x.constData() + i, 4);
            std::memcpy(&v, y.constData() + i, 4);
            if (std::fabs(u - v) > 1e-3f * std::max(1.0f, std::fabs(u))) return false;
        }
        return true;
    };
    const auto params = [](const QByteArray &x, const QByteArray &y) {
        const int n = int(x.size() / sizeof(FilterParams));
        for (int i = 0; i < n; ++i) {
            FilterParams p, q;
            std::memcpy(&p, x.constData() + i * sizeof(FilterParams), sizeof p);
            std::memcpy(&q, y.constData() + i * sizeof(FilterParams), sizeof q);
            if (p.filter_type != q.filter_type || p.bypass != q.bypass) return false;
            for (const auto &[u, v] : { std::pair{ p.freq, q.freq }, { p.q, q.q }, { p.gain, q.gain }, { p.qp, q.qp } })
                if (std::fabs(u - v) > 1e-3f * std::max(1.0f, std::fabs(u))) return false;
        }
        return true;
    };
    return params(a.filters, b.filters) && params(a.xover, b.xover) && floats(a.gains, b.gains) && floats(a.delays, b.delays);
}

// Which part of the chain differs, for the log
QString diff(const Chain &a, const Chain &b) {
    QStringList out;
    const auto part = [&](const char *name, const QByteArray &x, const QByteArray &y, int stride) {
        if (x == y) return;
        for (int i = 0; i < x.size() && i < y.size(); ++i)
            if (x.at(i) != y.at(i)) {
                out << QStringLiteral("%1 at byte %2 (item %3)").arg(QLatin1String(name)).arg(i).arg(i / stride);
                return;
            }
    };
    part("filters", a.filters, b.filters, int(sizeof(FilterParams)));
    part("xover", a.xover, b.xover, int(sizeof(FilterParams)));
    part("gains", a.gains, b.gains, 4);
    part("delays", a.delays, b.delays, 4);
    part("invert", a.invert, b.invert, 1);
    return out.join(QStringLiteral(", "));
}

// The device read again, then its chain
Chain freshChain(SmokeDriver &d) {
    d.app().device->refresh();
    d.waitIdle();
    return chainOf(d.app().device);
}

void press(QObject *button) {
    if (button) QMetaObject::invokeMethod(button, "clicked");
}

QQuickWindow *measureWindow(SmokeDriver &d) {
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("tools.measure")));
    if (w) d.waitFor(2000, [w] { return w->isVisible(); });
    d.spin(100);
    return w;
}

QQuickItem *in(SmokeDriver &d, QQuickWindow *w, const QString &name) { return w ? d.item(name, w->contentItem()) : nullptr; }

void showTab(SmokeDriver &d, QQuickWindow *w, int index) {
    if (QQuickItem *tabs = in(d, w, QStringLiteral("Measure.tabs"))) tabs->setProperty("currentIndex", index);
    d.spin(150);
}

float gUserVolume = 0;
bool gVolumeSet = false;

// The demo at a volume a measurement may start at (S2: -12 dB or below); the
// smoke's own write, put back by the last section
bool readyDemo(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    if (!d.waitFor(10000, [&] { return device->connected() && device->demo(); }) || !d.waitIdle()) return false;
    if (!gVolumeSet && device->state()) {
        gUserVolume = device->state()->user_volume_db;
        gVolumeSet = true;
    }
    device->write(QStringLiteral("p5qui volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
    return d.waitIdle();
}

void restoreVolume(SmokeDriver &d) {
    if (!gVolumeSet) return;
    const float v = gUserVolume;
    d.app().device->write(QStringLiteral("p5qui volume"), [v](FfiCore *c) { return dspi_set_user_volume(c, v); });
    d.waitIdle();
}

// The setup the smoke measures: the layout's first speaker, at most three
// ways, short sweeps (the core's minimum of three passes for timing)
QList<int> chooseOutputs(MeasureController *mc) {
    QList<int> on;
    QString speaker;
    for (const QVariant &v : mc->setupOutputs()) {
        const QVariantMap m = v.toMap();
        if (!m.value(QStringLiteral("on")).toBool()) continue;
        if (speaker.isEmpty()) speaker = m.value(QStringLiteral("speaker")).toString();
        if (m.value(QStringLiteral("speaker")).toString() == speaker && on.size() < 3) on << m.value(QStringLiteral("output")).toInt();
    }
    for (const QVariant &v : mc->setupOutputs()) {
        const int o = v.toMap().value(QStringLiteral("output")).toInt();
        mc->setOutputOn(o, on.contains(o));
    }
    mc->setPlacement(QStringLiteral("listeningPosition"));
    mc->setSweepMs(1000);
    mc->setPasses(3);
    mc->setRunName(QStringLiteral("P5QUI smoke"));
    return on;
}

bool generatorIdle(SmokeDriver &d) {
    bool idle = false;
    d.app().core->worker()->call(d.app().device, [](FfiCore *c) {
        SiggenStatus sg{};
        return dspi_siggen_get_status(c, &sg) && sg.state == 0;
    }, [&idle](bool v) { idle = v; });
    d.waitIdle();
    d.spin(50);
    return idle;
}

// A measurement through the window: Check, then Measure, to its result
bool measure(SmokeDriver &d, QQuickWindow *w, MeasureController *mc, int limitMs) {
    showTab(d, w, 0);
    press(in(d, w, QStringLiteral("MeasureSetup.start")));
    if (!d.waitFor(5000, [&] { return mc->measuring(); })) return false;
    return d.waitFor(limitMs, [&] { return !mc->measuring() && !mc->running(); }) && d.waitFor(5000, [&] { return mc->hasResult(); });
}

void measureSection(SmokeDriver &d) {
    auto *mc = Registrar::instance<MeasureController>();
    d.check(QStringLiteral("p5qui: the demo DSPi at a volume to measure"), mc && readyDemo(d));
    if (!mc) return;
    QQuickWindow *w = measureWindow(d);
    d.check(QStringLiteral("p5qui: Tools > Measure… opens the Measure window"), w && w->objectName() == QLatin1String("MeasureWindow"));
    if (!w) return;
    d.check(QStringLiteral("p5qui: the menu has the row"), d.app().shell->handles(QStringLiteral("tools.measure")));
    d.check(QStringLiteral("p5qui: the demo measures with its simulated microphone, labelled"),
            d.waitFor(5000, [&] { return mc->simAvailable() && mc->micChoice() == QLatin1String("sim"); }) &&
                mc->simLabel() == QLatin1String("Simulated microphone"));
    d.check(QStringLiteral("p5qui: the setup is the device's layout"),
            d.waitFor(5000, [&] { return !mc->setupProblem().contains(QLatin1String("at least one")); }));
    const QList<int> outs = chooseOutputs(mc);
    std::printf("smoke: p5qui: measuring outputs %s\n",
                qPrintable(QStringList([&] { QStringList l; for (int o : outs) l << QString::number(o); return l; }()).join(QLatin1Char(','))));
    d.check(QStringLiteral("p5qui: the setup is complete"), !outs.isEmpty() && mc->setupProblem().isEmpty());

    // The preflight in words
    QSignalSpy pre(mc, &MeasureController::preflightChanged);
    press(in(d, w, QStringLiteral("MeasureSetup.preflight")));
    const bool checked = d.waitFor(5000, [&] { return mc->preflightDone(); });
    if (!mc->preflightClear())
        for (const QVariant &l : mc->preflightLines()) std::printf("smoke: p5qui: preflight: %s\n", qPrintable(l.toMap().value(QStringLiteral("text")).toString()));
    d.check(QStringLiteral("p5qui: the preflight is clear on the demo at -20 dB"), checked && mc->preflightClear());

    // The run, with the Stop bar shown
    QElapsedTimer took;
    took.start();
    showTab(d, w, 0);
    press(in(d, w, QStringLiteral("MeasureSetup.start")));
    d.check(QStringLiteral("p5qui: Measure starts the run and the Stop bar shows"),
            d.waitFor(5000, [&] { return mc->measuring(); }) && in(d, w, QStringLiteral("Measure.stopBar")) &&
                d.waitFor(1000, [&] { return in(d, w, QStringLiteral("Measure.stopBar"))->isVisible(); }));
    QStringList phases;
    const bool ended = d.waitFor(110000, [&] {
        if (phases.isEmpty() || phases.last() != mc->phase()) phases << mc->phase();
        return !mc->measuring();
    });
    std::printf("smoke: p5qui: the run took %.0f s: %s; %s\n", took.elapsed() / 1000.0, qPrintable(phases.join(QStringLiteral(" > "))),
                qPrintable(mc->timingText()));
    if (mc->phase() != QLatin1String("done")) std::printf("smoke: p5qui: issue: %s\n", qPrintable(mc->issueText()));
    d.check(QStringLiteral("p5qui: the run went from the pre-roll through the ramp to done"),
            ended && mc->phase() == QLatin1String("done") && phases.contains(QLatin1String("ramp")) && phases.contains(QLatin1String("run")));
    d.check(QStringLiteral("p5qui: the timing verdict is said in words"), mc->timingText().startsWith(QLatin1String("Timing ")));
    d.check(QStringLiteral("p5qui: the result is taken and the microphone closed"),
            d.waitFor(5000, [&] { return mc->hasResult() && !mc->running(); }) && mc->resultFrom() == QLatin1String("run"));

    // The results: one curve an output, drawn
    showTab(d, w, 1);
    const QVariantList series = mc->resultGraph().value(QStringLiteral("series")).toList();
    QQuickItem *graph = in(d, w, QStringLiteral("MeasureResults.graph"));
    d.check(QStringLiteral("p5qui: Results draw one measured curve an output"),
            series.size() == outs.size() && graph && d.waitFor(2000, [&] { return graph->property("drawnSeries").toInt() == outs.size(); }));
    d.check(QStringLiteral("p5qui: Results say the placement, the microphone and the timing"),
            mc->resultSummary().contains(QLatin1String("Listening position")) && !mc->resultTiming().isEmpty() &&
                mc->resultOutputs().size() == outs.size());
    QDir().mkpath(d.outDir() + QStringLiteral("/p5qui"));
    d.check(QStringLiteral("p5qui: screenshot of the results"), d.grab(w, QStringLiteral("p5qui/results.png")));
    // An output hidden and shown again
    mc->setOutputShown(outs.first(), false);
    const int hiddenCount = mc->resultGraph().value(QStringLiteral("series")).toList().size();
    mc->setOutputShown(outs.first(), true);
    d.check(QStringLiteral("p5qui: an output can be hidden from the graph"),
            hiddenCount == outs.size() - 1 && mc->resultGraph().value(QStringLiteral("series")).toList().size() == outs.size());

    // The document: filed here, listed, exported as REW text
    QSignalSpy stored(mc, &MeasureController::stored);
    press(in(d, w, QStringLiteral("MeasureResults.save")));
    const auto storedOk = [&](const QString &op) {
        return d.waitFor(8000, [&] {
            for (const QList<QVariant> &a : stored)
                if (a.value(0).toString() == op) return a.value(1).toBool();
            return false;
        });
    };
    d.check(QStringLiteral("p5qui: the measurement is saved in this console"), storedOk(QStringLiteral("save")));
    d.check(QStringLiteral("p5qui: the documents list it"), d.waitFor(8000, [&] {
        for (const QVariant &v : mc->documents())
            if (v.toMap().value(QStringLiteral("name")).toString() == QLatin1String("P5QUI smoke") &&
                v.toMap().value(QStringLiteral("kind")).toString() == QLatin1String("measurement"))
                return true;
        return false;
    }));
    const QString rew = d.outDir() + QStringLiteral("/p5qui/rew.txt");
    mc->exportRewTo(outs.first(), rew);
    d.check(QStringLiteral("p5qui: an output exported as REW text"), storedOk(QStringLiteral("rew")) && d.waitFor(3000, [&] {
        QFile f(rew);
        return f.open(QIODevice::ReadOnly) && f.readAll().contains("Freq");
    }));
    const QString docPath = d.outDir() + QStringLiteral("/p5qui/measurement.dspipreset");
    mc->saveResultTo(docPath);
    const QByteArray kept = mc->resultDocument();
    mc->clearResult();
    mc->openResultFile(docPath);
    d.check(QStringLiteral("p5qui: the exported document opens again as the result"),
            storedOk(QStringLiteral("open")) && d.waitFor(3000, [&] { return mc->hasResult() && mc->resultDocument() == kept; }));

    // Stop: the run ends at once, the generator idle, nothing kept
    showTab(d, w, 0);
    press(in(d, w, QStringLiteral("MeasureSetup.start")));
    const bool playing = d.waitFor(15000, [&] { return mc->phase() == QLatin1String("ramp") || mc->phase() == QLatin1String("run"); });
    press(in(d, w, QStringLiteral("Measure.stop")));
    d.check(QStringLiteral("p5qui: Stop ends the run, nothing kept, and the generator is idle"),
            playing && d.waitFor(5000, [&] { return !mc->running(); }) && mc->phase() == QLatin1String("aborted") &&
                !mc->hasResult() && generatorIdle(d));
    // The measurement back for the tune (from its file)
    mc->openResultFile(docPath);
    d.waitFor(3000, [&] { return mc->hasResult(); });
}

void liveSection(SmokeDriver &d) {
    auto *mc = Registrar::instance<MeasureController>();
    auto *history = Registrar::instance<HistoryController>();
    if (!mc || !history || !readyDemo(d)) {
        d.check(QStringLiteral("p5qui: live: the demo is ready"), false);
        return;
    }
    QQuickWindow *w = measureWindow(d);
    QDir().mkpath(d.outDir() + QStringLiteral("/p5qui"));
    if (!w || !in(d, w, QStringLiteral("MeasureLive.graph"))) {
        d.check(QStringLiteral("p5qui: live: the Measure window"), false);
        return;
    }
    showTab(d, w, 2);
    // The first measured output and its way
    int output = 0;
    QString role = QStringLiteral("full");
    for (const QVariant &v : mc->setupOutputs())
        if (v.toMap().value(QStringLiteral("on")).toBool()) {
            output = v.toMap().value(QStringLiteral("output")).toInt();
            role = v.toMap().value(QStringLiteral("role")).toString();
            break;
        }
    mc->liveStart({ { QStringLiteral("output"), output },
                    { QStringLiteral("role"), role },
                    { QStringLiteral("mode"), QStringLiteral("sweep") },
                    { QStringLiteral("placement"), QStringLiteral("listeningPosition") },
                    { QStringLiteral("smoothing"), 6 },
                    { QStringLiteral("sweepMs"), 1000 } });
    const bool frames = d.waitFor(60000, [&] { return mc->liveActive() && mc->liveSeq() >= 1 && mc->liveMeasured(); });
    if (!frames) std::printf("smoke: p5qui: live: %s %s %s\n", qPrintable(mc->liveText()), qPrintable(mc->liveIssueText()), qPrintable(mc->error()));
    d.check(QStringLiteral("p5qui: the live view measures a curve a sweep"), frames);
    QQuickItem *graph = in(d, w, QStringLiteral("MeasureLive.graph"));
    d.check(QStringLiteral("p5qui: the live graph draws the measured curve and the target (dashed)"),
            graph && d.waitFor(3000, [&] {
                const QStringList kinds = graph->property("drawnKinds").toStringList();
                return kinds.contains(QLatin1String("measured")) && kinds.contains(QLatin1String("target"));
            }));

    // "Cut this peak" at the highest point: a preview first, nothing written
    const Chain before = chainOf(d.app().device);
    const bool couldUndo = history->canUndo();
    const double peak = mc->livePeakHz();
    mc->cutPeak(peak > 0 ? peak : 1000.0);
    bool proposed = d.waitFor(8000, [&] { return mc->proposal().contains(QStringLiteral("id")); });
    if (proposed && mc->proposal().value(QStringLiteral("changes")).toStringList().isEmpty()) {
        // Nothing to cut there: fit a range instead
        std::printf("smoke: p5qui: live: no cut at %.0f Hz (%s), fitting 100 Hz to 2 kHz\n", peak,
                    qPrintable(mc->proposal().value(QStringLiteral("reason")).toString()));
        mc->fitRange(100, 2000);
        proposed = d.waitFor(8000, [&] { return !mc->proposal().value(QStringLiteral("changes")).toStringList().isEmpty(); });
    }
    d.check(QStringLiteral("p5qui: an adjustment is previewed in words with its predicted curve (dotted), nothing written"),
            proposed && !mc->proposal().value(QStringLiteral("changes")).toStringList().isEmpty() &&
                d.waitFor(3000, [&] { return graph->property("drawnKinds").toStringList().contains(QLatin1String("predicted")); }) &&
                chainOf(d.app().device) == before);
    d.check(QStringLiteral("p5qui: screenshot of the live view"), d.grab(w, QStringLiteral("p5qui/live.png")));
    QSignalSpy stored(mc, &MeasureController::stored);
    press(in(d, w, QStringLiteral("MeasureLive.apply")));
    const bool applied = d.waitFor(8000, [&] {
        for (const QList<QVariant> &a : stored)
            if (a.value(0).toString() == QLatin1String("live-apply")) return a.value(1).toBool();
        return false;
    });
    d.waitIdle();
    d.check(QStringLiteral("p5qui: Apply writes it as one undo step"),
            applied && !(chainOf(d.app().device) == before) && d.waitFor(3000, [&] { return history->canUndo(); }) &&
                mc->liveSaid().startsWith(QLatin1String("Applied:")));
    press(in(d, w, QStringLiteral("MeasureLive.undo")));
    d.check(QStringLiteral("p5qui: Undo takes the adjustment back"),
            d.waitFor(5000, [&] { return d.waitIdle(500) && chainOf(d.app().device) == before; }) &&
                (couldUndo || d.waitFor(3000, [&] { return !history->canUndo(); })));
    press(in(d, w, QStringLiteral("MeasureLive.stop")));
    d.check(QStringLiteral("p5qui: Stop Live stops the generator"),
            d.waitFor(5000, [&] { return !mc->liveActive() && !mc->running(); }) && generatorIdle(d));
}

void tuneSection(SmokeDriver &d) {
    auto *mc = Registrar::instance<MeasureController>();
    auto *tc = Registrar::instance<TuneController>();
    if (!mc || !tc || !readyDemo(d)) {
        d.check(QStringLiteral("p5qui: tune: the demo is ready"), false);
        return;
    }
    QQuickWindow *w = measureWindow(d);
    if (!w || !in(d, w, QStringLiteral("MeasureTune.graph"))) {
        d.check(QStringLiteral("p5qui: tune: the Measure window"), false);
        return;
    }
    if (!mc->hasResult()) {
        chooseOutputs(mc);
        measure(d, w, mc, 110000);
    }
    d.check(QStringLiteral("p5qui: tune: a measurement to tune from"), mc->hasResult());
    showTab(d, w, 3);
    QDir().mkpath(d.outDir() + QStringLiteral("/p5qui"));
    // The layout: the device's, the measured outputs on
    tc->reloadLayout();
    QList<int> measured;
    for (const QVariant &v : mc->resultOutputs()) measured << v.toMap().value(QStringLiteral("output")).toInt();
    const auto onAre = [&] {
        for (const QVariant &v : tc->layout())
            if (v.toMap().value(QStringLiteral("on")).toBool() != measured.contains(v.toMap().value(QStringLiteral("output")).toInt())) return false;
        return true;
    };
    d.check(QStringLiteral("p5qui: the tune's layout is the device's, the measured outputs on, with each output's programme feed"),
            d.waitFor(5000, [&] { return !tc->layout().isEmpty() && onAre(); }) &&
                !tc->layout().first().toMap().value(QStringLiteral("feed")).toString().isEmpty());
    QSignalSpy steps(tc, &TuneController::stepDone);
    const auto step = [&](const QString &name, int ms) {
        return d.waitFor(ms, [&] {
            for (const QList<QVariant> &a : steps)
                if (a.value(0).toString() == name) return true;
            return false;
        });
    };
    const auto stepOk = [&](const QString &name) {
        for (const QList<QVariant> &a : steps)
            if (a.value(0).toString() == name) return a.value(1).toBool();
        return false;
    };
    press(in(d, w, QStringLiteral("MeasureTune.propose")));
    const bool proposed = step(QStringLiteral("propose"), 30000) && stepOk(QStringLiteral("propose"));
    if (!proposed) std::printf("smoke: p5qui: tune: %s\n", qPrintable(tc->error()));
    std::printf("smoke: p5qui: tune: %d write(s), sum check %s; %s\n", int(tc->writes().size()), qPrintable(tc->sumCheck()),
                qPrintable(tc->notes().join(QStringLiteral(" | "))));
    d.check(QStringLiteral("p5qui: a tune is proposed in words, nothing written"),
            proposed && tc->hasProposal() && tc->stage() == QLatin1String("proposed") &&
                (tc->writes().isEmpty() || !tc->writes().first().toMap().value(QStringLiteral("words")).toString().isEmpty()));
    QQuickItem *graph = in(d, w, QStringLiteral("MeasureTune.graph"));
    d.check(QStringLiteral("p5qui: the proposal's graph draws before, predicted (dotted) and the target (dashed)"),
            graph && d.waitFor(3000, [&] {
                const QStringList k = graph->property("drawnKinds").toStringList();
                return k.contains(QLatin1String("measured")) && k.contains(QLatin1String("predicted")) && k.contains(QLatin1String("target"));
            }));
    if (tc->needsSumCheck()) {
        steps.clear();
        press(in(d, w, QStringLiteral("MeasureTune.sumCheckRun")));
        const bool ran = step(QStringLiteral("sumCheck"), 110000);
        d.check(QStringLiteral("p5qui: the summed check runs to its verdict, said in words"),
                ran && (tc->sumCheck() == QLatin1String("passed") || tc->sumCheck() == QLatin1String("failed")) && !tc->sumCheckText().isEmpty() &&
                    d.waitFor(5000, [&] { return !mc->running(); }));
    } else {
        d.notRun(QStringLiteral("p5qui: the summed check"), QStringLiteral("the demo room's proposal has no timing write (") + tc->sumCheckText() + QStringLiteral(")"));
    }
    d.check(QStringLiteral("p5qui: screenshot of the proposal"), d.grab(w, QStringLiteral("p5qui/tune-proposal.png")));
    if (tc->writes().isEmpty()) {
        d.notRun(QStringLiteral("p5qui: the tune applied and reverted"), QStringLiteral("the proposal changes nothing"));
        return;
    }

    // Applied guarded, then reverted: everything back as it was
    const Chain before = freshChain(d);
    steps.clear();
    tc->apply(0);
    const bool applied = step(QStringLiteral("apply"), 15000) && stepOk(QStringLiteral("apply"));
    d.waitIdle();
    if (!applied) std::printf("smoke: p5qui: tune: apply: %s\n", qPrintable(tc->error()));
    d.check(QStringLiteral("p5qui: Apply writes the tune inside the guard, with its countdown"),
            applied && tc->applied() && !(chainOf(d.app().device) == before) && tc->guardRemainingS() > 0 &&
                d.waitFor(3000, [&] { return tc->guardText().contains(QLatin1String("Keep or Revert")); }));
    d.check(QStringLiteral("p5qui: screenshot of the applied tune"), d.grab(w, QStringLiteral("p5qui/tune-applied.png")));
    steps.clear();
    press(in(d, w, QStringLiteral("MeasureTune.revert")));
    const bool reverted = step(QStringLiteral("revert"), 15000) && stepOk(QStringLiteral("revert")) && d.waitIdle() &&
                          d.waitFor(5000, [&] { return tc->stage() == QLatin1String("reverted"); });
    const Chain after = freshChain(d);
    if (!(after == before)) std::printf("smoke: p5qui: tune: after Revert: %s\n", qPrintable(diff(before, after)));
    d.check(QStringLiteral("p5qui: Revert puts the device back as it was before the tune"), reverted && after == before);

    // Applied again, then Keep after a verify that did not pass: the core's
    // refusal (fed through the seam, as the core answers it) opens the
    // question; Keep again asks again; nothing keeps until the person answers
    steps.clear();
    tc->propose();
    if (!step(QStringLiteral("propose"), 30000) || !stepOk(QStringLiteral("propose")))
        std::printf("smoke: p5qui: tune: propose again: stage %s, %s\n", qPrintable(tc->stage()), qPrintable(tc->error()));
    if (tc->needsSumCheck()) {
        tc->sumCheckRun();
        step(QStringLiteral("sumCheck"), 110000);
        d.waitFor(5000, [&] { return !mc->running(); });
    }
    steps.clear();
    tc->apply(0);
    const bool again = step(QStringLiteral("apply"), 15000) && stepOk(QStringLiteral("apply")) && d.waitIdle() && tc->applied();
    if (!again) std::printf("smoke: p5qui: tune: apply again: stage %s, %s\n", qPrintable(tc->stage()), qPrintable(tc->error()));
    d.check(QStringLiteral("p5qui: the tune applied again"), again);
    if (!again) return;
    // The verify inside the guard, with the countdown running
    steps.clear();
    press(in(d, w, QStringLiteral("MeasureTune.verifyRun")));
    const bool verified = step(QStringLiteral("verify"), 110000) && d.waitFor(5000, [&] { return !mc->running(); });
    std::printf("smoke: p5qui: tune: verify: %s\n", qPrintable(tc->verifyText()));
    d.check(QStringLiteral("p5qui: the verify runs inside the guard and says its verdict in words"),
            verified && tc->verifyDone() && !tc->verifyText().isEmpty() && tc->applied() && tc->guardRemainingS() > 0);
    const QString refusal = QStringLiteral(
        "measure.tune.keep_unverified: the verify did not confirm the tune; Keep it only if you are sure (call Keep again to keep it anyway, or Revert)");
    QJsonObject state = QJsonDocument::fromJson([&] {
        QByteArray out;
        d.app().core->worker()->call(d.app().device, [](FfiCore *c) {
            QByteArray b(1 << 18, '\0');
            const uint32_t n = dspi_measure_tune_state(c, b.data(), uint32_t(b.size()));
            if (n > 0 && n <= uint32_t(b.size())) b.truncate(int(n) - 1);
            return b;
        }, [&out](QByteArray r) { out = r; });
        d.waitIdle();
        d.spin(50);
        return out;
    }()).object();
    state.insert(QStringLiteral("issue"), QJsonObject{ { QStringLiteral("key"), QStringLiteral("measure.tune.keep_unverified") },
                                                       { QStringLiteral("message"), refusal } });
    tc->keepAnswered(QJsonObject{ { QStringLiteral("ok"), false }, { QStringLiteral("code"), QStringLiteral("invalid") },
                                  { QStringLiteral("error"), refusal } },
                     state);
    d.spin(200);
    QObject *question = nullptr;
    for (QObject *o : w->findChildren<QObject *>(QStringLiteral("MeasureTune.keepQuestion"))) question = o;
    if (!question)
        if (QQuickItem *overlay = w->contentItem()->parentItem())
            question = overlay->findChild<QObject *>(QStringLiteral("MeasureTune.keepQuestion"));
    const auto questionOpen = [&] { return question && question->property("opened").toBool(); };
    d.check(QStringLiteral("p5qui: a refused Keep opens a clear second question, and nothing is kept"),
            tc->keepConfirmNeeded() && tc->keepRefusals() == 1 && d.waitFor(2000, questionOpen) &&
                tc->stage() != QLatin1String("kept") && tc->applied() && !tc->keepConfirmText().isEmpty());
    d.check(QStringLiteral("p5qui: screenshot of the second question"), d.grab(w, QStringLiteral("p5qui/tune-keep-question.png")));
    // "Not Now": the guard runs on; Keep pressed again asks again, never keeps by itself
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.spin(300);
    const bool stillApplied = tc->applied() && !tc->keepConfirmNeeded();
    press(in(d, w, QStringLiteral("MeasureTune.keep")));
    d.spin(400);
    d.check(QStringLiteral("p5qui: Not Now keeps nothing; Keep again asks again instead of keeping"),
            stillApplied && tc->keepConfirmNeeded() && questionOpen() && tc->applied() && tc->stage() != QLatin1String("kept") &&
                tc->keepRefusals() == 1);
    // The person's explicit answer keeps it. After a real verify that did not
    // pass the core refuses that Keep itself (its first): the question comes
    // again, and the person's second answer keeps it
    steps.clear();
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    bool keptNow = step(QStringLiteral("keep"), 15000) && stepOk(QStringLiteral("keep"));
    if (!keptNow && !tc->verifyPassed()) {
        d.check(QStringLiteral("p5qui: the core's own refusal after the failed verify asks again"),
                tc->keepConfirmNeeded() && tc->keepRefusals() == 2 && d.waitFor(2000, questionOpen) && tc->applied());
        steps.clear();
        QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
        keptNow = step(QStringLiteral("keep"), 15000) && stepOk(QStringLiteral("keep"));
    }
    d.check(QStringLiteral("p5qui: Keep Anyway, the person's answer, keeps it with its snapshots"),
            keptNow && tc->stage() == QLatin1String("kept") && !tc->keptText().isEmpty());

    // The A/B compare of the tune (A, the device now) and "before" (B),
    // switched once and ended on "before"
    steps.clear();
    press(in(d, w, QStringLiteral("MeasureTune.abStart")));
    const bool ab = step(QStringLiteral("ab"), 15000) && stepOk(QStringLiteral("ab")) && tc->abActive();
    d.check(QStringLiteral("p5qui: the A/B compare of before and after starts"), ab);
    if (ab) {
        const QString side = tc->abSide();
        steps.clear();
        press(in(d, w, QStringLiteral("MeasureTune.abToggle")));
        d.check(QStringLiteral("p5qui: A/B switches side"), step(QStringLiteral("ab"), 15000) && tc->abActive() && tc->abSide() != side);
        steps.clear();
        tc->abKeep(QStringLiteral("b"));
        const bool ended = step(QStringLiteral("ab"), 15000) && !tc->abActive() && d.waitIdle();
        const Chain now = freshChain(d);
        if (!(now == before)) std::printf("smoke: p5qui: tune: after A/B on B: %s\n", qPrintable(diff(before, now)));
        d.check(QStringLiteral("p5qui: the compare ends on the before side: the device as before the tune"), ended && nearChain(now, before));
    }
    if (!nearChain(chainOf(d.app().device), before)) std::printf("smoke: p5qui: tune: the demo was left tuned\n");
    w->close();
    restoreVolume(d);
}

const SmokeDriver::Section measureS("p5qui-measure", 210, &measureSection);
const SmokeDriver::Section liveS("p5qui-live", 211, &liveSection);
const SmokeDriver::Section tuneS("p5qui-tune", 212, &tuneSection);

} // namespace
