// -- WP QP4 -- The on-graph PEQ editor in the main window, on the demo device:
// the page's channel drawn by the editor and its curve the core's, the ghost
// dot and readouts, double-click and Ctrl-click (the card) to add, dragging
// with Ctrl (Q), Shift (fine) and Alt (one axis), pulling a band out of the
// curve, the marquee, the wheel (Q, Ctrl gain, the dB zoom strip), the keys,
// the chip (values, typing, bypass, the shape page and its slope step), the
// band and graph menus, the band list's side (selection, hover, live values,
// a row brought into view), an animated move made elsewhere, a linked input
// pair, and when the editor stands aside (overview, XO tab, read-only).
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "GraphController.h"
#include "PeqEditorController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"
#include "render/PeqOverlay.h"

#include <QGuiApplication>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QTest>
#include <QWheelEvent>
#include <cmath>
#include <functional>

namespace {

struct Peqs {
    SmokeDriver &d;
    QQuickWindow *w;
    PeqOverlay *ov;
    ChannelController *cc;
    GraphController *g;
    PeqEditorController *ed;
    DeviceModel *device;
    int ch = -1;

    const DspState *s() const { return device->state(); }
    FilterParams band(int b) const { return s() ? s()->filters[ch][b] : FilterParams{}; }
    QPoint at(QPointF local) const { return ov->mapToScene(local).toPoint(); }
    QPoint dot(int b) const { return at(ov->dotPoint(b)); }
    double freqAt(double x) const { return g->minFreq() * std::pow(g->maxFreq() / g->minFreq(), x / ov->width()); }
    double dbAt(double y) const { return g->dbTop() - y / ov->height() * (g->dbTop() - g->dbBottom()); }
    double yOf(double db) const { return (g->dbTop() - db) / (g->dbTop() - g->dbBottom()) * ov->height(); }
    double xOf(double hz) const { return std::log10(hz / g->minFreq()) / std::log10(g->maxFreq() / g->minFreq()) * ov->width(); }
    QObject *obj(const QString &name) const { return d.object(name); }

    void settle() {
        d.waitIdle();
        d.spin(40);
        ov->settle();
    }
    void clear() {
        cc->clearBands(ch, false);
        settle();
    }
    // A band written as the band list would write it
    void put(int b, int type, double freq, double q, double gain) {
        cc->setBandType(ch, b, false, type);
        cc->setBandValue(ch, b, false, QStringLiteral("freq"), freq);
        cc->setBandValue(ch, b, false, QStringLiteral("q"), q);
        cc->setBandValue(ch, b, false, QStringLiteral("gain"), gain);
        if (cc->bypassSupported()) cc->setBandBypass(ch, b, false, false);
        settle();
    }
    // A move with the button held and `mods` down (QTest's moves carry none)
    void move(QPoint p, Qt::KeyboardModifiers mods) {
        if (mods == Qt::NoModifier) {
            QTest::mouseMove(w, p);
            return;
        }
        QMouseEvent e(QEvent::MouseMove, QPointF(p), QPointF(p), w->mapToGlobal(QPointF(p)), Qt::NoButton, Qt::LeftButton, mods);
        QCoreApplication::sendEvent(w, &e);
    }
    void hover(QPoint p) {
        QTest::mouseMove(w, p);
        d.spin(20);
    }
    void drag(QPoint from, QPoint to, Qt::KeyboardModifiers mods = Qt::NoModifier, int steps = 8) {
        QTest::mouseMove(w, from);
        QTest::mousePress(w, Qt::LeftButton, mods, from);
        for (int i = 1; i <= steps; ++i) {
            move(from + (to - from) * i / steps, mods);
            d.spin(5);
        }
        QTest::mouseRelease(w, Qt::LeftButton, mods, to);
        settle();
    }
    void wheel(QPoint p, int angle, Qt::KeyboardModifiers mods = Qt::NoModifier) {
        QWheelEvent e(QPointF(p), w->mapToGlobal(QPointF(p)), QPoint(), QPoint(0, angle), Qt::NoButton, mods, Qt::NoScrollPhase, false);
        QCoreApplication::sendEvent(w, &e);
        d.spin(10);
    }
    void key(Qt::Key k, Qt::KeyboardModifiers mods = Qt::NoModifier) {
        QTest::keyClick(w, k, mods);
        d.spin(10);
    }
    // A point on empty graph, well away from the curve and every dot
    QPoint empty(double fx, double db) const { return at(QPointF(ov->width() * fx, yOf(db))); }
    QSet<int> selected() const { return ed->selection(); }
};

QPoint centre(QObject *o) {
    auto *i = qobject_cast<QQuickItem *>(o);
    return i ? i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint() : QPoint();
}

bool near(double a, double b, double tol) { return std::abs(a - b) <= tol; }
bool nearRatio(double a, double b, double tol) { return b != 0 && std::abs(a / b - 1) <= tol; }

void owner(Peqs &p) {
    SmokeDriver &d = p.d;
    d.check(QStringLiteral("qp4: the editor takes the page's channel"), p.ov->editedChannel() == p.ch && p.ed->editedChannel() == p.ch);
    d.check(QStringLiteral("qp4: the graph leaves the edited curve to the editor"), p.g->overlayChannel() == p.ch);
    QObject *curves = p.obj(QStringLiteral("GraphHost.plot.curves"));
    d.check(QStringLiteral("qp4: and the curve layer draws it no more"),
            curves && !curves->property("drawnChannels").value<QList<int>>().contains(p.ch));
}

void curveFromTheCore(Peqs &p) {
    SmokeDriver &d = p.d;
    p.put(0, FILTER_PEAKING, 1000, 1.0, 6.0);
    p.put(1, FILTER_LOWSHELF, 100, 0.707, -4.0);
    std::vector<FilterParams> chain;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) chain.push_back(p.ed->band(p.ch, b, false));
    for (int b = 0; b < MAX_XOVER_BANDS; ++b) chain.push_back(p.ed->band(p.ch, b, true));
    bool same = true;
    for (double hz : { 30.0, 100.0, 1000.0, 5000.0 }) {
        const double core = dspi_compute_response(chain.data(), uint32_t(chain.size()), float(hz)) + p.g->outputGain(p.ch);
        same = same && near(p.ov->curveDbAt(hz), core, 1e-3);
    }
    d.check(QStringLiteral("qp4: the edited curve is the core's response (PEQ, crossovers, gain)"), same);
    const QPointF dot = p.ov->dotPoint(0);
    d.check(QStringLiteral("qp4: a bell's dot sits at its frequency and gain"),
            near(dot.x(), p.xOf(1000), 0.6) && near(dot.y(), p.yOf(6.0), 0.6));
    const QPointF shelf = p.ov->dotPoint(1);
    d.check(QStringLiteral("qp4: a shelf's dot sits on its own curve at its frequency"),
            near(p.dbAt(shelf.y()), dspi_compute_response(&chain[1], 1, 100), 0.15));
    d.check(QStringLiteral("qp4: screenshot of the editor"), d.grab(p.w, QStringLiteral("window-main-peq.png")));
}

