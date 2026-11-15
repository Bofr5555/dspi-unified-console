// -- WP CLIPQT -- Copy / Paste Parameters through the core (dspi_copy_channel,
// dspi_paste_channel): a pasted output (PEQ, crossover, gain, delay, mute) is
// one undo step, "Paste Parameters", that puts all of it back; a paste while
// the installer's lock holds writes nothing and shows the core's notice.
// Every change is put back.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "DeviceModel.h"
#include "HistoryController.h"
#include "LimitsController.h"
#include "NoticeCenter.h"
#include "Qp18Notices.h"
#include "Registrar.h"
#include "SidebarController.h"

#include "CoreApi.h"

#include <cmath>
#include <cstdio>
#include <memory>

namespace {

constexpr const char *kPin = "2468";

bool ensureDemo(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    return d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle();
}

const DspState &st(SmokeDriver &d) { return *d.app().device->state(); }

bool same(const FilterParams &a, const FilterParams &b) {
    return a.filter_type == b.filter_type && a.bypass == b.bypass && a.freq == b.freq && a.q == b.q && a.gain == b.gain;
}

// What a paste may change on one output
struct OutputParams {
    FilterParams peq[BANDS_PER_CHANNEL];
    FilterParams xover[MAX_XOVER_BANDS];
    float gain = 0;
    float delay = 0;
    bool muted = false;
};

OutputParams paramsOf(const DspState &s, int channel) {
    OutputParams p;
    const int o = channel - int(s.num_input_channels);
    for (int k = 0; k < BANDS_PER_CHANNEL; ++k) p.peq[k] = s.filters[channel][k];
    for (int k = 0; k < MAX_XOVER_BANDS; ++k) p.xover[k] = s.xover[channel][k];
    p.gain = s.output_gain_db[o];
    p.delay = s.output_delay_ms[o];
    p.muted = s.output_muted[o];
    return p;
}

bool sameParams(const OutputParams &a, const OutputParams &b) {
    for (int k = 0; k < BANDS_PER_CHANNEL; ++k)
        if (!same(a.peq[k], b.peq[k])) return false;
    for (int k = 0; k < MAX_XOVER_BANDS; ++k)
        if (!same(a.xover[k], b.xover[k])) return false;
    return std::fabs(a.gain - b.gain) < 0.01f && std::fabs(a.delay - b.delay) < 0.01f && a.muted == b.muted;
}

bool write(SmokeDriver &d, const QString &what, std::function<bool(FfiCore *)> setter) {
    auto ok = std::make_shared<int>(-1);
    d.app().device->write(what, std::move(setter), [ok](bool done) { *ok = done ? 1 : 0; });
    d.waitFor(5000, [&] { return *ok >= 0; });
    d.waitIdle();
    return *ok == 1;
}

// Writes `p` back onto output channel `channel`
bool restore(SmokeDriver &d, int channel, const OutputParams &p) {
    const int o = channel - int(st(d).num_input_channels);
    return write(d, QStringLiteral("clipqt restore %1").arg(channel), [channel, o, p](FfiCore *c) {
        bool ok = true;
        for (int k = 0; k < BANDS_PER_CHANNEL; ++k) ok = dspi_set_filter(c, uint8_t(channel), uint8_t(k), p.peq[k]) && ok;
        for (int k = 0; k < MAX_XOVER_BANDS; ++k) ok = dspi_set_crossover(c, uint8_t(channel), uint8_t(k), p.xover[k]) && ok;
        ok = dspi_set_output_gain(c, uint8_t(o), p.gain) && ok;
        ok = dspi_set_output_delay(c, uint8_t(o), p.delay) && ok;
        return dspi_set_output_mute(c, uint8_t(o), p.muted) && ok;
    });
}

bool settled(SmokeDriver &d, LimitsController *lc) {
    d.waitIdle();
    const bool ok = d.waitFor(8000, [&] { return !lc->busy(); });
    d.waitIdle();
    d.spin(60);
    return ok;
}

void clipqtSection(SmokeDriver &d) {
    auto *channels = Registrar::instance<ChannelController>();
    auto *history = Registrar::instance<HistoryController>();
    auto *sc = Registrar::instance<SidebarController>();
    auto *lc = Registrar::instance<LimitsController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    d.check("clipqt: the controllers are made", channels && history && sc && lc && notices);
    if (!channels || !history || !sc || !lc || !notices) return;
    d.check("clipqt: on the demo", ensureDemo(d));
    const int a = channels->outputChannel(0), b = channels->outputChannel(1);
    d.check("clipqt: two outputs", a >= 0 && b >= 0 && a != b);
    if (a < 0 || b < 0 || a == b) return;
    auto undo = [&] {
        history->undo();
        d.waitIdle();
        d.spin(100);
        d.waitIdle();
    };
    auto title = [&] {
        d.waitFor(2000, [&] { return !history->entries().isEmpty(); });
        return history->entries().value(0).toMap().value(QStringLiteral("title")).toString();
    };

    // Output A differs from B in everything a paste carries
    const OutputParams savedA = paramsOf(st(d), a);
    const int oa = a - int(st(d).num_input_channels);
    d.check("clipqt: output A is seeded", write(d, QStringLiteral("clipqt seed"), [a, oa](FfiCore *c) {
        FilterParams bell{};
        bell.filter_type = FILTER_PEAKING;
        bell.freq = 777.0f;
        bell.q = 0.9f;
        bell.gain = 3.0f;
        FilterParams xo{};
        xo.filter_type = 35;
        xo.freq = 120.0f;
        xo.q = 0.707f;
        return dspi_set_filter(c, uint8_t(a), 0, bell) && dspi_set_crossover(c, uint8_t(a), 0, xo)
            && dspi_set_output_gain(c, uint8_t(oa), -4.5f) && dspi_set_output_delay(c, uint8_t(oa), 1.25f)
            && dspi_set_output_mute(c, uint8_t(oa), true);
    }));
    const OutputParams seeded = paramsOf(st(d), a);
    const OutputParams beforeB = paramsOf(st(d), b);
    d.check("clipqt: A and B differ before the paste", !sameParams(seeded, beforeB));

    sc->copyChannel(a);
    d.waitIdle();
    d.check("clipqt: the copy holds the core's clip", sc->hasClipboard());
    sc->pasteChannel(b);
    d.waitIdle();
    d.check("clipqt: the paste carries PEQ, crossover, gain, delay and mute", sameParams(paramsOf(st(d), b), seeded));
    std::printf("smoke: clipqt: the paste's step is \"%s\"\n", qPrintable(title()));
    d.check("clipqt: the paste's step is the core's \"Paste Parameters\"", title().startsWith(QLatin1String("Paste Parameters")));
    undo();
    d.check("clipqt: one undo restores the pasted output", sameParams(paramsOf(st(d), b), beforeB));

    // The installer's lock: the whole paste is refused (mute alone would pass)
    auto lockLine = [&](const char *step) {
        std::printf("smoke: clipqt: %s: pinSet %d locked %d error \"%s\"\n", step, int(lc->pinSet()), int(lc->locked()),
                    qPrintable(lc->pinError()));
    };
    settled(d, lc);
    lc->setPin(QString::fromLatin1(kPin), QString::fromLatin1(kPin));
    settled(d, lc);
    lockLine("PIN set");
    lc->dismissRecovery();
    lc->setCovers(true);
    settled(d, lc);
    lc->lockNow();
    settled(d, lc);
    lockLine("locked");
    d.check("clipqt: the console is locked", lc->locked());
    if (lc->locked()) {
        qp18NoticeMark();
        const int steps = int(history->entries().size());
        const int setters = d.app().device->settersRun();
        sc->pasteChannel(b);
        d.waitIdle();
        d.check("clipqt: a locked paste reached the core", d.app().device->settersRun() == setters + 1);
        d.check("clipqt: a locked paste writes nothing",
                sameParams(paramsOf(st(d), b), beforeB) && int(history->entries().size()) == steps);
        const QString lockTitle = notices->row(QStringLiteral("installer_lock.locked")).value(QStringLiteral("title")).toString();
        d.check("clipqt: with the core's lock notice",
                !lockTitle.isEmpty() && d.waitFor(3000, [&] {
                    return qp18NoticeNew() && notices->last().value(QStringLiteral("text")).toString().startsWith(lockTitle);
                }));
        lc->unlock(QString::fromLatin1(kPin));
        settled(d, lc);
    }
    if (lc->pinSet()) {
        lc->setCovers(false);
        settled(d, lc);
        lc->clearPin();
        settled(d, lc);
    }
    lockLine("cleared");
    d.check("clipqt: unlocked, no PIN", !lc->locked() && !lc->pinSet());
    notices->clear();

    d.check("clipqt: output A is put back", restore(d, a, savedA) && sameParams(paramsOf(st(d), a), savedA));
    d.check("clipqt: output B is as it was", sameParams(paramsOf(st(d), b), beforeB));
}

const SmokeDriver::Section clipqt("clipqt", 163, &clipqtSection);

}   // namespace
