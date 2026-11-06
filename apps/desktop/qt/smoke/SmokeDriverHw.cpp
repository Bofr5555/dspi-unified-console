// The hardware pass of the smoke driver (DSPI_SMOKE_HW=1, see run-hw.sh): what
// the demo device does not model, on a real DSPi.
//
// It must leave the device as it found it. It reads the bulk image first and
// compares at the end, and the only things it changes are global bypass (RAM,
// restored and checked) and an armed IR learn (cancelled). Never here: a preset
// save or load, a parameter save, a factory reset, a firmware action, a binding
// write, or anything that stages or starts the signal generator: that makes
// sound. DSPI_SMOKE_HW=dry runs the same pass on the demo device.

#include "SmokeDriver.h"

#include "CsController.h"
#include "DSPiBridge.h"
#include "PeqEditorItem.h"
#include "RtaController.h"
#include "SiggenController.h"
#include "StatsController.h"

#include <QCoreApplication>
#include <QImage>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QVariant>
#include <cmath>
#include <cstdio>
#include <cstring>

static const char *const kDemoSerial = "DEMO0000DSPI0001";

// ── Reporting ──

void SmokeDriver::pass(const QString &item, const QString &detail) {
    m_passed++;
    printf("smoke: PASS %s%s%s\n", qPrintable(item), detail.isEmpty() ? "" : ": ", qPrintable(detail));
    fflush(stdout);
}

void SmokeDriver::fail(const QString &item, const QString &detail) {
    m_failures++;
    printf("smoke: FAIL %s%s%s\n", qPrintable(item), detail.isEmpty() ? "" : ": ", qPrintable(detail));
    fflush(stdout);
}

void SmokeDriver::notRun(const QString &item, const QString &why) {
    m_notRun++;
    printf("smoke: NOT RUN %s: %s\n", qPrintable(item), qPrintable(why));
    fflush(stdout);
}

void SmokeDriver::outcome(const QString &item, bool ok, const QString &detail) {
    if (ok) pass(item, detail);
    else fail(item, detail);
}

// A read the demo device may not model: in a dry run its failure says so, and
// only real hardware can fail it.
void SmokeDriver::reading(const QString &item, bool ok, const QString &detail) {
    if (!ok && qEnvironmentVariable("DSPI_SMOKE_HW") == QLatin1String("dry")) notRun(item, "the demo device does not model it");
    else outcome(item, ok, detail);
}

static QString json(const QVariantMap &m) {
    return QString::fromUtf8(QJsonDocument(QJsonObject::fromVariantMap(m)).toJson(QJsonDocument::Compact)).left(300);
}

// A core call that fills a text buffer with JSON
template <typename F>
static QVariantMap coreJson(F call) {
    QByteArray buf(16 * 1024, '\0');
    const uint32_t needed = call(buf.data(), uint32_t(buf.size()));
    if (needed == 0 || needed > uint32_t(buf.size())) return {};
    buf.truncate(int(needed - 1));
    return QJsonDocument::fromJson(buf).object().toVariantMap();
}

// ── The pass ──

void SmokeDriver::finish() {
    m_failures += qmlErrors();
    printf("smoke: %d checks, %d failures (%d QML errors), %d not run\n", m_passed + m_failures, m_failures, qmlErrors(), m_notRun);
    fflush(stdout);
    QCoreApplication::exit(m_failures ? 1 : 0);
}

void SmokeDriver::runHardware() {
    const bool dry = qEnvironmentVariable("DSPI_SMOKE_HW") == QLatin1String("dry");
    printf("smoke: hardware pass%s\n", dry ? " (dry run on the demo device)" : "");
    if (!hwPreflight()) { finish(); return; }

    Baseline start;
    if (!readBaseline(&start)) {
        fail("baseline", "the device's image could not be read; nothing was changed");
        finish();
        return;
    }
    pass("baseline", QStringLiteral("%1 image bytes read from the device").arg(start.image.size()));

    // No view of the app's own may hold the analyser: the Dashboard's overlay is on
    // by default. These are settings in this run's throwaway home.
    root()->setProperty("rtaDashboardShowGraph", false);
    root()->setProperty("rtaDashboardShowBars", false);
    root()->setProperty("rtaChannelShowGraph", false);
    root()->setProperty("rtaChannelShowBars", false);
    spin(400);

    section("spectrum analyser");
    hwRta();
    section("control surfaces");
    hwControlSurfaces();
    section("stats, watermarks, clock diagnostics");
    hwStats();
    section("I/O status");
    hwIoStatus();
    section("meters");
    hwMeters();
    section("signal generator: caps only");
    hwSiggenCaps();
    section("bypass");
    hwBypass();
    compareBaseline(start, "after the bypass toggle");
    section("disconnect, reselect");
    hwReselect(start);

    section("the device as found");
    spin(300);
    SiggenStatus sg {};
    if (m_.siggen->supported())
        outcome("signal generator still idle", dspi_siggen_get_status(m_.bridge->core(), &sg) && sg.state == 0);
    outcome("no connection error", m_.bridge->connectionError().isEmpty(), m_.bridge->connectionError());
    outcome("final image identical to the first", compareBaseline(start, "at the end"));
    finish();
}