void hoverAndCreate(Peqs &p) {
    SmokeDriver &d = p.d;
    p.clear();
    const QPoint e = p.empty(0.55, 9);
    p.hover(e + QPoint(3, 0));
    p.hover(e);
    QObject *axis = p.obj(QStringLiteral("PeqEditor.frequencyReadout"));
    QObject *level = p.obj(QStringLiteral("PeqEditor.levelReadout"));
    const QPointF local = p.ov->mapFromScene(QPointF(e));
    d.check(QStringLiteral("qp4: hovering empty graph shows the ghost dot"), p.ov->ghostShown());
    d.check(QStringLiteral("qp4: the frequency readout names the pointer's frequency"),
            axis && axis->property("shownName").toString() == Peq::frequencyText(p.freqAt(local.x())));
    d.check(QStringLiteral("qp4: the level readout names the pointer's level"),
            level && level->property("shownName").toString() == Peq::gainText(p.dbAt(local.y())));
    p.g->setShowLevelReadout(false);
    p.hover(e + QPoint(1, 0));
    d.check(QStringLiteral("qp4: the level readout follows its switch"), level && level->property("shownName").toString().isEmpty());
    p.g->setShowLevelReadout(true);

    QTest::mouseDClick(p.w, Qt::LeftButton, Qt::NoModifier, e);
    p.settle();
    const FilterParams b0 = p.band(0);
    d.check(QStringLiteral("qp4: a double-click on empty graph makes a bell in the first free band"), b0.filter_type == FILTER_PEAKING);
    d.check(QStringLiteral("qp4: at the pointer's frequency and level"),
            nearRatio(b0.freq, p.freqAt(local.x()), 0.01) && near(b0.gain, p.dbAt(local.y()), 0.11) && near(b0.q, 1.0, 1e-3));
    d.check(QStringLiteral("qp4: the new band is selected"), p.selected() == QSet<int>{ 0 });
    // -- WP QPI2 -- the band bar docked under the graph (the owner's choice
    // over the chip beside the dot)
    d.check(QStringLiteral("qp4: and the band bar shows it"), p.ov->chipBand() == 0);
    auto *bar = qobject_cast<QQuickItem *>(p.obj(QStringLiteral("PeqBar")));
    auto *plotItem = qobject_cast<QQuickItem *>(p.obj(QStringLiteral("GraphHost.plot")));
    QObject *barBand = p.obj(QStringLiteral("PeqBar.band"));
    const QRectF barRect = bar ? bar->mapRectToScene(QRectF(0, 0, bar->width(), bar->height())) : QRectF();
    const QRectF plotRect = plotItem ? plotItem->mapRectToScene(QRectF(0, 0, plotItem->width(), plotItem->height())) : QRectF();
    d.check(QStringLiteral("qpi2: the bar sits right under the graph, as wide, inside the window"),
            bar && bar->isVisible() && barRect.top() >= plotRect.bottom() && barRect.top() - plotRect.bottom() <= 8
                && qAbs(barRect.width() - plotRect.width()) < 1 && barRect.bottom() <= p.w->height());
    d.check(QStringLiteral("qpi2: it names the band"), barBand && barBand->property("text").toString() == QLatin1String("Band 1"));
    QObject *tag = p.obj(QStringLiteral("PeqBar.channelTag"));
    d.check(QStringLiteral("qpi2: and tags its channel with the channel's colour, small"),
            tag && tag->property("width").toDouble() <= 8 && tag->property("color").value<QColor>() == p.ov->curveColor());
    for (int f : { 0, 1, 2 })
        d.check(QStringLiteral("qpi2: the bar has the %1 field").arg(f == 0 ? QStringLiteral("Freq") : f == 1 ? QStringLiteral("Gain") : QStringLiteral("Q")),
                p.obj(QStringLiteral("PeqBar.input.%1").arg(f)) && p.obj(QStringLiteral("PeqBar.input.%1").arg(f))->property("enabled").toBool());
    for (const char *b : { "shape", "bypass", "invert", "delete" })
        d.check(QStringLiteral("qpi2: and the %1 button").arg(QLatin1String(b)), p.obj(QStringLiteral("PeqBar.") + QLatin1String(b)) != nullptr);
    QObject *gainField = p.obj(QStringLiteral("PeqBar.input.1"));
    d.check(QStringLiteral("qp4: the gain field shows the band's gain, cut to two places"),
            gainField && gainField->property("text").toString() == Peq::truncated(p.ed->band(p.ch, 0, false).gain, true));
    d.check(QStringLiteral("qpi2: screenshot of a band in the bar"), d.grab(p.w, QStringLiteral("window-main-peq-bar.png")));

    // A click on empty graph deselects, and the bar says how to pick a band
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, p.empty(0.2, -18));
    p.settle();
    d.check(QStringLiteral("qp4: a click on empty graph deselects"), p.selected().isEmpty() && p.ov->chipBand() < 0);
    QObject *placeholder = p.obj(QStringLiteral("PeqBar.placeholder"));
    d.check(QStringLiteral("qpi2: with nothing selected the bar stays, with its placeholder"),
            bar && bar->isVisible() && placeholder && placeholder->property("visible").toBool());
    d.check(QStringLiteral("qpi2: screenshot of the empty bar"), d.grab(p.w, QStringLiteral("window-main-peq-bar-empty.png")));
    // Hovering a dot that is not selected lights it and leaves the bar alone
    p.hover(p.dot(0) + QPoint(1, 0));
    p.hover(p.dot(0));
    d.check(QStringLiteral("qpi2: hovering a dot does not put it in the bar"), p.ov->pointedBand() == 0 && p.ov->chipBand() < 0);

    // Double-click on a dot: type its values
    QTest::mouseDClick(p.w, Qt::LeftButton, Qt::NoModifier, p.dot(0));
    p.settle();
    QObject *input = p.obj(QStringLiteral("PeqBar.input.0"));
    d.check(QStringLiteral("qp4: a double-click on a dot opens its frequency for typing"),
            input && input->property("visible").toBool() && input->property("activeFocus").toBool());
    d.check(QStringLiteral("qp4: the field starts with the bare number"),
            input && input->property("text").toString() == Peq::shortFrequency(p.ed->band(p.ch, 0, false).freq));
    if (input) {
        // -- WP USR1 -- typed as a person types: leaving a field sends only what was typed
        QTest::keyClick(p.w, Qt::Key_A, Qt::ControlModifier);
        QTest::keyClick(p.w, '2');
        QTest::keyClick(p.w, 'k');
        QTest::keyClick(p.w, Qt::Key_Tab);
        p.settle();
    }
    d.check(QStringLiteral("qp4: \"2k\" types 2 kHz"), near(p.band(0).freq, 2000, 0.5));
    QObject *gainInput = p.obj(QStringLiteral("PeqBar.input.1"));
    d.check(QStringLiteral("qp4: Tab moves on to the gain"), gainInput && gainInput->property("activeFocus").toBool());
    if (gainInput) {
        gainInput->setProperty("text", QStringLiteral("-3.5 dB"));
        QTest::keyClick(p.w, Qt::Key_Return);
        p.settle();
    }
    d.check(QStringLiteral("qp4: a typed gain lands"), near(p.band(0).gain, -3.5, 0.06));
    d.check(QStringLiteral("qp4: a note name types its pitch"), p.ov->chipText(Peq::Freq, QStringLiteral("A4")) && (p.settle(), near(p.band(0).freq, 440, 0.5)));
    d.check(QStringLiteral("qp4: text that is not a number is refused"), !p.ov->chipText(Peq::Width, QStringLiteral("wide")));
    double hz = 0;
    d.check(QStringLiteral("qp4: C#2+13 is C sharp 2 raised 13 cents"),
            Peq::parseFrequency(QStringLiteral("C#2+13"), &hz) && near(hz, 440 * std::pow(2.0, (37.13 - 69) / 12), 1e-6));
}

