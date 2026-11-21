// -- WP QP3 -- The response graph: curves from the core, visibility with the
// page's solo and the overview's pills handed back, the gain preview, the PEQ
// overlay's channel left out, the phase trace, the preferences and their
// limits, the mapping, the options popover with its setup page and the
// anchor-click rule, the drag strip, and the pop-out window with its legend.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "GraphController.h"
#include "Registrar.h"
#include "Shell.h"
#include "render/BodePlot.h"

#include "CoreApi.h"

#include <QAccessible>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTest>
#include <cmath>

namespace {

GraphController *graph() { return Registrar::instance<GraphController>(); }

QList<int> drawn(QObject *plot) { return plot ? plot->property("drawnChannels").value<QList<int>>() : QList<int>(); }

// The level of `curve` nearest `hz` on the core's grid (10 Hz to 20 kHz, log)
double levelAt(const QVector<double> &curve, double hz) {
    if (curve.size() < 2) return NAN;
    const double lo = std::log10(10.0), hi = std::log10(20000.0);
    const int i = int(std::lround((std::log10(hz) - lo) / (hi - lo) * (curve.size() - 1)));
    return curve.value(qBound(0, i, int(curve.size()) - 1));
}

QPoint centreIn(QQuickItem *item) {
    return item->mapToScene(QPointF(item->width() / 2, item->height() / 2)).toPoint();
}

void curvesFromTheCore(SmokeDriver &d, GraphController *g) {
    DeviceModel *device = d.app().device;
    const DspState *s = device->state();
    d.check(QStringLiteral("qp3: the graph knows the device's channels"),
            s && g->channelCount() == qMin(int(s->num_input_channels) + int(s->num_output_channels), int(MAX_CHANNELS)));
    d.check(QStringLiteral("qp3: a curve has the core's points"), g->magnitudes(0).size() == MAGNITUDE_POINTS);
    if (!s) return;

    // The output's curve is the core's over its PEQ and crossover bands
    const int out = g->firstOutput();
    std::vector<FilterParams> chain;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) chain.push_back(s->filters[out][b]);
    for (int b = 0; b < MAX_XOVER_BANDS; ++b) chain.push_back(s->xover[out][b]);
    QVector<double> expected(MAGNITUDE_POINTS);
    dspi_compute_magnitude_curve(chain.data(), uint32_t(chain.size()), expected.data());
    d.check(QStringLiteral("qp3: an output's curve is the core's (PEQ and crossover)"), g->magnitudes(out) == expected);
    QVector<double> phase(MAGNITUDE_POINTS);
    dspi_compute_phase_curve(chain.data(), uint32_t(chain.size()), true, phase.data());
    d.check(QStringLiteral("qp3: its unwrapped phase is the core's"), g->phases(out, true) == phase);

    // A band written to the device moves the curve
    const FilterParams before = s->filters[0][0];
    FilterParams peak = before;
    peak.filter_type = FILTER_PEAKING;
    peak.bypass = false;
    peak.freq = 1000;
    peak.q = 1.0f;
    peak.gain = 6.0f;
    const int rev = g->curveRevision();
    device->write(QStringLiteral("filter 0 0"), [peak](FfiCore *c) { return dspi_set_filter(c, 0, 0, peak); });
    d.check(QStringLiteral("qp3: a band write brings a new curve"), d.waitFor(3000, [&]() { return g->curveRevision() != rev; }));
    d.waitIdle();
    const double at1k = levelAt(g->magnitudes(0), 1000);
    QVector<double> one(MAGNITUDE_POINTS);
    std::vector<FilterParams> chain0;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) chain0.push_back(device->state()->filters[0][b]);
    dspi_compute_magnitude_curve(chain0.data(), uint32_t(chain0.size()), one.data());
    d.check(QStringLiteral("qp3: the new curve is the core's"), g->magnitudes(0) == one);
    d.check(QStringLiteral("qp3: a +6 dB peak at 1 kHz shows near +6 dB there"), at1k > 4.5 && at1k < 7.5);
    d.spin(300);
    d.check(QStringLiteral("qp3: screenshot of a curve with a band"), d.grab(d.mainWindow(), QStringLiteral("window-main-graph-band.png")));
    device->write(QStringLiteral("filter 0 0"), [before](FfiCore *c) { return dspi_set_filter(c, 0, 0, before); });
    d.waitIdle();
}

