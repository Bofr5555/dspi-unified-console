// -- WP PAIRBAND -- A linked input pair's band edit is one undo step (the
// core's dspi_set_input_pair_filter): from the band list (type, a value, the
// bypass) and from a graph drag (writes between beginDrag and endDrag, the
// fields changing kind mid-way). One undo puts both sides back together; the
// pair never shows two different filters. Every change is put back.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "DeviceModel.h"
#include "HistoryController.h"
#include "Registrar.h"
#include "SidebarController.h"

#include "CoreApi.h"

#include <cmath>
#include <cstdio>

namespace {

bool ensureDemo(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    return d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle();
}

const DspState &st(SmokeDriver &d) { return *d.app().device->state(); }

bool same(const FilterParams &a, const FilterParams &b) {
    return a.filter_type == b.filter_type && a.bypass == b.bypass && a.freq == b.freq && a.q == b.q && a.gain == b.gain;
}

// Both sides of pair 0, band b, as the device holds them
bool pairIs(SmokeDriver &d, int b, const FilterParams &l, const FilterParams &r) {
    return same(st(d).filters[0][b], l) && same(st(d).filters[1][b], r);
}

void pairbandSection(SmokeDriver &d) {
    auto *channels = Registrar::instance<ChannelController>();
    auto *history = Registrar::instance<HistoryController>();
    d.check("pairband: the controllers are made", channels && history);
    if (!channels || !history) return;
    d.check("pairband: on the demo", ensureDemo(d));
    DeviceModel *device = d.app().device;
    const bool wasLinked = channels->isLinked(0);
    if (!wasLinked) channels->linkPair(0, 0);
    d.waitIdle();
    d.check("pairband: pair 0 is linked", channels->isLinked(0) && channels->linkedPartner(0) == 1);
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

    // The band list: a type, then a value; each one step, both sides
    const int b = 2;
    const FilterParams l0 = st(d).filters[0][b], r0 = st(d).filters[1][b];
    channels->setBandType(1, b, false, FILTER_PEAKING);
    d.waitIdle();
    const FilterParams l1 = st(d).filters[0][b], r1 = st(d).filters[1][b];
    d.check("pairband: a type from the band list reaches both sides", l1.filter_type == FILTER_PEAKING && same(l1, r1));
    channels->setBandValue(0, b, false, QStringLiteral("gain"), 4.5);
    d.waitIdle();
    const FilterParams l2 = st(d).filters[0][b];
    d.check("pairband: a value reaches both sides", std::fabs(l2.gain - 4.5f) < 0.05f && same(l2, st(d).filters[1][b]));
    std::printf("smoke: pairband: the newest step is \"%s\"\n", qPrintable(title()));
    undo();
    d.check("pairband: one undo puts the value back on both sides", pairIs(d, b, l1, r1));
    if (channels->bypassSupported()) {
        channels->setBandBypass(0, b, false, true);
        d.waitIdle();
        d.check("pairband: the bypass reaches both sides", st(d).filters[0][b].bypass && st(d).filters[1][b].bypass);
        undo();
        d.check("pairband: one undo enables both sides", pairIs(d, b, l1, r1));
    }
    undo();
    d.check("pairband: one undo puts the type back on both sides", pairIs(d, b, l0, r0));

    // A graph drag: gain ticks, then frequency and gain (the fields change
    // kind mid-way, P0W2), on either side; one step
    const int g = 3;
    const FilterParams lo = st(d).filters[0][g], ro = st(d).filters[1][g];
    channels->setBandType(0, g, false, FILTER_PEAKING);
    d.waitIdle();
    const FilterParams ls = st(d).filters[0][g], rs = st(d).filters[1][g];
    device->beginDrag();
    for (int i = 1; i <= 4; ++i) {
        channels->setBandValue(0, g, false, QStringLiteral("gain"), 1.0 * i);
        d.spin(20);
    }
    for (int i = 1; i <= 4; ++i) {
        channels->setBandValue(1, g, false, QStringLiteral("freq"), 1000.0 + 150.0 * i);
        channels->setBandValue(1, g, false, QStringLiteral("gain"), 4.0 - 0.5 * i);
        d.spin(20);
    }
    d.waitIdle();
    device->endDrag();
    d.waitIdle();
    const FilterParams le = st(d).filters[0][g];
    d.check("pairband: the drag lands on both sides", std::fabs(le.freq - 1600.f) < 0.5f && same(le, st(d).filters[1][g]));
    std::printf("smoke: pairband: the drag's step is \"%s\"\n", qPrintable(title()));
    undo();
    d.check("pairband: one undo puts the whole drag back on both sides", pairIs(d, g, ls, rs));
    history->redo();
    d.waitIdle();
    d.spin(100);
    d.waitIdle();
    d.check("pairband: one redo puts the whole drag on both sides", pairIs(d, g, le, le));
    undo();
    undo();
    d.check("pairband: and the band is as it was", pairIs(d, g, lo, ro));
    // A paste of a whole channel onto the linked input: one undo step ("Paste
    // EQ") puts every band of both sides back
    if (auto *sc = Registrar::instance<SidebarController>()) {
        const int out = channels->outputChannel(0);
        for (int k = 0; k < 3; ++k) {
            channels->setBandType(out, k, false, FILTER_PEAKING);
            channels->setBandValue(out, k, false, QStringLiteral("freq"), 300.0 + 700.0 * k);
            channels->setBandValue(out, k, false, QStringLiteral("gain"), 1.5 + k);
        }
        d.waitIdle();
        const DspState saved = st(d);
        sc->copyChannel(out);
        sc->pasteChannel(0);
        d.waitIdle();
        bool pasted = true;
        for (int k = 0; k < 3; ++k) pasted = pasted && same(st(d).filters[0][k], st(d).filters[out][k]) && same(st(d).filters[1][k], st(d).filters[out][k]);
        d.check("pairband: a paste onto the linked input reaches both sides", pasted);
        std::printf("smoke: pairband: the paste's step is \"%s\"\n", qPrintable(title()));
        d.check("pairband: the paste's step is labelled Paste EQ", title().startsWith(QLatin1String("Paste EQ")));
        undo();
        bool back = true;
        for (int k = 0; k < BANDS_PER_CHANNEL; ++k) back = back && pairIs(d, k, saved.filters[0][k], saved.filters[1][k]);
        d.check("pairband: one undo puts the whole paste back on both sides", back);
        channels->clearBands(out, false);
        d.waitIdle();
    }
    if (!wasLinked) channels->unlinkPair(0);
    d.waitIdle();
}

const SmokeDriver::Section pairband("pairband", 161, &pairbandSection);

}   // namespace