void dragging(Peqs &p) {
    SmokeDriver &d = p.d;
    p.put(0, FILTER_PEAKING, 1000, 1.0, 0.0);
    // A plain drag: frequency and gain
    const QPointF start = p.ov->dotPoint(0);
    QSignalSpy live(p.ed, &PeqEditorController::changed);
    QTest::mouseMove(p.w, p.at(start));
    QTest::mousePress(p.w, Qt::LeftButton, Qt::NoModifier, p.at(start));
    bool sawLive = false;
    for (int i = 1; i <= 8; ++i) {
        QTest::mouseMove(p.w, p.at(start + QPointF(40, -30) * i / 8));
        d.spin(10);
        sawLive = sawLive || p.ed->live(0).isValid();
    }
    d.check(QStringLiteral("qp4: the band list gets live values during a drag"), sawLive);
    d.check(QStringLiteral("qp4: a drag lights its band"), p.ov->chipBand() == 0);
    QTest::mouseRelease(p.w, Qt::LeftButton, Qt::NoModifier, p.at(start + QPointF(40, -30)));
    p.settle();
    const double wantHz = p.freqAt(start.x() + 40);
    const double wantDb = 30 * (p.g->dbTop() - p.g->dbBottom()) / p.ov->height();
    d.check(QStringLiteral("qp4: dragging a bell moves its frequency with the pointer"), nearRatio(p.band(0).freq, wantHz, 0.01));
    d.check(QStringLiteral("qp4: and its gain"), near(p.band(0).gain, wantDb, 0.11));
    d.check(QStringLiteral("qp4: the live values end with the drag"), !p.ed->live(0).isValid());

    // Alt: one axis
    const FilterParams before = p.band(0);
    p.drag(p.dot(0), p.dot(0) + QPoint(50, 6), Qt::AltModifier);
    d.check(QStringLiteral("qp4: Alt holds a mostly sideways drag to frequency"), near(p.band(0).gain, before.gain, 0.06) && p.band(0).freq > before.freq * 1.05);

    // Shift: fine
    const FilterParams fine0 = p.band(0);
    p.drag(p.dot(0), p.dot(0) + QPoint(0, -50), Qt::ShiftModifier);
    const double fineDb = 50 * 0.12 * (p.g->dbTop() - p.g->dbBottom()) / p.ov->height();
    d.check(QStringLiteral("qp4: Shift drags at an eighth of the speed"), near(p.band(0).gain - fine0.gain, fineDb, 0.12));

    // Ctrl: Q of the selection
    const FilterParams q0 = p.band(0);
    p.drag(p.dot(0), p.dot(0) + QPoint(0, -60), Qt::ControlModifier);
    d.check(QStringLiteral("qp4: Ctrl-drag widens Q, an octave per 60 points"), nearRatio(p.band(0).q, q0.q * 2.0, 0.02) && near(p.band(0).freq, q0.freq, 0.5));

    // A second-order cut's dot changes Q vertically
    p.put(2, FILTER_HIGHPASS, 80, 0.707, 0);
    const double q2 = p.band(2).q;
    p.drag(p.dot(2), p.dot(2) + QPoint(0, -int(p.yOf(0) - p.yOf(3))));
    d.check(QStringLiteral("qp4: dragging a high-pass up raises its resonance"), p.band(2).q > q2 * 1.2);

    // -- WP P0W2 -- one drag mixing sideways, diagonal and upward moves of
    // one band (frequency alone, both, gain alone) is one undo step
    p.put(0, FILTER_PEAKING, 1000, 1.0, 0.0);
    const FilterParams mixed0 = p.band(0);
    const QPoint from = p.dot(0);
    QTest::mouseMove(p.w, from);
    QTest::mousePress(p.w, Qt::LeftButton, Qt::NoModifier, from);
    QPoint at = from;
    for (const QPoint step : { QPoint(8, 0), QPoint(8, 0), QPoint(8, -8), QPoint(8, -8), QPoint(0, -8), QPoint(0, -8), QPoint(8, 0) }) {
        at += step;
        QTest::mouseMove(p.w, at);
        d.spin(10);
    }
    QTest::mouseRelease(p.w, Qt::LeftButton, Qt::NoModifier, at);
    p.settle();
    const FilterParams mixed1 = p.band(0);
    d.check(QStringLiteral("p0w2: a mixed-axis drag moves the band's frequency and gain"),
            mixed1.freq > mixed0.freq * 1.05 && mixed1.gain > mixed0.gain + 0.5);
    d.app().shell->trigger(QStringLiteral("edit.undo"));
    d.check(QStringLiteral("p0w2: one undo puts the band back as it was before the drag"), d.waitFor(3000, [&]() {
        const FilterParams now = p.band(0);
        return near(now.freq, mixed0.freq, 0.01) && near(now.gain, mixed0.gain, 0.001) && near(now.q, mixed0.q, 0.0001);
    }));
    p.settle();
}