void visibility(SmokeDriver &d, GraphController *g, QObject *curves) {
    DeviceModel *device = d.app().device;
    const DspState *s = device->state();
    g->select(-1);
    g->resetDefaultVisibility();
    d.spin(50);
    // Defaults: the inputs the host plays into and every enabled output
    bool defaultsOk = true;
    for (int o = 0; s && o < int(s->num_output_channels); ++o)
        defaultsOk = defaultsOk && g->isCurveVisible(g->firstOutput() + o) == s->output_enabled[o];
    d.check(QStringLiteral("qp3: the overview shows every enabled output's curve"), defaultsOk);
    d.check(QStringLiteral("qp3: the overview shows the first input's curve"), g->isCurveVisible(0));
    d.check(QStringLiteral("qp3: the plot draws the visible curves"), drawn(curves) == g->visibleChannels());
    d.check(QStringLiteral("qp3: equal curves are drawn as one line"),
            curves && curves->property("drawnLines").toInt() >= 1 && curves->property("drawnLines").toInt() <= drawn(curves).size());

    // A pill hidden on the overview, a page soloed, and the overview's pills back
    g->setCurveVisible(0, false);
    const QList<int> overview = g->visibleChannels();
    g->select(1);
    d.check(QStringLiteral("qp3: a channel page solos its curve"), g->visibleChannels() == QList<int>{ 1 });
    g->select(2, 3);
    d.check(QStringLiteral("qp3: a linked pair shows both curves"), g->visibleChannels() == (QList<int>{ 2, 3 }));
    g->select(-1);
    d.check(QStringLiteral("qp3: the overview gets its pills back"), g->visibleChannels() == overview);
    g->toggleCurve(0);
    d.check(QStringLiteral("qp3: a pill toggles its curve"), g->isCurveVisible(0));
    d.spin(50);
    d.check(QStringLiteral("qp3: the plot follows the pills"), drawn(curves) == g->visibleChannels());
    const QList<int> shown = g->visibleChannels();

    // The main window's selection reaches the graph
    QQuickWindow *w = d.mainWindow();
    w->setProperty("selectedChannel", g->firstOutput());
    d.spin(50);
    d.check(QStringLiteral("qp3: selecting an output in the window solos it"),
            g->selectedChannel() == g->firstOutput() && g->visibleChannels() == QList<int>{ g->firstOutput() });
    w->setProperty("selectedChannel", -1);
    d.spin(50);
    d.check(QStringLiteral("qp3: back to the overview from the window"), g->selectedChannel() == -1 && g->visibleChannels() == shown);
}

void previewsAndOverlay(SmokeDriver &d, GraphController *g, QObject *curves) {
    const int out = g->firstOutput();
    g->setCurveVisible(out, true);
    const double base = g->magnitudeCurve(out).value(100).toDouble();
    const double gain = g->outputGain(out);
    g->previewOutputGain(0, gain + 6.0);
    d.check(QStringLiteral("qp3: a gain preview moves the output's curve"),
            qAbs(g->magnitudeCurve(out).value(100).toDouble() - (base + 6.0)) < 1e-6);
    g->endOutputGainPreview(0);
    d.check(QStringLiteral("qp3: the curve returns when the preview ends"), qAbs(g->magnitudeCurve(out).value(100).toDouble() - base) < 1e-6);

    g->setOverlayChannel(out);
    d.spin(50);
    d.check(QStringLiteral("qp3: the PEQ overlay's channel is left out of the curves"), !drawn(curves).contains(out));
    g->setOverlayChannel(-1);
    d.spin(50);
    d.check(QStringLiteral("qp3: and drawn again when the overlay lets go"), drawn(curves).contains(out));

    // The page's phase
    g->select(out);
    g->setShowPhase(true);
    d.spin(50);
    d.check(QStringLiteral("qp3: the page's phase trace shows"), curves && curves->property("drawnPhase").toBool());
    g->setShowPhase(false);
    d.spin(50);
    d.check(QStringLiteral("qp3: and goes when switched off"), curves && !curves->property("drawnPhase").toBool());
    g->select(-1);
}

