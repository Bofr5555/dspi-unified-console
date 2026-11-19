// -- WP P5QUI -- Tools > Measure… on the demo DSPi with its simulated
// microphone (P5DEMO: the core's arithmetic room; nothing is played or
// captured by the host): a walked measurement from the setup to its results
// and its document, Stop stopping the generator; the live view with "cut this
// peak" applied as one undo step and taken back by Undo; the auto-tune
// proposed, applied guarded and reverted to the "before" state, then applied
// again and kept through the second confirmation the core asks for after a
// verify that did not pass, and the A/B compare. Each section leaves the demo
// as it found it (the volume back, every write undone or reverted).
//
// -- WP P5QUI2 -- Results through the core's curves (smoothing, the levelled
// target); review P5-D's three fixes: a Stop or a closed window withdraws a
// start on its way (F5), a hub's Viewer is offered no tab that plays (F6, on
// a fake hub), Revert withdraws a delayed "Keep Anyway" (F7); the passive
// microphone RTA in the client read-only mode, fed by a sine source through
// AudioCapture's seam (frames arrive, the generator idle, nothing written).
#include "SmokeDriver.h"

#include "LinkController.h"
#include "LinkFakeHub.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "HistoryController.h"
#include "MeasureController.h"
#include "Registrar.h"
#include "Shell.h"
#include "TuneController.h"

#include "CoreApi.h"

#include <QAudioFormat>
#include <QDir>
#include <QElapsedTimer>
#include <QIODevice>
#include <QTimer>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <cmath>
#include <cstring>
#include <memory>
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

// -- WP P5QUI2 -- how many series of `kind` a graph has
int kindCount(const QVariantMap &graph, const char *kind) {
    int n = 0;
    for (const QVariant &v : graph.value(QStringLiteral("series")).toList())
        if (v.toMap().value(QStringLiteral("kind")).toString() == QLatin1String(kind)) ++n;
    return n;
}

QVariantList firstPoints(const QVariantMap &graph) {
    for (const QVariant &v : graph.value(QStringLiteral("series")).toList())
        if (v.toMap().value(QStringLiteral("kind")).toString() == QLatin1String("measured")) return v.toMap().value(QStringLiteral("points")).toList();
    return {};
}

// -- WP P5QUI2 -- Results through dspi_measure_doc_curves: 1/6 octave and the
// levelled target by default; another smoothing and the target off are the
// core's answers too; back to the defaults at the end
void resultCurvesChecks(SmokeDriver &d, QQuickWindow *w, MeasureController *mc, int outs) {
    QQuickItem *graph = in(d, w, QStringLiteral("MeasureResults.graph"));
    d.check(QStringLiteral("p5qui2: Results are the core's curves: 1/6 octave and the target levelled to each output by default"),
            mc->resultSmoothing() == 6 && mc->resultTarget() && mc->resultCurvesSmoothing() == 6 &&
                kindCount(mc->resultGraph(), "target") == outs && graph &&
                d.waitFor(2000, [&] { return graph->property("drawnKinds").toStringList().contains(QLatin1String("target")); }));
    QQuickItem *note = in(d, w, QStringLiteral("MeasureResults.curvesNote"));
    d.check(QStringLiteral("p5qui2: Results say the core's smoothing in words"),
            note && note->property("text").toString() == QLatin1String("Smoothed by the core: 1/6 octave."));
    const QVariantList sixth = firstPoints(mc->resultGraph());
    // The choice from the window's own control
    QQuickItem *combo = in(d, w, QStringLiteral("MeasureResults.smoothing"));
    if (combo) QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, 0));
    d.check(QStringLiteral("p5qui2: no smoothing: the core's curves as stored, drawn"),
            combo && d.waitFor(8000, [&] { return mc->resultCurvesSmoothing() == 0; }) && mc->resultSmoothing() == 0 &&
                kindCount(mc->resultGraph(), "measured") == outs && firstPoints(mc->resultGraph()) != sixth && !sixth.isEmpty());
    mc->setResultSmoothing(3);
    d.check(QStringLiteral("p5qui2: 1/3 octave from the core"), d.waitFor(8000, [&] { return mc->resultCurvesSmoothing() == 3; }));
    QQuickItem *target = in(d, w, QStringLiteral("MeasureResults.target"));
    if (target) {
        target->setProperty("checked", false);
        QMetaObject::invokeMethod(target, "toggled");
    }
    d.check(QStringLiteral("p5qui2: the target switched off: none drawn"),
            target && d.waitFor(8000, [&] { return !mc->resultTarget() && mc->resultCurvesSmoothing() == 3 && kindCount(mc->resultGraph(), "target") == 0 &&
                                                   kindCount(mc->resultGraph(), "measured") == outs; }) &&
                d.waitFor(2000, [&] { return !graph->property("drawnKinds").toStringList().contains(QLatin1String("target")); }));
    mc->setResultTarget(true);
    mc->setResultSmoothing(6);
    d.check(QStringLiteral("p5qui2: back to 1/6 octave with the target"),
            d.waitFor(8000, [&] { return mc->resultCurvesSmoothing() == 6 && kindCount(mc->resultGraph(), "target") == outs; }) &&
                firstPoints(mc->resultGraph()) == sixth);
}