void pullAndMarquee(Peqs &p) {
    SmokeDriver &d = p.d;
    p.clear();
    // Pull a band out of the flat curve near the low end: a low shelf
    const double curveY = p.yOf(p.ov->curveDbAt(p.freqAt(p.ov->width() * 0.05)));
    const QPoint onCurve = p.at(QPointF(p.ov->width() * 0.05, curveY));
    p.drag(onCurve, onCurve + QPoint(0, -30));
    d.check(QStringLiteral("qp4: pulling the curve near the low end makes a low shelf"),
            p.band(0).filter_type == FILTER_LOWSHELF && p.band(0).gain > 1.0);
    const QPoint middle = p.at(QPointF(p.ov->width() * 0.5, p.yOf(p.ov->curveDbAt(p.freqAt(p.ov->width() * 0.5)))));
    p.drag(middle, middle + QPoint(0, 25));
    d.check(QStringLiteral("qp4: pulling it in the middle makes a bell, down when pulled down"),
            p.band(1).filter_type == FILTER_PEAKING && p.band(1).gain < -1.0);

    // A marquee around both dots selects both; Shift adds to the selection
    p.put(2, FILTER_PEAKING, 8000, 1, 8);
    const QPointF a = p.ov->dotPoint(0), b = p.ov->dotPoint(1);
    const QPointF tl(qMin(a.x(), b.x()) - 8, 4), br(qMax(a.x(), b.x()) + 8, p.ov->height() - 4);
    p.drag(p.at(QPointF(tl.x(), 4)), p.at(br));
    d.check(QStringLiteral("qp4: a marquee selects the dots inside it"), p.selected() == (QSet<int>{ 0, 1 }));
    const QPointF c = p.ov->dotPoint(2);
    p.drag(p.at(c + QPointF(-12, -12)), p.at(c + QPointF(12, 12)), Qt::ShiftModifier);
    d.check(QStringLiteral("qp4: Shift-marquee adds to the selection"), p.selected() == (QSet<int>{ 0, 1, 2 }));
    d.check(QStringLiteral("qp4: the marquee is gone after the release"), !p.ov->marqueeShown());

    // Clicks: Ctrl toggles, Shift takes a frequency range
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, p.dot(0));
    p.settle();
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::ControlModifier, p.dot(2));
    p.settle();
    d.check(QStringLiteral("qp4: Ctrl-click adds a band to the selection"), p.selected() == (QSet<int>{ 0, 2 }));
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::ControlModifier, p.dot(0));
    p.settle();
    d.check(QStringLiteral("qp4: and takes it out again"), p.selected() == QSet<int>{ 2 });
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, p.dot(0));
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::ShiftModifier, p.dot(2));
    p.settle();
    d.check(QStringLiteral("qp4: Shift-click selects the run by frequency"), p.selected() == (QSet<int>{ 0, 1, 2 }));

    // A drag of a selection moves every band by the same ratio
    const FilterParams f0 = p.band(0), f2 = p.band(2);
    p.drag(p.dot(1), p.dot(1) + QPoint(30, 0), Qt::NoModifier);
    d.check(QStringLiteral("qp4: dragging a selection moves all its bands by one ratio"),
            nearRatio(p.band(0).freq / f0.freq, p.band(2).freq / f2.freq, 0.01) && p.band(0).freq > f0.freq * 1.05);

    // -- WP P0W2 (O1) -- the whole selection's drag, sideways then up, is
    // one undo step: one undo puts every band back
    {
        const FilterParams m0 = p.band(0), m1 = p.band(1), m2 = p.band(2);
        const QPoint from = p.dot(1);
        QTest::mouseMove(p.w, from);
        QTest::mousePress(p.w, Qt::LeftButton, Qt::NoModifier, from);
        QPoint at = from;
        for (const QPoint step : { QPoint(8, 0), QPoint(8, -8), QPoint(0, -8), QPoint(8, 0) }) {
            at += step;
            QTest::mouseMove(p.w, at);
            d.spin(10);
        }
        QTest::mouseRelease(p.w, Qt::LeftButton, Qt::NoModifier, at);
        p.settle();
        d.check(QStringLiteral("p0w2: a selection's drag moves every band"),
                p.band(0).freq > m0.freq * 1.02 && p.band(1).freq > m1.freq * 1.02 && p.band(2).freq > m2.freq * 1.02);
        auto same = [](const FilterParams &a, const FilterParams &b) {
            return near(a.freq, b.freq, 0.01) && near(a.gain, b.gain, 0.001) && near(a.q, b.q, 0.0001);
        };
        d.app().shell->trigger(QStringLiteral("edit.undo"));
        d.check(QStringLiteral("p0w2: one undo puts every band of the selection back"), d.waitFor(3000, [&]() {
            return same(p.band(0), m0) && same(p.band(1), m1) && same(p.band(2), m2);
        }));
        p.settle();
    }

    // Alt-click: bypass
    if (p.cc->bypassSupported()) {
        QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, p.dot(1));
        p.settle();
        QTest::mouseClick(p.w, Qt::LeftButton, Qt::AltModifier, p.dot(1));
        p.settle();
        d.check(QStringLiteral("qp4: Alt-click bypasses a band"), p.band(1).bypass && !p.band(0).bypass);
        QObject *power = p.obj(QStringLiteral("PeqBar.bypass"));
        if (power) QMetaObject::invokeMethod(power, "clicked");
        p.settle();
        d.check(QStringLiteral("qp4: the bar's power button enables it again"), !p.band(1).bypass);
    }
}