void preferences(SmokeDriver &d, GraphController *g, BodePlot *curves) {
    QSettings settings;
    g->setDbRange(60);
    d.check(QStringLiteral("qp3: the range is kept under graph/dbRange"), settings.value(QStringLiteral("graph/dbRange")).toDouble() == 60);
    d.check(QStringLiteral("qp3: top and bottom follow range and centre"), g->dbTop() == 30 && g->dbBottom() == -30);
    d.check(QStringLiteral("qp3: the phase axis follows the zoom"), qAbs(g->phaseTop() - 216.0) < 1e-9);
    g->setDbRange(500);
    d.check(QStringLiteral("qp3: the range stops at 100 dB"), g->dbRange() == 100);
    g->setDbCenter(-90);
    d.check(QStringLiteral("qp3: the centre stops at -40 dB"), g->dbCenter() == -40);
    g->setMinFreq(33);
    d.check(QStringLiteral("qp3: the lowest frequency is one of the offered ones"), g->minFreq() == 20);
    g->setGraphLineWidth(2.2);
    d.check(QStringLiteral("qp3: the line width moves in half points"), g->graphLineWidth() == 2.0);
    g->setGraphHeight(1000);
    d.check(QStringLiteral("qp3: the graph height stops at 350"), g->graphHeight() == 350);
    g->resetScale();
    d.check(QStringLiteral("qp3: Reset restores the scale"),
            g->minFreq() == 15 && g->maxFreq() == 20000 && g->dbRange() == 50 && g->dbCenter() == 0);
    g->setGraphLineWidth(1.5);
    g->setGraphHeight(250);

    if (curves) {
        d.check(QStringLiteral("qp3: the lowest frequency is the left edge"), qAbs(curves->xForFrequency(g->minFreq())) < 1e-6);
        d.check(QStringLiteral("qp3: the highest frequency is the right edge"), qAbs(curves->xForFrequency(g->maxFreq()) - curves->width()) < 1e-6);
        d.check(QStringLiteral("qp3: the top dB is the top edge"), qAbs(curves->yForDb(g->dbTop())) < 1e-6);
        d.check(QStringLiteral("qp3: frequency and x map back"), qAbs(curves->frequencyForX(curves->xForFrequency(1000)) - 1000) < 1e-6);
        d.check(QStringLiteral("qp3: the dB step at 50 dB is 5"), curves->dbStep() == 5);
    }
}

void options(SmokeDriver &d) {
    QQuickWindow *w = d.mainWindow();
    auto *gear = qobject_cast<QQuickItem *>(d.item(QStringLiteral("GraphHost.options")));
    QObject *panel = d.object(QStringLiteral("GraphHost.options.popover"));
    d.check(QStringLiteral("qp3: the graph has its gear"), gear && panel);
    if (!gear || !panel) return;
    // The gear shows while the pointer is over the graph
    auto *plot = d.item(QStringLiteral("GraphHost.plot"));
    d.check(QStringLiteral("qp3: the gear is hidden while the pointer is elsewhere"), gear->opacity() < 0.01);
    if (plot) QTest::mouseMove(w, centreIn(plot));
    d.check(QStringLiteral("qp3: the gear shows under the pointer"), d.waitFor(1000, [&]() { return gear->opacity() > 0.99; }));
    QMetaObject::invokeMethod(gear, "click");
    d.check(QStringLiteral("qp3: the gear opens the options"), d.waitFor(2000, [&]() { return panel->property("opened").toBool(); }));
    d.spin(250);
    d.check(QStringLiteral("qp3: screenshot of the graph options"), d.grab(w, QStringLiteral("window-graph-options.png")));
    QObject *setupRow = d.item(QStringLiteral("GraphHost.options.setup"));
    d.check(QStringLiteral("qp3: the options offer Graph Setup"), setupRow != nullptr);
    d.check(QStringLiteral("qp3: the main window's options offer Pop Out Graph"),
            d.item(QStringLiteral("GraphHost.options.popOut")) && d.item(QStringLiteral("GraphHost.options.popOut"))->isVisible());
    if (setupRow) QMetaObject::invokeMethod(setupRow, "activated");
    d.check(QStringLiteral("qp3: Graph Setup opens its page"), d.waitFor(2000, [&]() { return panel->property("setup").toBool(); }));
    d.spin(300);
    QQuickItem *follow = d.item(QStringLiteral("GraphHost.options.follow"));
    d.check(QStringLiteral("qp3: the follow switch is the pop-out's only"), follow && !follow->isVisible());
    // A switch on the page changes the graph at once
    auto *glow = d.item(QStringLiteral("GraphHost.options.glow"));
    const bool glowBefore = graph()->showGraphGlow();
    if (glow) {
        QMetaObject::invokeMethod(glow, "toggle");
        QMetaObject::invokeMethod(glow, "toggled");
    }
    d.check(QStringLiteral("qp3: the Glow switch sets the preference"), graph()->showGraphGlow() != glowBefore);
    graph()->setShowGraphGlow(glowBefore);
    d.check(QStringLiteral("qp3: screenshot of the graph setup page"), d.grab(w, QStringLiteral("window-graph-setup.png")));

    // One click on the gear while open closes it, once, and it stays closed
    QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, centreIn(gear));
    d.spin(400);
    d.check(QStringLiteral("qp3: a click on the gear closes the options and does not reopen them"), !panel->property("visible").toBool());
    QMetaObject::invokeMethod(gear, "click");
    d.check(QStringLiteral("qp3: reopened, the options start on the main page"),
            d.waitFor(2000, [&]() { return panel->property("opened").toBool(); }) && !panel->property("setup").toBool());
    w->requestActivate();
    d.spin(100);
    QTest::keyClick(w, Qt::Key_Escape);
    d.check(QStringLiteral("qp3: Escape closes the options"), d.waitFor(2000, [&]() { return !panel->property("visible").toBool(); }));
}

