// -- WP MDQT2c -- The job controllers over two open devices (two demos, A and
// B): a measurement goes on where it was started and its result lands there; the
// signal generator, the live view and solo stop on the device that is left, or
// closed; the spectrum follows the active device; each device keeps its tune's
// guard, its speaker draft, its log and its stats.
#include "SmokeDriver.h"

#include "AudioCapture.h"
#include "Core.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "MeasureController.h"
#include "MeasureSession.h"
#include "MonitorController.h"
#include "NoticeCenter.h"
#include "Poller.h"
#include "PrefsMap.h"
#include "Registrar.h"
#include "RtaController.h"
#include "SessionModel.h"
#include "Shell.h"
#include "SiggenController.h"
#include "Snapshot.h"
#include "SpeakersController.h"
#include "StatsController.h"
#include "ToolsController.h"
#include "TuneController.h"
#include "TuneSession.h"

#include <QBuffer>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPointer>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSemaphore>
#include <QSignalSpy>
#include <atomic>
#include <cmath>
#include <cstdio>
#include <memory>

namespace {

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

// An input that opens and never delivers a block
class SilentSource final : public dspi_capture::Source {
public:
    bool open(dspi_capture::Failure *, QString *) override {
        m_format.setSampleRate(48000);
        m_format.setChannelCount(1);
        m_format.setSampleFormat(QAudioFormat::Float);
        m_device = std::make_unique<QBuffer>();
        m_device->open(QIODevice::ReadOnly);
        return true;
    }
    QIODevice *device() override { return m_device.get(); }
    QAudioFormat format() const override { return m_format; }
    qint64 producedFrames() const override { return 0; }
    QString name() const override { return QStringLiteral("Smoke silence"); }
    void close() override { m_device.reset(); }

private:
    QAudioFormat m_format;
    std::unique_ptr<QBuffer> m_device;
};

struct Two {
    SmokeDriver &d;
    DeviceRegistry *r;
    MeasureController *mc;
    TuneController *tc;
    DeviceSession *a = nullptr;
    DeviceSession *b = nullptr;

