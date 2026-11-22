// -- WP ATQT -- Tools > Measure…'s "Several units" tab and Window > System's
// S/PDIF units, against the real hub `uconsole --demo hub --alink
// --with-measure-units` (DSPI_SMOKE_UCONSOLE; WP ATFAKE's units on the
// time-base contract: DEMO-TB-A/B/C on one audio link time base, DEMO-TB-C
// re-anchoring every 15 s, DEMO-TB-X on another sender's, an S/PDIF source and
// a follower that hears no DSPi time base; the settle is 8 s).
//
// No audio: the microphone is a silent source through AudioCapture's seam (no
// device is opened), and the units' generators are the fakes' arithmetic. It
// cannot answer them, so a real run here is planned, started (the settle wait
// in words) and stopped or ended by the hub; as the web's ATWEB e2e. The
// result, the proposal, the apply with the hub's guard, the verify, Keep asked
// twice and Revert are driven through the controller's call seam with stand-in
// answers of the core (the demo hub has no microphone that hears the fakes:
// WP ATDEMO's is not merged), the units' fresh checks always the real ones.
//
// -- WP ATQTSIM -- The demo hub's microphone that hears the fakes is merged now:
// `atqtsim-units` (218, at the end of this file) runs that whole path on the
// real core; the stand-ins above stay for what the demo cannot make happen (an
// old hub without the guard, a Keep after the guard ran out).
//
// Sections: `atqt-units` (216), `atqt-spdif` (217). Screenshots in <out>/atqt/.
#include "SmokeDriver.h"

#include "AudioCapture.h"
#include "Core.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"   // -- WP RCFIX --
#include "LinkController.h"
#include "MeasureController.h"
#include "ModeController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SpeakersController.h"   // -- WP RCFIX --
#include "SystemController.h"
#include "UnitsMeasureController.h"

#include "CoreApi.h"

#include <QAudioFormat>
#include <QDir>
#include <QElapsedTimer>
#include <QIODevice>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMutex>
#include <QProcess>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QThread>
#include <QTimer>
#include <algorithm>
#include <cmath>   // -- WP ATQTSIM --
#include <cstdio>
#include <cstring>
#include <memory>

namespace {

const QString A = QStringLiteral("DEMO-TB-A");
const QString B = QStringLiteral("DEMO-TB-B");
const QString C = QStringLiteral("DEMO-TB-C");
const QString X = QStringLiteral("DEMO-TB-X");

// One `uconsole --demo hub --alink --with-measure-units` process
class MeasureHub {
public:
    bool start(const QString &store) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        QDir(store).removeRecursively();
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList{ "--demo", "hub", "--alink", "--with-measure-units", "--loopback", "--json", "--no-keep-awake",
                                          "--open", "--forget-clients", "--store", store, "--port", "0" });
        if (!m_proc.waitForStarted(5000)) return false;
        QElapsedTimer t;
        t.start();
        while (t.elapsed() < 20000) {
            if (!m_proc.canReadLine() && !m_proc.waitForReadyRead(1000)) {
                if (m_proc.state() != QProcess::Running) return false;
                continue;
            }
            while (m_proc.canReadLine()) {
                const QJsonObject e = QJsonDocument::fromJson(m_proc.readLine()).object();
                if (e.value(QLatin1String("event")).toString() == QLatin1String("status")) port = e.value(QLatin1String("port")).toInt();
            }
            if (port > 0) {
                url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(port);
                return true;
            }
        }
        return false;
    }
    void stop() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    ~MeasureHub() { stop(); }
    int port = 0;
    QString url;

private:
    QProcess m_proc;
};

bool isOkJson(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }

QJsonObject cjson(const std::function<uint32_t(char *, uint32_t)> &f) {
    QByteArray buf(256 * 1024, '\0');
    const uint32_t n = f(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    return QJsonDocument::fromJson(QByteArray(buf.constData(), int(n) - 1)).object();
}

// A unit's volume lowered through a Link client of the smoke's own (what a
// person would do on that unit): S2 refuses a measurement at the demo's
// volume
bool lowerVolume(const QString &url, const QString &serial) {
    FfiCore *c = dspi_core_new();
    if (!c) return false;
    const QByteArray u = url.toUtf8(), s = serial.toUtf8();
    bool ok = isOkJson(cjson([&](char *b, uint32_t l) { return dspi_link_connect(c, u.constData(), "atqt-smoke", "0", "", "", b, l); }));
    int handle = -1;
    for (int i = 0; ok && handle < 0 && i < 50; ++i) {
        for (const QJsonValue &d : cjson([&](char *b, uint32_t l) { return dspi_link_devices(c, b, l); }).value(QLatin1String("value")).toArray())
            if (d.toObject().value(QLatin1String("serial")).toString() == serial) handle = d.toObject().value(QLatin1String("handle")).toInt(-1);
        if (handle < 0) QThread::msleep(100);
    }
    ok = ok && handle >= 0
         && isOkJson(cjson([&](char *b, uint32_t l) { return dspi_link_select_device(c, uint32_t(handle), s.constData(), b, l); }))
         && dspi_fetch_all(c) && dspi_set_user_volume(c, -20.0f);
    dspi_link_close(c);
    dspi_core_free(c);
    return ok;
}

// A sequential device the silent source appends to
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

// A quiet room (-80 dBFS noise), mono f32 at 48 kHz, 10 ms every 10 ms,
// through AudioCapture's reader: no audio device is opened
class QuietSource final : public dspi_capture::Source {
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
    QString name() const override { return QStringLiteral("Smoke quiet room"); }
    void close() override {
        m_timer.reset();
        m_device.reset();
    }

private:
    void next() {
        constexpr int n = 480;
        QByteArray bytes(n * int(sizeof(float)), '\0');
        auto *f = reinterpret_cast<float *>(bytes.data());
        for (int i = 0; i < n; ++i) {
            m_seed = m_seed * 1664525u + 1013904223u;
            f[i] = float((double(m_seed >> 8) / double(1u << 24) - 0.5) * 2e-4);
        }
        m_produced += n;
        m_device->push(bytes);
    }
    QAudioFormat m_format;
    std::unique_ptr<PushDevice> m_device;
    std::unique_ptr<QTimer> m_timer;
    qint64 m_produced = 0;
    quint32 m_seed = 7;
};

bool click(QQuickItem *item) { return item && item->isVisible() && item->isEnabled() && QMetaObject::invokeMethod(item, "clicked"); }

// The calls the controller made, by name and unit (the seam's record)
struct Record {
    QMutex lock;
    QStringList calls;   // "unitCheck A", "start", ...
    QByteArray lastChecks;
    void add(const QString &c) {
        QMutexLocker l(&lock);
        calls << c;
    }
    int count(const QString &c) {
        QMutexLocker l(&lock);
        return int(calls.count(c));
    }
    void clear() {
        QMutexLocker l(&lock);
        calls.clear();
        lastChecks.clear();
    }
};

// Runs `f` when the scope ends
struct ScopeExit {
    std::function<void()> f;
    ~ScopeExit() {
        if (f) f();
    }
};

} // namespace