bool SmokeDriver::hwPreflight() {
    section("preflight");
    auto *b = m_.bridge;
    if (!waitFor(3000, [b]() { return b->connected(); })) {
        fail("device", QStringLiteral("none connected; inaccessible: %1; %2").arg(b->inaccessibleDevices()).arg(b->connectionError()));
        return false;
    }
    if (b->availableSerials().size() != 1) {
        fail("one device", QStringLiteral("%1 are visible; give the container exactly one").arg(b->availableSerials().size()));
        return false;
    }
    const bool dry = qEnvironmentVariable("DSPI_SMOKE_HW") == QLatin1String("dry");
    if (!dry && b->selectedSerial() == QLatin1String(kDemoSerial)) {
        fail("real device", "DSPI_SMOKE_HW=1 is on the demo device: run without --demo");
        return false;
    }
    pass("device", QStringLiteral("%1, %2, firmware %3").arg(b->selectedSerial(), b->platformName(), b->firmwareVersion()));
    if (b->compat() != COMPAT_OK) {
        fail("writable", "read-only connection: " + b->compatMessage());
        return false;
    }
    if (m_.siggen->running()) {
        fail("signal generator idle", "it is running already; stop it first, this pass will not touch it");
        return false;
    }
    return true;
}

bool SmokeDriver::readBaseline(Baseline *out) {
    if (!dspi_fetch_all(m_.bridge->core())) return false;
    emit m_.bridge->stateChanged();
    const DspState *s = dspi_get_state(m_.bridge->core());
    if (!s || !s->bulk_valid) return false;
    out->image = QByteArray(reinterpret_cast<const char *>(s->bulk_raw), int(sizeof(s->bulk_raw)));
    QByteArray extra;
    const auto add = [&extra](const void *p, size_t n) { extra.append(static_cast<const char *>(p), int(n)); };
    add(&s->preset_occupied, sizeof(s->preset_occupied));
    add(s->preset_names, sizeof(s->preset_names));
    add(&s->active_preset_slot, sizeof(s->active_preset_slot));
    add(&s->preset_startup_mode, sizeof(s->preset_startup_mode));
    add(&s->preset_default_slot, sizeof(s->preset_default_slot));
    add(&s->core1_mode, sizeof(s->core1_mode));
    add(&s->subharm_solo, sizeof(s->subharm_solo));
    out->perItem = extra;
    return true;
}

bool SmokeDriver::compareBaseline(const Baseline &want, const char *when) {
    Baseline now;
    if (!readBaseline(&now)) {
        fail(QStringLiteral("device state %1").arg(QLatin1String(when)), "could not be re-read");
        return false;
    }
    if (now.image == want.image && now.perItem == want.perItem) {
        printf("smoke: device state %s: identical\n", when);
        fflush(stdout);
        return true;
    }
    int differing = 0;
    QString list;
    for (int i = 0; i < qMin(now.image.size(), want.image.size()); i++) {
        if (now.image[i] == want.image[i]) continue;
        if (++differing <= 32)
            list += QStringLiteral(" %1:%2>%3").arg(i).arg(quint8(want.image[i]), 2, 16, QLatin1Char('0')).arg(quint8(now.image[i]), 2, 16, QLatin1Char('0'));
    }
    printf("smoke: FAIL ******** THE DEVICE'S STATE CHANGED %s ********\n", when);
    printf("smoke: FAIL %d image bytes differ (offset:was>now):%s\n", differing, qPrintable(list));
    if (now.perItem != want.perItem) printf("smoke: FAIL the preset directory, Core 1 mode or solo differ\n");
    fflush(stdout);
    m_failures++;
    return false;
}

// ── Spectrum analyser ──