void wheelAndKeys(Peqs &p) {
    SmokeDriver &d = p.d;
    p.clear();
    p.put(0, FILTER_PEAKING, 500, 1.0, 3.0);
    p.put(1, FILTER_PEAKING, 4000, 2.0, -3.0);
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, p.dot(0));
    p.settle();
    // Wheel: Q; Ctrl-wheel: gain; committed after a pause
    p.wheel(p.dot(0), 120);
    d.check(QStringLiteral("qp4: the wheel shows the new Q at once"), nearRatio(p.ov->shownBand(0).value(QStringLiteral("q")).toDouble(), std::pow(2.0, 0.08), 0.01));
    d.waitFor(1500, [&]() { return nearRatio(p.band(0).q, std::pow(2.0, 0.08), 0.01); });
    d.check(QStringLiteral("qp4: a wheel notch narrows Q by a twelfth of an octave"), nearRatio(p.band(0).q, std::pow(2.0, 0.08), 0.01));
    p.wheel(p.dot(0), 120, Qt::ControlModifier);
    d.waitFor(1500, [&]() { return near(p.band(0).gain, 3.4, 0.06); });
    d.check(QStringLiteral("qp4: Ctrl-wheel raises the gain"), near(p.band(0).gain, 3.4, 0.06));
    // The selection owns the wheel, even over another band
    p.wheel(p.dot(1), 120);
    d.waitIdle();
    d.spin(600);
    p.settle();
    d.check(QStringLiteral("qp4: the wheel over another band adjusts the selection"), near(p.band(1).q, 2.0, 1e-3) && p.band(0).q > 1.1);
    // The dB zoom strip at the left edge
    const double span = p.g->dbRange();
    p.wheel(p.at(QPointF(10, p.ov->height() * 0.1)), 120);
    d.check(QStringLiteral("qp4: the wheel at the left edge zooms the dB scale"), near(p.g->dbRange(), span - 3, 1e-6));
    p.g->setDbRange(span);
    p.settle();

    // Keys
    p.ov->forceActiveFocus();
    const FilterParams k0 = p.band(0);
    p.key(Qt::Key_Right);
    p.key(Qt::Key_Up);
    p.key(Qt::Key_Up, Qt::AltModifier);
    d.waitFor(1500, [&]() { return near(p.band(0).gain, k0.gain + 0.5, 0.06); });
    p.settle();
    d.check(QStringLiteral("qp4: Right raises the frequency a semitone"), nearRatio(p.band(0).freq, k0.freq * std::pow(2.0, 1.0 / 12), 0.002));
    d.check(QStringLiteral("qp4: Up raises the gain half a dB"), near(p.band(0).gain, k0.gain + 0.5, 0.06));
    d.check(QStringLiteral("qp4: Alt+Up narrows Q"), nearRatio(p.band(0).q, k0.q * std::pow(2.0, 0.1), 0.01));
    p.key(Qt::Key_Tab);
    d.check(QStringLiteral("qp4: Tab steps to the next band by frequency"), p.selected() == QSet<int>{ 1 } && p.ov->chipBand() == 1);
    p.key(Qt::Key_Tab, Qt::ShiftModifier);
    d.check(QStringLiteral("qp4: Shift+Tab steps back"), p.selected() == QSet<int>{ 0 });
    p.key(Qt::Key_A, Qt::ControlModifier);
    d.check(QStringLiteral("qp4: Ctrl+A selects every band (not the Edit menu's Select All)"), p.selected() == (QSet<int>{ 0, 1 }));
    p.key(Qt::Key_Escape);
    d.check(QStringLiteral("qp4: Escape deselects"), p.selected().isEmpty());
    // -- WP QP18 -- With nothing selected Tab leaves the graph (no keyboard
    // trap); back on it, an arrow picks the band after the last one stepped to
    p.key(Qt::Key_Tab);
    d.check(QStringLiteral("qp18: Tab after Escape leaves the graph"), !p.ov->hasActiveFocus() && p.selected().isEmpty());
    p.ov->forceActiveFocus();
    p.key(Qt::Key_Right);
    d.check(QStringLiteral("qp18: an arrow with nothing selected picks the next band"), p.selected() == QSet<int>{ 1 });
    p.key(Qt::Key_Return);
    p.settle();
    QObject *input = p.obj(QStringLiteral("PeqBar.input.0"));
    d.check(QStringLiteral("qp4: Enter opens the bar's values for typing"), input && input->property("visible").toBool());
    QTest::keyClick(p.w, Qt::Key_Escape);
    p.settle();
    d.check(QStringLiteral("qp4: Escape in the field leaves the band as it was, and the keys go back to the graph"),
            input && !input->property("activeFocus").toBool() && p.ov->hasActiveFocus() && p.selected() == QSet<int>{ 1 });
    p.ov->forceActiveFocus();
    QSignalSpy menus(p.ov, &PeqOverlay::menuRequested);
    p.key(Qt::Key_F10, Qt::ShiftModifier);
    QObject *bandMenu = p.obj(QStringLiteral("PeqEditor.bandMenu"));
    d.check(QStringLiteral("qp4: Shift+F10 opens the band menu"),
            menus.count() == 1 && bandMenu && d.waitFor(1000, [&]() { return bandMenu->property("opened").toBool(); }));
    if (bandMenu) QMetaObject::invokeMethod(bandMenu, "close");
    d.spin(50);
    p.ov->forceActiveFocus();
    p.key(Qt::Key_Delete);
    p.settle();
    d.check(QStringLiteral("qp4: Delete removes the selected band"), p.band(1).filter_type == FILTER_FLAT && p.band(0).filter_type == FILTER_PEAKING);
    // (The pointer still rests on band 0's dot, whose chip may show)
    d.check(QStringLiteral("qp4: and the selection and the chip with it"), p.selected().isEmpty() && p.ov->chipBand() != 1);
}