    bool idle(DeviceSession *s) {
        return d.waitFor(8000, [&] { return !s->model()->busy() && s->worker()->pending() == 0; });
    }
    template <class T>
    T ask(DeviceSession *s, std::function<T(FfiCore *)> job) {
        auto got = std::make_shared<T>();
        auto done = std::make_shared<bool>(false);
        s->worker()->call(d.app().device, std::move(job), [got, done](const T &v) {
            *got = v;
            *done = true;
        });
        d.waitFor(8000, [&] { return *done; });
        return *got;
    }
    // The generator's state on a session's core
    int generator(DeviceSession *s) {
        return ask<int>(s, [](FfiCore *c) {
            SiggenStatus st{};
            return dspi_siggen_get_status(c, &st) ? int(st.state) : -1;
        });
    }
    QJsonObject json(DeviceSession *s, uint32_t (*call)(FfiCore *, char *, uint32_t)) {
        const QByteArray text =
            ask<QByteArray>(s, [call](FfiCore *c) { return readSized([c, call](char *b, uint32_t n) { return call(c, b, n); }, 1 << 18); });
        return QJsonDocument::fromJson(text).object();
    }
    MeasureSession *measure(DeviceSession *s) {
        for (MeasureSession *m : mc->sessions())
            if (m->session() == s) return m;
        return nullptr;
    }
    QVariantMap row(DeviceSession *s) {
        for (const QVariant &v : r->devices())
            if (v.toMap().value(QStringLiteral("id")).toString() == s->id()) return v.toMap();
        return {};
    }
    // Two demos, A (the main session's) active, both at a volume to measure at
    bool open() {
        a = r->mainSession();
        r->setKeepOpen(true);
        if (!a->model()->connected() || !a->model()->demo()) a->model()->openDemo();
        if (!d.waitFor(10000, [&] { return a->model()->connected() && a->model()->demo(); })) return false;
        // -- WP LAY7 -- (review RC, RC15) a demo still on its way (a hub's session
        // left opens the one before it) drops what is written meanwhile
        if (!idle(a)) return false;
        r->openDemo();
        if (!d.waitFor(10000, [&] { return r->opening() == 0 && r->sessions().size() == 2; })) return false;
        b = r->sessions().at(1);
        for (DeviceSession *s : { b, a }) {
            r->activate(s);
            s->model()->write(QStringLiteral("mdqt2c volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
        }
        return idle(a) && idle(b) && d.waitFor(5000, [&] { return !a->id().isEmpty() && !b->id().isEmpty(); });
    }
    void close() {
        r->activate(a);
        for (DeviceSession *s : r->sessions())
            if (!s->isMain()) r->close(s->id());
        d.waitFor(10000, [&] { return r->sessions().size() == 1; });
        a->model()->write(QStringLiteral("mdqt2c volume"), [](FfiCore *c) { return dspi_set_user_volume(c, 0.0f); });
        r->setKeepOpen(false);
        d.waitIdle();
    }
    // -- WP LAY7 -- (review RC, RC15) Why nothing starts on the active device
    void why(const char *what) {
        std::printf("smoke: mdqt2c: %s: error '%s', blocker '%s', setup '%s', issue '%s', sim %d, microphone '%s', %d outputs\n", what,
                    qPrintable(mc->error()), qPrintable(mc->playBlocker()), qPrintable(mc->setupProblem()), qPrintable(mc->issueText()),
                    int(mc->simAvailable()), qPrintable(mc->micChoice()), int(mc->setupOutputs().size()));
        for (const QVariant &v : mc->preflightLines()) std::printf("smoke: mdqt2c:   %s\n", qPrintable(QJsonDocument::fromVariant(v).toJson(QJsonDocument::Compact)));
    }
    // The active device's setup: one output, short sweeps
    void setup() {
        const bool ready = d.waitFor(5000, [&] { return mc->simAvailable() && mc->micChoice() == QLatin1String("sim") && !mc->setupOutputs().isEmpty(); });
        if (!ready) why("not ready");
        bool one = false;
        for (const QVariant &v : mc->setupOutputs()) {
            const QVariantMap m = v.toMap();
            const bool on = m.value(QStringLiteral("on")).toBool() && !one;
            one = one || on;
            mc->setOutputOn(m.value(QStringLiteral("output")).toInt(), on);
        }
        mc->setPlacement(QStringLiteral("listeningPosition"));
        mc->setSweepMs(1000);
        mc->setPasses(3);
    }
};

// ---- A measurement -----------------------------------------------------------------------

void measuring(Two &t, QByteArray *result) {
    SmokeDriver &d = t.d;
    MeasureController *mc = t.mc;
    t.r->activate(t.a);
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("tools.measure")));
    d.waitFor(3000, [&] { return w && w->isVisible(); });
    t.setup();
    mc->setRunName(QStringLiteral("On A"));
    t.r->activate(t.b);
    d.check("mdqt2c: B has no placement: what was chosen for A is not assumed", mc->placement().isEmpty() && mc->runName().isEmpty());
    t.r->activate(t.a);
    d.check("mdqt2c: A keeps its setup", mc->placement() == QLatin1String("listeningPosition") && mc->runName() == QLatin1String("On A"));
    mc->start();
    d.check("mdqt2c: a measurement starts on A", d.waitFor(8000, [&] { return mc->measuring(); }));
    if (!mc->measuring()) t.why("no measurement on A");   // -- WP LAY7 --
    QQuickItem *bar = w ? d.item(QStringLiteral("Measure.runningText"), w->contentItem()) : nullptr;
    d.check("mdqt2c: its page says on which device it runs",
            !mc->runningOn().isEmpty() && mc->runningOn() == t.a->title() && bar && bar->property("text").toString().contains(t.a->title()));

    // B is active: A's run goes on, and is said to
    t.r->activate(t.b);
    d.spin(60);
    d.check("mdqt2c: B's page shows no run", !mc->measuring() && !mc->running() && mc->phase() == QLatin1String("idle") && !mc->hasResult());
    const QVariantList others = mc->otherRuns();
    d.check("mdqt2c: but what runs on A", others.size() == 1 && others.first().toMap().value(QStringLiteral("id")).toString() == t.a->id() &&
                                              !others.first().toMap().value(QStringLiteral("text")).toString().isEmpty());
    QQuickItem *other = w ? d.item(QStringLiteral("Measure.otherRun"), w->contentItem()) : nullptr;
    d.check("mdqt2c: with a Stop of its own in B's window", other && other->isVisible() && d.item(QStringLiteral("Measure.otherRun.stop"), w->contentItem()));
    d.check("mdqt2c: A's tab is busy, B's is not", t.row(t.a).value(QStringLiteral("busy")).toBool() &&
                                                      !t.row(t.a).value(QStringLiteral("job")).toString().isEmpty() &&
                                                      !t.row(t.b).value(QStringLiteral("busy")).toBool());
    MeasureSession *ma = t.measure(t.a);
    d.check("mdqt2c: A's run is active on A's core alone", ma && ma->measuring() && t.json(t.a, dspi_measure_state).value(QLatin1String("active")).toBool() &&
                                                              !t.json(t.b, dspi_measure_state).value(QLatin1String("active")).toBool());
    if (!ma) return;
    // A run on B beside it, stopped again: A's is not touched
    t.setup();
    mc->start();
    d.check("mdqt2c: B measures beside it", d.waitFor(8000, [&] { return mc->measuring(); }) && ma->measuring());
    mc->stop();
    d.check("mdqt2c: Stop on B stops B's", d.waitFor(8000, [&] { return !mc->running() && t.generator(t.b) == 0; }));
    d.check("mdqt2c: and A's goes on", ma->measuring() && t.json(t.a, dspi_measure_state).value(QLatin1String("active")).toBool());
    mc->clearError();