void SmokeDriver::hwRtaTap(int tap, const char *name) {
    auto *rta = m_.rta;
    const QString item = QStringLiteral("RTA frames, %1 tap").arg(QLatin1String(name));
    int mask = 0;
    for (const QVariant &c : rta->availableChannels(tap)) {
        const int index = rta->tapIndex(c.toInt());
        if (index >= 0 && index < 16 && qPopulationCount(uint(mask)) < 2) mask |= 1 << index;
    }
    if (!mask) { notRun(item, "no live channel at this tap"); return; }
    int frames = 0;
    QMetaObject::Connection c = QObject::connect(rta, &RtaController::frameChanged, this, [&frames]() { frames++; });
    const int id = rta->subscribe(tap, mask, true);
    const bool running = waitFor(4000, [rta]() { return rta->running(); });
    const bool got = waitFor(4000, [&frames]() { return frames >= 3; });
    const RtaSnapshot &s = rta->snapshot();
    outcome(item, running && got && s.tap == tap && s.count > 0,
            QString::fromLatin1(got ? "" : "is a stream playing? digital silence changes no picture; ") + QStringLiteral("mask 0x%1, %2 frames, %3 channels, %4 frames/s, %5 µs a transform, order %6")
                .arg(mask, 0, 16).arg(frames).arg(int(s.count)).arg(rta->framesPerSecond()).arg(rta->transformUs()).arg(rta->appliedOrder()));
    QObject::disconnect(c);
    rta->release(id);
    spin(200);
}

void SmokeDriver::hwRta() {
    auto *rta = m_.rta;
    if (!rta->supported()) {
        notRun("RTA", "the device reports no spectrum analyser");
        return;
    }
    pass("RTA caps", QStringLiteral("%1 inputs, %2 outputs, order %3..%4, %5 dB")
                         .arg(rta->inputChannels()).arg(rta->outputChannels()).arg(rta->orderMin()).arg(rta->orderMax()).arg(rta->dynamicRange()));
    if (!waitFor(3000, [rta]() { return !rta->running(); })) {
        notRun("RTA frames and stop", "a view holds the analyser with nothing subscribed by this pass");
        return;
    }
    hwRtaTap(RTA_TAP_INPUT, "input");
    hwRtaTap(RTA_TAP_OUTPUT, "output");

    // Configuration: the device applies a new order, and frames keep coming
    const int order = rta->fftOrder(), avg = rta->avgMs(), decay = rta->peakDecay();
    int mask = 0;
    const QVariantList live = rta->availableChannels(RTA_TAP_INPUT);
    if (!live.isEmpty()) mask = 1 << qMax(0, rta->tapIndex(live.first().toInt()));
    if (!mask) {
        notRun("RTA configuration", "no live input");
    } else {
        int frames = 0;
        QMetaObject::Connection c = QObject::connect(rta, &RtaController::frameChanged, this, [&frames]() { frames++; });
        const int id = rta->subscribe(RTA_TAP_INPUT, mask, true);
        waitFor(4000, [rta]() { return rta->running(); });
        const int other = order == rta->orderMin() ? rta->orderMin() + 1 : rta->orderMin();
        rta->setFftOrder(other);
        const bool applied = waitFor(4000, [rta, other]() { return rta->appliedOrder() == other; });
        rta->setAvgMs(avg == 100 ? 200 : 100);
        rta->setPeakDecay(decay == 6 ? 12 : 6);
        frames = 0;
        const bool more = waitFor(4000, [&frames]() { return frames >= 3; });
        outcome("RTA configuration", applied && more && !rta->configRejected(),
                QStringLiteral("order %1 -> applied %2; rejected: %3; frames after: %4")
                    .arg(other).arg(rta->appliedOrder()).arg(rta->configRejected() ? "yes" : "no").arg(frames));
        rta->setFftOrder(order);
        rta->setAvgMs(avg);
        rta->setPeakDecay(decay);

        // Closing: the last subscriber gone, the worker stops the device and polls no more
        rta->release(id);
        const bool stopped = waitFor(4000, [rta]() { return !rta->running(); });
        spin(500);
        frames = 0;
        spin(1500);
        outcome("RTA closing stops traffic", stopped && frames == 0,
                QStringLiteral("running: %1; frames in the 1.5 s after: %2").arg(rta->running() ? "yes" : "no").arg(frames));
        QObject::disconnect(c);
    }
}

// ── Control surfaces ──

// What a write would change: the configuration, not the live status or learn state
static QByteArray csConfiguration(const QVariantMap &model) {
    QJsonObject o;
    for (const char *k : { "bindings", "names", "ir", "groups", "macros", "display" })
        o[QLatin1String(k)] = QJsonValue::fromVariant(model.value(QLatin1String(k)));
    return QJsonDocument(o).toJson(QJsonDocument::Compact);
}