void chipShapesCardMenus(Peqs &p) {
    SmokeDriver &d = p.d;
    p.clear();
    const QList<int> types = p.ed->availableTypes();
    // Ctrl-click: the card, then a shape; one with two orders asks its slope
    const QPoint where = p.empty(0.7, 6);
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::ControlModifier, where);
    p.settle();
    d.check(QStringLiteral("qp4: Ctrl-click on empty graph opens the shape card"), p.ov->cardOpen());
    d.check(QStringLiteral("qp4: its ghost dot marks where the band will go"), p.ov->ghostShown());
    QObject *card = p.obj(QStringLiteral("PeqCard"));
    d.check(QStringLiteral("qp4: screenshot of the shape card"), d.grab(p.w, QStringLiteral("window-main-peq-card.png")));
    QObject *lowShelf = p.obj(QStringLiteral("PeqCard.shape.%1").arg(Peq::LowShelf));
    d.check(QStringLiteral("qp4: the card offers the low shelf"), card && lowShelf);
    const bool twoShelves = types.contains(FILTER_LOWSHELF1);
    if (lowShelf) QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, centre(lowShelf));
    p.settle();
    if (twoShelves) {
        QObject *six = p.obj(QStringLiteral("PeqCard.order.1"));
        d.check(QStringLiteral("qp4: a shelf asks its slope first"), six && six->property("visible").toBool() && p.band(0).filter_type == FILTER_FLAT);
        if (six) QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, centre(six));
        p.settle();
    }
    const QPointF local = p.ov->mapFromScene(QPointF(where));
    d.check(QStringLiteral("qp4: the card makes the shape at the click, its level as the gain"),
            p.band(0).filter_type == (twoShelves ? FILTER_LOWSHELF1 : FILTER_LOWSHELF) && near(p.band(0).gain, p.dbAt(local.y()), 0.11)
                && nearRatio(p.band(0).freq, p.freqAt(local.x()), 0.01));
    d.check(QStringLiteral("qp4: the card closes into the band's chip"), !p.ov->cardOpen() && p.ov->chipBand() == 0 && p.selected() == QSet<int>{ 0 });
    // A click elsewhere closes an open card and does nothing else
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::ControlModifier, p.empty(0.3, -10));
    p.settle();
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, p.empty(0.5, -15));
    p.settle();
    d.check(QStringLiteral("qp4: a click beside the card closes it, adding nothing"), !p.ov->cardOpen() && p.band(1).filter_type == FILTER_FLAT);

    // The bar's shape chooser: High Cut, then 12 dB
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, p.dot(0));
    p.settle();
    QObject *shapeButton = p.obj(QStringLiteral("PeqBar.shape"));
    if (shapeButton) QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, centre(shapeButton));
    p.settle();
    QObject *shapesPopup = p.obj(QStringLiteral("PeqBar.shapesPopup"));
    d.check(QStringLiteral("qp4: the bar's shape button opens the shape chooser"),
            p.ov->shapePage() && shapesPopup && d.waitFor(1000, [&]() { return shapesPopup->property("opened").toBool(); }));
    QObject *own = p.obj(QStringLiteral("PeqBar.shapes.shape.%1").arg(Peq::LowShelf));
    d.check(QStringLiteral("qp4: the page marks the band's own shape"), own && own->property("marked").toBool());
    d.check(QStringLiteral("qp4: screenshot of the shape page"), d.grab(p.w, QStringLiteral("window-main-peq-shapes.png")));
    QObject *highCut = p.obj(QStringLiteral("PeqBar.shapes.shape.%1").arg(Peq::HighCut));
    if (highCut) QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, centre(highCut));
    p.settle();
    if (types.contains(FILTER_LOWPASS1)) {
        QObject *twelve = p.obj(QStringLiteral("PeqBar.shapes.order.2"));
        if (twelve) QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, centre(twelve));
        p.settle();
    }
    d.check(QStringLiteral("qp4: the shape page reshapes the band, keeping its frequency"), p.band(0).filter_type == FILTER_LOWPASS && near(p.band(0).gain, 0, 1e-6));
    d.check(QStringLiteral("qp4: and closes"), !p.ov->shapePage() && shapesPopup && d.waitFor(1000, [&]() { return !shapesPopup->property("visible").toBool(); }));

    // The band menu: slope, invert, delete
    QSignalSpy menus(p.ov, &PeqOverlay::menuRequested);
    QTest::mouseClick(p.w, Qt::RightButton, Qt::NoModifier, p.dot(0));
    d.spin(50);
    const QVariantMap m = p.ov->menuModel();
    d.check(QStringLiteral("qp4: a right-click on a dot opens the band menu"), menus.count() == 1 && m.value(QStringLiteral("kind")) == QLatin1String("band")
                && m.value(QStringLiteral("title")) == QLatin1String("Band 1"));
    d.check(QStringLiteral("qp4: it offers the cut's two slopes"), m.value(QStringLiteral("orders")).toList().size() == (types.contains(FILTER_LOWPASS1) ? 2 : 0));
    QObject *bandMenu = p.obj(QStringLiteral("PeqEditor.bandMenu"));
    if (bandMenu) QMetaObject::invokeMethod(bandMenu, "close");
    if (types.contains(FILTER_LOWPASS1)) {
        p.ov->menuAction(QStringLiteral("order"), 1);
        p.settle();
        d.check(QStringLiteral("qp4: the slope row makes it 6 dB/oct"), p.band(0).filter_type == FILTER_LOWPASS1);
    }
    p.put(1, FILTER_PEAKING, 3000, 1, 4);
    QTest::mouseClick(p.w, Qt::RightButton, Qt::NoModifier, p.dot(1));
    d.spin(50);
    d.check(QStringLiteral("qp4: a bell's menu offers Invert Gain"), p.ov->menuModel().value(QStringLiteral("invert")).toBool());
    if (bandMenu) QMetaObject::invokeMethod(bandMenu, "close");
    p.ov->menuAction(QStringLiteral("invert"));
    p.settle();
    d.check(QStringLiteral("qp4: Invert Gain flips the gain"), near(p.band(1).gain, -4, 0.06));
    // The graph menu
    QTest::mouseClick(p.w, Qt::RightButton, Qt::NoModifier, p.empty(0.15, -20));
    d.spin(50);
    QObject *graphMenu = p.obj(QStringLiteral("PeqEditor.graphMenu"));
    d.check(QStringLiteral("qp4: a right-click on empty graph opens the graph menu"),
            p.ov->menuModel().value(QStringLiteral("kind")) == QLatin1String("graph") && graphMenu && d.waitFor(1000, [&]() { return graphMenu->property("opened").toBool(); }));
    d.check(QStringLiteral("qp4: screenshot of the graph menu"), d.grab(p.w, QStringLiteral("window-main-peq-menu.png")));
    if (graphMenu) QMetaObject::invokeMethod(graphMenu, "close");
    p.ov->menuAction(QStringLiteral("selectAll"));
    d.check(QStringLiteral("qp4: Select All Bands selects every band"), p.selected() == (QSet<int>{ 0, 1 }));
    p.ov->menuAction(QStringLiteral("deleteSelected"));
    p.settle();
    d.check(QStringLiteral("qp4: Delete Selected Bands clears them"), p.band(0).filter_type == FILTER_FLAT && p.band(1).filter_type == FILTER_FLAT);
}

void chipValues(Peqs &p) {
    // -- WP QPI2 -- the bar's values and buttons (the chip's drag on a value is
    // gone with the chip: the bar's fields are typed or scrolled)
    SmokeDriver &d = p.d;
    p.clear();
    p.put(0, FILTER_PEAKING, 700, 1.0, 2.0);
    QTest::mouseClick(p.w, Qt::LeftButton, Qt::NoModifier, p.dot(0));
    p.settle();
    // Scrolling a value: the Q by an eightieth of an octave per half point
    QObject *q = p.obj(QStringLiteral("PeqBar.input.2"));
    if (q) p.wheel(centre(q), 120);
    d.waitFor(1500, [&]() { return p.band(0).q > 1.01; });
    d.check(QStringLiteral("qp4: scrolling the bar's Q changes Q"), nearRatio(p.band(0).q, std::pow(2.0, 4.0 / 80), 0.01));
    d.check(QStringLiteral("qp4: and leaves the gain"), near(p.band(0).gain, 2.0, 0.06));
    QObject *invert = p.obj(QStringLiteral("PeqBar.invert"));
    if (invert) QMetaObject::invokeMethod(invert, "clicked");
    p.settle();
    d.check(QStringLiteral("qpi2: the bar's invert flips the gain"), near(p.band(0).gain, -2.0, 0.06));
    QObject *remove = p.obj(QStringLiteral("PeqBar.delete"));
    if (remove) QMetaObject::invokeMethod(remove, "clicked");
    p.settle();
    d.check(QStringLiteral("qpi2: and its delete removes the band, leaving the bar empty"),
            p.band(0).filter_type == FILTER_FLAT && p.ov->chipBand() < 0 && p.selected().isEmpty());
}