static void unitsSection(SmokeDriver &d) {
    auto *link = Registrar::instance<LinkController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *uc = Registrar::instance<UnitsMeasureController>();
    auto *modes = Registrar::instance<ModeController>();
    DeviceModel *device = d.app().device;
    d.check("atqt: the controllers", link && mc && uc && modes);
    if (!link || !mc || !uc || !modes) return;
    const QString out = d.outDir() + QStringLiteral("/atqt");
    QDir().mkpath(out);
    const bool demoBefore = device->demo() && device->connected();
    const QString modeBefore = modes->mode();
    modes->choose(QStringLiteral("advanced"), false);
    QStringList steps;
    QHash<QString, bool> results;
    const QMetaObject::Connection conn = QObject::connect(uc, &UnitsMeasureController::stepDone, uc, [&](const QString &s, bool ok) {
        steps << s;
        results.insert(s, ok);
    });
    // Nothing of this section's stays behind it: the record and the stand-ins
    const ScopeExit cleanup{ [uc, conn] {
        QObject::disconnect(conn);
        uc->setCallsForTest(nullptr);
        uc->setRoleForTest(QString());
    } };
    const auto step = [&](const QString &s, int ms) { return d.waitFor(ms, [&] { return steps.contains(s); }); };
    QQuickWindow *w = nullptr;
    const auto in = [&](const QString &name) { return w ? d.item(name, w->contentItem()) : nullptr; };
    // The tab scrolled to its end (the proposal and what follows), for a screenshot
    const auto toEnd = [&] {
        QQuickItem *sv = in(QStringLiteral("MeasureUnits.scroll"));
        auto *flick = sv ? sv->property("contentItem").value<QQuickItem *>() : nullptr;
        if (!flick) return;
        d.spin(100);
        flick->setProperty("contentY", std::max(0.0, flick->property("contentHeight").toDouble() - flick->height()));
        d.spin(150);
    };
    const auto showUnits = [&] {
        w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("tools.measure")));
        if (!w) return false;
        d.waitFor(2000, [&] { return w->isVisible(); });
        QQuickItem *tabs = in(QStringLiteral("Measure.tabs"));
        if (!tabs) return false;
        tabs->setProperty("currentIndex", 1);
        d.spin(100);
        steps.clear();
        tabs->setProperty("currentIndex", 6);
        return step(QStringLiteral("enter"), 10000);
    };

    // ---- Not on the hub: it says it needs the DSPi hub ------------------------------
    d.check("atqt: the Several units tab opens on the demo DSPi", showUnits());
    QQuickItem *none = in(QStringLiteral("MeasureUnits.unavailable"));
    d.check("atqt: off our hub it says measuring several units needs the DSPi hub (measure.system.needs_hub), nothing to start",
            none && none->isVisible() && none->property("key").toString() == QLatin1String("measure.system.needs_hub")
                && none->property("text").toString().contains(QLatin1String("needs the DSPi hub"))
                && !(in(QStringLiteral("MeasureUnits.start")) && in(QStringLiteral("MeasureUnits.start"))->isVisible()));
    if (w) w->close();

    // ---- On the hub -----------------------------------------------------------------
    MeasureHub hub;
    const bool up = hub.start(out + QStringLiteral("/hub"));
    d.check("atqt: uconsole --demo hub --alink --with-measure-units runs (DSPI_SMOKE_UCONSOLE)", up);
    if (!up) return;
    bool lowered = true;
    for (const QString &s : { A, B, C, X }) lowered = lowerVolume(hub.url, s) && lowered;
    d.check("atqt: each unit's volume lowered through a Link client of its own (S2)", lowered);
    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    link->connectToHub(hub.url);
    const bool listed = settled() && d.waitFor(15000, [&] {
        int n = 0;
        // -- review AT-D2 -- the four measured here (WP ATDEMO's sender DEMO-TB-TX is a fifth DEMO-TB- device)
        for (const QVariant &v : link->devices()) n += QStringList{ A, B, C, X }.contains(v.toMap().value(QStringLiteral("serial")).toString());
        return link->phase() == QLatin1String("choose_device") && n == 4;
    });
    for (const QVariant &v : link->devices())
        if (v.toMap().value(QStringLiteral("serial")).toString() == A) link->bindDevice(v.toMap().value(QStringLiteral("handle")).toInt(), A);
    const bool bound = listed && settled()
                       && d.waitFor(15000, [&] { return link->phase() == QLatin1String("bound") && device->remote() && device->connected(); })
                       && d.waitIdle(20000);
    d.check("atqt: on the hub, DEMO-TB-A bound", bound && device->serial() == A);
    mc->setCaptureSource([]() -> std::unique_ptr<dspi_capture::Source> { return std::make_unique<QuietSource>(); });
    const auto finish = [&] {
        uc->setCallsForTest(nullptr);
        uc->setRoleForTest(QString());
        if (w) w->close();
        mc->setCaptureSource(nullptr);
        link->closeSession();
        d.waitFor(10000, [&] { return !link->busy() && !link->inSession(); });
        hub.stop();
        if (demoBefore && !device->connected()) device->openDemo();
        d.waitFor(10000, [&] { return !demoBefore || device->connected(); });
        modes->choose(modeBefore, false);
    };
    if (!bound || !showUnits()) {
        d.check("atqt: the tab on the hub", false);
        finish();
        return;
    }
    d.check("atqt: on our hub the step is offered", uc->unavailable().isEmpty() && in(QStringLiteral("MeasureUnits.start")));
    const auto unit = [&](const QString &s) {
        for (const QVariant &v : uc->units())
            if (v.toMap().value(QStringLiteral("serial")).toString() == s) return v.toMap();
        return QVariantMap();
    };
    d.check("atqt: the hub's units are listed, this DSPi among them", unit(A).value(QStringLiteral("own")).toBool() && !unit(B).isEmpty()
                                                                     && !unit(C).isEmpty() && !unit(X).isEmpty());

    // Every core call recorded (the real ones), to see each unit judged by its own
    // core right before each call that takes checks
    Record rec;
    uc->setCallsForTest([&rec](FfiCore *c, const QString &op, const QByteArray &a, const QByteArray &b) {
        if (op == QLatin1String("unitCheck")) rec.add(QStringLiteral("unitCheck ") + QJsonDocument::fromJson(b).object().value(QLatin1String("serial")).toString());
        else rec.add(op);
        if (op == QLatin1String("start") || op == QLatin1String("preflight")) {
            QMutexLocker l(&rec.lock);
            rec.lastChecks = b;
        }
        return UnitsMeasureController::coreCall(c, op, a, b);
    });

    // The placement first: none is assumed, headphones are refused
    QQuickItem *start = in(QStringLiteral("MeasureUnits.start"));
    QQuickItem *checkButton = in(QStringLiteral("MeasureUnits.check"));
    QQuickItem *problem = in(QStringLiteral("MeasureUnits.placementProblem"));
    d.check("atqt: no placement is assumed: nothing can be checked or started",
            uc->placement().isEmpty() && problem && problem->isVisible()
                && problem->property("text").toString().contains(QLatin1String("Choose where the microphone is first")) && start
                && !start->isEnabled() && checkButton && !checkButton->isEnabled());
    uc->setPlacement(QStringLiteral("headphones"));
    d.spin(100);
    d.check("atqt: headphones are never measured across units", problem->property("text").toString().contains(QLatin1String("Headphones are never measured"))
                                                                   && !start->isEnabled());
    uc->setPlacement(QStringLiteral("nearField"));
    d.spin(100);
    d.check("atqt: across units the listening position only", problem->property("text").toString().contains(QLatin1String("listening position only"))
                                                                  && !start->isEnabled());
    uc->setPlacement(QStringLiteral("listeningPosition"));
    // Two units, each with its own connection
    steps.clear();
    uc->setUnitChosen(A, true);
    uc->setUnitChosen(B, true);
    const bool opened = step(QStringLiteral("unit"), 20000) && d.waitFor(20000, [&] { return uc->unitOpen(B) && !unit(B).value(QStringLiteral("outputs")).toList().isEmpty(); });
    d.check("atqt: a chosen unit gets a connection of its own, its outputs read on it", opened);
    d.spin(200);
    d.check("atqt: with the listening position and two units the step may be checked and started",
            !problem->isVisible() && uc->blocker().isEmpty() && start->isEnabled() && checkButton->isEnabled());

    // -- WP RCFIX -- (review RC, RC3) the rows' roles are each unit's own confirmed
    // layout's, and the core holds a unit to it whatever the row says
    {
        auto *sc = Registrar::instance<SpeakersController>();
        const auto role = [&](const QString &s) {
            for (const QVariant &v : unit(s).value(QStringLiteral("outputs")).toList())
                if (v.toMap().value(QStringLiteral("output")).toInt() == 0) return v.toMap().value(QStringLiteral("role")).toString();
            return QString();
        };
        const auto confirmA = [&](const QString &r) {
            for (const QVariant &v : sc->rows()) {
                const int o = v.toMap().value(QStringLiteral("output")).toInt();
                sc->setIncluded(o, o == 0);
            }
            sc->setRole(0, r);
            sc->confirm();
            return d.waitFor(10000, [&] { return sc->status() == QLatin1String("confirmed") && role(A) == r; });
        };
        const auto confirmB = [&](const char *r) {
            const QByteArray text = QByteArrayLiteral("{\"outputs\":[{\"output\":0,\"role\":\"") + r
                                    + QByteArrayLiteral("\",\"speaker\":\"B\",\"group\":0}],\"confirmedByUser\":true}");
            QByteArray buf(1 << 16, '\0');
            if (!uc->unitCore(B) || dspi_speaker_layout_set(uc->unitCore(B), text.constData(), buf.data(), uint32_t(buf.size())) == 0) return false;
            uc->setUnitChosen(B, false);
            // Its connection gives the serial back first
            d.waitFor(10000, [] { return !DeviceRegistry::instance()->extraSerials().contains(B); });
            steps.clear();
            uc->setUnitChosen(B, true);
            return step(QStringLiteral("unit"), 20000) && d.waitFor(20000, [&] { return uc->unitOpen(B) && role(B) == QLatin1String(r); });
        };
        const auto tweeterHeld = [&] {
            steps.clear();
            click(checkButton);
            step(QStringLiteral("check"), 20000);
            for (const QVariant &l : uc->preflightLines())
                if (l.toMap().value(QStringLiteral("text")).toString().contains(B) && l.toMap().value(QStringLiteral("text")).toString().contains(QLatin1String("tweeter")))
                    return true;
            return false;
        };
        d.check("atqt: this DSPi's rows take its confirmed layout's role when it is confirmed",
                sc && d.waitFor(10000, [&] { return sc->loaded(); }) && role(A) == QLatin1String("full") && confirmA(QStringLiteral("sub")));
        d.check("atqt: another unit's rows are pre-filled from its own confirmed layout", confirmB("high"));
        d.check("atqt: its tweeter is held at the check", tweeterHeld());
        uc->setOutputRole(B, 0, QStringLiteral("full"));
        d.check("atqt: and stays held when its row says full range", role(B) == QLatin1String("full") && tweeterHeld());
        d.check("atqt: the layouts back to full range", confirmB("full") && confirmA(QStringLiteral("full")) && uc->blocker().isEmpty());
        d.spin(200);
    }

    // The check: each unit judged by its own core
    rec.clear();
    steps.clear();
    click(checkButton);
    const bool checked = step(QStringLiteral("check"), 20000) && results.value(QStringLiteral("check"));
    bool ready = false;
    for (const QVariant &l : uc->preflightLines()) ready = ready || l.toMap().value(QStringLiteral("kind")).toString() == QLatin1String("ready");
    for (const QVariant &l : uc->preflightLines()) std::printf("smoke: atqt: preflight: %s\n", qPrintable(l.toMap().value(QStringLiteral("text")).toString()));
    d.check("atqt: Check asks each unit's own core, then the preflight: ready", checked && ready && rec.count(QStringLiteral("unitCheck ") + A) == 1
                                                                                   && rec.count(QStringLiteral("unitCheck ") + B) == 1 && rec.count(QStringLiteral("preflight")) == 1);
    d.grab(w, QStringLiteral("atqt/units-checked.png"));

    // -- review AT-D2 -- the demo hub's simulated microphone (WP ATDEMO) is offered here and is the
    // default; with it a run opens no microphone on this machine, and a group with a unit the
    // hub's room does not hear starts nothing
    d.check("atqt: the demo hub's simulated microphone is offered and chosen by default",
            uc->simAvailable() && uc->useSim() && uc->simLabel() == QLatin1String("Simulated microphone"));
    {
        QQuickItem *simBox = in(QStringLiteral("MeasureUnits.sim"));
        steps.clear();
        uc->setSweepMs(1000);
        click(start);
        const bool simStarted = step(QStringLiteral("start"), 30000) && results.value(QStringLiteral("start"));
        std::printf("smoke: atqt: sim start: %s %s\n", qPrintable(uc->error()), qPrintable(uc->phaseText()));
        QQuickItem *simNote = in(QStringLiteral("MeasureUnits.simOn"));
        d.check("atqt: with the simulated microphone a run starts and no microphone is opened here; the run's bar says so",
                simBox && simBox->isVisible() && simStarted && uc->running() && uc->simOn() && !mc->micOpen() && simNote && simNote->isVisible());
        click(in(QStringLiteral("MeasureUnits.stop")));
        d.check("atqt: Stop ends the simulated run too", d.waitFor(15000, [&] { return !uc->running() && !mc->running(); }));
        // DEMO-TB-X is no unit of the hub's room: the hub refuses, nothing starts
        uc->setUnitChosen(B, false);
        steps.clear();
        uc->setUnitChosen(X, true);
        step(QStringLiteral("unit"), 20000);
        d.waitFor(20000, [&] { return uc->unitOpen(X) && uc->blocker().isEmpty(); });
        steps.clear();
        click(start);
        step(QStringLiteral("start"), 30000);
        std::printf("smoke: atqt: sim refused: %s (%s)\n", qPrintable(uc->error()), qPrintable(uc->errorKey()));
        d.check("atqt: the simulated microphone hears only the hub's own demo units: with another unit nothing starts, said in words",
                !results.value(QStringLiteral("start")) && !uc->running() && !mc->micOpen()
                    && uc->error().contains(QLatin1String("simulated microphone cannot be used")));
        uc->setUnitChosen(X, false);
        steps.clear();
        uc->setUnitChosen(B, true);
        step(QStringLiteral("unit"), 20000);
        d.waitFor(20000, [&] { return uc->unitOpen(B) && uc->blocker().isEmpty(); });
        // What follows uses the Measure window's input (the quiet source), as before
        uc->setUseSim(false);
        d.check("atqt: the simulated microphone can be turned off", !uc->useSim());
    }

    // The run: the microphone, fresh checks before the preflight and again before the
    // start; Stop in view with the settle wait in words; Stop reaches the hub
    rec.clear();
    steps.clear();
    uc->setSweepMs(1000);
    click(start);
    const bool started = step(QStringLiteral("start"), 30000) && results.value(QStringLiteral("start"));
    std::printf("smoke: atqt: start: %s %s\n", qPrintable(uc->error()), qPrintable(uc->phaseText()));
    d.check("atqt: the run starts; each unit's fresh check before the preflight and again before the start",
            started && rec.count(QStringLiteral("unitCheck ") + A) == 2 && rec.count(QStringLiteral("unitCheck ") + B) == 2
                && rec.count(QStringLiteral("start")) == 1);
    {
        QMutexLocker l(&rec.lock);
        const QJsonArray checks = QJsonDocument::fromJson(rec.lastChecks).array();
        d.check("atqt: the start gets one check per unit, each its own",
                checks.size() == 2 && checks.at(0).toObject().value(QLatin1String("serial")).toString() == A
                    && checks.at(1).toObject().value(QLatin1String("serial")).toString() == B);
    }
    QQuickItem *bar = in(QStringLiteral("MeasureUnits.runBar"));
    QQuickItem *stopBar = in(QStringLiteral("Measure.stopBar"));
    d.check("atqt: Stop is in view, here and in the window's Stop bar", started && bar && bar->isVisible() && stopBar
                                                                          && d.waitFor(2000, [&] { return stopBar->isVisible(); }) && mc->running());
    const bool settleWords = started && d.waitFor(20000, [&] { return uc->phaseText().contains(QLatin1String("waiting for the units' clocks to settle")); });
    std::printf("smoke: atqt: phase: %s\n", qPrintable(uc->phaseText()));
    d.check("atqt: the settle wait is said in words (MeasureSystemState.words), in the window's Stop bar too",
            settleWords && mc->runningText() == uc->phaseText());
    d.grab(w, QStringLiteral("atqt/units-settling.png"));
    click(in(QStringLiteral("MeasureUnits.stop")));
    const bool stopped = d.waitFor(15000, [&] { return !uc->running(); });
    std::printf("smoke: atqt: stopped: %s (%s)\n", qPrintable(uc->issueText()), qPrintable(uc->issueKey()));
    d.check("atqt: Stop ends it (system.measure.abort), said in words; nothing plays or listens",
            stopped && uc->issueKey() == QLatin1String("measure.stopped") && !uc->issueText().isEmpty()
                && d.waitFor(5000, [&] { return !mc->running() && !mc->micOpen(); }));

    // The window's own Stop bar stops it too
    steps.clear();
    click(start);
    const bool again = step(QStringLiteral("start"), 30000) && results.value(QStringLiteral("start"));
    click(in(QStringLiteral("Measure.stop")));
    d.check("atqt: the Measure window's Stop stops the run across units", again && d.waitFor(15000, [&] { return !uc->running() && !mc->running(); }));

    // Not one clock: the hub refuses at open, in words
    uc->setUnitChosen(B, false);
    steps.clear();
    uc->setUnitChosen(X, true);
    step(QStringLiteral("unit"), 20000);
    d.waitFor(20000, [&] { return uc->unitOpen(X) && uc->blocker().isEmpty(); });
    steps.clear();
    click(start);
    step(QStringLiteral("start"), 30000);
    std::printf("smoke: atqt: not one clock: %s (%s)\n", qPrintable(uc->error()), qPrintable(uc->errorKey()));
    QQuickItem *err = in(QStringLiteral("MeasureUnits.error"));
    d.check("atqt: units on two senders' time bases: \"These units do not share a clock\" (measure.system.not_one_domain)",
            !results.value(QStringLiteral("start")) && uc->errorKey() == QLatin1String("measure.system.not_one_domain")
                && uc->error().startsWith(QLatin1String("These units do not share a clock: The units report")) && err && err->isVisible() && !uc->running());
    uc->setUnitChosen(X, false);

    // A group that does not fit one capture: several runs, said as soon as it starts
    steps.clear();
    uc->setUnitChosen(B, true);
    step(QStringLiteral("unit"), 20000);
    d.waitFor(20000, [&] { return uc->unitOpen(B) && !unit(B).value(QStringLiteral("outputs")).toList().isEmpty(); });
    uc->setOutputOn(A, 1, true);
    uc->setOutputOn(B, 1, true);
    uc->setSweepMs(15000);
    steps.clear();
    click(start);
    const bool long_ = step(QStringLiteral("start"), 30000) && results.value(QStringLiteral("start"));
    QQuickItem *runs = in(QStringLiteral("MeasureUnits.runs"));
    const bool runsSaid = long_ && d.waitFor(5000, [&] { return runs && runs->isVisible(); });
    std::printf("smoke: atqt: runs: %s\n", qPrintable(uc->runsText()));
    d.check("atqt: \"needs N runs\" (measure_system_runs) shows as soon as a chained group starts",
            runsSaid && runs->property("key").toString() == QLatin1String("measure_system_runs")
                && QRegularExpression(QStringLiteral("this group needs \\d+ runs, chained through the anchor output")).match(uc->runsText()).hasMatch());
    d.grab(w, QStringLiteral("atqt/units-runs.png"));
    uc->stop();
    d.waitFor(15000, [&] { return !uc->running(); });
    uc->setOutputOn(A, 1, false);
    uc->setOutputOn(B, 1, false);
    uc->setUnitChosen(B, false);

    // A unit whose time base re-anchors (DEMO-TB-C, every 15 s): the run ends with
    // the unit named, as having lost its lock
    steps.clear();
    uc->setUnitChosen(C, true);
    step(QStringLiteral("unit"), 20000);
    d.waitFor(20000, [&] { return uc->unitOpen(C) && uc->blocker().isEmpty(); });
    uc->setSweepMs(1000);
    QString lost;
    // DEMO-TB-C re-anchors every 15 s: a run of 1 s sweeps meets it within
    // a few tries (the section's own time limit is 120 s: 60 s for this)
    QElapsedTimer budget;
    budget.start();
    for (int attempt = 0; attempt < 5 && lost.isEmpty() && budget.elapsed() < 60000; ++attempt) {
        steps.clear();
        click(start);
        if (!step(QStringLiteral("start"), 30000)) break;
        if (!results.value(QStringLiteral("start"))) {
            // The hub found DEMO-TB-C anchoring at open: try again
            std::printf("smoke: atqt: lost lock, attempt %d: %s\n", attempt, qPrintable(uc->error()));
            d.spin(1000);
            continue;
        }
        d.waitFor(int(std::max<qint64>(1000, 60000 - budget.elapsed())), [&] { return !uc->running(); });
        if (uc->running()) uc->stop();
        std::printf("smoke: atqt: lost lock, attempt %d: %s (%s)\n", attempt, qPrintable(uc->issueText()), qPrintable(uc->issueKey()));
        if (uc->issueKey() == QLatin1String("measure.system.abort.disturbance")) lost = uc->issueText();
    }
    d.check("atqt: a run ends on DEMO-TB-C's lost lock: \"Unit lost lock\", the unit named",
            lost.startsWith(QLatin1String("Unit lost lock")) && lost.contains(C));
    uc->setUnitChosen(C, false);

    // ---- The result, the proposal, the apply and what follows (stand-in answers) -----
    steps.clear();
    uc->setUnitChosen(B, true);
    step(QStringLiteral("unit"), 20000);
    d.waitFor(20000, [&] { return uc->unitOpen(B) && uc->blocker().isEmpty(); });
    // The group A and B measured for real (the summed runs and the verify use its
    // setup), stopped while its clocks settle
    steps.clear();
    click(start);
    const bool measuredAB = step(QStringLiteral("start"), 30000) && results.value(QStringLiteral("start"));
    uc->stop();
    d.check("atqt: the group A and B started and stopped for the steps after it", measuredAB && d.waitFor(15000, [&] { return !uc->running(); }));
    // The core's answers for a measurement that finished (the units' checks stay real)
    QJsonObject proposal = QJsonDocument::fromJson(R"({"ok":true,"id":7,
        "arrivals":[{"serial":"DEMO-TB-A","output":0,"arrivalUs":-412.5,"measuredUs":-380.0,"uncertaintyUs":2.1,"offered":true,"reason":""},
                    {"serial":"DEMO-TB-B","output":0,"arrivalUs":0.0,"measuredUs":0.0,"uncertaintyUs":1.9,"offered":true,"reason":""}],
        "withheld":[],
        "transitions":[{"aSerial":"DEMO-TB-A","aOutput":0,"bSerial":"DEMO-TB-B","bOutput":0,"bandLoHz":80,"bandHiHz":120,"checked":false,"passed":false,"rmsDb":null,"message":""}],
        "sumCheckNeeded":true,"withdrawn":false,"offered":true,"mayApply":true,"notice":null,
        "guardS":0,"guardNote":"This hub has no guard for an alignment across units: nothing puts it back by itself. Keep or Revert it yourself."})").object();
    QJsonObject applied{ { QStringLiteral("ok"), true }, { QStringLiteral("stage"), QStringLiteral("none") } };
    QJsonArray keeps;   // the answers Keep gives, in turn
    QJsonObject revertAs; // -- WP REVERTW -- Revert's answer when set (else a clean revert)
    Record stand;
    QMutex standIn;     // the answers above, set here between the steps, read on the measurement loop
    uc->setCallsForTest([&](FfiCore *c, const QString &op, const QByteArray &a, const QByteArray &b) -> QByteArray {
        const auto j = [](const QJsonObject &o) { return QJsonDocument(o).toJson(QJsonDocument::Compact); };
        QMutexLocker held(&standIn);
        if (op == QLatin1String("unitCheck")) {
            stand.add(QStringLiteral("unitCheck ") + QJsonDocument::fromJson(b).object().value(QLatin1String("serial")).toString());
            return UnitsMeasureController::coreCall(c, op, a, b);
        }
        stand.add(op);
        if (op == QLatin1String("result"))
            return R"({"ok":true,"documents":[],"arrivals":[{"serial":"DEMO-TB-A","output":0,"arrivalUs":-412.5,"measuredUs":-380.0,"uncertaintyUs":2.1,"offered":true,"reason":""},{"serial":"DEMO-TB-B","output":0,"arrivalUs":0.0,"measuredUs":0.0,"uncertaintyUs":1.9,"offered":true,"reason":""}],"delays":[{"fromSerial":"DEMO-TB-A","fromOutput":0,"toSerial":"DEMO-TB-B","toOutput":0,"delayUs":412.5,"uncertaintyUs":2.8,"offered":true,"reason":""}],"transitions":[],"timingReliable":true,"reasons":[],"runIds":[1,2],"runs":1})";
        if (op == QLatin1String("proposal")) return j(proposal);
        if (op == QLatin1String("applied")) return j(applied);
        if (op == QLatin1String("sumCheck") || op == QLatin1String("verify")) {
            QMutexLocker l(&stand.lock);
            stand.lastChecks = a;
            return R"({"ok":false,"code":"invalid","error":"invalid argument: stand-in: the runs are the smoke's"})";
        }
        if (op == QLatin1String("apply")) {
            applied = QJsonDocument::fromJson(R"({"ok":true,"stage":"applied","revisionBefore":3,"revisionAfter":4,"snapshots":[],"writesOk":2,"writeFailures":[],"verified":null,"message":"Applied: verify it with the summed runs, then Keep or Revert.","guardS":300})").object();
            return j(applied);
        }
        if (op == QLatin1String("keep")) {
            const QJsonObject k = keeps.isEmpty() ? QJsonObject{ { QStringLiteral("ok"), false }, { QStringLiteral("error"), QStringLiteral("no answer") } }
                                                  : keeps.takeAt(0).toObject();
            if (isOkJson(k)) applied = k;
            return j(k);
        }
        if (op == QLatin1String("revert")) {
            applied = !revertAs.isEmpty() ? revertAs
                                          : QJsonObject{ { QStringLiteral("ok"), true }, { QStringLiteral("stage"), QStringLiteral("reverted") }, { QStringLiteral("message"), QString() } };
            return j(applied);
        }
        return UnitsMeasureController::coreCall(c, op, a, b);
    });
    const auto answer = [&](QJsonObject *what, const QJsonObject &now) {
        QMutexLocker held(&standIn);
        *what = now;
    };
    const auto answers = [&](const QJsonArray &now) {
        QMutexLocker held(&standIn);
        keeps = now;
    };
    const auto session = [&](const char *purpose, const char *phase, bool active) {
        uc->stateArrived(QJsonDocument(QJsonObject{ { QStringLiteral("ok"), true }, { QStringLiteral("phase"), QLatin1String(phase) },
                                                    { QStringLiteral("purpose"), QLatin1String(purpose) }, { QStringLiteral("active"), active },
                                                    { QStringLiteral("runs"), 1 }, { QStringLiteral("run"), 1 } })
                             .toJson(QJsonDocument::Compact));
    };
    // A measurement's session finishes: the result and its proposal are read
    steps.clear();
    session("measure", "run", true);
    session("measure", "done", false);
    const bool haveResult = step(QStringLiteral("run"), 10000) && d.waitFor(5000, [&] { return uc->hasResult() && uc->hasProposal(); });
    d.check("atqt: the result: arrivals with their uncertainty, the delay across units, the timing in words",
            haveResult && uc->arrivals().size() == 2
                && uc->arrivals().at(0).toMap().value(QStringLiteral("arrival")).toString().contains(QStringLiteral("± 2.1 µs"))
                && uc->delays().size() == 1 && uc->timingText() == QLatin1String("The timing of every run was reliable."));
    d.check("atqt: the proposal: the arrivals to write, the transition not checked yet",
            uc->proposalArrivals().size() == 2 && uc->transitions().size() == 1
                && uc->transitions().at(0).toMap().value(QStringLiteral("text")).toString().contains(QLatin1String("not checked yet")));
    QQuickItem *note = in(QStringLiteral("MeasureUnits.guardNote"));
    d.check("atqt: an old hub's guard note says nothing reverts by itself (guardNote)", note && note->isVisible()
                                                                                         && note->property("text").toString().contains(QLatin1String("nothing puts it back by itself")));
    // The summed check: each unit's fresh check, then the core's call with them
    stand.clear();
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.sumCheck")));
    const bool sumAsked = step(QStringLiteral("sumCheck"), 20000);
    {
        QMutexLocker l(&stand.lock);
        const QJsonArray checks = QJsonDocument::fromJson(stand.lastChecks).array();
        d.check("atqt: the summed check gets each unit's fresh check from its own core, right before it",
                sumAsked && stand.calls.indexOf(QStringLiteral("unitCheck ") + A) >= 0 && stand.calls.indexOf(QStringLiteral("unitCheck ") + B) >= 0
                    && stand.calls.indexOf(QStringLiteral("sumCheck")) > stand.calls.indexOf(QStringLiteral("unitCheck ") + B) && checks.size() == 2);
    }
    d.check("atqt: its refusal is shown in words", uc->error().startsWith(QLatin1String("Stand-in: the runs are the smoke's")));
    // The summed check passed: Apply for Admin only
    QJsonObject passed = proposal;
    passed.insert(QStringLiteral("sumCheckNeeded"), false);
    passed.insert(QStringLiteral("transitions"), QJsonArray{ QJsonObject{ { QStringLiteral("aSerial"), A }, { QStringLiteral("aOutput"), 0 },
                                                                             { QStringLiteral("bSerial"), B }, { QStringLiteral("bOutput"), 0 },
                                                                             { QStringLiteral("bandLoHz"), 80 }, { QStringLiteral("bandHiHz"), 120 },
                                                                             { QStringLiteral("checked"), true }, { QStringLiteral("passed"), true },
                                                                             { QStringLiteral("message"), QString() } } });
    passed.insert(QStringLiteral("guardS"), 300);
    passed.insert(QStringLiteral("guardNote"), QStringLiteral("The hub puts it back by itself in 300 s unless you keep it."));
    answer(&proposal, passed);
    steps.clear();
    session("sum_check", "run", true);
    session("sum_check", "done", false);
    step(QStringLiteral("sumCheck"), 10000);
    uc->setRoleForTest(QStringLiteral("control"));
    d.spin(200);
    QQuickItem *applyButton = in(QStringLiteral("MeasureUnits.apply"));
    QQuickItem *why = in(QStringLiteral("MeasureUnits.applyWhy"));
    stand.clear();
    steps.clear();
    uc->apply();
    d.check("atqt: Control: no Apply and no Revert; why in words (measure.system.admin); an apply sends nothing",
            applyButton && !applyButton->isVisible() && why && why->isVisible() && why->property("key").toString() == QLatin1String("measure.system.admin")
                && !(in(QStringLiteral("MeasureUnits.revert")) && in(QStringLiteral("MeasureUnits.revert"))->isVisible()) && step(QStringLiteral("apply"), 2000)
                && !results.value(QStringLiteral("apply")) && stand.count(QStringLiteral("apply")) == 0);
    uc->setRoleForTest(QStringLiteral("admin"));
    d.spin(200);
    d.check("atqt: Admin: Apply is offered", applyButton->isVisible() && applyButton->isEnabled());
    steps.clear();
    click(applyButton);
    const bool appliedOk = step(QStringLiteral("apply"), 10000) && results.value(QStringLiteral("apply")) && uc->stage() == QLatin1String("applied");
    QQuickItem *guard = in(QStringLiteral("MeasureUnits.guard"));
    const int left0 = uc->guardRemainingS();
    d.spin(2200);
    std::printf("smoke: atqt: guard: %s\n", qPrintable(uc->guardText()));
    d.check("atqt: Admin's apply: the hub's guard counts down from guardS, in words",
            appliedOk && left0 >= 299 && guard && guard->isVisible() && uc->guardRemainingS() <= left0 - 2
                && uc->guardText().contains(QLatin1String("without Keep the hub puts everything back by itself")));
    toEnd();
    d.grab(w, QStringLiteral("atqt/units-applied.png"));
    // The verify: fresh checks first (the stand-in refuses the runs), then a verify that did not pass
    stand.clear();
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.verify")));
    const bool verifyAsked = step(QStringLiteral("verify"), 20000);
    d.check("atqt: the verify gets each unit's fresh check from its own core, right before it",
            verifyAsked && stand.count(QStringLiteral("unitCheck ") + A) == 1 && stand.count(QStringLiteral("unitCheck ") + B) == 1
                && stand.count(QStringLiteral("verify")) == 1);
    answer(&applied, QJsonDocument::fromJson(R"({"ok":true,"stage":"verified","verified":false,"guardS":300,
        "message":"The verify did not confirm the alignment: DEMO-TB-A output 1 with DEMO-TB-B output 1 differs by 3.1 dB. Keep it only if it sounds right; Revert puts it back."})").object());
    steps.clear();
    session("verify", "run", true);
    session("verify", "done", false);
    step(QStringLiteral("verify"), 10000);
    d.check("atqt: a verify that did not pass is said in words", uc->verifyFailed() && uc->appliedText().contains(QLatin1String("differs by 3.1 dB")));
    // Keep: the core refuses the first; its words are the question; not pressed for the person
    answers(QJsonArray{ QJsonObject{ { QStringLiteral("ok"), false }, { QStringLiteral("code"), QStringLiteral("invalid") },
                                     { QStringLiteral("error"), QStringLiteral("invalid argument: the verify did not pass: keep again to keep it anyway") } } });
    stand.clear();
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.keep")));
    step(QStringLiteral("keep"), 10000);
    QQuickItem *question = in(QStringLiteral("MeasureUnits.keepQuestion"));
    QQuickItem *anyway = in(QStringLiteral("MeasureUnits.keepAnyway"));
    d.check("atqt: the first Keep after a failed verify is refused: the second question, with applied.message",
            uc->keepQuestion() && uc->keepRefusals() == 1 && question && d.waitFor(1000, [&] { return question->isVisible(); })
                && uc->keepQuestionText().contains(QLatin1String("differs by 3.1 dB")) && uc->stage() == QLatin1String("verified"));
    d.check("atqt: \"Keep anyway\" counts only a second after the refusal", anyway && !anyway->isEnabled()
                                                                             && anyway->property("text").toString().contains(QLatin1String("(in 1 s)")));
    toEnd();
    d.grab(w, QStringLiteral("atqt/units-keep-question.png"));
    // A Keep asked for within the second waits for it; Revert withdraws it
    answers(QJsonArray{ QJsonObject{ { QStringLiteral("ok"), true }, { QStringLiteral("stage"), QStringLiteral("kept") } } });
    uc->confirmKeep();
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.revert")));
    step(QStringLiteral("revert"), 10000);
    d.spin(1800);
    d.check("atqt: a delayed Keep is withdrawn by Revert: no second Keep is sent, reverted",
            stand.count(QStringLiteral("keep")) == 1 && stand.count(QStringLiteral("revert")) == 1 && uc->stage() == QLatin1String("reverted") && !uc->keepQuestion());
    // Again: applied, the verify did not pass, Keep refused, then the person's own second press a second later
    steps.clear();
    click(applyButton);
    step(QStringLiteral("apply"), 10000);
    answer(&applied, QJsonDocument::fromJson(R"({"ok":true,"stage":"verified","verified":false,"guardS":300,"message":"The verify did not confirm the alignment."})").object());
    session("verify", "run", true);
    session("verify", "done", false);
    d.waitFor(5000, [&] { return uc->verifyFailed(); });
    answers(QJsonArray{ QJsonObject{ { QStringLiteral("ok"), false }, { QStringLiteral("error"), QStringLiteral("invalid argument: the verify did not pass") } },
                        QJsonObject{ { QStringLiteral("ok"), true }, { QStringLiteral("stage"), QStringLiteral("kept") }, { QStringLiteral("message"), QString() } } });
    stand.clear();
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.keep")));
    step(QStringLiteral("keep"), 10000);
    const bool waited = d.waitFor(3000, [&] { return anyway->isEnabled(); });
    steps.clear();
    click(anyway);
    const bool kept = step(QStringLiteral("keep"), 10000) && results.value(QStringLiteral("keep"));
    d.check("atqt: only the person's second press, a second later, keeps it", waited && kept && stand.count(QStringLiteral("keep")) == 2
                                                                                 && uc->stage() == QLatin1String("kept"));
    // Keep after the hub's guard ran out: Reverted, with the hub's words
    steps.clear();
    click(applyButton);
    step(QStringLiteral("apply"), 10000);
    answer(&applied, QJsonDocument::fromJson(R"({"ok":true,"stage":"verified","verified":true,"guardS":300,"message":""})").object());
    session("verify", "run", true);
    session("verify", "done", false);
    d.waitFor(5000, [&] { return uc->stage() == QLatin1String("verified"); });
    answers(QJsonArray{ QJsonObject{ { QStringLiteral("ok"), true }, { QStringLiteral("stage"), QStringLiteral("reverted") },
                                     { QStringLiteral("message"), QStringLiteral("the hub's guard ran out (timeout): it set the installation and every unit back") } } });
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.keep")));
    step(QStringLiteral("keep"), 10000);
    QQuickItem *outcome = in(QStringLiteral("MeasureUnits.outcome"));
    d.check("atqt: a Keep the hub's guard already reverted comes back Reverted, with the hub's words",
            uc->stage() == QLatin1String("reverted") && outcome && outcome->isVisible()
                && outcome->property("text").toString().startsWith(QLatin1String("The hub's guard ran out (timeout)")) && uc->guardText().isEmpty());
    toEnd();
    d.grab(w, QStringLiteral("atqt/units-reverted-by-hub.png"));

    // -- WP REVERTW -- a Revert whose writes failed: its own stage, in the notice's words (owner 2026-11-19)
    steps.clear();
    click(applyButton);
    step(QStringLiteral("apply"), 10000);
    answer(&revertAs, QJsonDocument::fromJson(R"J({"ok":true,"stage":"revert_incomplete","revertNotice":"revert.incomplete","guardS":300,"message":"reverted, but not wholly: DEMO-TB-B out.delay: refused (not written)"})J").object());
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.revert")));
    step(QStringLiteral("revert"), 10000);
    outcome = in(QStringLiteral("MeasureUnits.outcome"));
    const QString ended = outcome ? outcome->property("text").toString() : QString();
    d.check("revertw: a revert not fully successful is its own stage, said in the notice's words with what was not put back",
            uc->stage() == QLatin1String("revert_incomplete") && outcome && outcome->isVisible()
                && ended.startsWith(QLatin1String("The revert was not fully successful."))
                && ended.contains(QLatin1String("DEMO-TB-B out.delay: refused"))
                && ended.endsWith(QLatin1String("Check the settings, or restore a backup (a snapshot)."))
                && uc->guardText().isEmpty());
    toEnd();
    d.grab(w, QStringLiteral("atqt/units-revert-incomplete.png"));
    answer(&revertAs, QJsonObject{});

    // Leaving the tab closes the units' connections
    QQuickItem *tabs = in(QStringLiteral("Measure.tabs"));
    if (tabs) tabs->setProperty("currentIndex", 1);
    d.check("atqt: leaving the tab closes the units' own connections", d.waitFor(3000, [&] { return !uc->unitOpen(B); }));
    finish();
}