// -- WP P5QUI2 -- the tune's state as the core has it now
QJsonObject tuneStateOf(SmokeDriver &d) {
    QByteArray out;
    d.app().core->worker()->call(d.app().device, [](FfiCore *c) {
        QByteArray b(1 << 18, '\0');
        const uint32_t n = dspi_measure_tune_state(c, b.data(), uint32_t(b.size()));
        if (n > 0 && n <= uint32_t(b.size())) b.truncate(int(n) - 1);
        else b.clear();
        return b;
    }, [&out](QByteArray r) { out = r; });
    d.waitIdle();
    d.spin(50);
    return QJsonDocument::fromJson(out).object();
}

// -- WP P5QUI2 -- a JSON call of the core on its worker (the smoke's reads)
QJsonObject coreJson(SmokeDriver &d, std::function<uint32_t(FfiCore *, char *, uint32_t)> call) {
    QByteArray out;
    d.app().core->worker()->call(d.app().device, [call](FfiCore *c) {
        QByteArray b(1 << 18, '\0');
        const uint32_t n = call(c, b.data(), uint32_t(b.size()));
        if (n > 0 && n <= uint32_t(b.size())) b.truncate(int(n) - 1);
        else b.clear();
        return b;
    }, [&out](QByteArray r) { out = r; });
    d.waitIdle();
    d.spin(50);
    return QJsonDocument::fromJson(out).object();
}