    auto *notices = Registrar::instance<NoticeCenter>();
    d.check("mdqt2c: A's run ends with its result, on A", d.waitFor(60000, [&] { return !ma->running() && ma->hasResult(); }));
    d.check("mdqt2c: B has none", !mc->hasResult() && t.json(t.b, dspi_measure_result).value(QLatin1String("ok")).toBool(true) == false);
    d.check("mdqt2c: a notice says it, by the device's name",
            notices && notices->last().value(QStringLiteral("text")).toString().startsWith(t.a->title() + QLatin1String(": ")));
    d.check("mdqt2c: A's tab is not busy any more", d.waitFor(3000, [&] { return !t.row(t.a).value(QStringLiteral("busy")).toBool(); }) && mc->otherRuns().isEmpty());
    QSignalSpy shown(mc, &MeasureController::resultChanged);
    t.r->activate(t.a);
    d.check("mdqt2c: back on A: the result", shown.count() > 0 && mc->hasResult() && mc->resultName() == QLatin1String("On A"));
    *result = mc->resultDocument();
    d.check("mdqt2c: its graph", d.waitFor(5000, [&] { return !mc->resultGraph().isEmpty(); }));

    // Stopped from the other device's window
    mc->start();
    d.waitFor(8000, [&] { return mc->measuring(); });
    t.r->activate(t.b);
    d.spin(60);
    const QString id = mc->otherRuns().value(0).toMap().value(QStringLiteral("id")).toString();
    mc->stopOther(id);
    d.check("mdqt2c: A's run is stopped from B's window", id == t.a->id() && d.waitFor(8000, [&] { return !ma->running() && t.generator(t.a) == 0; }));
    // The window closed: what runs on every device stops
    t.r->activate(t.a);
    mc->clearError();
    mc->start();
    d.waitFor(8000, [&] { return mc->measuring(); });
    t.r->activate(t.b);
    d.spin(60);
    if (w) w->close();
    d.check("mdqt2c: closing the window on B stops A's run", d.waitFor(8000, [&] { return !ma->running() && t.generator(t.a) == 0; }));
    mc->clearError();
}

void microphone(Two &t) {
    SmokeDriver &d = t.d;
    MeasureController *mc = t.mc;
    const auto silent = []() -> std::unique_ptr<dspi_capture::Source> { return std::make_unique<SilentSource>(); };
    t.r->activate(t.a);
    mc->setMicChoice(QString());   // not the simulated one
    mc->setCaptureSource(silent);
    mc->start();
    MeasureSession *ma = t.measure(t.a);
    d.check("mdqt2c: A's run opens the microphone", d.waitFor(8000, [&] { return ma && ma->capturing(); }));
    t.r->activate(t.b);
    d.check("mdqt2c: B keeps its simulated microphone", mc->micChoice() == QLatin1String("sim") && !mc->running());
    mc->setMicChoice(QString());
    mc->setCaptureSource(silent);
    mc->start();
    d.check("mdqt2c: the one microphone is refused to B, A named",
            d.waitFor(3000, [&] { return mc->error().contains(t.a->title()); }) && !t.measure(t.b)->capturing() && ma && ma->capturing());
    mc->stopOther(t.a->id());
    d.check("mdqt2c: A's lets go of it", d.waitFor(8000, [&] { return !ma->capturing() && !ma->running(); }));
    mc->clearError();
    for (DeviceSession *s : { t.b, t.a }) {
        t.r->activate(s);
        mc->setCaptureSource(nullptr);
        mc->clearError();
        mc->setMicChoice(QStringLiteral("sim"));
    }
}

void live(Two &t) {
    SmokeDriver &d = t.d;
    MeasureController *mc = t.mc;
    t.r->activate(t.a);
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("tools.measure")));
    d.waitFor(3000, [&] { return w && w->isVisible(); });
    QObject *tab = nullptr;
    if (QQuickItem *start = w ? d.item(QStringLiteral("MeasureLive.start"), w->contentItem()) : nullptr)
        for (QQuickItem *i = start; i && !tab; i = i->parentItem())
            if (i->property("placementChoice").isValid()) tab = i;
    if (tab) tab->setProperty("placementChoice", QStringLiteral("listeningPosition"));
    mc->liveStart({ { QStringLiteral("output"), 0 }, { QStringLiteral("role"), QStringLiteral("full") },
                    { QStringLiteral("mode"), QStringLiteral("sweep") }, { QStringLiteral("placement"), QStringLiteral("listeningPosition") },
                    { QStringLiteral("smoothing"), 6 }, { QStringLiteral("sweepMs"), 1000 } });
    MeasureSession *ma = t.measure(t.a);
    d.check("mdqt2c: a live view on A plays", d.waitFor(20000, [&] { return mc->liveActive() && t.generator(t.a) > 0; }));
    t.r->activate(t.b);
    d.check("mdqt2c: it has no end of its own: it stops when A is left",
            d.waitFor(8000, [&] { return !ma->running() && !ma->liveActive() && t.generator(t.a) == 0; }) && !mc->liveActive());
    d.check("mdqt2c: the Live tab's placement is B's to choose", tab && tab->property("placementChoice").toString().isEmpty());
    // A start still on its way is withdrawn
    t.r->activate(t.a);
    mc->clearError();
    mc->liveStart({ { QStringLiteral("output"), 0 }, { QStringLiteral("role"), QStringLiteral("full") },
                    { QStringLiteral("mode"), QStringLiteral("sweep") }, { QStringLiteral("placement"), QStringLiteral("listeningPosition") },
                    { QStringLiteral("smoothing"), 6 }, { QStringLiteral("sweepMs"), 1000 } });
    t.r->activate(t.b);
    d.spin(1500);
    d.check("mdqt2c: a live start on its way is withdrawn", !ma->running() && !ma->liveActive() && t.generator(t.a) == 0);
    if (w) w->close();
}