void SmokeDriver::hwControlSurfaces() {
    auto *cs = m_.cs;
    cs->refresh();
    if (!cs->supported()) {
        notRun("control surfaces", "the device reports none, or the read failed");
        return;
    }
    const QVariantMap model = cs->model();
    const QByteArray config = csConfiguration(model);
    const bool dirty = cs->unsaved();
    outcome("control surfaces: model", model.contains("bindings") && model.contains("status"),
            QStringLiteral("%1 bindings, caps version %2, unsaved: %3")
                .arg(model.value("bindings").toList().size()).arg(model.value("capsVersion").toInt()).arg(dirty ? "yes" : "no"));
    const QVariantMap status = cs->apply({ { "op", "refreshStatus" } });
    outcome("control surfaces: status", status.value("ok").toBool(), json(cs->model().value("status").toMap()));

    const QVariantMap arm = cs->apply({ { "op", "learnArm" } });
    if (arm.value("ok").toBool()) {
        pass("control surfaces: arm IR learn");
        spin(200);
        const QVariantMap cancel = cs->apply({ { "op", "learnCancel" } });
        outcome("control surfaces: cancel IR learn", cancel.value("ok").toBool(), json(cancel));
    } else {
        // A device without a receiver refuses; there is nothing to cancel
        const int st = arm.value("status", 0xFF).toInt();
        notRun("control surfaces: arm and cancel IR learn", QStringLiteral("refused, status 0x%1: %2").arg(st, 2, 16, QLatin1Char('0')).arg(cs->statusMessage(st)));
    }

    cs->refresh();
    outcome("control surfaces: configuration unchanged", csConfiguration(cs->model()) == config && cs->unsaved() == dirty);
}

// ── Stats ──

void SmokeDriver::hwStats() {
    auto *st = m_.stats;
    auto *core = m_.bridge->core();
    st->setWatching(true);
    reading("stats: info", waitFor(3000, [st]() { return !st->info().isEmpty(); }), json(st->info()));
    uint32_t total = 0;
    reading("stats: starvation counter", dspi_fetch_stat(core, STAT_STARVATION_TOTAL, &total), QString::number(total));
    if (!waitFor(3000, [st]() { return !st->buffers().isEmpty(); })) {
        reading("buffer watermarks", false, "no reading in 3 s");
    } else {
        spin(500);   // a few fast polls
        const QVariantMap b = st->buffers();
        const QVariantList rows = b.value("spdif").toList();
        bool ordered = true;
        for (const QVariant &r : rows) ordered = ordered && r.toMap().value("min").toInt() <= r.toMap().value("max").toInt();
        if (rows.isEmpty()) notRun("buffer watermarks", "no S/PDIF output reports a buffer");
        else reading("buffer watermarks", ordered, json(b));
    }
    notRun("buffer watermark reset", "it can't be undone, and this pass leaves the device as found");
    st->setWatching(false);

    const QVariantMap diag = coreJson([core](char *b, uint32_t n) { return dspi_fetch_clock_diag(core, b, n); });
    if (diag.value("ok").toBool() && !diag.value("available").toBool())
        notRun("clock diagnostics", "not built into this firmware (DSPI_CLOCK_DIAG)");
    else
        reading("clock diagnostics", diag.value("ok").toBool(), json(diag));
}

// ── I/O status ──

