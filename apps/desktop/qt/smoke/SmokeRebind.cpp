// -- WP MDQT2a -- The editing controllers over two open devices (two demos, A
// and B): what is edited stays with the device it was edited on, however late
// its write runs, and each device keeps its page, curves, selection and drafts.
// Each core's own journal (dspi_history) records what was written to it.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "CoreApi.h"
#include "DashboardController.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "EffectsController.h"
#include "GraphController.h"
#include "MatrixController.h"
#include "NamesController.h"
#include "OutputLatencyController.h"
#include "PeqEditorController.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "SidebarController.h"
#include "Snapshot.h"
#include "render/PeqOverlay.h"

#include <QJsonObject>
#include <QPointer>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSemaphore>
#include <QSignalSpy>
#include <QTest>
#include <QWheelEvent>
#include <cmath>
#include <memory>

namespace {

bool near(double a, double b, double tol = 0.05) { return std::abs(a - b) <= tol; }

const DspState *stateOf(const DeviceSession *s) { return s->model()->state(); }

// A session's worker, held until let go: what is posted meanwhile waits
class Held {
public:
    explicit Held(DeviceSession *s) : m_gate(std::make_shared<QSemaphore>()) {
        std::shared_ptr<QSemaphore> gate = m_gate;
        s->worker()->post([gate](FfiCore *) { gate->tryAcquire(1, 20000); });
    }
    ~Held() { release(); }
    void release() {
        if (!m_done) m_gate->release();
        m_done = true;
    }

private:
    std::shared_ptr<QSemaphore> m_gate;
    bool m_done = false;
};

struct Two {
    SmokeDriver &d;
    DeviceRegistry *r;
    DeviceModel *device;
    QQuickWindow *w;
    DeviceSession *a = nullptr;
    DeviceSession *b = nullptr;