// ---- The tune's guard --------------------------------------------------------------------

bool propose(Two &t, const QByteArray &result) {
    SmokeDriver &d = t.d;
    t.mc->setResultDocument(result, QStringLiteral("run"));
    t.tc->reloadLayout();
    d.waitFor(5000, [&] { return !t.tc->layout().isEmpty(); });
    d.spin(100);
    t.tc->setKeepCrossover(true);
    t.tc->setKeepDelays(true);
    QSignalSpy steps(t.tc, &TuneController::stepDone);
    t.tc->propose();
    return d.waitFor(30000, [&] { return steps.count() > 0; }) && t.tc->hasProposal() && !t.tc->needsSumCheck() && !t.tc->writes().isEmpty();
}

// -- WP INTE -- a state's filters field by field, padding left out (SHOWFIX's p5qui finding: a
// FilterParams has padding after its type and bypass that a copy need not keep)
QByteArray filterBytes(const DspState &s) {
    QByteArray out;
    const FilterParams *f = &s.filters[0][0];
    for (size_t i = 0; i < sizeof s.filters / sizeof(FilterParams); ++i) {
        out.append(char(f[i].filter_type)).append(char(f[i].bypass ? 1 : 0));
        for (const float v : { f[i].freq, f[i].q, f[i].gain, f[i].qp })
            out.append(reinterpret_cast<const char *>(&v), sizeof v);
    }
    return out;
}