bool coreBool(SmokeDriver &d, std::function<bool(FfiCore *)> call) {
    bool v = false;
    d.app().core->worker()->call(d.app().device, [call](FfiCore *c) { return call(c); }, [&v](bool r) { v = r; });
    d.waitIdle();
    d.spin(50);
    return v;
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
    // -- WP P5FOLLOW -- (coordinator, headphone safety) no placement is
    // assumed: Measure and Check wait for the person's choice, said in words
    {
        showTab(d, w, 0);
        QQuickItem *startButton = in(d, w, QStringLiteral("MeasureSetup.start"));
        QQuickItem *checkButton = in(d, w, QStringLiteral("MeasureSetup.preflight"));
        d.check(QStringLiteral("p5follow: no placement is chosen for the person: Measure and Check are disabled, the reason in words"),
                mc->placement().isEmpty() && mc->setupProblem().contains(QLatin1String("placement")) && startButton &&
                    !startButton->isEnabled() && checkButton && !checkButton->isEnabled());
        mc->start();
        d.spin(200);
        d.check(QStringLiteral("p5follow: a start without a placement is refused in words, nothing plays"),
                !mc->running() && !mc->measuring() && mc->error().contains(QLatin1String("placement")) && generatorIdle(d));
        mc->clearError();
        mc->setPasses(2);
        d.spin(100);
        QQuickItem *hint = in(d, w, QStringLiteral("MeasureSetup.passesHint"));
        const bool shown = hint && hint->isVisible() && hint->property("text").toString().contains(QLatin1String("three or more"));
        mc->setPasses(3);
        d.spin(100);
        d.check(QStringLiteral("p5follow: two passes show the timing hint, three hide it"), shown && hint && !hint->isVisible());
    }
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

    // The results: one curve an output, drawn (-- WP P5QUI2 -- the core's
    // curves, asked for when the result came: wait for its answer)
    showTab(d, w, 1);
    QQuickItem *graph = in(d, w, QStringLiteral("MeasureResults.graph"));
    const bool curves = d.waitFor(8000, [&] { return mc->resultCurvesSmoothing() >= 0; });
    const QVariantList series = mc->resultGraph().value(QStringLiteral("series")).toList();
    d.check(QStringLiteral("p5qui: Results draw one measured curve an output"),
            curves && kindCount(mc->resultGraph(), "measured") == outs.size() && graph &&
                d.waitFor(2000, [&] { return graph->property("drawnSeries").toInt() == series.size(); }));
    resultCurvesChecks(d, w, mc, int(outs.size()));
    d.check(QStringLiteral("p5qui: Results say the placement, the microphone and the timing"),
            mc->resultSummary().contains(QLatin1String("Listening position")) && !mc->resultTiming().isEmpty() &&
                mc->resultOutputs().size() == outs.size());
    QDir().mkpath(d.outDir() + QStringLiteral("/p5qui"));
    d.check(QStringLiteral("p5qui: screenshot of the results"), d.grab(w, QStringLiteral("p5qui/results.png")));
    // An output hidden and shown again
    mc->setOutputShown(outs.first(), false);
    const int hiddenCount = kindCount(mc->resultGraph(), "measured");
    mc->setOutputShown(outs.first(), true);
    d.check(QStringLiteral("p5qui: an output can be hidden from the graph"),
            hiddenCount == outs.size() - 1 && kindCount(mc->resultGraph(), "measured") == outs.size());

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

    // -- WP P5QUI2 -- review P5-D F7: "Keep Anyway" pressed within the core's
    // second waits for it (a timer); a Revert pressed meanwhile withdraws it,
    // so no Keep is sent at all and the device is as before
    steps.clear();
    tc->propose();
    const bool reproposed = step(QStringLiteral("propose"), 30000) && stepOk(QStringLiteral("propose"));
    if (reproposed && tc->needsSumCheck()) {
        tc->sumCheckRun();
        step(QStringLiteral("sumCheck"), 110000);
        d.waitFor(5000, [&] { return !mc->running(); });
    }
    steps.clear();
    tc->apply(0);
    const bool raceApplied = reproposed && step(QStringLiteral("apply"), 15000) && stepOk(QStringLiteral("apply")) && d.waitIdle() && tc->applied();
    d.check(QStringLiteral("p5qui2: f7: the tune applied for the race"), raceApplied);
    if (raceApplied) {
        const QString why = QStringLiteral("measure.tune.keep_unverified: the verify did not confirm the tune");
        QJsonObject st = tuneStateOf(d);
        st.insert(QStringLiteral("issue"), QJsonObject{ { QStringLiteral("key"), QStringLiteral("measure.tune.keep_unverified") },
                                                        { QStringLiteral("message"), why } });
        tc->keepAnswered(QJsonObject{ { QStringLiteral("ok"), false }, { QStringLiteral("code"), QStringLiteral("invalid") },
                                      { QStringLiteral("error"), why } },
                         st);
        steps.clear();
        // Keep Anyway at once (the core's second has not passed: it waits),
        // then Revert before it is sent
        tc->confirmKeep();
        tc->revert();
        const bool revertedRace = step(QStringLiteral("revert"), 15000) && stepOk(QStringLiteral("revert"));
        d.spin(2000);   // past the delayed Keep's 1.2 s
        bool keepSent = false;
        for (const QList<QVariant> &a : steps)
            if (a.value(0).toString() == QLatin1String("keep")) keepSent = true;
        const Chain raced = freshChain(d);
        d.check(QStringLiteral("p5qui2: f7: Revert withdraws a delayed Keep Anyway: no Keep sent, the device as before"),
                revertedRace && !keepSent && tc->stage() == QLatin1String("reverted") && !tc->applied() && raced == before &&
                    !tc->keepConfirmNeeded());
    }

    // Applied again, then Keep after a verify that did not pass: the core's
    // refusal (fed through the seam, as the core answers it) opens the
    // question; Keep again asks again; nothing keeps until the person answers
    const int baseRefusals = tc->keepRefusals();   // -- WP P5QUI2 -- after F7's synthetic one
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
            tc->keepConfirmNeeded() && tc->keepRefusals() == baseRefusals + 1 && d.waitFor(2000, questionOpen) &&
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
                tc->keepRefusals() == baseRefusals + 1);
    // The person's explicit answer keeps it. After a real verify that did not
    // pass the core refuses that Keep itself (its first): the question comes
    // again, and the person's second answer keeps it
    steps.clear();
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    bool keptNow = step(QStringLiteral("keep"), 15000) && stepOk(QStringLiteral("keep"));
    if (!keptNow && !tc->verifyPassed()) {
        d.check(QStringLiteral("p5qui: the core's own refusal after the failed verify asks again"),
                tc->keepConfirmNeeded() && tc->keepRefusals() == baseRefusals + 2 && d.waitFor(2000, questionOpen) && tc->applied());
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

// ---- WP P5QUI2 ----------------------------------------------------------------

// A sequential device the sine source appends to (readyRead on each append)
class PushDevice : public QIODevice {
public:
    PushDevice() { open(QIODevice::ReadOnly); }
    void push(const QByteArray &bytes) {
        m_buf.append(bytes);
        emit readyRead();
    }
    bool isSequential() const override { return true; }
    qint64 bytesAvailable() const override { return m_buf.size() + QIODevice::bytesAvailable(); }

protected:
    qint64 readData(char *data, qint64 max) override {
        const qint64 n = qMin<qint64>(max, m_buf.size());
        std::memcpy(data, m_buf.constData(), size_t(n));
        m_buf.remove(0, n);
        return n;
    }
    qint64 writeData(const char *, qint64) override { return -1; }

private:
    QByteArray m_buf;
};

// "Programme audio" for the passive RTA: a 1 kHz sine at -12 dBFS, mono f32
// at 48 kHz, 10 ms every 10 ms, through AudioCapture's reader (its seam). No
// audio device is opened.
class SineSource final : public dspi_capture::Source {
public:
    bool open(dspi_capture::Failure *, QString *) override {
        m_format.setSampleRate(48000);
        m_format.setChannelCount(1);
        m_format.setSampleFormat(QAudioFormat::Float);
        m_device = std::make_unique<PushDevice>();
        m_timer = std::make_unique<QTimer>();
        m_timer->setInterval(10);
        QObject::connect(m_timer.get(), &QTimer::timeout, m_timer.get(), [this] { next(); });
        m_timer->start();
        return true;
    }
    QIODevice *device() override { return m_device.get(); }
    QAudioFormat format() const override { return m_format; }
    qint64 producedFrames() const override { return m_produced; }
    QString name() const override { return QStringLiteral("Smoke sine"); }
    void close() override {
        m_timer.reset();
        m_device.reset();
    }

private:
    void next() {
        constexpr int n = 480;
        QByteArray bytes(n * int(sizeof(float)), '\0');
        auto *f = reinterpret_cast<float *>(bytes.data());
        for (int i = 0; i < n; ++i) f[i] = float(0.25 * std::sin(2 * 3.14159265358979323846 * 1000.0 * double(m_produced + i) / 48000.0));
        m_produced += n;
        m_device->push(bytes);
    }
    QAudioFormat m_format;
    std::unique_ptr<PushDevice> m_device;
    std::unique_ptr<QTimer> m_timer;
    qint64 m_produced = 0;
};

bool coreMeasureActive(SmokeDriver &d) {
    return coreJson(d, [](FfiCore *c, char *b, uint32_t l) { return dspi_measure_state(c, b, l); }).value(QStringLiteral("active")).toBool();
}

bool corePassiveActive(SmokeDriver &d) {
    return coreJson(d, [](FfiCore *c, char *b, uint32_t l) { return dspi_measure_rta_passive_state(c, b, l); }).value(QStringLiteral("active")).toBool();
}

bool tabEnabled(SmokeDriver &d, QQuickWindow *w, const char *tab) {
    QQuickItem *t = in(d, w, QStringLiteral("Measure.tab.") + QLatin1String(tab));
    return t && t->isEnabled();
}

bool tabVisible(SmokeDriver &d, QQuickWindow *w, const char *tab) {
    QQuickItem *t = in(d, w, QStringLiteral("Measure.tab.") + QLatin1String(tab));
    return t && t->isVisible();
}

// Review P5-D F5: a Stop, or the window closed, while a start is on its way
// withdraws it: after it, nothing plays. Several timings, so the Stop lands
// before the microphone answers, while the start is with the core, and after.
void racesSection(SmokeDriver &d) {
    auto *mc = Registrar::instance<MeasureController>();
    if (!mc || !readyDemo(d)) {
        d.check(QStringLiteral("p5qui2: races: the demo is ready"), false);
        return;
    }
    QQuickWindow *w = measureWindow(d);
    d.check(QStringLiteral("p5qui2: races: the Measure window"), w != nullptr);
    if (!w) return;
    d.waitFor(5000, [&] { return mc->simAvailable() && mc->micChoice() == QLatin1String("sim"); });
    chooseOutputs(mc);
    bool allStopped = true;
    QStringList seen;
    for (int delay : { 0, 2, 10, 40, 150 }) {
        showTab(d, w, 0);
        mc->start();
        if (delay) d.spin(delay);
        mc->stop();
        const bool settled = d.waitFor(10000, [&] { return !mc->running() && !mc->busy(); });
        d.spin(300);
        const bool idle = settled && !mc->running() && !coreMeasureActive(d) && generatorIdle(d);
        seen << QStringLiteral("%1 ms: %2").arg(delay).arg(mc->phase());
        allStopped = allStopped && idle;
    }
    std::printf("smoke: p5qui2: f5: Stop after Measure: %s\n", qPrintable(seen.join(QStringLiteral(", "))));
    d.check(QStringLiteral("p5qui2: f5: a Stop while Measure is on its way withdraws it: nothing plays after it"),
            allStopped && !mc->measuring());
    // The live view, the window closed right after its start
    int output = 0;
    for (const QVariant &v : mc->setupOutputs())
        if (v.toMap().value(QStringLiteral("on")).toBool()) {
            output = v.toMap().value(QStringLiteral("output")).toInt();
            break;
        }
    bool liveStopped = true;
    seen.clear();
    for (int delay : { 0, 10, 60 }) {
        if (!w->isVisible()) w = measureWindow(d);
        if (!w) break;
        mc->liveStart({ { QStringLiteral("output"), output },
                        { QStringLiteral("role"), QStringLiteral("full") },
                        { QStringLiteral("mode"), QStringLiteral("sweep") },
                        { QStringLiteral("placement"), QStringLiteral("listeningPosition") },
                        { QStringLiteral("smoothing"), 6 } });
        if (delay) d.spin(delay);
        w->close();
        const bool settled = d.waitFor(10000, [&] { return !mc->running() && !mc->busy(); });
        d.spin(300);
        seen << QStringLiteral("%1 ms: live %2").arg(delay).arg(mc->liveActive() ? QStringLiteral("on") : QStringLiteral("off"));
        liveStopped = liveStopped && settled && !mc->liveActive() && !coreMeasureActive(d) && generatorIdle(d);
    }
    std::printf("smoke: p5qui2: f5: window closed after Live: %s\n", qPrintable(seen.join(QStringLiteral(", "))));
    d.check(QStringLiteral("p5qui2: f5: closing the window while the live view starts withdraws it: nothing plays after it"), liveStopped);
    if (w && w->isVisible()) w->close();
    restoreVolume(d);
}

// The passive microphone RTA in the client read-only mode (P5CURV's calls):
// the tabs that play are not offered, the passive one is; frames arrive from
// the capture, hold and compare work, the generator stays idle and nothing is
// written. Then the mode off: the tabs back, and the passive RTA refused
// beside the live view.
void passiveSection(SmokeDriver &d) {
    auto *mc = Registrar::instance<MeasureController>();
    if (!mc || !readyDemo(d)) {
        d.check(QStringLiteral("p5qui2: passive: the demo is ready"), false);
        return;
    }
    QQuickWindow *w = measureWindow(d);
    d.check(QStringLiteral("p5qui2: passive: the Measure window"), w != nullptr);
    if (!w) return;
    d.check(QStringLiteral("p5qui2: a connection that may play gets no passive tab"),
            mc->playBlocker().isEmpty() && !tabVisible(d, w, "passive") && tabEnabled(d, w, "measure"));
    showTab(d, w, 0);
    const bool on = coreBool(d, [](FfiCore *c) { return dspi_set_client_read_only(c, true); });
    mc->refreshAccess();
    d.check(QStringLiteral("p5qui2: the client read-only mode blocks the generator, said in words (playBlocker)"),
            on && d.waitFor(3000, [&] { return mc->playBlocker().contains(QLatin1String("read-only mode")); }));
    d.spin(200);
    QQuickItem *tabs = in(d, w, QStringLiteral("Measure.tabs"));
    d.check(QStringLiteral("p5qui2: read-only mode: no tab that plays, the passive RTA offered and shown"),
            !tabEnabled(d, w, "measure") && !tabEnabled(d, w, "live") && !tabEnabled(d, w, "tune") && tabEnabled(d, w, "results") &&
                tabVisible(d, w, "passive") && tabs && tabs->property("currentIndex").toInt() == 5);
    // Nothing that plays starts: the controller refuses with the same words
    mc->start();
    d.spin(200);
    d.check(QStringLiteral("p5qui2: read-only mode: Measure is refused in words, nothing starts"),
            mc->error() == mc->playBlocker() && !mc->measuring() && !coreMeasureActive(d));
    mc->clearError();
    uint32_t refusedBefore = 0;
    d.app().core->worker()->call(d.app().device, [](FfiCore *c) { return dspi_client_read_only_refusals(c); },
                                 [&refusedBefore](uint32_t n) { refusedBefore = n; });
    d.waitIdle();
    d.spin(50);

    // The microphone: a sine through the capture's seam (no audio device)
    mc->setCaptureSource([]() -> std::unique_ptr<dspi_capture::Source> { return std::make_unique<SineSource>(); });
    press(in(d, w, QStringLiteral("MeasurePassive.start")));
    const bool frames = d.waitFor(15000, [&] { return mc->passiveActive() && mc->passiveReady() && mc->passiveFrames() > 0; });
    if (!frames) std::printf("smoke: p5qui2: passive: %s\n", qPrintable(mc->error()));
    QQuickItem *graph = in(d, w, QStringLiteral("MeasurePassive.graph"));
    d.check(QStringLiteral("p5qui2: the passive RTA's frames arrive from the microphone and are drawn"),
            frames && corePassiveActive(d) && graph &&
                d.waitFor(3000, [&] { return graph->property("drawnKinds").toStringList().contains(QLatin1String("measured")); }));
    d.check(QStringLiteral("p5qui2: the Stop bar shows that the microphone listens and nothing plays"),
            mc->running() && mc->runningText().contains(QLatin1String("passive RTA")) &&
                d.waitFor(1000, [&] { return in(d, w, QStringLiteral("Measure.stopBar"))->isVisible(); }));
    d.check(QStringLiteral("p5qui2: while the passive RTA listens the generator is idle and no run is active"),
            generatorIdle(d) && !coreMeasureActive(d));
    const int framesNow = mc->passiveFrames();
    d.check(QStringLiteral("p5qui2: the frames go on arriving"), d.waitFor(5000, [&] { return mc->passiveFrames() > framesNow; }));
    press(in(d, w, QStringLiteral("MeasurePassive.hold")));
    d.check(QStringLiteral("p5qui2: Hold keeps a copy, and the compare is drawn"),
            d.waitFor(5000, [&] { return mc->passiveHeld() && mc->passiveCompareGraph().contains(QStringLiteral("series")); }) &&
                d.waitFor(3000, [&] { return graph->property("drawnKinds").toStringList().contains(QLatin1String("held")); }));
    mc->setPassiveAveraging(QStringLiteral("fast"));
    d.check(QStringLiteral("p5qui2: the averaging changes in the core"), d.waitFor(5000, [&] { return mc->passiveAveraging() == QLatin1String("fast"); }));
    QDir().mkpath(d.outDir() + QStringLiteral("/p5qui"));
    d.check(QStringLiteral("p5qui2: screenshot of the passive RTA"), d.grab(w, QStringLiteral("p5qui/passive.png")));
    uint32_t refusedAfter = 0;
    d.app().core->worker()->call(d.app().device, [](FfiCore *c) { return dspi_client_read_only_refusals(c); },
                                 [&refusedAfter](uint32_t n) { refusedAfter = n; });
    d.waitIdle();
    d.spin(50);
    d.check(QStringLiteral("p5qui2: the passive RTA writes nothing (no write refused by the read-only mode)"), refusedAfter == refusedBefore);
    press(in(d, w, QStringLiteral("Measure.stop")));
    d.check(QStringLiteral("p5qui2: Stop ends the passive RTA and closes the microphone"),
            d.waitFor(5000, [&] { return !mc->passiveActive() && !mc->running() && !mc->capture()->isRunning(); }) && !corePassiveActive(d) &&
                generatorIdle(d));

    // The mode off: the tabs that play again, the passive tab gone
    const bool off = coreBool(d, [](FfiCore *c) { return dspi_set_client_read_only(c, false); });
    mc->refreshAccess();
    d.check(QStringLiteral("p5qui2: read-only mode off: the tabs that play are offered again"),
            off && d.waitFor(3000, [&] { return mc->playBlocker().isEmpty(); }) && d.waitFor(1000, [&] {
                return tabEnabled(d, w, "measure") && tabEnabled(d, w, "live") && !tabVisible(d, w, "passive") &&
                       tabs->property("currentIndex").toInt() != 5;
            }));
    // Never beside the generator: refused while the live view plays
    int output = 0;
    for (const QVariant &v : mc->setupOutputs())
        if (v.toMap().value(QStringLiteral("on")).toBool()) {
            output = v.toMap().value(QStringLiteral("output")).toInt();
            break;
        }
    mc->setCaptureSource(nullptr);
    mc->liveStart({ { QStringLiteral("output"), output },
                    { QStringLiteral("role"), QStringLiteral("full") },
                    { QStringLiteral("mode"), QStringLiteral("sweep") },
                    { QStringLiteral("placement"), QStringLiteral("listeningPosition") },
                    { QStringLiteral("smoothing"), 6 } });
    const bool live = d.waitFor(10000, [&] { return mc->liveActive(); });
    mc->setCaptureSource([]() -> std::unique_ptr<dspi_capture::Source> { return std::make_unique<SineSource>(); });
    mc->passiveStart(QStringLiteral("slow"));
    d.spin(500);
    d.check(QStringLiteral("p5qui2: the passive RTA is not offered while the live view plays, said in words"),
            live && !mc->passiveBlocker().isEmpty() && mc->error() == mc->passiveBlocker() && !mc->passiveActive() && !corePassiveActive(d));
    // And the core refuses it too (`busy`), whatever a screen thinks
    const QByteArray setup = R"({"captureRateHz":48000})";
    const QJsonObject refused = coreJson(d, [setup](FfiCore *c, char *b, uint32_t l) { return dspi_measure_rta_passive_start(c, setup.constData(), b, l); });
    d.check(QStringLiteral("p5qui2: the core refuses the passive RTA beside the live view (busy), in words"),
            !refused.value(QStringLiteral("ok")).toBool() && refused.value(QStringLiteral("error")).toString().contains(QLatin1String("busy")) &&
                !MeasureController::failureText(refused).isEmpty() && !corePassiveActive(d));
    mc->setCaptureSource(nullptr);
    mc->liveStop();
    d.check(QStringLiteral("p5qui2: passive: Stop Live, the generator idle"), d.waitFor(8000, [&] { return !mc->liveActive() && !mc->running(); }) && generatorIdle(d));
    mc->clearError();
    w->close();
    restoreVolume(d);
}

// Review P5-D F6: a hub's Viewer (a fake hub granting Viewer) is offered no
// tab that plays; the passive RTA, the results and the documents only
void viewerSection(SmokeDriver &d) {
    auto *mc = Registrar::instance<MeasureController>();
    auto *link = Registrar::instance<LinkController>();
    DeviceModel *device = d.app().device;
    if (!mc || !link) {
        d.check(QStringLiteral("p5qui2: viewer: the controllers"), false);
        return;
    }
    linksmoke::FakeHubProcess hub;
    const QString pin = QStringLiteral("731864");
    const bool up = hub.start({ "--auth", QStringLiteral("window:") + pin, "--role", "viewer", "--name", "P5QUI2 hub" });
    d.check(QStringLiteral("p5qui2: viewer: the fake hub runs (DSPI_SMOKE_FAKEHUB)"), up);
    if (!up) return;
    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    link->connectToHub(hub.url, hub.id);
    const bool asks = settled() && d.waitFor(5000, [&] { return link->phase() == QLatin1String("needs_pairing"); });
    link->pair(pin);
    const bool listed = asks && settled() && d.waitFor(5000, [&] { return link->phase() == QLatin1String("choose_device") && link->devices().size() == 1; });
    const QVariantMap row = link->devices().value(0).toMap();
    if (listed) link->bindDevice(row.value(QStringLiteral("handle")).toInt(), row.value(QStringLiteral("serial")).toString());
    const bool bound = listed && settled() && d.waitFor(10000, [&] {
        return link->phase() == QLatin1String("bound") && device->remote() && device->connected();
    });
    std::printf("smoke: p5qui2: viewer: phase %s, role %s\n", qPrintable(link->phase()), qPrintable(link->role()));
    d.check(QStringLiteral("p5qui2: viewer: bound to the hub's device as a Viewer"), bound && link->role() == QLatin1String("viewer"));
    QQuickWindow *w = bound ? measureWindow(d) : nullptr;
    if (w) {
        d.check(QStringLiteral("p5qui2: f6: a Viewer is told why, in words"),
                d.waitFor(3000, [&] { return mc->playBlocker().contains(QLatin1String("Viewer")); }));
        d.spin(200);
        QQuickItem *tabs = in(d, w, QStringLiteral("Measure.tabs"));
        d.check(QStringLiteral("p5qui2: f6: a Viewer gets no tab that plays; the passive RTA, results and documents"),
                !tabEnabled(d, w, "measure") && !tabEnabled(d, w, "live") && !tabEnabled(d, w, "tune") && tabEnabled(d, w, "results") &&
                    tabEnabled(d, w, "documents") && tabVisible(d, w, "passive") && tabs && tabs->property("currentIndex").toInt() == 5);
        mc->start();
        d.spin(200);
        const bool measureRefused = mc->error() == mc->playBlocker() && !mc->measuring();
        mc->clearError();
        mc->liveStart({ { QStringLiteral("output"), 0 }, { QStringLiteral("mode"), QStringLiteral("sweep") } });
        d.spin(200);
        const bool liveRefused = mc->error() == mc->playBlocker() && !mc->liveActive();
        mc->clearError();
        d.check(QStringLiteral("p5qui2: f6: a Viewer's Measure and Live are refused in words, nothing starts"),
                measureRefused && liveRefused && !mc->running());
        d.grab(w, QStringLiteral("p5qui/viewer.png"));
        w->close();
    }
    link->closeSession();
    d.waitFor(10000, [&] { return !link->busy() && !link->inSession(); });
    device->openDemo();
    d.check(QStringLiteral("p5qui2: viewer: back on the demo, the generator offered again"),
            d.waitFor(10000, [&] { return device->connected() && device->demo() && !device->remote(); }) &&
                d.waitFor(3000, [&] { return mc->playBlocker().isEmpty(); }));
}

const SmokeDriver::Section measureS("p5qui-measure", 210, &measureSection);
const SmokeDriver::Section liveS("p5qui-live", 211, &liveSection);
const SmokeDriver::Section tuneS("p5qui-tune", 212, &tuneSection);
const SmokeDriver::Section racesS("p5qui2-races", 213, &racesSection);
const SmokeDriver::Section passiveS("p5qui2-passive", 214, &passiveSection);
const SmokeDriver::Section viewerS("p5qui2-viewer", 215, &viewerSection);

} // namespace