    bool idle(DeviceSession *s) {
        return d.waitFor(8000, [&] { return !s->model()->busy() && s->worker()->pending() == 0; });
    }
    bool bothIdle() {
        const bool ok = idle(a) && idle(b);
        d.spin(60);
        return ok && idle(a) && idle(b);
    }
    // What was written to a session's core, newest first
    QJsonArray journal(DeviceSession *s) {
        auto got = std::make_shared<QJsonArray>();
        auto done = std::make_shared<bool>(false);
        s->worker()->call(
            d.app().device,
            [](FfiCore *c) {
                return jsonValue(readSized([c](char *buf, uint32_t n) { return dspi_history(c, 400, buf, n); }, 1 << 16)).toArray();
            },
            [got, done](const QJsonArray &v) {
                *got = v;
                *done = true;
            });
        d.waitFor(8000, [&] { return *done; });
        return *got;
    }
    int writes(DeviceSession *s) { return int(journal(s).size()); }
    QPoint centre(const QString &name) {
        QQuickItem *i = d.item(name);
        return i ? i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint() : QPoint();
    }
};

void bands(Two &t, ChannelController *cc, int ch) {
    SmokeDriver &d = t.d;
    t.r->activate(t.a);
    cc->clearBands(ch, false);
    t.idle(t.a);
    t.r->activate(t.b);
    cc->clearBands(ch, false);
    t.idle(t.b);
    t.r->activate(t.a);
    cc->setBandType(ch, 0, false, FILTER_PEAKING);
    d.check("mdqt2a: a band on A", t.idle(t.a));
    const int beforeB = t.writes(t.b);
    {
        Held held(t.a);
        cc->setBandValue(ch, 0, false, QStringLiteral("freq"), 432);
        cc->setBandValue(ch, 0, false, QStringLiteral("gain"), 5.5);
        d.check("mdqt2a: the edit shows at once", near(cc->band(ch, 0, false).value(QStringLiteral("gain")).toDouble(), 5.5));
        t.r->activate(t.b);
        const QVariantMap onB = cc->band(ch, 0, false);
        d.check("mdqt2a: B shows its own band, not A's edit",
                onB.value(QStringLiteral("type")).toInt() == FILTER_FLAT && near(onB.value(QStringLiteral("gain")).toDouble(), 0));
        // A's write is still waiting: B's edits go through meanwhile
        cc->setBandType(ch, 0, false, FILTER_LOWSHELF);
        cc->setBandValue(ch, 0, false, QStringLiteral("gain"), -3);
        d.check("mdqt2a: B takes its edit while A's still waits",
                t.idle(t.b) && t.a->model()->busy() && stateOf(t.b)->filters[ch][0].filter_type == FILTER_LOWSHELF &&
                    near(stateOf(t.b)->filters[ch][0].gain, -3));
        // Back before A's write ran: the edit is still staged, and the next
        // field builds on it
        t.r->activate(t.a);
        d.check("mdqt2a: back on A its edit is there", near(cc->band(ch, 0, false).value(QStringLiteral("gain")).toDouble(), 5.5) &&
                                                          near(cc->band(ch, 0, false).value(QStringLiteral("freq")).toDouble(), 432));
        cc->setBandValue(ch, 0, false, QStringLiteral("q"), 2.5);
        t.r->activate(t.b);
    }
    d.check("mdqt2a: both go idle", t.bothIdle());
    const FilterParams fa = stateOf(t.a)->filters[ch][0], fb = stateOf(t.b)->filters[ch][0];
    d.check("mdqt2a: A has its whole edit", fa.filter_type == FILTER_PEAKING && near(fa.gain, 5.5) && near(fa.freq, 432, 0.5) && near(fa.q, 2.5));
    d.check("mdqt2a: B has its own", fb.filter_type == FILTER_LOWSHELF && near(fb.gain, -3) && !near(fb.freq, 432, 0.5) && !near(fb.q, 2.5));
    // The journals: B's took B's two edits (a type, a gain) and none of A's
    const QJsonArray jb = t.journal(t.b);
    bool clean = jb.size() > beforeB;
    for (int i = 0; i < jb.size() - beforeB; ++i) {
        const QJsonValue after = jb.at(i).toObject().value(QLatin1String("after"));
        clean = clean && !near(after.toDouble(), 432, 0.5) && !near(after.toDouble(), 5.5) && !near(after.toDouble(), 2.5);
    }
    d.check("mdqt2a: B's journal holds nothing of A's", clean);
    t.r->activate(t.a);
    d.check("mdqt2a: A's edits survive the way back", near(cc->band(ch, 0, false).value(QStringLiteral("gain")).toDouble(), 5.5));
    t.r->activate(t.b);
    d.check("mdqt2a: and B's", near(cc->band(ch, 0, false).value(QStringLiteral("gain")).toDouble(), -3));
}

void curves(Two &t, GraphController *g, SidebarController *sc, int ch, int other) {
    SmokeDriver &d = t.d;
    const auto ofCore = [&](DeviceSession *s) {
        FilterParams chain[BANDS_PER_CHANNEL + MAX_XOVER_BANDS];
        uint32_t n = 0;
        for (int b = 0; b < BANDS_PER_CHANNEL; ++b) chain[n++] = stateOf(s)->filters[ch][b];
        for (int b = 0; b < MAX_XOVER_BANDS; ++b) chain[n++] = stateOf(s)->xover[ch][b];
        QVector<double> mag(MAGNITUDE_POINTS, 0.0);
        dspi_compute_magnitude_curve(chain, n, mag.data());
        return mag;
    };
    t.r->activate(t.a);
    const QVector<double> a = g->magnitudes(ch);
    QSignalSpy redrawn(g, &GraphController::curvesChanged);
    t.r->activate(t.b);
    const QVector<double> b = g->magnitudes(ch);
    d.check("mdqt2a: the curves follow the active device", redrawn.count() > 0 && a == ofCore(t.a) && b == ofCore(t.b) && a != b);
    t.r->activate(t.a);
    d.check("mdqt2a: and back", g->magnitudes(ch) == a);

    // The page and the curves on show are each device's
    sc->select(ch);
    d.spin(40);
    g->setCurveVisible(other, true);
    d.check("mdqt2a: A's page", sc->selectedChannel() == ch && g->selectedChannel() == ch);
    t.r->activate(t.b);
    sc->select(-1);
    d.spin(1300);   // its first status read brings the default set of curves
    g->setCurveVisible(ch, false);
    d.check("mdqt2a: B's overview", sc->selectedChannel() == -1 && g->selectedChannel() == -1 && !g->isCurveVisible(ch));
    t.r->activate(t.a);
    d.spin(40);
    QQuickItem *page = d.item(QStringLiteral("RightPane.page"));
    d.check("mdqt2a: back on A: its page, and the curves shown on it",
            sc->selectedChannel() == ch && g->selectedChannel() == ch && g->isCurveVisible(ch) && g->isCurveVisible(other) &&
                page && page->property("active").toBool());
    t.r->activate(t.b);
    d.spin(1300);   // past a status read, which brings the live input count
    d.check("mdqt2a: back on B: its overview, its curve still off", sc->selectedChannel() == -1 && !g->isCurveVisible(ch));
    g->setCurveVisible(ch, true);
}

void held(Two &t, ChannelController *cc, SidebarController *sc, PeqEditorController *ed, int ch) {
    SmokeDriver &d = t.d;
    // ---- A slider held with the mouse
    t.r->activate(t.a);
    sc->setVolumeMode(QStringLiteral("user"));
    sc->setUserVolume(-20);
    t.idle(t.a);
    t.r->activate(t.b);
    sc->setUserVolume(-30);
    t.idle(t.b);
    t.r->activate(t.a);
    d.spin(60);
    QQuickItem *slider = d.item(QStringLiteral("GlobalControls.volumeSlider"));
    d.check("mdqt2a: the volume slider", slider && slider->isEnabled());
    if (!slider) return;
    const auto at = [&](double x) { return slider->mapToScene(QPointF(slider->width() * x, slider->height() / 2)).toPoint(); };
    int beforeB = t.b->model()->settersRun();
    QTest::mousePress(t.w, Qt::LeftButton, Qt::NoModifier, at(slider->property("visualPosition").toDouble()));
    QTest::mouseMove(t.w, at(0.6));
    QTest::mouseMove(t.w, at(0.7));
    d.check("mdqt2a: it is held, a drag on A", slider->property("pressed").toBool() && t.a->model()->dragging());
    const double position = slider->property("value").toDouble();
    t.r->activate(t.b);
    d.check("mdqt2a: the switch ends the drag", !t.a->model()->dragging() && !t.b->model()->dragging());
    QTest::mouseMove(t.w, at(0.9));
    d.spin(80);
    QTest::mouseRelease(t.w, Qt::LeftButton, Qt::NoModifier, at(0.9));
    t.bothIdle();
    d.check("mdqt2a: A has the value held", near(stateOf(t.a)->user_volume_db, SidebarController::userPositionToDb(position), 0.2));
    d.check("mdqt2a: nothing reached B", near(stateOf(t.b)->user_volume_db, -30) && t.b->model()->settersRun() == beforeB);
    d.check("mdqt2a: the slider shows B's", near(SidebarController::userPositionToDb(slider->property("value").toDouble()), -30, 0.2));

    // ---- A handle of the graph, dragged
    t.r->activate(t.a);
    sc->select(ch);
    d.waitFor(3000, [&] { return ed->editedChannel() == ch; });
    d.spin(100);
    auto *ov = qobject_cast<PeqOverlay *>(d.object(QStringLiteral("PeqEditor.overlay")));
    d.check("mdqt2a: the graph's editor", ov && ov->editedChannel() == ch);
    if (!ov) return;
    ov->settle();
    ed->rowClicked(0, Qt::NoModifier);
    const FilterParams bandB = stateOf(t.b)->filters[ch][0];
    t.r->activate(t.b);
    sc->select(ch);
    d.waitFor(3000, [&] { return ed->editedChannel() == ch; });
    d.check("mdqt2a: A's band selection is not B's", ed->selection().isEmpty());
    t.r->activate(t.a);
    d.spin(60);
    ov->settle();
    d.check("mdqt2a: and is there back on A", ed->selection() == QSet<int>{ 0 } && ed->editedChannel() == ch);
    d.check("mdqt2a: the editor draws A's bands", near(ov->shownBand(0).value(QStringLiteral("gain")).toDouble(), stateOf(t.a)->filters[ch][0].gain, 0.1));
    beforeB = t.b->model()->settersRun();
    const QPoint dot = ov->mapToScene(ov->dotPoint(0)).toPoint();
    QTest::mouseMove(t.w, dot);
    QTest::mousePress(t.w, Qt::LeftButton, Qt::NoModifier, dot);
    for (int i = 1; i <= 8; ++i) {
        QTest::mouseMove(t.w, dot + QPoint(i * 5, i * 3));
        d.spin(5);
    }
    d.check("mdqt2a: the handle is held", t.a->model()->dragging());
    const QVariantMap live = ov->shownBand(0);
    t.r->activate(t.b);
    d.check("mdqt2a: the switch lets go of the handle", !t.a->model()->dragging() && !t.b->model()->dragging() && ed->live(0).isNull());
    for (int i = 9; i <= 14; ++i) {
        QTest::mouseMove(t.w, dot + QPoint(i * 5, i * 3));
        d.spin(5);
    }
    QTest::mouseRelease(t.w, Qt::LeftButton, Qt::NoModifier, dot + QPoint(70, 42));
    d.spin(400);
    t.bothIdle();
    const FilterParams dragged = stateOf(t.a)->filters[ch][0];
    d.check("mdqt2a: A has the band as it was when left", near(dragged.freq, live.value(QStringLiteral("freq")).toDouble(), 1) &&
                                                              near(dragged.gain, live.value(QStringLiteral("gain")).toDouble(), 0.05) &&
                                                              !near(dragged.freq, 432, 1));
    d.check("mdqt2a: B's band is as it was", stateOf(t.b)->filters[ch][0].freq == bandB.freq && stateOf(t.b)->filters[ch][0].gain == bandB.gain &&
                                                 t.b->model()->settersRun() == beforeB);
    ov->settle();
    d.check("mdqt2a: and what its editor draws", near(ov->shownBand(0).value(QStringLiteral("gain")).toDouble(), bandB.gain, 0.05));

    // ---- A wheel edit, not yet committed
    t.r->activate(t.a);
    d.spin(60);
    ov->settle();
    beforeB = t.b->model()->settersRun();
    const QPointF on = ov->mapToScene(ov->dotPoint(0));
    QTest::mouseMove(t.w, on.toPoint() + QPoint(1, 0));
    QTest::mouseMove(t.w, on.toPoint());
    d.spin(30);
    QWheelEvent wheel(on, t.w->mapToGlobal(on), QPoint(), QPoint(0, 240), Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
    QCoreApplication::sendEvent(t.w, &wheel);
    const double q = ov->shownBand(0).value(QStringLiteral("q")).toDouble();
    d.check("mdqt2a: the wheel turns the band's Q", !near(q, 2.5, 0.01));
    t.r->activate(t.b);
    d.spin(800);
    t.bothIdle();
    d.check("mdqt2a: a wheel edit waiting to be committed is A's", near(stateOf(t.a)->filters[ch][0].q, q, 0.02) &&
                                                                       stateOf(t.b)->filters[ch][0].q == bandB.q &&
                                                                       t.b->model()->settersRun() == beforeB);

    // ---- A field typed into
    t.r->activate(t.a);
    d.spin(60);
    const int out = ch - cc->numInputs();
    cc->setOutputGain(out, -1);
    t.idle(t.a);
    t.r->activate(t.b);
    cc->setOutputGain(out, -2);
    t.idle(t.b);
    t.r->activate(t.a);
    d.spin(60);
    beforeB = t.b->model()->settersRun();
    auto *field = d.item(QStringLiteral("OutputCard.gainValue.field"));
    d.check("mdqt2a: the gain field", field && near(field->property("text").toString().toDouble(), -1));
    if (field) {
        field->forceActiveFocus();
        field->setProperty("text", QStringLiteral("-7"));
        t.r->activate(t.b);
        t.bothIdle();
        d.check("mdqt2a: what was typed is committed on A", !field->hasActiveFocus() && near(stateOf(t.a)->output_gain_db[out], -7));
        d.check("mdqt2a: and not on B", near(stateOf(t.b)->output_gain_db[out], -2) && t.b->model()->settersRun() == beforeB &&
                                            near(field->property("text").toString().toDouble(), -2));
    }
    sc->select(-1);
    t.r->activate(t.a);
    sc->select(-1);
    d.spin(40);
}

void questions(Two &t, ChannelController *cc, MatrixController *mc, SidebarController *sc) {
    SmokeDriver &d = t.d;
    const int pdm = cc->pdmOutput();
    d.check("mdqt2a: the demo has a PDM output", pdm >= 4);
    if (pdm < 4) return;
    for (DeviceSession *s : { t.b, t.a }) {
        t.r->activate(s);
        cc->requestOutputEnable(pdm, false);
        t.idle(s);
        cc->requestOutputEnable(2, true);
        t.idle(s);
    }
    QObject *ask = d.object(QStringLiteral("GlobalControls.clearQuestion"));
    d.check("mdqt2a: enabling PDM on A asks", !cc->requestOutputEnable(pdm, true));
    // A question about A on screen (any of the pages')
    d.check("mdqt2a: a question is shown", ask && QMetaObject::invokeMethod(ask, "ask", Q_ARG(QVariant, 0)) &&
                                               d.waitFor(3000, [&] { return ask->property("visible").toBool(); }));
    t.r->activate(t.b);
    d.check("mdqt2a: the question goes with A, refused", ask && ask->property("pending").toString() == QLatin1String("reject") &&
                                                             d.waitFor(3000, [&] { return !ask->property("visible").toBool(); }));
    const int writesB = t.b->model()->settersRun();
    cc->confirmOutputEnable(pdm);
    t.bothIdle();
    const auto asItWas = [&] {
        return t.b->model()->settersRun() == writesB && !stateOf(t.b)->output_enabled[pdm] && stateOf(t.b)->output_enabled[2] &&
               !stateOf(t.a)->output_enabled[pdm];
    };
    d.check("mdqt2a: A's question answered on B enables nothing", asItWas());
    t.r->activate(t.a);
    QSignalSpy matrixAsked(mc, &MatrixController::enableQuestion);
    mc->toggleEnable(pdm);
    t.r->activate(t.b);
    mc->confirmEnable(pdm);
    t.bothIdle();
    d.check("mdqt2a: nor the matrix's", matrixAsked.count() == 1 && asItWas());
    t.r->activate(t.a);
    sc->select(-1);
    d.spin(60);
}

void presets(Two &t, SidebarController *sc) {
    SmokeDriver &d = t.d;
    t.r->activate(t.a);
    t.idle(t.a);
    d.check("mdqt2a: A has unsaved changes", d.waitFor(3000, [&] { return sc->unsaved(); }));
    const int slotA = stateOf(t.a)->active_preset_slot, slotB = stateOf(t.b)->active_preset_slot;
    const int wanted = slotA == 1 ? 2 : 1;
    QSignalSpy asked(sc, &SidebarController::unsavedQuestion);
    const int beforeB = t.b->model()->settersRun();
    {
        Held held(t.a);
        sc->requestPresetSwitch(wanted);
        d.check("mdqt2a: A's preset operation is under way", sc->presetBusy());
        t.r->activate(t.b);
        d.check("mdqt2a: B's is not", !sc->presetBusy());
    }
    t.bothIdle();
    d.check("mdqt2a: nobody is asked about a device that was left", asked.isEmpty());
    d.check("mdqt2a: nothing was loaded on either", stateOf(t.a)->active_preset_slot == slotA && stateOf(t.b)->active_preset_slot == slotB &&
                                                        t.b->model()->settersRun() == beforeB);
    t.r->activate(t.a);
    d.check("mdqt2a: A's is done", !sc->presetBusy());
    // Asked, then answered on the other device
    sc->requestPresetSwitch(wanted);
    d.check("mdqt2a: A asks about its changes", d.waitFor(5000, [&] { return asked.count() == 1; }));
    t.r->activate(t.b);
    sc->resolveUnsaved(QStringLiteral("switch"), wanted, QStringLiteral("alternative"));
    t.bothIdle();
    d.check("mdqt2a: B takes no answer to A's question", stateOf(t.b)->active_preset_slot == slotB && t.b->model()->settersRun() == beforeB);
    // A clean device's switch goes through on it, wherever the user is by then
    t.r->activate(t.a);
    sc->savePreset();
    t.idle(t.a);
    d.check("mdqt2a: A is saved", d.waitFor(3000, [&] { return !sc->unsaved(); }));
    sc->requestPresetCopy(wanted);   // an empty slot does not load
    d.check("mdqt2a: another slot to switch to", d.waitFor(8000, [&] { return !sc->presetBusy() && sc->presetOccupied(wanted) && !sc->unsaved(); }));
    t.idle(t.a);
    QSignalSpy failed(sc, &SidebarController::presetFailed);
    {
        Held held(t.a);
        sc->requestPresetSwitch(wanted);
        t.r->activate(t.b);
    }
    const bool loaded = d.waitFor(8000, [&] { return stateOf(t.a)->active_preset_slot == wanted; });
    d.check("mdqt2a: a clean A switches its preset in the background",
            loaded && failed.isEmpty() && stateOf(t.b)->active_preset_slot == slotB && t.b->model()->settersRun() == beforeB);
    t.bothIdle();
    t.r->activate(t.a);
    d.check("mdqt2a: A's preset row shows it", t.device->presets()->activeSlot() == wanted && !sc->presetBusy());
    sc->requestPresetSwitch(slotA);
    t.idle(t.a);
    d.waitFor(8000, [&] { return stateOf(t.a)->active_preset_slot == slotA && !sc->presetBusy(); });
    sc->clearPreset(wanted);
    t.bothIdle();
}

void matrix(Two &t, MatrixController *mc) {
    SmokeDriver &d = t.d;
    for (DeviceSession *s : { t.b, t.a }) {
        t.r->activate(s);
        mc->setGain(0, 1, 0);
        mc->setInvert(0, 1, false);
        mc->setConnected(0, 1, false);
        t.idle(s);
    }
    const auto cell = [&] { return mc->model()->cell(0, 1); };
    {
        Held held(t.a);
        mc->setGain(0, 1, -6);
        mc->setInvert(0, 1, true);
        t.r->activate(t.b);
        d.check("mdqt2a: B's crosspoint is its own", near(cell().gain, 0) && !cell().invert && !cell().connected);
        // The whole cell is sent: it must be built on B's
        mc->setConnected(0, 1, true);
        d.check("mdqt2a: B's edit goes through", t.idle(t.b));
        t.r->activate(t.a);
        d.check("mdqt2a: A's is still staged", near(cell().gain, -6) && cell().invert);
        mc->setConnected(0, 1, true);
        t.r->activate(t.b);
    }
    t.bothIdle();
    const DspState *a = stateOf(t.a), *b = stateOf(t.b);
    d.check("mdqt2a: A's crosspoint as edited on A", a->matrix_routing[0][1] && near(a->matrix_gain[0][1], -6) && a->matrix_invert[0][1]);
    d.check("mdqt2a: B's as edited on B", b->matrix_routing[0][1] && near(b->matrix_gain[0][1], 0) && !b->matrix_invert[0][1]);
    t.r->activate(t.a);
    d.check("mdqt2a: nothing stays staged", mc->model()->stagedCount() == 0 && near(cell().gain, -6));
}

void clipboard(Two &t, ChannelController *cc, SidebarController *sc, int ch) {
    SmokeDriver &d = t.d;
    t.r->activate(t.a);
    cc->setBandType(ch, 3, false, FILTER_PEAKING);
    cc->setBandValue(ch, 3, false, QStringLiteral("freq"), 2222);
    cc->setBandValue(ch, 3, false, QStringLiteral("gain"), -4.5);
    t.idle(t.a);
    sc->copyChannel(ch);
    t.idle(t.a);
    t.r->activate(t.b);
    cc->clearBands(ch, false);
    t.idle(t.b);
    const int writesA = t.writes(t.a);
    sc->pasteChannel(ch);
    d.check("mdqt2a: a channel copied on A is pasted on B", t.idle(t.b) && near(stateOf(t.b)->filters[ch][3].gain, -4.5) && near(stateOf(t.b)->filters[ch][3].freq, 2222, 1));
    d.check("mdqt2a: and A is left alone", t.writes(t.a) == writesA);
    // Pasted on B before A has answered the copy: it waits for that copy
    t.r->activate(t.a);
    cc->setBandValue(ch, 3, false, QStringLiteral("gain"), 7.5);
    t.idle(t.a);
    {
        Held held(t.a);
        sc->copyChannel(ch);
        t.r->activate(t.b);
        sc->pasteChannel(ch);
        t.idle(t.b);
        d.check("mdqt2a: a paste waits for the copy of the other device", near(stateOf(t.b)->filters[ch][3].gain, -4.5));
        t.r->activate(t.a);
    }
    d.check("mdqt2a: and lands on the device it was asked of",
            d.waitFor(8000, [&] { return near(stateOf(t.b)->filters[ch][3].gain, 7.5); }) && t.bothIdle() && t.writes(t.a) == writesA + 1);
}

void drafts(Two &t, NamesController *names, OutputLatencyController *latency) {
    SmokeDriver &d = t.d;
    t.r->activate(t.b);
    t.idle(t.b);
    t.r->activate(t.a);
    t.idle(t.a);
    d.check("mdqt2a: the demo has names", d.waitFor(3000, [&] { return names->deviceNameSupported(); }));
    const QString stored = names->deviceName();
    names->setDeviceName(QStringLiteral("Unit A"));
    t.r->activate(t.b);
    d.check("mdqt2a: a name typed for A is no draft of B's", names->deviceName() != QLatin1String("Unit A") && !names->hasPending());
    names->setDeviceName(QStringLiteral("Unit B"));
    t.r->activate(t.a);
    t.idle(t.a);
    d.check("mdqt2a: and is there back on A", names->deviceName() == QLatin1String("Unit A") && names->hasPending());
    QStringList failed{ QStringLiteral("?") };
    {
        Held held(t.a);
        names->save([&failed](const QStringList &f) { failed = f; });
        t.r->activate(t.b);
        d.check("mdqt2a: B's draft while A's is saved", names->deviceName() == QLatin1String("Unit B"));
    }
    d.check("mdqt2a: A's name is saved on A", d.waitFor(8000, [&] { return failed.isEmpty(); }) && t.bothIdle() &&
                                                 t.a->model()->deviceName() == QLatin1String("Unit A"));
    d.check("mdqt2a: B keeps its name and its draft", t.b->model()->deviceName() != QLatin1String("Unit A") &&
                                                        t.b->model()->deviceName() != QLatin1String("Unit B") && names->hasPending() &&
                                                        names->deviceName() == QLatin1String("Unit B"));
    names->revert();
    t.r->activate(t.a);
    t.idle(t.a);
    d.check("mdqt2a: nothing pending on A", !names->hasPending() && names->deviceName() == QLatin1String("Unit A"));
    names->setDeviceName(stored);
    failed = QStringList{ QStringLiteral("?") };
    names->save([&failed](const QStringList &f) { failed = f; });
    d.waitFor(5000, [&] { return failed.isEmpty(); });

    d.check("mdqt2a: the demo has an output latency", d.waitFor(3000, [&] { return latency->supported(); }));
    const int idA = latency->selectedId();
    int other = -1;
    for (const QVariant &v : latency->options())
        if (v.toMap().value(QStringLiteral("id")).toInt() != idA && v.toMap().value(QStringLiteral("available")).toBool())
            other = v.toMap().value(QStringLiteral("id")).toInt();
    d.check("mdqt2a: with another choice", other >= 0);
    latency->choose(other);
    t.r->activate(t.b);
    t.idle(t.b);
    d.check("mdqt2a: A's latency choice is not B's", !latency->edited() && !latency->hasPending());
    t.r->activate(t.a);
    t.idle(t.a);
    d.check("mdqt2a: and is there back on A", latency->edited() && latency->selectedId() == other);
    latency->revert();
}

void readings(Two &t, DashboardController *dash, SidebarController *sc, EffectsController *fx, ChannelController *cc) {
    SmokeDriver &d = t.d;
    t.r->activate(t.b);
    cc->requestOutputEnable(1, true);
    t.device->setChannelName(cc->outputChannel(0), QStringLiteral("B out"));
    fx->setEnabled(QStringLiteral("crossfeed"), true);
    fx->setCrossfeedFreq(1234);
    t.idle(t.b);
    t.r->activate(t.a);
    t.device->setChannelName(cc->outputChannel(0), QStringLiteral("A out"));
    fx->setEnabled(QStringLiteral("crossfeed"), false);
    fx->setCrossfeedFreq(777);
    t.bothIdle();
    const auto shows = [&](const char *name) {
        for (const QVariant &card : dash->cards())
            for (const QVariant &c : card.toMap().value(QStringLiteral("columns")).toList())
                if (c.toMap().value(QStringLiteral("heading")).toString() == QLatin1String(name)) return true;
        return false;
    };
    d.check("mdqt2a: A's cards", d.waitFor(3000, [&] { return shows("A out"); }));
    d.check("mdqt2a: A's effects", !fx->crossfeedOn() && near(fx->crossfeedFreq(), 777, 1));
    {
        // A slow read of A holds up nothing of B's, and is not shown as B's
        Held held(t.a);
        t.r->activate(t.b);
        t.r->activate(t.a);   // its cards and preset state are asked for, behind the hold
        sc->renameSource(0, QStringLiteral("Src A"));
        t.r->activate(t.b);
        d.check("mdqt2a: B's cards come while A's worker is busy", d.waitFor(3000, [&] { return shows("B out") && !shows("A out"); }));
        d.check("mdqt2a: B's effects", fx->crossfeedOn() && near(fx->crossfeedFreq(), 1234, 1));
        d.check("mdqt2a: B's preset state is read meanwhile", d.waitFor(3000, [&] { return sc->unsaved(); }));
    }
    t.bothIdle();
    d.check("mdqt2a: A's answers do not show as B's", shows("B out") && !shows("A out") && sc->sourceName(0) != QLatin1String("Src A"));
    t.r->activate(t.a);
    t.idle(t.a);
    d.check("mdqt2a: A's are there back on A", d.waitFor(3000, [&] { return shows("A out") && sc->sourceName(0) == QLatin1String("Src A"); }));
    sc->renameSource(0, QString());
    t.idle(t.a);
}

void closing(Two &t, ChannelController *cc, int ch) {
    SmokeDriver &d = t.d;
    // The last edit of a device that is closed, its write still on the way
    t.r->activate(t.a);
    cc->setBandValue(ch, 0, false, QStringLiteral("gain"), 1.25);
    t.idle(t.a);
    t.r->activate(t.b);
    auto ran = std::make_shared<std::atomic<float>>(0.0f);
    QPointer<DeviceSession> gone(t.b);
    {
        Held held(t.b);
        cc->setBandValue(ch, 0, false, QStringLiteral("gain"), 9.5);
        t.b->worker()->post([ran, ch](FfiCore *c) {
            if (const DspState *s = dspi_get_state(c)) ran->store(s->filters[ch][0].gain);
        });
        t.r->close(t.b->id());
    }
    d.check("mdqt2a: closed: A is active", t.r->active() == t.a && near(cc->band(ch, 0, false).value(QStringLiteral("gain")).toDouble(), 1.25));
    d.check("mdqt2a: B's session goes", d.waitFor(10000, [&] { return gone.isNull(); }));
    d.check("mdqt2a: its last edit ran there first", near(ran->load(), 9.5));
    t.idle(t.a);
    d.check("mdqt2a: and not on A", near(stateOf(t.a)->filters[ch][0].gain, 1.25));
}

void rebindSection(SmokeDriver &d) {
    auto *cc = Registrar::instance<ChannelController>();
    auto *g = Registrar::instance<GraphController>();
    auto *ed = Registrar::instance<PeqEditorController>();
    auto *sc = Registrar::instance<SidebarController>();
    auto *mc = Registrar::instance<MatrixController>();
    auto *names = Registrar::instance<NamesController>();
    auto *latency = Registrar::instance<OutputLatencyController>();
    auto *dash = Registrar::instance<DashboardController>();
    auto *fx = Registrar::instance<EffectsController>();
    DeviceRegistry *r = DeviceRegistry::instance();
    d.check("mdqt2a: the controllers are made", cc && g && ed && sc && mc && names && latency && dash && fx && r);
    if (!cc || !g || !ed || !sc || !mc || !names || !latency || !dash || !fx || !r) return;
    DeviceModel *device = d.app().device;
    device->openDemo();
    d.waitIdle();
    Two t{ d, r, device, d.mainWindow() };
    t.a = r->mainSession();
    r->setKeepOpen(true);
    r->openDemo();
    d.check("mdqt2a: two demos", d.waitFor(10000, [&] { return r->opening() == 0 && r->sessions().size() == 2; }));
    if (r->sessions().size() != 2) return;
    t.b = r->sessions().at(1);
    t.bothIdle();
    const int ch = cc->outputChannel(0), other = cc->outputChannel(1);
    bands(t, cc, ch);
    curves(t, g, sc, ch, other);
    held(t, cc, sc, ed, ch);
    matrix(t, mc);
    clipboard(t, cc, sc, ch);
    drafts(t, names, latency);
    readings(t, dash, sc, fx, cc);
    presets(t, sc);
    questions(t, cc, mc, sc);
    closing(t, cc, ch);
    r->setKeepOpen(false);
    r->activate(t.a);
    sc->select(-1);
    device->openDemo();
    d.waitIdle();
    d.waitFor(3000, [&] { return r->devices().size() == 1; });
}

const SmokeDriver::Section rebind("mdqt2a-rebind", 965, &rebindSection);
} // namespace