void tune(Two &t, const QByteArray &result) {
    SmokeDriver &d = t.d;
    TuneController *tc = t.tc;
    t.r->activate(t.a);
    const bool proposed = propose(t, result);
    if (!proposed) std::printf("smoke: mdqt2c: tune: %s; %d write(s), sum check %s\n", qPrintable(tc->error()), int(tc->writes().size()), qPrintable(tc->sumCheck()));
    d.check("mdqt2c: a tune is proposed on A", proposed);
    if (!proposed) return;
    t.r->activate(t.b);
    d.check("mdqt2c: not on B", !tc->hasProposal() && tc->stage() == QLatin1String("idle") && !tc->keepCrossover());
    t.r->activate(t.a);
    const QByteArray before = filterBytes(*t.a->model()->state());
    tc->apply(60);
    d.check("mdqt2c: applied in its guard on A", d.waitFor(15000, [&] { return tc->applied(); }));
    t.r->activate(t.b);
    d.spin(60);
    TuneSession *ta = t.measure(t.a)->tune();
    d.check("mdqt2c: B shows no tune, A's tab is busy with it", !tc->applied() && ta->applied() && t.row(t.a).value(QStringLiteral("busy")).toBool());
    tc->keep();
    tc->confirmKeep();
    d.spin(300);
    const int left = ta->guardRemainingS();
    d.check("mdqt2c: Keep on B keeps nothing of A's", ta->applied());
    d.check("mdqt2c: the guard counts on meanwhile", d.waitFor(4000, [&] { return ta->guardRemainingS() < left; }));
    t.r->activate(t.a);
    d.check("mdqt2c: back on A: still applied, the guard counting", tc->applied() && tc->guardRemainingS() > 0);
    tc->revert();
    d.check("mdqt2c: Revert on A", d.waitFor(15000, [&] { return tc->stage() == QLatin1String("reverted"); }) && t.idle(t.a));
    const QByteArray after = filterBytes(*t.a->model()->state());
    d.check("mdqt2c: A is as before", before == after && d.waitFor(3000, [&] { return !t.row(t.a).value(QStringLiteral("busy")).toBool(); }));

    // B is closed inside its guard: it goes back before it is closed
    t.r->activate(t.b);
    const QByteArray bBefore = filterBytes(*t.b->model()->state());
    const bool again = propose(t, result);
    if (again) tc->apply(60);
    d.check("mdqt2c: a tune applied on B", again && d.waitFor(15000, [&] { return tc->applied(); }) && t.idle(t.b));
    const QByteArray bTuned = filterBytes(*t.b->model()->state());
    auto found = std::make_shared<QByteArray>();
    QObject probe;
    QObject::connect(t.r, &DeviceRegistry::sessionStopping, &probe, [found, &t](DeviceSession *s) {
        if (s != t.b) return;
        s->worker()->post([found](FfiCore *c) {
            dspi_fetch_all(c);
            if (const DspState *st = readSnapshot(c).state.get()) *found = filterBytes(*st);
            const QByteArray state = readSized([c](char *b, uint32_t n) { return dspi_measure_tune_state(c, b, n); }, 1 << 18);
            std::printf("smoke: mdqt2c: closing: the tune is %s\n", qPrintable(QJsonDocument::fromJson(state).object().value(QLatin1String("stage")).toString()));
        });
    });
    QPointer<DeviceSession> gone(t.b);
    t.r->close(t.b->id());
    d.check("mdqt2c: closing B ends its session", d.waitFor(12000, [&] { return gone.isNull(); }) && t.r->active() == t.a && !t.measure(gone));
    std::printf("smoke: mdqt2c: closing: tuned %d, read %d byte(s), as before %d, as tuned %d\n", bTuned != bBefore, int(found->size()), *found == bBefore, *found == bTuned);
    d.check("mdqt2c: its unkept tune went back first", bTuned != bBefore && *found == bBefore);
    t.b = nullptr;
}

// ---- The signal generator ----------------------------------------------------------------