void dragStrip(SmokeDriver &d, GraphController *g) {
    QQuickWindow *w = d.mainWindow();
    auto *strip = d.item(QStringLiteral("GraphHost.resize"));
    d.check(QStringLiteral("qp3: the graph has its drag strip"), strip != nullptr);
    if (!strip) return;
    const double before = g->graphHeight();
    const QPoint start = centreIn(strip);
    QTest::mousePress(w, Qt::LeftButton, Qt::NoModifier, start);
    for (int dy = 5; dy <= 40; dy += 5) {
        QTest::mouseMove(w, start + QPoint(0, dy));
        d.spin(10);
    }
    QTest::mouseRelease(w, Qt::LeftButton, Qt::NoModifier, start + QPoint(0, 40));
    d.spin(50);
    d.check(QStringLiteral("qp3: dragging the strip down makes the graph taller"), g->graphHeight() > before + 20);
    auto *plot = d.item(QStringLiteral("GraphHost.plot"));
    d.check(QStringLiteral("qp3: the graph takes the new height"), plot && qAbs(plot->height() - g->graphHeight()) < 1.0);
    g->setGraphHeight(before);
}

void popOut(SmokeDriver &d, GraphController *g) {
    Shell *shell = d.app().shell;
    shell->trigger(QStringLiteral("window.popOutGraph"));
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("window.popOutGraph")));
    d.check(QStringLiteral("qp3: Pop Out Graph opens the Filter Response window"),
            w && d.waitFor(3000, [&]() { return w->isVisible(); }) && w->title() == QLatin1String("Filter Response"));
    if (!w) return;
    d.check(QStringLiteral("qp3: the pop-out has the Mac's minimum size"), w->minimumWidth() == 500 && w->minimumHeight() == 250);
    d.check(QStringLiteral("qp3: the main window knows the graph is out"), g->poppedOut());
    d.spin(100);
    auto *mainPlot = d.item(QStringLiteral("GraphHost.plot"));
    auto *closeButton = d.item(QStringLiteral("GraphHost.closePopOut"));
    d.check(QStringLiteral("qp3: the main window's graph steps aside"), mainPlot && !mainPlot->isVisible());
    d.check(QStringLiteral("qp3: the main header offers to bring it back"), closeButton && closeButton->isVisible());

    QQuickItem *root = w->contentItem();
    QObject *popCurves = d.item(QStringLiteral("GraphWindow.plot.curves"), root);
    d.check(QStringLiteral("qp3: the pop-out draws the main graph's curves while following"),
            popCurves && drawn(popCurves) == g->visibleChannels());

    // Its legend toggles the main graph's set while following
    QObject *pill = d.item(QStringLiteral("GraphLegend.channel.0"), root);
    d.check(QStringLiteral("qp3: the legend has a pill per channel"), pill != nullptr);
    const bool shown = g->isCurveVisible(0);
    if (pill) QMetaObject::invokeMethod(pill, "click");
    d.check(QStringLiteral("qp3: a legend pill toggles the main graph's curve while following"), g->isCurveVisible(0) != shown);
    d.check(QStringLiteral("qp3: the pill shows the curve's state"), pill && pill->property("checked").toBool() == g->isCurveVisible(0));

    // Not following: its own set
    g->setPopoutFollowsSelection(false);
    const QList<int> mainSet = g->visibleChannels();
    const bool ownShown = g->isPopOutCurveVisible(0);
    if (pill) QMetaObject::invokeMethod(pill, "click");
    d.check(QStringLiteral("qp3: not following, the pop-out's pills leave the main graph alone"), g->visibleChannels() == mainSet);
    d.check(QStringLiteral("qp3: and change the pop-out's own curves"), g->isPopOutCurveVisible(0) != ownShown);
    d.spin(50);
    d.check(QStringLiteral("qp3: the pop-out draws its own set"), drawn(popCurves) == g->popOutVisibleChannels());
    d.check(QStringLiteral("qp3: screenshot of the pop-out graph"), d.grab(w, QStringLiteral("window-graph.png")));

    // Its options: no Pop Out row, and the follow switch on the setup page
    QObject *gear = d.item(QStringLiteral("GraphWindow.options"), root);
    if (gear) QMetaObject::invokeMethod(gear, "click");
    d.spin(250);
    QQuickItem *popRow = d.item(QStringLiteral("GraphWindow.options.popOut"), root);
    d.check(QStringLiteral("qp3: the pop-out's options have no Pop Out row"), popRow && !popRow->isVisible());
    if (QObject *setupRow = d.item(QStringLiteral("GraphWindow.options.setup"), root)) QMetaObject::invokeMethod(setupRow, "activated");
    d.spin(300);
    QQuickItem *follow = d.item(QStringLiteral("GraphWindow.options.follow"), root);
    d.check(QStringLiteral("qp3: the pop-out's setup page offers the follow switch"), follow && follow->isVisible());
    d.check(QStringLiteral("qp3: screenshot of the pop-out's setup page"), d.grab(w, QStringLiteral("window-graph-setup-popout.png")));
    QTest::keyClick(w, Qt::Key_Escape);
    d.spin(100);
    g->setPopoutFollowsSelection(true);
    if (pill && pill->property("checked").toBool() != shown) QMetaObject::invokeMethod(pill, "click");

    g->closePopOut();
    d.check(QStringLiteral("qp3: the header button closes the pop-out"), d.waitFor(2000, [&]() { return !w->isVisible(); }));
    d.check(QStringLiteral("qp3: and the graph is back in the main window"),
            !g->poppedOut() && d.waitFor(1000, [&]() { return mainPlot && mainPlot->isVisible(); }));
}