void SmokeDriver::hwIoStatus() {
    auto *b = m_.bridge;
    auto *core = b->core();
    const bool rp2350 = b->platformName() == QLatin1String("RP2350");
    SystemStatus status {};
    bool ok = true;
    for (int i = 0; i < 5; i++) { ok = ok && dspi_fetch_status(core, &status); spin(20); }
    reading("system status", ok && status.cpu0 <= 100 && status.cpu1 <= 100,
            QStringLiteral("CPU %1/%2 %, %3 active inputs").arg(int(status.cpu0)).arg(int(status.cpu1)).arg(int(status.active_input_channels)));
    SpdifRxStatus rx {};
    reading("S/PDIF input status", dspi_fetch_spdif_rx_status(core, &rx), QStringLiteral("state %1").arg(int(rx.state)));
    const QVariantMap i2s = b->fetchInputLock(0);
    reading("I2S slave status", !i2s.isEmpty(), json(i2s));
    if (rp2350) {
        const QVariantMap adat = b->fetchInputLock(1);
        reading("ADAT input status", !adat.isEmpty(), json(adat));
        const QVariantMap adatOut = b->fetchAdatOutStatus();
        reading("ADAT output status", !adatOut.isEmpty(), json(adatOut));
        const QVariantMap upmix = b->fetchUpmixStatus();
        reading("upmixer status", !upmix.isEmpty(), json(upmix));
    } else {
        notRun("ADAT and upmixer status", "RP2350 only");
    }
    const QVariantMap lg = b->fetchLgStatus();
    reading("LG Sound Sync status", !lg.isEmpty(), json(lg));
    const int rate = b->fetchInputRate();
    reading("input rate", rate >= 0, QString::number(rate));
    const QVariantMap latency = coreJson([core](char *o, uint32_t n) { return dspi_fetch_output_latency(core, o, n); });
    reading("output latency", latency.value("ok").toBool(), json(latency));
    const QVariantMap ext = coreJson([core](char *o, uint32_t n) { return dspi_ext_status(core, o, n); });
    reading("extension channel", ext.value("ok").toBool(), json(ext));
}

// ── Meters ──

void SmokeDriver::hwMeters() {
    auto *b = m_.bridge;
    const QVariantList sub = b->fetchSubharmMeter();
    reading("subharmonic meter", !sub.isEmpty(), QStringLiteral("%1 outputs").arg(sub.size()));
    const QVariant headroom = b->fetchSubharmHeadroom();
    reading("subharmonic headroom", headroom.isValid(), headroom.toString());
    float gr[MAX_OUTPUTS] = {};
    bool sane = dspi_fetch_limiter_meter(b->core(), gr);
    const bool read = sane;
    QString values;
    for (int o = 0; o < b->numOutputChannels() && o < MAX_OUTPUTS; o++) {
        sane = sane && std::isfinite(gr[o]);
        values += QStringLiteral(" %1").arg(double(gr[o]), 0, 'f', 1);
    }
    reading("limiter meter", sane, read ? QStringLiteral("gain reduction, dB:%1").arg(values) : QStringLiteral("the read failed"));
}

// ── Signal generator: never staged, never started ──

void SmokeDriver::hwSiggenCaps() {
    auto *sg = m_.siggen;
    if (!sg->supported()) {
        notRun("signal generator caps", "the device reports none");
    } else {
        outcome("signal generator caps", !sg->running() && sg->outputChannels() > 0,
                QStringLiteral("%1 outputs, valid mask 0x%2, %3 tones at most; state %4")
                    .arg(sg->outputChannels()).arg(sg->validMask(), 0, 16).arg(sg->multitoneMax()).arg(sg->state()));
    }
    notRun("signal generator start", "never in this pass: it makes sound");
}

// ── Bypass ──

void SmokeDriver::hwBypass() {
    auto *b = m_.bridge;
    auto *core = b->core();
    const bool original = b->bypass();
    b->setBypass(!original);
    spin(150);
    const bool wrote = dspi_fetch_all(core) && dspi_get_state(core)->bypass == !original;
    b->setBypass(original);
    const bool restored = dspi_fetch_all(core) && dspi_get_state(core)->bypass == original;
    emit b->stateChanged();
    outcome("bypass: toggled on the device", wrote);
    if (!restored) {
        b->setBypass(original);   // again, then say so loudly
        fail("bypass: RESTORED", "the device did not report the original value; written again");
    } else {
        pass("bypass: restored", original ? "on" : "off");
    }
    QString why;
    const Outcome o = bypassRepaints(&why);
    if (o == Outcome::NotRun) notRun("bypass: repaint", why);
    else outcome("bypass: repaint", o == Outcome::Pass, why);
    outcome("bypass: as found after the repaint check", dspi_fetch_all(core) && dspi_get_state(core)->bypass == original);
    emit b->stateChanged();
}

// ── Disconnect and reselect ──

void SmokeDriver::hwReselect(const Baseline &want) {
    auto *b = m_.bridge;
    const QString serial = b->selectedSerial();
    b->disconnectDevice();    // dspi_disconnect
    outcome("disconnect", !b->connected());
    b->selectDevice(serial);
    if (!b->connected()) {
        fail("reselect", b->connectionError());
        return;
    }
    outcome("reselect", b->compat() == COMPAT_OK && b->connectionError().isEmpty(), b->connectionError());
    outcome("state re-read after reselect", compareBaseline(want, "after the reselect"));
    SystemStatus status {};
    outcome("status polled again", dspi_fetch_status(b->core(), &status));
}