void generator(Two &t, SiggenController *sg) {
    SmokeDriver &d = t.d;
    t.r->activate(t.b);
    d.check("mdqt2c: B has a generator", d.waitFor(5000, [&] { return sg->available(); }));
    sg->selectType(SiggenController::Pink);
    t.r->activate(t.a);
    d.check("mdqt2c: A has its own", d.waitFor(5000, [&] { return sg->available(); }));
    sg->selectType(SiggenController::Sine);
    sg->setParameter(0, 432);
    sg->setLevel(-40);
    sg->play();
    d.check("mdqt2c: the generator plays on A", d.waitFor(5000, [&] { return sg->playing() && t.generator(t.a) > 0; }));
    t.r->activate(t.b);
    d.check("mdqt2c: B's page: not playing, its own draft", !sg->playing() && sg->signalType() == SiggenController::Pink);
    d.check("mdqt2c: leaving A stopped its generator", d.waitFor(5000, [&] { return t.generator(t.a) == 0; }));
    d.check("mdqt2c: and started none on B", t.generator(t.b) == 0);
    t.r->activate(t.a);
    d.check("mdqt2c: back on A: stopped, its draft kept",
            d.waitFor(3000, [&] { return !sg->playing(); }) && sg->signalType() == SiggenController::Sine &&
                std::abs(sg->params().value(0).toDouble() - 432) < 0.5 && std::abs(sg->levelDb() + 40) < 0.1);
    {
        // A Start that still waits in A's queue: never sent
        Held held(t.a);
        sg->play();
        t.r->activate(t.b);
    }
    t.idle(t.a);
    d.check("mdqt2c: a Start on its way to A does not play on", d.waitFor(5000, [&] { return t.generator(t.a) == 0; }));
    d.check("mdqt2c: an edit on B sends nothing anywhere", (sg->setLevel(-50), d.spin(600), t.generator(t.a) == 0 && t.generator(t.b) == 0));
    // What was not started here is not stopped here (another console's)
    t.ask<bool>(t.a, [](FfiCore *c) {
        SiggenConfig cfg{};
        cfg.channel_mask = 1;
        cfg.level_db = -40;
        cfg.p[0] = 1000;
        return dspi_siggen_set_config(c, &cfg) && dspi_siggen_control(c, SIGGEN_CTL_START);
    });
    t.r->activate(t.a);
    d.spin(400);
    t.r->activate(t.b);
    d.spin(400);
    d.check("mdqt2c: a generator another console started plays on", t.generator(t.a) > 0);
    t.ask<bool>(t.a, [](FfiCore *c) { return dspi_siggen_control(c, SIGGEN_CTL_STOP_NOW); });

    // B is closed while it plays
    sg->play();
    d.check("mdqt2c: the generator plays on B", d.waitFor(5000, [&] { return sg->playing() && t.generator(t.b) > 0; }));
    auto state = std::make_shared<std::atomic<int>>(-1);
    QObject probe;
    QObject::connect(t.r, &DeviceRegistry::sessionStopping, &probe, [state, &t](DeviceSession *s) {
        if (s != t.b) return;
        s->worker()->post([state](FfiCore *c) {
            SiggenStatus st{};
            if (dspi_siggen_get_status(c, &st)) state->store(int(st.state));
        });
    });
    QPointer<DeviceSession> gone(t.b);
    t.r->close(t.b->id());
    d.check("mdqt2c: closing it ends its session", d.waitFor(12000, [&] { return gone.isNull(); }));
    d.check("mdqt2c: the Stop went out first", state->load() == SiggenController::Idle || state->load() == SiggenController::FadeOut);
    t.b = nullptr;
}

// Classic: another device chosen in place of the active one (here one that is
// not there, so that nothing is opened)
void replaced(Two &t, SiggenController *sg) {
    SmokeDriver &d = t.d;
    d.waitFor(5000, [&] { return sg->available(); });
    sg->setLevel(-40);
    sg->play();
    d.check("mdqt2c: Classic: the generator plays", d.waitFor(5000, [&] { return sg->playing() && t.generator(t.a) > 0; }));
    auto state = std::make_shared<std::atomic<int>>(-1);
    QObject probe;
    QObject::connect(t.r, &DeviceRegistry::sessionStopping, &probe, [state, &t](DeviceSession *s) {
        if (s != t.a) return;
        s->worker()->post([state](FfiCore *c) {
            SiggenStatus st{};
            if (dspi_siggen_get_status(c, &st)) state->store(int(st.state));
        });
    });
    t.a->model()->selectDevice(QStringLiteral("MDQT2C-NONE"));
    d.waitIdle();
    d.check("mdqt2c: choosing another device stops it first", state->load() == SiggenController::Idle || state->load() == SiggenController::FadeOut);
    t.a->model()->openDemo();
    d.waitIdle();
    d.waitFor(5000, [&] { return t.a->model()->connected(); });
}

// ---- The spectrum ------------------------------------------------------------------------

void spectrum(Two &t, RtaController *rta) {
    SmokeDriver &d = t.d;
    const auto running = [&](DeviceSession *s) {
        return t.ask<bool>(s, [](FfiCore *c) {
            auto snap = std::make_unique<RtaSnapshot>();
            dspi_rta_get_snapshot(c, snap.get());
            return bool(snap->running);
        });
    };
    // The dashboard's own choice, whatever an earlier section stored
    const QVariant source = rta->prefs()->value(QStringLiteral("dashboardSource"));
    rta->prefs()->insert(QStringLiteral("dashboardSource"), QString());
    t.r->activate(t.a);
    rta->setView(QStringLiteral("mdqt2c"), RtaController::Curves, true);
    d.check("mdqt2c: the spectrum of A", d.waitFor(8000, [&] { return rta->analysing() && rta->snapshot().count > 0; }) && running(t.a));
    const uint32_t atA = rta->snapshot().version;
    t.r->activate(t.b);
    QSignalSpy pictures(rta, &RtaController::pictureChanged);
    d.check("mdqt2c: the spectrum follows to B", d.waitFor(8000, [&] { return rta->analysing() && rta->snapshot().count > 0 && pictures.count() > 2; }) && running(t.b));
    d.check("mdqt2c: A is asked for nothing", d.waitFor(8000, [&] { return !running(t.a); }));
    t.r->activate(t.a);
    d.check("mdqt2c: and back to A", d.waitFor(8000, [&] { return rta->analysing() && rta->snapshot().version != atA && running(t.a); }));
    d.check("mdqt2c: B rests", d.waitFor(8000, [&] { return !running(t.b); }));
    rta->setView(QStringLiteral("mdqt2c"), RtaController::Curves, false);
    d.waitFor(8000, [&] { return !running(t.a); });
    rta->prefs()->insert(QStringLiteral("dashboardSource"), source);
}