void graphSection(SmokeDriver &d) {
    GraphController *g = graph();
    d.check(QStringLiteral("qp3: GraphController is registered"), g != nullptr);
    if (!g) return;
    d.check(QStringLiteral("qp3: the demo device is open"), d.waitFor(5000, [&]() { return d.app().device->connected(); }));
    d.waitIdle();
    d.check(QStringLiteral("qp3: the graph host is in the right pane"), d.item(QStringLiteral("GraphHost")) != nullptr);
    // -- WP QTTWEAK -- no heading over the graph; the plot keeps the name
    QAccessibleInterface *named = QAccessible::queryAccessibleInterface(d.item(QStringLiteral("GraphHost.plot")));
    d.check(QStringLiteral("qp3: the graph is named Filter Response, with no heading over it"),
            named && named->text(QAccessible::Name) == QLatin1String("Filter Response") && d.item(QStringLiteral("GraphHost.title")) == nullptr);
    auto *curves = qobject_cast<BodePlot *>(d.item(QStringLiteral("GraphHost.plot.curves")));
    d.check(QStringLiteral("qp3: the graph has its curve layer"), curves != nullptr);
    d.check(QStringLiteral("qp3: the plot is an accessible graphic"), d.item(QStringLiteral("GraphHost.plot")) != nullptr);
    // -- WP QP4 -- the PEQ slot is QP4's editor now (its section checks it)
    // -- WP QP9 -- the analyser's slots are QP9's now (qp9-spectrum checks them)
    d.check(QStringLiteral("qp3: the analyser fills its slots"), g->slotType(GraphController::RtaOverlay) == QLatin1String("RtaGraphOverlay")
                                                                     && g->slotType(GraphController::RtaOptions) == QLatin1String("RtaGraphOptions"));

    curvesFromTheCore(d, g);
    visibility(d, g, curves);
    previewsAndOverlay(d, g, curves);
    preferences(d, g, curves);
    d.check(QStringLiteral("qp3: screenshot of the main window's graph"), d.grab(d.mainWindow(), QStringLiteral("window-main-graph.png")));
    options(d);
    dragStrip(d, g);
    popOut(d, g);
    g->resetDefaultVisibility();
}

const SmokeDriver::Section section("qp3-graph", 30, &graphSection);

} // namespace