void popOut(Peqs &p) {
    SmokeDriver &d = p.d;
    Shell *shell = d.app().shell;
    shell->trigger(QStringLiteral("window.popOutGraph"));
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("window.popOutGraph")));
    d.check(QStringLiteral("qp4: the pop-out window opens"), w && d.waitFor(3000, [&]() { return w->isVisible(); }));
    if (!w) return;
    d.spin(100);
    auto *pop = qobject_cast<PeqOverlay *>(d.item(QStringLiteral("GraphWindow.peq.overlay"), w->contentItem()));
    d.check(QStringLiteral("qp4: the pop-out has its own editor"), pop != nullptr);
    d.check(QStringLiteral("qp4: it edits the page while it follows, and the main graph's stands aside"),
            pop && d.waitFor(2000, [&]() { return pop->editedChannel() == p.ch; }) && p.ov->editedChannel() == -1 && p.g->overlayChannel() == p.ch);
    // -- WP QPI2 -- the pop-out has the band bar under its graph too
    p.put(0, FILTER_PEAKING, 1000, 1, 3);
    p.ed->rowClicked(0, Qt::NoModifier);
    p.settle();
    auto *popBar = qobject_cast<QQuickItem *>(d.item(QStringLiteral("GraphWindow.peqBar"), w->contentItem()));
    auto *popPlot = qobject_cast<QQuickItem *>(d.item(QStringLiteral("GraphWindow.plot"), w->contentItem()));
    QObject *popBand = d.item(QStringLiteral("GraphWindow.peqBar.band"), w->contentItem());
    d.check(QStringLiteral("qpi2: the pop-out's bar sits under its graph and shows the selected band"),
            popBar && popPlot && popBar->isVisible()
                && popBar->mapToScene(QPointF(0, 0)).y() >= popPlot->mapToScene(QPointF(0, popPlot->height())).y()
                && popBand && popBand->property("text").toString() == QLatin1String("Band 1") && pop && pop->chipBand() == 0);
    p.g->setPopoutFollowsSelection(false);
    d.spin(50);
    d.check(QStringLiteral("qp4: a pop-out that does not follow edits nothing"), pop && pop->editedChannel() == -1 && p.g->overlayChannel() == -1);
    p.g->setPopoutFollowsSelection(true);
    p.ed->rowClicked(0, Qt::NoModifier); // -- WP QPI2 -- the shot shows the bar with a band
    p.settle();
    d.check(QStringLiteral("qp4: screenshot of the pop-out's editor"), (d.spin(100), d.grab(w, QStringLiteral("window-graph-peq.png"))));
    p.g->closePopOut();
    d.waitFor(3000, [&]() { return !p.g->poppedOut(); });
    d.check(QStringLiteral("qp4: closing it hands the editing back"), d.waitFor(2000, [&]() { return p.ov->editedChannel() == p.ch && p.g->overlayChannel() == p.ch; }));
}

// -- WP QPI2 -- The closed pop-out keeps its window and its editor; a range of
// rows picked with a snapshot between the clicks (which resets that editor) once
// sorted the selection over its empty band list (review w2 F11)
void closedPopOut(Peqs &p) {
    SmokeDriver &d = p.d;
    PeqOverlay *pop = nullptr;
    for (QWindow *w : QGuiApplication::allWindows())
        if (auto *q = qobject_cast<QQuickWindow *>(w); q && q->objectName() == QLatin1String("GraphWindow"))
            pop = qobject_cast<PeqOverlay *>(d.item(QStringLiteral("GraphWindow.peq.overlay"), q->contentItem()));
    d.check(QStringLiteral("qpi2: the closed pop-out's editor is still there and edits nothing"),
            pop && !pop->window()->isVisible() && pop->editedChannel() == -1);
    p.clear();
    p.put(1, FILTER_PEAKING, 3000, 1, -3);
    p.put(4, FILTER_PEAKING, 9000, 1, 2);
    p.ed->rowClicked(1, Qt::NoModifier);
    p.settle();
    p.put(2, FILTER_PEAKING, 5000, 1, 1);
    p.ed->rowClicked(4, Qt::ShiftModifier);
    p.settle();
    d.check(QStringLiteral("qpi2: Shift-click after a snapshot selects the run on the main graph"),
            p.selected() == (QSet<int>{ 1, 2, 4 }) && p.ov->chipBand() >= 0);
    d.check(QStringLiteral("qpi2: and the closed pop-out's editor shows no chip for it"), pop && pop->chipBand() == -1);
    p.ed->rowClicked(1, Qt::NoModifier);
    p.clear();
}

void bandList(Peqs &p) {
    SmokeDriver &d = p.d;
    p.clear();
    p.put(0, FILTER_PEAKING, 300, 1, 3);
    p.put(1, FILTER_PEAKING, 3000, 1, -3);
    p.put(4, FILTER_PEAKING, 9000, 1, 2);
    QObject *row = p.obj(QStringLiteral("OutputDetail.bands.band.1"));
    d.check(QStringLiteral("qp4: the page's band list is the editor's"), row != nullptr);
    p.ed->rowClicked(1, Qt::NoModifier);
    p.settle();
    d.check(QStringLiteral("qp4: a click on a row's number selects the band on the graph"), p.selected() == QSet<int>{ 1 } && p.ov->chipBand() == 1);
    d.check(QStringLiteral("qp4: its row shows it selected"), row && row->property("selected").toBool());
    d.check(QStringLiteral("qp4: and in its band's colour"), row && row->property("bandColor").value<QColor>() == p.ov->bandColors().value(1).value<QColor>());
    p.ed->rowClicked(4, Qt::ShiftModifier);
    d.check(QStringLiteral("qp4: Shift-click takes the run of rows, empty ones skipped"), p.selected() == (QSet<int>{ 1, 4 }));
    p.ed->rowClicked(0, Qt::ControlModifier);
    d.check(QStringLiteral("qp4: Ctrl-click on a row adds it"), p.selected() == (QSet<int>{ 0, 1, 4 }));
    p.ed->setRowHovered(4);
    d.check(QStringLiteral("qp4: a hovered row lights its dot"), p.ed->rowHovered() == 4);
    p.ed->setRowHovered(-1);
    // A rest on a dot brings its row into view
    QSignalSpy reveal(p.ed, &PeqEditorController::showRow);
    p.ed->rowClicked(0, Qt::NoModifier);
    p.hover(p.dot(4) + QPoint(1, 0));
    p.hover(p.dot(4));
    d.check(QStringLiteral("qp4: resting on a dot asks for its row"), reveal.wait(1500) && reveal.last().value(0).toInt() == 4);
    d.check(QStringLiteral("qp4: the graph's hovered band is shared with the list"), p.ed->pointedBand() == 4);
    p.hover(p.empty(0.02, -20));
}