// ---- Speakers ----------------------------------------------------------------------------

void speakers(Two &t, SpeakersController *sc) {
    SmokeDriver &d = t.d;
    const auto roleOf = [&] { return sc->rows().value(0).toMap().value(QStringLiteral("role")).toString(); };
    for (DeviceSession *s : { t.b, t.a }) {
        t.r->activate(s);
        d.waitFor(5000, [&] { return sc->loaded() && !sc->rows().isEmpty(); });
        sc->useSuggestion();
    }
    const QString was = roleOf();
    const QString edited = was == QLatin1String("sub") ? QStringLiteral("mid") : QStringLiteral("sub");
    sc->setRole(0, edited);
    t.r->activate(t.b);
    d.waitFor(5000, [&] { return sc->loaded(); });
    d.check("mdqt2c: B has its own speaker draft", roleOf() == was);
    t.idle(t.b);
    t.r->activate(t.a);
    t.idle(t.a);
    d.check("mdqt2c: A's draft is there on the way back", roleOf() == edited && sc->edited());
    // A Confirm answered after the switch is A's
    QSignalSpy stored(sc, &SpeakersController::storedChanged);
    {
        Held held(t.a);
        sc->confirm();
        t.r->activate(t.b);
    }
    t.idle(t.a);
    t.idle(t.b);
    d.check("mdqt2c: a Confirm answered over a switch is not B's", !sc->stored().value(QLatin1String("confirmedByUser")).toBool() ||
                                                                      sc->stored().value(QLatin1String("outputs")).toArray().at(0).toObject().value(QLatin1String("role")).toString() != edited);
    bool leaked = false;
    for (const QList<QVariant> &s : stored) leaked = leaked || s.value(0).toString() == QLatin1String("confirm");
    d.check("mdqt2c: nor said on B's page", !leaked);
    t.r->activate(t.a);
    t.idle(t.a);
    d.check("mdqt2c: but A's, kept", d.waitFor(5000, [&] { return sc->status() == QLatin1String("confirmed"); }) && roleOf() == edited && !sc->said().isEmpty());
}

// ---- Tools, stats, the monitor -----------------------------------------------------------

void tools(Two &t, ToolsController *tc) {
    SmokeDriver &d = t.d;
    t.r->activate(t.a);
    // Two places show one tool
    QSignalSpy upmix(tc, &ToolsController::upmixStatusChanged);
    const auto polls = [&] {
        upmix.clear();
        d.spin(500);
        return upmix.count() > 1;
    };
    tc->windowShown(QStringLiteral("upmixer"), true);
    tc->windowShown(QStringLiteral("upmixer"), true);
    d.check("mdqt2c: the upmixer's poll runs", tc->upmixSupported() && polls());
    tc->windowShown(QStringLiteral("upmixer"), false);
    d.check("mdqt2c: one of two places gone does not stop it", polls());
    tc->windowShown(QStringLiteral("upmixer"), false);
    d.spin(100);
    d.check("mdqt2c: the last one does", !polls());
    tc->windowShown(QStringLiteral("upmixer"), false);   // a page unloaded that never showed
    tc->windowShown(QStringLiteral("upmixer"), true);
    d.check("mdqt2c: and one more report of gone counts for nothing", polls());
    tc->windowShown(QStringLiteral("upmixer"), false);
    // Solo is the window's, on the device that is left too
    tc->setSubharm(QStringLiteral("enabled"), 1);
    tc->windowShown(QStringLiteral("subharmonic"), true);
    tc->windowShown(QStringLiteral("subharmonic"), true);
    tc->setSubharmSolo(true);
    t.idle(t.a);
    d.check("mdqt2c: solo on A", d.waitFor(3000, [&] { return tc->subharmSolo(); }));
    tc->windowShown(QStringLiteral("subharmonic"), false);
    t.idle(t.a);
    d.check("mdqt2c: one of two places gone leaves it", tc->subharmSolo());
    t.r->activate(t.b);
    d.check("mdqt2c: leaving A switches it off there", t.idle(t.a) && !t.a->model()->state()->subharm_solo && !tc->subharmSolo());
    tc->windowShown(QStringLiteral("subharmonic"), false);
    t.r->activate(t.a);
    tc->setSubharm(QStringLiteral("enabled"), 0);
    t.idle(t.a);
}