// ---- The S/PDIF transport in Window > System ----------------------------------------

static void spdifSection(SmokeDriver &d) {
    auto *link = Registrar::instance<LinkController>();
    auto *sys = Registrar::instance<SystemController>();
    auto *modes = Registrar::instance<ModeController>();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    d.check("atqt-spdif: the controllers", link && sys && modes);
    if (!link || !sys || !modes) return;
    const QString out = d.outDir() + QStringLiteral("/atqt");
    QDir().mkpath(out);
    const bool demoBefore = device->demo() && device->connected();
    const QString modeBefore = modes->mode();
    modes->choose(QStringLiteral("advanced"), false);
    MeasureHub hub;
    const bool up = hub.start(out + QStringLiteral("/hub-spdif"));
    d.check("atqt-spdif: the hub runs", up);
    if (!up) return;
    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    link->connectToHub(hub.url);
    settled();
    d.waitFor(15000, [&] {
        if (link->phase() != QLatin1String("choose_device")) return false;
        for (const QVariant &v : link->devices())
            if (v.toMap().value(QStringLiteral("serial")).toString() == QLatin1String("DEMO-ALINK-A")) return true;
        return false;
    });
    for (const QVariant &v : link->devices())
        if (v.toMap().value(QStringLiteral("serial")).toString() == QLatin1String("DEMO-ALINK-A"))
            link->bindDevice(v.toMap().value(QStringLiteral("handle")).toInt(), QStringLiteral("DEMO-ALINK-A"));
    const bool bound = settled() && d.waitFor(15000, [&] { return link->phase() == QLatin1String("bound") && device->remote(); }) && d.waitIdle(20000);
    d.check("atqt-spdif: on the hub", bound);
    const auto row = [&](const QString &id) {
        for (const QVariant &v : sys->spdifUnits())
            if (v.toMap().value(QStringLiteral("id")).toString() == id) return v.toMap();
        return QVariantMap();
    };
    const auto fresh = [&] {
        d.spin(1100);
        sys->refresh();
        return d.waitFor(10000, [&] { return !sys->view().value(QLatin1String("stale")).toBool() && !sys->busy(); });
    };
    const bool read = bound && d.waitFor(20000, [&] { return !row(QStringLiteral("S")).isEmpty() && !row(QStringLiteral("SR")).isEmpty(); })
                      && d.waitFor(30000, [&] {
                             fresh();
                             return row(QStringLiteral("SR")).value(QStringLiteral("noTimeBase")).toBool();
                         });
    const QVariantMap s = row(QStringLiteral("S")), sr = row(QStringLiteral("SR"));
    std::printf("smoke: atqt-spdif: S %s %s; SR %s %s\n", qPrintable(s.value(QStringLiteral("slotsWord")).toString()),
                qPrintable(s.value(QStringLiteral("stateWord")).toString()), qPrintable(sr.value(QStringLiteral("stateWord")).toString()),
                qPrintable(sr.value(QStringLiteral("sourceName")).toString()));
    d.check("atqt-spdif: the source and its time-base slots; the follower, its source and its lock error",
            read && s.value(QStringLiteral("role")).toString() == QLatin1String("source") && s.value(QStringLiteral("txMask")).toInt() == 8
                && s.value(QStringLiteral("slotsWord")).toString() == QLatin1String("Outputs 7-8")
                && sr.value(QStringLiteral("role")).toString() == QLatin1String("follower") && sr.value(QStringLiteral("source")).toString() == QLatin1String("S")
                && sr.value(QStringLiteral("lockError")).toBool());
    shell->trigger(QStringLiteral("window.system"));
    QQuickWindow *window = nullptr;
    d.waitFor(3000, [&] {
        window = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("window.system")));
        return window && window->isVisible();
    });
    const auto in = [&](const QString &name) { return window ? d.item(name, window->contentItem()) : nullptr; };
    d.spin(400);
    QQuickItem *notice = in(QStringLiteral("SystemWindow.spdifNoTimeBase.SR"));
    d.check("atqt-spdif: the follower says \"This S/PDIF source does not carry a DSPi time base\" (spdif_no_time_base)",
            notice && notice->isVisible() && notice->property("text").toString().startsWith(QLatin1String("This S/PDIF source does not carry a DSPi time base")));
    QQuickItem *source = in(QStringLiteral("SystemWindow.spdifSource.SR"));
    d.check("atqt-spdif: Admin chooses the follower's source among the sources", source && source->isVisible()
                                                                                    && source->property("currentText").toString() == QLatin1String("Follows S/PDIF source"));
    QQuickItem *alinkRow = in(QStringLiteral("SystemWindow.unit.SR"));
    QQuickItem *sourceRow = in(QStringLiteral("SystemWindow.unit.S"));
    std::printf("smoke: atqt-spdif: %s\n", qPrintable(sys->summary()));
    d.check("atqt-spdif: units fed by S/PDIF are not in the audio link's table nor counted in its summary",
            (!alinkRow || !alinkRow->isVisible()) && (!sourceRow || !sourceRow->isVisible())
                // -- review AT-D2 -- 7: the demo's four and WP ATDEMO's three time-base units, now in the
                // model; the two S/PDIF units are not among them (9 if they were)
                && sys->summary().startsWith(QLatin1String("Audio link: 7 of 7 locked")));
    if (window) {
        window->resize(880, 1300);
        d.spin(300);
        d.grab(window, QStringLiteral("atqt/system-spdif.png"));
    }
    // The hub validates the model: its speaker's slot never carries the time base
    const bool refusedEdit = sys->setSpdifSlot(QStringLiteral("S"), 0, true) && sys->dirty();
    sys->save();
    const bool refused = refusedEdit && d.waitFor(15000, [&] { return !sys->busy(); }) && sys->actionFailed()
                         && sys->actionMessage().contains(QLatin1String("never a speaker"));
    std::printf("smoke: atqt-spdif: refused: %s\n", qPrintable(sys->actionMessage()));
    d.check("atqt-spdif: the hub refuses a speaker's slot for the time base, in its words", refused);
    sys->reload();
    d.waitFor(10000, [&] { return !sys->busy() && !sys->dirty(); });
    // A source's slots and its D, saved with the draft, then put back
    const int revision = sys->revision();
    const bool edited = sys->setSpdifSlot(QStringLiteral("S"), 2, true) && sys->setSpdifD(QStringLiteral("S"), QStringLiteral("200"))
                        && sys->dirty() && !sys->spdifDrafted().isEmpty();
    sys->save();
    // The view is read again until it shows the hub's model (a read within a
    // second of the last is the last view again)
    const auto until = [&](const std::function<bool()> &ok) {
        return d.waitFor(20000, [&] {
            sys->refresh();
            return ok();
        });
    };
    const bool saved = edited && d.waitFor(15000, [&] { return !sys->busy() && !sys->dirty(); })
                       && until([&] { return row(QStringLiteral("S")).value(QStringLiteral("txMask")).toInt() == 12; });
    std::printf("smoke: atqt-spdif: saved: %s; S mask %d, D %s, %s; revision %d -> %d\n", qPrintable(sys->actionMessage()),
                row(QStringLiteral("S")).value(QStringLiteral("txMask")).toInt(), qPrintable(row(QStringLiteral("S")).value(QStringLiteral("d")).toString()),
                qPrintable(row(QStringLiteral("S")).value(QStringLiteral("slotsWord")).toString()), revision, sys->revision());
    d.check("atqt-spdif: a source's time-base slots and its D are saved on the hub",
            saved && sys->revision() > revision && row(QStringLiteral("S")).value(QStringLiteral("d")).toInt() == 200
                && row(QStringLiteral("S")).value(QStringLiteral("slotsWord")).toString() == QLatin1String("Outputs 5-6, 7-8"));
    sys->setSpdifSlot(QStringLiteral("S"), 2, false);
    sys->setSpdifD(QStringLiteral("S"), QString());
    sys->save();
    const bool back = d.waitFor(15000, [&] { return !sys->busy() && !sys->dirty(); }) && until([&] {
                          return row(QStringLiteral("S")).value(QStringLiteral("txMask")).toInt() == 8 && row(QStringLiteral("S")).value(QStringLiteral("d")).isNull();
                      });
    d.check("atqt-spdif: and put back (D the hub's again)", back);
    // An empty slot mask and a follower of a follower are not taken
    d.check("atqt-spdif: a source keeps one time-base slot at least; a follower's source must be a source",
            !sys->setSpdifSlot(QStringLiteral("S"), 3, false) && !sys->setSpdifSource(QStringLiteral("SR"), QStringLiteral("SR")) && !sys->dirty());
    if (window) window->close();
    link->closeSession();
    d.waitFor(10000, [&] { return !link->busy() && !link->inSession(); });
    hub.stop();
    if (demoBefore && !device->connected()) device->openDemo();
    d.waitFor(10000, [&] { return !demoBefore || device->connected(); });
    modes->choose(modeBefore, false);
}

