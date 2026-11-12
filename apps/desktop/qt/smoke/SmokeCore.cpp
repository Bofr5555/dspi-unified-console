// -- WP QP0 -- The core wrapper's schedules (fixup g0): the meter and status
// polls run on the demo device, a slow poll skips ticks instead of queueing
// reads, nothing polls without a device, and writes with one key coalesce.
#include "SmokeDriver.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "Poller.h"

#include <QThread>
#include <atomic>

namespace {

void checkPolls(SmokeDriver &d) {
    Poller *poller = Poller::instance();
    d.check(QStringLiteral("qp0: the poller exists and runs with a device"), poller && poller->running());
    if (!poller) return;
    const int meters0 = poller->counts(poller->meterPoll()).completed;
    d.check(QStringLiteral("qp0: the meter poll reads"),
            d.waitFor(3000, [&]() { return poller->counts(poller->meterPoll()).completed >= meters0 + 5; }));
    d.check(QStringLiteral("qp0: the meters carry a peak per channel"), poller->peaks().size() >= 4);
    d.check(QStringLiteral("qp0: the status poll reads"),
            d.waitFor(3000, [&]() { return poller->counts(poller->statusPoll()).completed >= 1; }));

    // A poll every 10 ms whose read takes 80 ms: ticks are skipped, reads never queue
    std::atomic<int> concurrent{ 0 }, maxConcurrent{ 0 };
    int applied = 0;
    const Poller::Id slow = poller->add(QStringLiteral("qp0 slow"), 10, poller, [&](FfiCore *) {
        const int now = ++concurrent;
        int seen = maxConcurrent.load();
        while (now > seen && !maxConcurrent.compare_exchange_weak(seen, now)) {}
        QThread::msleep(80);
        --concurrent;
        return 1;
    }, [&](int) { ++applied; });
    d.spin(600);
    const Poller::Counts c = poller->counts(slow);
    poller->remove(slow);
    d.waitIdle();
    d.check(QStringLiteral("qp0: a slow poll skips ticks"), c.skipped > 10);
    d.check(QStringLiteral("qp0: a slow poll never has two reads in flight"),
            maxConcurrent.load() == 1 && c.posted - c.completed <= 1);
    d.check(QStringLiteral("qp0: a slow poll's results are applied"), applied >= 3);

    // Over a slow transport every poll runs at 2 Hz at most
    poller->setSlowTransport(true);
    const int before = poller->counts(poller->meterPoll()).completed;
    d.spin(1100);
    const int during = poller->counts(poller->meterPoll()).completed - before;
    poller->setSlowTransport(false);
    d.check(QStringLiteral("qp0: a slow transport polls the meters at 2 Hz"), during >= 1 && during <= 3);
}

void checkNoDevice(SmokeDriver &d) {
    Poller *poller = Poller::instance();
    DeviceModel *device = d.app().device;
    d.app().core->worker()->post([](FfiCore *c) { dspi_disconnect(c); });
    device->refresh();
    d.check(QStringLiteral("qp0: the device closes"), d.waitFor(3000, [&]() { return !device->connected(); }));
    d.check(QStringLiteral("qp0: nothing polls without a device"), !poller->running());
    d.waitIdle();
    const int frozen = poller->counts(poller->meterPoll()).posted;
    d.spin(400);
    d.check(QStringLiteral("qp0: no meter tick without a device"), poller->counts(poller->meterPoll()).posted == frozen);
    d.check(QStringLiteral("qp0: a write without a device is refused"), !device->setChannelName(0, QStringLiteral("x")));
    device->openDemo();
    d.check(QStringLiteral("qp0: the demo device opens again"), d.waitFor(10000, [&]() { return device->connected(); }));
    d.check(QStringLiteral("qp0: polling resumes with the device"),
            d.waitFor(3000, [&]() { return poller->running() && poller->counts(poller->meterPoll()).posted > frozen; }));
    d.waitIdle();
}

void checkCoalescing(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    d.waitIdle();
    const QString before = device->channels()->get(0, QStringLiteral("name")).toString();
    const int run0 = device->settersRun();
    int done = 0;
    QString last;
    // Thirty writes to one target in one go, as a slider drag delivers them
    for (int i = 0; i < 30; ++i) {
        last = QStringLiteral("QP0 Drag %1").arg(i);
        const QByteArray utf8 = last.toUtf8();
        device->write(QStringLiteral("channel name 0"),
                      [utf8](FfiCore *c) { return dspi_set_channel_name(c, 0, utf8.constData()); },
                      [&done](bool) { ++done; });
    }
    d.check(QStringLiteral("qp0: coalesced writes finish"), d.waitFor(5000, [&]() { return done == 30; }));
    const int ran = device->settersRun() - run0;
    d.check(QStringLiteral("qp0: thirty writes to one target run at most twice (ran %1)").arg(ran), ran >= 1 && ran <= 2);
    d.check(QStringLiteral("qp0: the newest value wins"),
            d.waitFor(3000, [&]() { return device->channels()->get(0, QStringLiteral("name")).toString() == last; }));
    // Writes to different targets are not merged
    const int run1 = device->settersRun();
    for (int ch = 0; ch < 2; ++ch) {
        const QByteArray utf8 = device->channels()->get(ch, QStringLiteral("name")).toString().toUtf8();
        device->write(QStringLiteral("channel name %1").arg(ch),
                      [ch, utf8](FfiCore *c) { return dspi_set_channel_name(c, uint8_t(ch), utf8.constData()); });
    }
    d.waitIdle();
    d.check(QStringLiteral("qp0: writes to two targets both run"), device->settersRun() - run1 == 2);
    device->setChannelName(0, before);
    d.waitIdle();
}

void coreSection(SmokeDriver &d) {
    d.check(QStringLiteral("qp0: a device is open"), d.waitFor(10000, [&]() { return d.app().device->connected(); }));
    checkPolls(d);
    checkCoalescing(d);
    checkNoDevice(d);
}

const SmokeDriver::Section core("qp0-core", 1, &coreSection);

} // namespace