void readings(Two &t, StatsController *st, MonitorController *mon) {
    SmokeDriver &d = t.d;
    t.r->activate(t.a);
    st->setWindowShown(true);
    const QVariant reconnects = st->device().value(QStringLiteral("reconnects"));
    const auto known = [&] { return st->starvation().value(QStringLiteral("known")).toBool(); };
    d.check("mdqt2c: A's stats", d.waitFor(5000, [&] { return known() && !st->fillRows().isEmpty(); }));
    const auto samples = [&] {
        int n = 0;
        for (const QVariant &v : st->fillRows())
            for (const QVariant &s : v.toMap().value(QStringLiteral("history")).toList())
                if (!std::isnan(s.toDouble())) ++n;
        return n;
    };
    d.waitFor(3000, [&] { return samples() > 20; });
    {
        Held held(t.b);
        t.r->activate(t.b);
        d.check("mdqt2c: B's page shows none of A's", !known() && samples() == 0);
    }
    d.check("mdqt2c: B's come at once", d.waitFor(1500, known));
    t.r->activate(t.a);
    d.check("mdqt2c: back on A: what it last said", known() && samples() > 20);
    d.check("mdqt2c: a switch is no reconnect", st->device().value(QStringLiteral("reconnects")) == reconnects);
    st->setWindowShown(false);

    // The log: rows, Pause and Clear are each device's
    const auto log = [&](DeviceSession *s, const char *name) {
        s->model()->write(QString::fromLatin1(name), [](FfiCore *c) { return dspi_set_user_volume(c, -21.0f) && dspi_set_user_volume(c, -20.0f); });
        t.idle(s);
    };
    mon->setWindowShown(true);
    log(t.a, "mdqt2c log A");
    d.spin(300);
    const QString atA = mon->text();
    mon->setPaused(true);
    t.r->activate(t.b);
    d.check("mdqt2c: B's monitor is not paused", !mon->paused() && mon->status() == QLatin1String("listening"));
    d.spin(300);
    mon->clear();
    t.idle(t.b);
    t.r->activate(t.a);
    d.check("mdqt2c: A's rows and its Pause are there on the way back", mon->paused() && mon->text() == atA);
    mon->setPaused(false);
    mon->setWindowShown(false);
    t.idle(t.a);
}

// ---- The sections ------------------------------------------------------------------------

void measureSection(SmokeDriver &d) {
    Two t{ d, DeviceRegistry::instance(), Registrar::instance<MeasureController>(), Registrar::instance<TuneController>() };
    const bool ok = t.mc && t.tc && t.r && t.open();
    d.check("mdqt2c: two demos", ok);
    if (!ok) return;
    d.allowSectionTime(240);
    QByteArray result;
    measuring(t, &result);
    microphone(t);
    live(t);
    if (!result.isEmpty()) tune(t, result);
    t.close();
}

void controllersSection(SmokeDriver &d) {
    auto *sg = Registrar::instance<SiggenController>();
    auto *rta = Registrar::instance<RtaController>();
    auto *sc = Registrar::instance<SpeakersController>();
    auto *tools_ = Registrar::instance<ToolsController>();
    auto *st = Registrar::instance<StatsController>();
    auto *mon = Registrar::instance<MonitorController>();
    Two t{ d, DeviceRegistry::instance(), Registrar::instance<MeasureController>(), Registrar::instance<TuneController>() };
    const bool ok = sg && rta && sc && tools_ && st && mon && t.mc && t.tc && t.r && t.open();
    d.check("mdqt2c: the controllers and two demos", ok);
    if (!ok) return;
    speakers(t, sc);
    tools(t, tools_);
    readings(t, st, mon);
    spectrum(t, rta);
    generator(t, sg);
    t.close();
    replaced(t, sg);
}

const SmokeDriver::Section jobs("mdqt2c-jobs", 966, &measureSection);
const SmokeDriver::Section controllers("mdqt2c-controllers", 967, &controllersSection);
} // namespace