static const SmokeDriver::Section atqtUnits("atqt-units", 216, &unitsSection);
static const SmokeDriver::Section atqtSpdif("atqt-spdif", 217, &spdifSection);

// -- WP ATQTSIM -- A whole measurement across units on the demo hub's simulated
// microphone (WP ATDEMO: the hub's room hears its demo units' generators; nothing
// plays anywhere, no microphone is opened here), as the web's ATDEMO e2e:
// DEMO-TB-A and DEMO-TB-B measured to a result (the room has DEMO-TB-A's first
// speaker 2.37 ms nearer), the summed check, Control offered no Apply, Admin's
// apply with the hub's guard counted down, a verify that passes, Revert as
// filed; then applied again, DEMO-TB-B's delay moved by 5 samples behind the
// session's back, the verify fails and Keep asks twice. Every core call is the
// real one. Section `atqtsim-units` (218); screenshots in <out>/atqtsim/.
namespace {

// What `f` reads or writes on unit `serial`, through a Link client of the
// smoke's own (another person's console on the same hub)
bool onUnit(const QString &url, const QString &serial, const std::function<bool(FfiCore *)> &f) {
    FfiCore *c = dspi_core_new();
    if (!c) return false;
    const QByteArray u = url.toUtf8(), s = serial.toUtf8();
    bool ok = isOkJson(cjson([&](char *b, uint32_t l) { return dspi_link_connect(c, u.constData(), "atqtsim-smoke", "0", "", "", b, l); }));
    int handle = -1;
    for (int i = 0; ok && handle < 0 && i < 50; ++i) {
        for (const QJsonValue &d : cjson([&](char *b, uint32_t l) { return dspi_link_devices(c, b, l); }).value(QLatin1String("value")).toArray())
            if (d.toObject().value(QLatin1String("serial")).toString() == serial) handle = d.toObject().value(QLatin1String("handle")).toInt(-1);
        if (handle < 0) QThread::msleep(100);
    }
    ok = ok && handle >= 0
         && isOkJson(cjson([&](char *b, uint32_t l) { return dspi_link_select_device(c, uint32_t(handle), s.constData(), b, l); }))
         && dspi_fetch_all(c) && f(c);
    dspi_link_close(c);
    dspi_core_free(c);
    return ok;
}

// Output 1's delay on `serial`, in ms (NaN when it cannot be read)
double firstDelayMs(const QString &url, const QString &serial) {
    double ms = std::nan("");
    onUnit(url, serial, [&](FfiCore *c) {
        const DspState *st = dspi_get_state(c);
        if (st) ms = double(st->output_delay_ms[0]);
        return st != nullptr;
    });
    return ms;
}

} // namespace