void animated(Peqs &p) {
    SmokeDriver &d = p.d;
    p.clear();
    p.put(0, FILTER_PEAKING, 1000, 1, 0);
    // A change made in the band list moves the dot over a fifth of a second
    p.cc->setBandValue(p.ch, 0, false, QStringLiteral("gain"), 9);
    d.spin(30);
    d.check(QStringLiteral("qp4: a band changed elsewhere moves on the graph"), p.ov->animating());
    const double mid = p.ov->shownBand(0).value(QStringLiteral("gain")).toDouble();
    d.check(QStringLiteral("qp4: part-way at first"), mid > 0.01 && mid < 9);
    d.waitFor(1500, [&]() { return !p.ov->animating(); });
    d.check(QStringLiteral("qp4: and lands on the new value"), near(p.ov->shownBand(0).value(QStringLiteral("gain")).toDouble(), 9, 0.06));
}

void allBands(Peqs &p) {
    SmokeDriver &d = p.d;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) {
        p.cc->setBandType(p.ch, b, false, FILTER_PEAKING);
        p.cc->setBandValue(p.ch, b, false, QStringLiteral("freq"), 50 * std::pow(1.6, b));
    }
    p.settle();
    p.hover(p.empty(0.5, -20) + QPoint(1, 0));
    p.hover(p.empty(0.5, -20));
    QObject *axis = p.obj(QStringLiteral("PeqEditor.frequencyReadout"));
    d.check(QStringLiteral("qp4: with every band in use the readout says so"),
            axis && axis->property("shownName").toString() == QStringLiteral("All 10 bands in use") && !p.ov->ghostShown());
    QSignalSpy refused(p.ov, &PeqOverlay::refused);
    QTest::mouseDClick(p.w, Qt::LeftButton, Qt::NoModifier, p.empty(0.5, -20));
    p.settle();
    d.check(QStringLiteral("qp4: and a double-click adds nothing"), refused.count() >= 1);
    p.clear();
}

void linkedPair(Peqs &p, SidebarController *sc) {
    SmokeDriver &d = p.d;
    if (!p.cc->pairAvailable(0)) return;
    if (!p.cc->isLinked(0)) p.cc->linkPair(0, 0);
    sc->select(0);
    p.ch = 0;
    d.waitFor(2000, [&]() { return p.ov->editedChannel() == 0; });
    p.clear();
    p.cc->clearBands(1, false);
    p.settle();
    QTest::mouseDClick(p.w, Qt::LeftButton, Qt::NoModifier, p.empty(0.5, 6));
    p.settle();
    d.check(QStringLiteral("qp4: on a linked input a new band lands on both inputs"),
            p.s() && p.s()->filters[0][0].filter_type == FILTER_PEAKING && p.s()->filters[1][0].filter_type == FILTER_PEAKING
                && near(p.s()->filters[1][0].freq, p.s()->filters[0][0].freq, 0.01));
    p.clear();
    p.cc->clearBands(1, false);
    p.settle();
}

void standsAside(Peqs &p, SidebarController *sc) {
    SmokeDriver &d = p.d;
    const int out = p.g->firstOutput();
    sc->select(out);
    p.ch = out;
    d.waitFor(2000, [&]() { return p.ov->editedChannel() == out; });
    p.cc->setCrossoverTabShown(true);
    d.spin(30);
    d.check(QStringLiteral("qp4: the XO tab leaves the graph to its crossovers"), p.ov->editedChannel() == -1 && p.g->overlayChannel() == -1);
    p.cc->setCrossoverTabShown(false);
    d.spin(30);
    d.check(QStringLiteral("qp4: back on the PEQ tab the editor returns"), p.ov->editedChannel() == out);
    p.g->setCurveVisible(out, false);
    d.spin(30);
    d.check(QStringLiteral("qp4: a hidden curve is not edited"), p.ov->editedChannel() == -1);
    p.g->setCurveVisible(out, true);
    sc->select(-1);
    d.spin(50);
    d.check(QStringLiteral("qp4: the overview edits nothing"), p.ov->editedChannel() == -1 && p.g->overlayChannel() == -1 && p.ed->editedChannel() == -1);
    // The dB zoom strip still works there
    const double span = p.g->dbRange();
    p.wheel(p.at(QPointF(10, 20)), -120);
    d.check(QStringLiteral("qp4: the zoom strip works with nothing edited"), near(p.g->dbRange(), span + 3, 1e-6));
    p.g->setDbRange(span);

    // Read-only firmware: shown, never edited
    p.d.app().core->worker()->post([](FfiCore *c) { dspi_demo_open_wire(c, 17); });
    p.device->refresh();
    d.waitFor(5000, [&]() { return p.device->readOnly(); });
    d.waitIdle();
    sc->select(p.g->firstOutput());
    d.spin(100);
    d.check(QStringLiteral("qp4: read-only firmware is not edited on the graph"), p.device->readOnly() && p.ov->editedChannel() == -1);
    p.device->openDemo();
    d.waitFor(5000, [&]() { return !p.device->readOnly() && p.device->connected(); });
    d.waitIdle();
    sc->select(-1);
    d.spin(50);
}

void peqSection(SmokeDriver &d) {
    QQuickWindow *w = d.mainWindow();
    auto *ov = qobject_cast<PeqOverlay *>(d.object(QStringLiteral("PeqEditor.overlay")));
    auto *cc = Registrar::instance<ChannelController>();
    auto *g = Registrar::instance<GraphController>();
    auto *ed = Registrar::instance<PeqEditorController>();
    auto *sc = Registrar::instance<SidebarController>();
    d.check(QStringLiteral("qp4: the editor fills the graph's PEQ slot"), ov != nullptr && g && g->slotType(GraphController::PeqOverlay) == QLatin1String("PeqGraphOverlay"));
    if (!w || !ov || !cc || !g || !ed || !sc) return;
    d.waitIdle();
    const int out = g->firstOutput();
    sc->select(out);
    d.check(QStringLiteral("qp4: an output page puts its channel in the editor"), d.waitFor(3000, [&]() { return ov->editedChannel() == out; }));
    Peqs p{ d, w, ov, cc, g, ed, d.app().device, out };
    // Bands as the demo had them, put back at the end
    std::vector<FilterParams> saved;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) saved.push_back(p.band(b));

    owner(p);
    curveFromTheCore(p);
    hoverAndCreate(p);
    dragging(p);
    pullAndMarquee(p);
    wheelAndKeys(p);
    chipShapesCardMenus(p);
    chipValues(p);
    bandList(p);
    animated(p);
    allBands(p);
    popOut(p);
    closedPopOut(p); // -- WP QPI2 --

    QList<QPair<int, FilterParams>> back;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) back << qMakePair(b, saved[size_t(b)]);
    ed->writeBands(out, back);
    p.settle();

    linkedPair(p, sc);
    standsAside(p, sc);
}

const SmokeDriver::Section peq("qp4-peq", 35, &peqSection);

} // namespace
