// -- WP QP19 -- The hardware pass (DSPI_SMOKE_HW=1, run-hw.sh): what the demo
// device does not model, read from a real DSPi through the app's own core.
// DSPI_SMOKE_HW=dry runs the same pass on the demo (run.sh, after the main run),
// where a read the demo does not model counts as not run.
//
// It leaves the device as it found it: it reads the device's image first and
// compares at the end, and the one thing it changes is global bypass (RAM,
// restored and checked). Never here: a preset save or load, a parameter save, a
// factory reset, a firmware action, a binding write, or anything that starts the
// signal generator (that makes sound).
//
// The replug pass (DSPI_SMOKE_HW=replug, run-replug.sh) is below: three unplug
// and replug cycles by the owner, timed from sysfs, writing nothing.
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DevicesController.h"
#include "Registrar.h"

extern "C" {
#include "dspi_core.h"
}

#include <QByteArray>
#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickWindow>
#include <cmath>
#include <cstdio>
#include <array>
#include <atomic>
#include <memory>
#include <optional>
#include <vector>

namespace {

bool dry() { return qEnvironmentVariable("DSPI_SMOKE_HW") == QLatin1String("dry"); }

// What the pass compares: the parameter image, and what the image leaves out
// that a write could change (the preset directory, Core 1's mode, solo)
struct Baseline {
    QByteArray image;
    QByteArray rest;
    bool operator==(const Baseline &o) const { return image == o.image && rest == o.rest; }
};

struct Pass {
    bool ready = false;   // the preflight passed; every later section needs it
    Baseline start;
};
Pass &pass() {
    static Pass p;
    return p;
}

// -- WP QPI4 -- The app's rule: every dspi_* call runs on the core's worker
// (the state record is rewritten by each call, so it is read where it is
// fetched). Runs `fn` there and waits; nothing when the worker did not answer.
template <class F>
auto onWorker(SmokeDriver &d, F fn) -> std::optional<decltype(fn(static_cast<FfiCore *>(nullptr)))> {
    using R = decltype(fn(static_cast<FfiCore *>(nullptr)));
    auto answer = std::make_shared<std::optional<R>>();
    auto ready = std::make_shared<std::atomic<bool>>(false);
    d.app().core->worker()->post([fn = std::move(fn), answer, ready](FfiCore *c) {
        answer->emplace(fn(c));
        ready->store(true);
    });
    if (!d.waitFor(10000, [&] { return ready->load(); })) return std::nullopt;
    return *answer;
}

// Worker thread
bool readImage(FfiCore *core, Baseline *out) {
    if (!dspi_fetch_all(core)) return false;
    const DspState *s = dspi_get_state(core);
    if (!s || !s->bulk_valid) return false;
    out->image = QByteArray(reinterpret_cast<const char *>(s->bulk_raw), int(sizeof(s->bulk_raw)));
    out->rest.clear();
    const auto add = [out](const void *p, size_t n) { out->rest.append(static_cast<const char *>(p), qsizetype(n)); };
    add(&s->preset_occupied, sizeof(s->preset_occupied));
    add(s->preset_names, sizeof(s->preset_names));
    add(&s->active_preset_slot, sizeof(s->active_preset_slot));
    add(&s->preset_startup_mode, sizeof(s->preset_startup_mode));
    add(&s->preset_default_slot, sizeof(s->preset_default_slot));
    add(&s->core1_mode, sizeof(s->core1_mode));
    add(&s->subharm_solo, sizeof(s->subharm_solo));
    return true;
}

bool readBaseline(SmokeDriver &d, Baseline *out) {
    const auto read = onWorker(d, [](FfiCore *c) {
        Baseline b;
        const bool ok = readImage(c, &b);
        return std::make_pair(ok, b);
    });
    if (!read || !read->first) return false;
    *out = read->second;
    return true;
}

// The device as found, or a loud failure naming the bytes that differ
bool asFound(SmokeDriver &d, const Baseline &want, const char *when) {
    Baseline now;
    if (!readBaseline(d, &now)) {
        d.check(QStringLiteral("qp19-hw: the device reads again %1").arg(QLatin1String(when)), false);
        return false;
    }
    if (now == want) {
        d.check(QStringLiteral("qp19-hw: the device as found %1").arg(QLatin1String(when)), true);
        return true;
    }
    QString list;
    int differing = 0;
    for (qsizetype i = 0; i < qMin(now.image.size(), want.image.size()); i++) {
        if (now.image[i] == want.image[i]) continue;
        if (++differing <= 32)
            list += QStringLiteral(" %1:%2>%3").arg(i).arg(quint8(want.image[i]), 2, 16, QLatin1Char('0')).arg(quint8(now.image[i]), 2, 16, QLatin1Char('0'));
    }
    std::printf("smoke: ******** THE DEVICE'S STATE CHANGED %s ********\n", when);
    std::printf("smoke: %d image bytes differ (offset:was>now):%s%s\n", differing, qPrintable(list),
                now.rest == want.rest ? "" : "; the preset directory, Core 1's mode or solo differ too");
    d.check(QStringLiteral("qp19-hw: the device as found %1").arg(QLatin1String(when)), false);
    return false;
}

// A read the demo may not model: in the dry run its failure is not run
void reading(SmokeDriver &d, const QString &what, bool ok, const QString &detail = QString()) {
    if (!detail.isEmpty()) std::printf("smoke: qp19-hw: %s: %s\n", qPrintable(what), qPrintable(detail));
    if (!ok && dry()) d.notRun(what, QStringLiteral("the demo device does not model it"));
    else d.check(QStringLiteral("qp19-hw: ") + what, ok);
}

// A core call that fills a buffer with JSON: what it says, or nothing when the
// answer did not fit or is not an object
template <typename F>
QVariantMap coreJson(F call) {
    constexpr uint32_t kCapacity = 16384;
    std::vector<char> text(kCapacity);
    const uint32_t needed = call(text.data(), kCapacity);   // counts the closing NUL
    if (needed == 0 || needed > kCapacity) return {};
    const QJsonDocument doc = QJsonDocument::fromJson(QByteArray(text.data(), qsizetype(needed) - 1));
    return doc.isObject() ? doc.object().toVariantMap() : QVariantMap();
}
QString compact(const QVariantMap &m) {
    return QString::fromUtf8(QJsonDocument(QJsonObject::fromVariantMap(m)).toJson(QJsonDocument::Compact)).left(300);
}

bool siggenIdle(SmokeDriver &d) {
    return onWorker(d, [](FfiCore *c) {
               SiggenStatus sg{};
               return !dspi_siggen_get_status(c, &sg) || sg.state == 0;
           }).value_or(false);
}

// ---- The sections, in order ----

void preflight(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    std::printf("smoke: hardware pass%s\n", dry() ? " (dry run on the demo device)" : "");
    const bool connected = d.waitFor(15000, [device] { return device->connected(); }) && d.waitIdle(10000);
    d.check("qp19-hw: a device is open", connected);
    if (!connected) return;
    auto *list = Registrar::instance<DevicesController>();
    if (dry()) {
        d.check("qp19-hw: the dry run is on the demo", device->demo());
        if (!device->demo()) return;
    } else {
        d.check("qp19-hw: a real DSPi, not the demo (run without --demo)", !device->demo());
        // -- WP SWAPFIX -- (QP22b F6) the list is filled by a scan, and scans
        // run only while it is shown: scan now. The worker runs in order, so
        // once a call queued after the scan has run, the scan's answer is
        // queued for this thread; deliver it, then count.
        if (list) {
            list->rescan();
            onWorker(d, [](FfiCore *) { return true; });
            QCoreApplication::processEvents();
        }
        const int seen = list ? int(list->usbDevices().size()) : -1;
        d.check(QStringLiteral("qp19-hw: exactly one DSPi on USB (%1 seen)").arg(seen), seen == 1);
        if (device->demo() || seen != 1) return;
    }
    std::printf("smoke: qp19-hw: %s, %s, firmware %s\n", qPrintable(device->serial()), qPrintable(device->platformName()),
                qPrintable(device->firmwareVersion()));
    d.check("qp19-hw: writable (not read-only)", !device->readOnly());
    d.check("qp19-hw: the signal generator is idle (stop it first; this pass never touches it)", siggenIdle(d));
    if (device->readOnly() || !siggenIdle(d)) return;
    pass().ready = readBaseline(d, &pass().start);
    d.check("qp19-hw: the device's image reads (nothing was changed)", pass().ready);
    if (pass().ready) std::printf("smoke: qp19-hw: %d image bytes read\n", int(pass().start.image.size()));
}

void analyser(SmokeDriver &d) {
    if (!pass().ready) return;
    const RtaCapsInfo caps = onWorker(d, [](FfiCore *c) {
                                 RtaCapsInfo k{};
                                 dspi_rta_get_caps(c, &k);
                                 return k;
                             }).value_or(RtaCapsInfo{});
    if (!caps.supported) {
        d.notRun("analyser", QStringLiteral("the device reports none"));
        return;
    }
    std::printf("smoke: qp19-hw: analyser: %d inputs, %d outputs, order %d..%d\n", caps.input_channels, caps.output_channels,
                caps.order_min, caps.order_max);
    auto snapshot = std::make_shared<RtaSnapshot>();
    const auto take = [&d, snapshot] {
        onWorker(d, [snapshot](FfiCore *c) {
            dspi_rta_get_snapshot(c, snapshot.get());
            return true;
        });
    };
    const auto ask = [&d](RtaRequest request) {
        onWorker(d, [request](FfiCore *c) {
            dspi_rta_set_request(c, &request);
            return true;
        });
    };
    for (uint8_t tap : { uint8_t(RTA_TAP_INPUT), uint8_t(RTA_TAP_OUTPUT) }) {
        const QString what = QStringLiteral("analyser frames, %1 tap").arg(tap == RTA_TAP_INPUT ? "input" : "output");
        RtaRequest request{};
        request.tap = tap;
        request.mask = 0x3;
        request.fft_order = caps.order_min;
        request.avg_ms = 100;
        request.peak_decay_db_s = 12;
        ask(request);
        uint32_t first = 0;
        const bool running = d.waitFor(5000, [&] {
            take();
            first = snapshot->version;
            return snapshot->running && snapshot->tap == tap && snapshot->count > 0;
        });
        // The picture's version moves only when the picture does: digital silence never moves it
        const bool moving = running && d.waitFor(5000, [&] {
            take();
            return snapshot->version != first;
        });
        reading(d, what, running && moving,
                QStringLiteral("running %1, %2 channels%3").arg(running).arg(int(snapshot->count)).arg(moving ? "" : ", no new picture: is a stream playing?"));
    }
    ask(RtaRequest{});
    d.spin(500);
}

void status(SmokeDriver &d) {
    if (!pass().ready) return;
    SystemStatus s{};
    bool ok = true;
    for (int i = 0; i < 5 && ok; i++) {
        const auto read = onWorker(d, [](FfiCore *c) {
            SystemStatus st{};
            const bool fetched = dspi_fetch_status(c, &st);
            return std::make_pair(fetched, st);
        });
        ok = read && read->first;
        if (ok) s = read->second;
        d.spin(20);
    }
    reading(d, "system status", ok && s.cpu0 <= 100 && s.cpu1 <= 100,
            QStringLiteral("CPU %1/%2 %, %3 active inputs").arg(int(s.cpu0)).arg(int(s.cpu1)).arg(int(s.active_input_channels)));
    const auto rx = onWorker(d, [](FfiCore *c) {
        SpdifRxStatus r{};
        const bool fetched = dspi_fetch_spdif_rx_status(c, &r);
        return std::make_pair(fetched, r);
    });
    reading(d, "S/PDIF input status", rx && rx->first, QStringLiteral("state %1").arg(rx ? int(rx->second.state) : -1));
    const auto starved = onWorker(d, [](FfiCore *c) {
        uint32_t n = 0;
        const bool fetched = dspi_fetch_stat(c, STAT_STARVATION_TOTAL, &n);
        return std::make_pair(fetched, n);
    });
    reading(d, "starvation counter", starved && starved->first, QString::number(starved ? starved->second : 0));
    const auto json = [&d](uint32_t (*call)(FfiCore *, char *, uint32_t)) {
        return onWorker(d, [call](FfiCore *c) { return coreJson([c, call](char *b, uint32_t n) { return call(c, b, n); }); })
            .value_or(QVariantMap());
    };
    const QVariantMap latency = json(&dspi_fetch_output_latency);
    reading(d, "output latency", latency.value(QStringLiteral("ok")).toBool(), compact(latency));
    const QVariantMap ext = json(&dspi_ext_status);
    reading(d, "extension channel", ext.value(QStringLiteral("ok")).toBool(), compact(ext));
    const QVariantMap clock = json(&dspi_fetch_clock_diag);
    if (clock.value(QStringLiteral("ok")).toBool() && !clock.value(QStringLiteral("available")).toBool())
        d.notRun("clock diagnostics", QStringLiteral("not built into this firmware"));
    else
        reading(d, "clock diagnostics", clock.value(QStringLiteral("ok")).toBool(), compact(clock));
    const auto meter = onWorker(d, [](FfiCore *c) {
        std::array<float, MAX_OUTPUTS> g{};
        const bool fetched = dspi_fetch_limiter_meter(c, g.data());
        return std::make_pair(fetched, g);
    });
    const std::array<float, MAX_OUTPUTS> gr = meter ? meter->second : std::array<float, MAX_OUTPUTS>{};
    bool sane = meter && meter->first;
    QString values;
    for (int o = 0; o < d.app().device->numOutputs() && o < MAX_OUTPUTS; o++) {
        sane = sane && std::isfinite(gr[o]);
        values += QStringLiteral(" %1").arg(double(gr[o]), 0, 'f', 1);
    }
    reading(d, "limiter meter", sane, QStringLiteral("gain reduction, dB:%1").arg(values));
    d.notRun("signal generator start", QStringLiteral("never in this pass: it makes sound"));
    d.notRun("buffer watermark reset", QStringLiteral("it can't be undone, and this pass leaves the device as found"));
}

void bypass(SmokeDriver &d) {
    if (!pass().ready) return;
    DeviceModel *device = d.app().device;
    const auto onDevice = [&] {
        return onWorker(d, [](FfiCore *c) { return dspi_fetch_all(c) ? int(dspi_get_state(c)->bypass) : -1; }).value_or(-1);
    };
    const int original = onDevice();
    d.check("qp19-hw: bypass reads", original >= 0);
    if (original < 0) return;
    const auto set = [&](bool on) {
        int outcome = -1;
        device->write(QStringLiteral("bypass"), [on](FfiCore *c) { return dspi_set_bypass(c, on); },
                      [&outcome](bool ok) { outcome = ok ? 1 : 0; });
        return d.waitFor(5000, [&] { return outcome >= 0; }) && outcome == 1 && d.waitIdle();
    };
    d.check("qp19-hw: bypass toggles on the device", set(!original) && onDevice() == !original);
    const bool restored = set(bool(original)) && onDevice() == original;
    if (!restored) set(bool(original));   // again, then say so loudly
    d.check("qp19-hw: bypass RESTORED", restored);
    asFound(d, pass().start, "after the bypass toggle");
    device->refresh();
    d.waitIdle();
}

void reselect(SmokeDriver &d) {
    if (!pass().ready) return;
    DeviceModel *device = d.app().device;
    const QString serial = device->serial();
    device->reconnect();
    const bool back = d.waitFor(15000, [&] { return device->connected() && device->serial() == serial; }) && d.waitIdle(10000);
    d.check("qp19-hw: closed and opened again (the device button's reconnect)", back);
    if (!back) return;
    d.check("qp19-hw: writable again", !device->readOnly());
    asFound(d, pass().start, "after the reconnect");
}

void end(SmokeDriver &d) {
    if (!pass().ready) return;
    d.spin(300);
    d.check("qp19-hw: the signal generator is still idle", siggenIdle(d));
    asFound(d, pass().start, "at the end");   // one check, passed or failed
    d.grab(d.mainWindow(), QStringLiteral("window-main.png"));
}

using Mode = SmokeDriver::Mode;
const SmokeDriver::Section hw1("qp19-hw-preflight", 10, &preflight, Mode::Hardware);
const SmokeDriver::Section hw2("qp19-hw-analyser", 20, &analyser, Mode::Hardware);
const SmokeDriver::Section hw3("qp19-hw-status", 30, &status, Mode::Hardware);
const SmokeDriver::Section hw4("qp19-hw-bypass", 40, &bypass, Mode::Hardware);
const SmokeDriver::Section hw5("qp19-hw-reconnect", 50, &reselect, Mode::Hardware);
const SmokeDriver::Section hw6("qp19-hw-end", 90, &end, Mode::Hardware);

// ---- Replug (DSPI_SMOKE_HW=replug) ----

constexpr int kCycles = 3;
constexpr int kOwnerMs = 45000;          // run.sh's timeout allows for six of these
constexpr int kDisconnectedMs = 2500;    // the hotplug poll is 500 ms, and a read fails at once
constexpr int kReconnectedMs = 15000;    // the firmware's boot, enumeration, the connect sequence

// The device with this serial is on the bus, by sysfs alone: nothing opens it
bool onBus(const QString &serial) {
    const QString base = QStringLiteral("/sys/bus/usb/devices/");
    for (const QString &dir : QDir(base).entryList(QDir::Dirs | QDir::NoDotAndDotDot)) {
        QFile f(base + dir + QStringLiteral("/serial"));
        if (f.open(QIODevice::ReadOnly) && QString::fromLatin1(f.readAll()).trimmed() == serial) return true;
    }
    return false;
}

void ownerPrompt(const QString &text) {
    std::printf("smoke:\nsmoke: >>> %s\n", qPrintable(text));
    std::fflush(stdout);
}

bool replugCycle(SmokeDriver &d, int cycle, const QString &serial, Baseline *want) {
    DeviceModel *device = d.app().device;
    const QString name = QStringLiteral("qp19-replug: cycle %1/%2").arg(cycle).arg(kCycles);
    const auto shot = [&](const char *what) { d.grab(d.mainWindow(), QStringLiteral("cycle-%1-%2.png").arg(cycle).arg(QLatin1String(what))); };

    ownerPrompt(QStringLiteral("cycle %1/%2: UNPLUG the DSPi now (%3 s)").arg(cycle).arg(kCycles).arg(kOwnerMs / 1000));
    if (!d.waitFor(kOwnerMs, [&] { return !onBus(serial); })) {
        d.check(name + QStringLiteral(": unplugged"), false);
        return false;
    }
    QElapsedTimer t;
    t.start();
    const bool gone = d.waitFor(kDisconnectedMs, [device] { return !device->connected(); });
    std::printf("smoke: %s: closed %lld ms after the bus said so\n", qPrintable(name), qint64(t.elapsed()));
    d.check(name + QStringLiteral(": the app shows it gone within %1 ms").arg(kDisconnectedMs), gone);
    d.spin(1000);
    d.check(name + QStringLiteral(": and stays so, running"), !device->connected());
    shot("unplugged");

    ownerPrompt(QStringLiteral("cycle %1/%2: PLUG the DSPi back in now (%3 s)").arg(cycle).arg(kCycles).arg(kOwnerMs / 1000));
    if (!d.waitFor(kOwnerMs, [&] { return onBus(serial); })) {
        d.check(name + QStringLiteral(": back on the bus"), false);
        return false;
    }
    t.restart();
    const bool back = d.waitFor(kReconnectedMs, [&] { return device->connected() && device->serial() == serial && !device->readOnly(); });
    std::printf("smoke: %s: open again %lld ms after the bus said so\n", qPrintable(name), qint64(t.elapsed()));
    d.check(name + QStringLiteral(": opens again by itself within %1 ms").arg(kReconnectedMs), back);
    if (!back) return false;
    d.waitIdle(10000);
    d.spin(800);
    d.check(name + QStringLiteral(": status reads"), onWorker(d, [](FfiCore *c) {
                                                          SystemStatus s{};
                                                          return dspi_fetch_status(c, &s);
                                                      }).value_or(false));
    if (!asFound(d, *want, "after the replug")) {
        // A replug that cut the power drops what RAM held unsaved: the next cycle starts from here
        std::printf("smoke: a power cycle drops unsaved changes: was anything unsaved when this run began?\n");
        readBaseline(d, want);
    }
    shot("replugged");
    return true;
}

void replug(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    std::printf("smoke: replug pass: %d cycles, each step waits %d s for you\n", kCycles, kOwnerMs / 1000);
    const bool open = d.waitFor(15000, [device] { return device->connected(); }) && d.waitIdle(10000);
    d.check("qp19-replug: a real DSPi is open", open && !device->demo());
    if (!open || device->demo()) return;
    const QString serial = device->serial();
    d.check("qp19-replug: it is in /sys/bus/usb/devices (real sysfs)", onBus(serial));
    Baseline want;
    d.check("qp19-replug: the device's image reads", readBaseline(d, &want));
    if (!onBus(serial) || want.image.isEmpty()) return;
    for (int c = 1; c <= kCycles; c++) {
        if (!replugCycle(d, c, serial, &want)) {
            std::printf("smoke: cycle %d/%d failed, stopping here\n", c, kCycles);
            break;
        }
    }
}

const SmokeDriver::Section replugPass("qp19-replug", 10, &replug, Mode::Replug);

} // namespace