static void simSection(SmokeDriver &d) {
    // Four runs on the simulated microphone (77 s on a quiet host): a loaded host may need more than the run's 120 s
    d.allowSectionTime(300);
    auto *link = Registrar::instance<LinkController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *uc = Registrar::instance<UnitsMeasureController>();
    auto *modes = Registrar::instance<ModeController>();
    DeviceModel *device = d.app().device;
    d.check("atqtsim: the controllers", link && mc && uc && modes);
    if (!link || !mc || !uc || !modes) return;
    const QString out = d.outDir() + QStringLiteral("/atqtsim");
    QDir().mkpath(out);
    const bool demoBefore = device->demo() && device->connected();
    const QString modeBefore = modes->mode();
    modes->choose(QStringLiteral("advanced"), false);
    QStringList steps;
    QHash<QString, bool> results;
    const QMetaObject::Connection conn = QObject::connect(uc, &UnitsMeasureController::stepDone, uc, [&](const QString &s, bool ok) {
        steps << s;
        results.insert(s, ok);
    });
    // Every core call is the real one; recorded to see what was sent
    Record rec;
    uc->setCallsForTest([&rec](FfiCore *c, const QString &op, const QByteArray &a, const QByteArray &b) {
        if (op == QLatin1String("unitCheck")) rec.add(QStringLiteral("unitCheck ") + QJsonDocument::fromJson(b).object().value(QLatin1String("serial")).toString());
        else rec.add(op);
        return UnitsMeasureController::coreCall(c, op, a, b);
    });
    const ScopeExit cleanup{ [uc, conn] {
        QObject::disconnect(conn);
        uc->setCallsForTest(nullptr);
        uc->setRoleForTest(QString());
    } };
    const auto step = [&](const QString &s, int ms) { return d.waitFor(ms, [&] { return steps.contains(s); }); };
    QQuickWindow *w = nullptr;
    const auto in = [&](const QString &name) { return w ? d.item(name, w->contentItem()) : nullptr; };
    const auto visible = [&](const QString &name) {
        QQuickItem *i = in(name);
        return i && i->isVisible();
    };
    const auto toEnd = [&] {
        QQuickItem *sv = in(QStringLiteral("MeasureUnits.scroll"));
        auto *flick = sv ? sv->property("contentItem").value<QQuickItem *>() : nullptr;
        if (!flick) return;
        d.spin(100);
        flick->setProperty("contentY", std::max(0.0, flick->property("contentHeight").toDouble() - flick->height()));
        d.spin(150);
    };

    MeasureHub hub;
    const bool up = hub.start(out + QStringLiteral("/hub"));
    d.check("atqtsim: uconsole --demo hub --alink --with-measure-units runs (DSPI_SMOKE_UCONSOLE)", up);
    if (!up) return;
    bool lowered = true;
    for (const QString &s : { A, B }) lowered = lowerVolume(hub.url, s) && lowered;
    d.check("atqtsim: DEMO-TB-A's and DEMO-TB-B's volume lowered through a Link client of the smoke's own (S2)", lowered);
    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    link->connectToHub(hub.url);
    const bool listed = settled() && d.waitFor(15000, [&] {
        int n = 0;
        for (const QVariant &v : link->devices()) n += QStringList{ A, B }.contains(v.toMap().value(QStringLiteral("serial")).toString());
        return link->phase() == QLatin1String("choose_device") && n == 2;
    });
    for (const QVariant &v : link->devices())
        if (v.toMap().value(QStringLiteral("serial")).toString() == A) link->bindDevice(v.toMap().value(QStringLiteral("handle")).toInt(), A);
    const bool bound = listed && settled()
                       && d.waitFor(15000, [&] { return link->phase() == QLatin1String("bound") && device->remote() && device->connected(); })
                       && d.waitIdle(20000);
    d.check("atqtsim: on the hub, DEMO-TB-A bound", bound && device->serial() == A);
    // A quiet source in the Measure window's place: should anything open "the
    // microphone" it is this, never a device (and the checks below say none is)
    mc->setCaptureSource([]() -> std::unique_ptr<dspi_capture::Source> { return std::make_unique<QuietSource>(); });
    const auto finish = [&] {
        if (uc->running()) uc->stop();
        d.waitFor(15000, [&] { return !uc->running(); });
        uc->setCallsForTest(nullptr);
        uc->setRoleForTest(QString());
        if (w) w->close();
        mc->setCaptureSource(nullptr);
        link->closeSession();
        d.waitFor(10000, [&] { return !link->busy() && !link->inSession(); });
        hub.stop();
        if (demoBefore && !device->connected()) device->openDemo();
        d.waitFor(10000, [&] { return !demoBefore || device->connected(); });
        modes->choose(modeBefore, false);
    };
    bool shown = false;
    if (bound) {
        w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("tools.measure")));
        if (w) {
            d.waitFor(2000, [&] { return w->isVisible(); });
            if (QQuickItem *tabs = in(QStringLiteral("Measure.tabs"))) {
                tabs->setProperty("currentIndex", 1);
                d.spin(100);
                steps.clear();
                tabs->setProperty("currentIndex", 6);
                shown = step(QStringLiteral("enter"), 10000);
            }
        }
    }
    d.check("atqtsim: the Several units tab on the hub", shown && uc->unavailable().isEmpty());
    if (!shown) {
        finish();
        return;
    }

    // ---- The group: the listening position, A and B (each its first output), the simulated microphone
    uc->setPlacement(QStringLiteral("listeningPosition"));
    steps.clear();
    uc->setUnitChosen(A, true);
    uc->setUnitChosen(B, true);
    const bool opened = step(QStringLiteral("unit"), 20000) && d.waitFor(20000, [&] { return uc->unitOpen(B) && uc->blocker().isEmpty(); });
    uc->setSweepMs(1000);
    uc->setPasses(3);
    d.check("atqtsim: DEMO-TB-A and DEMO-TB-B chosen, each with its own connection; nothing blocks", opened);
    // Chosen again: the choice stays with the controller, and atqt-units turns it off
    uc->setUseSim(true);
    d.check("atqtsim: the demo hub's simulated microphone is offered and chosen", uc->simAvailable() && uc->useSim());

    // ---- Measure to a result
    rec.clear();
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.start")));
    const bool started = step(QStringLiteral("start"), 30000) && results.value(QStringLiteral("start"));
    std::printf("smoke: atqtsim: start: %s %s\n", qPrintable(uc->error()), qPrintable(uc->phaseText()));
    d.check("atqtsim: the run starts on the simulated microphone, no microphone opened here; its bar and Stop in view",
            started && uc->simOn() && visible(QStringLiteral("MeasureUnits.simOn")) && visible(QStringLiteral("MeasureUnits.stop")) && !mc->micOpen());
    bool micEver = false;
    QElapsedTimer runT;
    runT.start();
    const bool measured = started && d.waitFor(150000, [&] {
        micEver = micEver || mc->micOpen();
        return steps.contains(QStringLiteral("run"));
    });
    std::printf("smoke: atqtsim: measured in %.1f s: %s; %s (%s); %s\n", runT.elapsed() / 1000.0, qPrintable(uc->timingText()),
                qPrintable(uc->issueText()), qPrintable(uc->issueKey()), qPrintable(uc->error()));
    const bool haveResult = measured && results.value(QStringLiteral("run")) && uc->hasResult();
    d.check("atqtsim: the measurement ran to its result; no microphone was opened", haveResult && !micEver);
    if (!haveResult) {
        // Nothing after it can run
        finish();
        return;
    }
    d.check("atqtsim: the result: the timing of every run was reliable", uc->timingText() == QLatin1String("The timing of every run was reliable."));
    bool bothOffered = uc->arrivals().size() == 2;
    for (const QVariant &v : uc->arrivals()) {
        std::printf("smoke: atqtsim: arrival %s: %s (%s)\n", qPrintable(v.toMap().value(QStringLiteral("label")).toString()),
                    qPrintable(v.toMap().value(QStringLiteral("arrival")).toString()), qPrintable(v.toMap().value(QStringLiteral("offered")).toString()));
        bothOffered = bothOffered && v.toMap().value(QStringLiteral("offered")).toString() == QLatin1String("Yes");
    }
    d.check("atqtsim: both arrivals offered", bothOffered);
    double delayUs = std::nan("");
    if (!uc->delays().isEmpty()) {
        const QString t = uc->delays().at(0).toMap().value(QStringLiteral("delay")).toString();
        std::printf("smoke: atqtsim: delay %s\n", qPrintable(t));
        const QRegularExpressionMatch m = QRegularExpression(QStringLiteral("^(-?[\\d.]+) µs")).match(t);
        if (m.hasMatch()) delayUs = m.captured(1).toDouble();
    }
    d.check("atqtsim: the delay across units is the room's, 2.37 ms within 10 µs", std::isfinite(delayUs) && std::abs(std::abs(delayUs) - 2370.0) < 10.0);
    d.check("atqtsim: the proposal: the arrivals to write, the summed check still to run",
            uc->hasProposal() && uc->proposalArrivals().join(QLatin1Char('\n')).contains(A) && uc->sumCheckNeeded()
                && visible(QStringLiteral("MeasureUnits.sumCheck")));
    d.grab(w, QStringLiteral("atqtsim/result.png"));

    // ---- The summed check
    rec.clear();
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.sumCheck")));
    runT.restart();
    const bool summed = d.waitFor(150000, [&] {
        micEver = micEver || mc->micOpen();
        return steps.contains(QStringLiteral("sumCheck"));
    });
    bool agreed = !uc->transitions().isEmpty();
    for (const QVariant &v : uc->transitions()) {
        std::printf("smoke: atqtsim: transition: %s\n", qPrintable(v.toMap().value(QStringLiteral("text")).toString()));
        agreed = agreed && !v.toMap().value(QStringLiteral("failed")).toBool() && !v.toMap().value(QStringLiteral("text")).toString().contains(QLatin1String("not checked"));
    }
    std::printf("smoke: atqtsim: summed check in %.1f s: %s\n", runT.elapsed() / 1000.0, qPrintable(uc->error()));
    d.check("atqtsim: the summed check ran on the simulated microphone, each unit's fresh check first; every transition agrees",
            summed && results.value(QStringLiteral("sumCheck")) && agreed && !uc->sumCheckNeeded() && rec.count(QStringLiteral("unitCheck ") + A) == 1
                && rec.count(QStringLiteral("unitCheck ") + B) == 1 && rec.count(QStringLiteral("sumCheck")) == 1 && !micEver);

    // ---- Control: no Apply, no Revert, nothing sent
    uc->setRoleForTest(QStringLiteral("control"));
    d.spin(200);
    rec.clear();
    steps.clear();
    uc->apply();
    QQuickItem *why = in(QStringLiteral("MeasureUnits.applyWhy"));
    d.check("atqtsim: Control: no Apply and no Revert, why in words (measure.system.admin); an apply sends nothing",
            !visible(QStringLiteral("MeasureUnits.apply")) && !visible(QStringLiteral("MeasureUnits.revert")) && why && why->isVisible()
                && why->property("key").toString() == QLatin1String("measure.system.admin") && step(QStringLiteral("apply"), 2000)
                && !results.value(QStringLiteral("apply")) && rec.count(QStringLiteral("apply")) == 0);
    d.grab(w, QStringLiteral("atqtsim/control.png"));
    // The hub's own role for this console (it is --open: Admin)
    uc->setRoleForTest(QString());
    d.spin(200);
    QQuickItem *note = in(QStringLiteral("MeasureUnits.guardNote"));
    std::printf("smoke: atqtsim: guard note: %s\n", note ? qPrintable(note->property("text").toString()) : "-");
    d.check("atqtsim: Admin (the hub's role): Apply offered; the guard said before it",
            uc->admin() && visible(QStringLiteral("MeasureUnits.apply")) && note && note->isVisible()
                && QRegularExpression(QStringLiteral("^The hub reverts the alignment by itself unless it is kept within \\d+ s")).match(note->property("text").toString()).hasMatch());

    // ---- Apply, the guard counted down, the units' delays
    const double filedA = firstDelayMs(hub.url, A), filedB = firstDelayMs(hub.url, B);
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.apply")));
    const bool appliedOk = step(QStringLiteral("apply"), 30000) && results.value(QStringLiteral("apply")) && uc->stage() == QLatin1String("applied");
    std::printf("smoke: atqtsim: applied: %s | %s | %s\n", qPrintable(uc->appliedText()), qPrintable(uc->said()), qPrintable(uc->error()));
    const int left0 = uc->guardRemainingS();
    d.spin(2200);
    QQuickItem *guard = in(QStringLiteral("MeasureUnits.guard"));
    std::printf("smoke: atqtsim: guard: %s\n", qPrintable(uc->guardText()));
    d.check("atqtsim: Admin's apply: applied, the hub's guard counting down in words",
            appliedOk && uc->appliedText().startsWith(QLatin1String("Applied: the summed runs are measured again")) && uc->said().startsWith(QLatin1String("Applied: "))
                && left0 > 0 && guard
                && guard->isVisible() && uc->guardRemainingS() <= left0 - 2
                && QRegularExpression(QStringLiteral("Keep or Revert within \\d+ s: without Keep the hub puts everything back by itself")).match(uc->guardText()).hasMatch());
    const double a1 = firstDelayMs(hub.url, A), b1 = firstDelayMs(hub.url, B);
    std::printf("smoke: atqtsim: delays filed %.4f %.4f, applied %.4f %.4f ms\n", filedA, filedB, a1, b1);
    d.check("atqtsim: on the units: DEMO-TB-A delayed 2.37 ms against DEMO-TB-B", std::abs(a1 - b1 - 2.37) < 0.02);
    toEnd();
    d.grab(w, QStringLiteral("atqtsim/applied.png"));

    // ---- The verify passes
    rec.clear();
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.verify")));
    runT.restart();
    const bool verified = d.waitFor(150000, [&] {
        micEver = micEver || mc->micOpen();
        return steps.contains(QStringLiteral("verify"));
    });
    std::printf("smoke: atqtsim: verify in %.1f s: %s | %s\n", runT.elapsed() / 1000.0, qPrintable(uc->appliedText()), qPrintable(uc->error()));
    d.check("atqtsim: the verify ran, each unit's fresh check first, and passed",
            verified && results.value(QStringLiteral("verify")) && uc->stage() == QLatin1String("verified") && !uc->verifyFailed()
                && uc->appliedText() == QLatin1String("Verified: the summed runs agree with the new delays.") && rec.count(QStringLiteral("unitCheck ") + A) == 1
                && rec.count(QStringLiteral("unitCheck ") + B) == 1 && rec.count(QStringLiteral("verify")) == 1 && !micEver);
    toEnd();
    d.grab(w, QStringLiteral("atqtsim/verified.png"));

    // ---- Revert: as filed
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.revert")));
    const bool reverted = step(QStringLiteral("revert"), 30000) && results.value(QStringLiteral("revert"));
    QQuickItem *outcome = in(QStringLiteral("MeasureUnits.outcome"));
    const double a2 = firstDelayMs(hub.url, A), b2 = firstDelayMs(hub.url, B);
    std::printf("smoke: atqtsim: reverted: %s; delays %.4f %.4f ms\n", outcome ? qPrintable(outcome->property("text").toString()) : "-", a2, b2);
    d.check("atqtsim: Revert: reverted, said in words, the guard gone",
            reverted && uc->stage() == QLatin1String("reverted") && outcome && outcome->isVisible()
                && outcome->property("text").toString().startsWith(QLatin1String("Reverted")) && uc->guardText().isEmpty() && !visible(QStringLiteral("MeasureUnits.guard")));
    d.check("atqtsim: Revert: every unit's delay as filed", std::abs(a2 - filedA) < 0.0015 && std::abs(b2 - filedB) < 0.0015);

    // ---- Applied again; DEMO-TB-B's delay moved by 5 samples behind the session's back: the verify fails
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.apply")));
    const bool again = step(QStringLiteral("apply"), 30000) && results.value(QStringLiteral("apply")) && uc->stage() == QLatin1String("applied");
    d.check("atqtsim: applied again after the Revert", again);
    const double b3 = firstDelayMs(hub.url, B);
    const bool moved = std::isfinite(b3) && onUnit(hub.url, B, [b3](FfiCore *c) { return dspi_set_output_delay(c, 0, float(b3 + 5.0 / 48.0)); });
    d.check("atqtsim: DEMO-TB-B's delay moved by 5 samples through another console", moved && std::abs(firstDelayMs(hub.url, B) - b3 - 5.0 / 48.0) < 0.005);
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.verify")));
    runT.restart();
    const bool verified2 = d.waitFor(150000, [&] {
        micEver = micEver || mc->micOpen();
        return steps.contains(QStringLiteral("verify"));
    });
    std::printf("smoke: atqtsim: verify after the move in %.1f s: %s | %s\n", runT.elapsed() / 1000.0, qPrintable(uc->appliedText()), qPrintable(uc->error()));
    d.check("atqtsim: the verify after the move did not confirm the alignment, said in words",
            verified2 && uc->verifyFailed() && uc->appliedText().contains(QLatin1String("did not confirm")) && !micEver);
    // The core's words already say it: the notice's title is not put before them again
    d.check("atqtsim: the failed verify's words say \"did not confirm\" once", uc->appliedText().count(QLatin1String("did not confirm"), Qt::CaseInsensitive) == 1);

    // ---- Keep asks twice: refused once with the core's words; only a press a second later keeps
    rec.clear();
    steps.clear();
    click(in(QStringLiteral("MeasureUnits.keep")));
    step(QStringLiteral("keep"), 15000);
    QQuickItem *question = in(QStringLiteral("MeasureUnits.keepQuestion"));
    QQuickItem *anyway = in(QStringLiteral("MeasureUnits.keepAnyway"));
    std::printf("smoke: atqtsim: keep question: %s\n", qPrintable(uc->keepQuestionText()));
    d.check("atqtsim: the first Keep is refused by the core: the second question opens, with its words",
            !results.value(QStringLiteral("keep")) && uc->keepQuestion() && uc->keepRefusals() >= 1 && question
                && d.waitFor(1000, [&] { return question->isVisible(); }) && uc->keepQuestionText().startsWith(QLatin1String("The summed run differs")) && uc->stage() == QLatin1String("verified")
                && rec.count(QStringLiteral("keep")) == 1);
    d.check("atqtsim: \"Keep anyway\" is not yet enabled", anyway && !anyway->isEnabled() && anyway->property("text").toString().contains(QLatin1String("(in ")));
    toEnd();
    d.grab(w, QStringLiteral("atqtsim/keep-question.png"));
    const bool ready = anyway && d.waitFor(5000, [&] { return anyway->isEnabled(); });
    steps.clear();
    click(anyway);
    const bool kept = ready && step(QStringLiteral("keep"), 15000) && results.value(QStringLiteral("keep"));
    std::printf("smoke: atqtsim: kept: %s\n", outcome ? qPrintable(outcome->property("text").toString()) : "-");
    d.check("atqtsim: the person's second press, a second later, keeps it: Kept",
            kept && uc->stage() == QLatin1String("kept") && rec.count(QStringLiteral("keep")) == 2 && outcome && outcome->isVisible()
                && outcome->property("text").toString() == QLatin1String("Kept.") && uc->guardText().isEmpty());
    toEnd();
    d.grab(w, QStringLiteral("atqtsim/kept.png"));
    finish();
}

static const SmokeDriver::Section atqtsimUnits("atqtsim-units", 218, &simSection);
