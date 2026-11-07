// WP QD34: the status and Stats counter reads run off the GUI thread. A hook
// makes each read slow; the event loop must keep turning while it runs.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "StatsController.h"

#include <QElapsedTimer>
#include <QEventLoop>
#include <QThread>
#include <QTimer>
#include <atomic>
#include <cstdio>
#include <memory>

namespace {
// Longest gap between 10 ms timer ticks over `ms`: what a blocked GUI thread shows
qint64 longestGap(int ms) {
    QEventLoop loop;
    QElapsedTimer since;
    qint64 gap = 0;
    QTimer tick;
    tick.setInterval(10);
    QObject::connect(&tick, &QTimer::timeout, &loop, [&]() { gap = qMax(gap, since.restart()); });
    QTimer::singleShot(ms, &loop, &QEventLoop::quit);
    since.start();
    tick.start();
    loop.exec();
    return gap;
}

struct SlowHook {
    std::atomic<int> calls { 0 };
    std::atomic<bool> onGui { false };
    QThread *gui = QThread::currentThread();
    void run(int ms) {
        if (QThread::currentThread() == gui) onGui = true;
        calls++;
        QThread::msleep(ms);
    }
};
}

void SmokeDriver::testPollingOffUiThread() {
    section("polling off the GUI thread (QD34)");
    auto *b = m_.bridge;
    auto *st = m_.stats;
    constexpr int kSlowMs = 400;   // a blocking read would show a gap at least this long

    // The bridge's 60 ms status read
    auto bridgeHook = std::make_shared<SlowHook>();
    b->setStatusReadHook([bridgeHook]() { bridgeHook->run(kSlowMs); });
    const qint64 statusGap = longestGap(1200);
    b->setStatusReadHook({});
    printf("smoke: QD34 status reads %d, longest GUI gap %lld ms\n", bridgeHook->calls.load(), qint64(statusGap));
    check("QD34: the status read runs", bridgeHook->calls >= 2);
    check("QD34: on the worker, not the GUI thread", !bridgeHook->onGui);
    check("QD34: a slow status read leaves the GUI turning", statusGap < kSlowMs - 150);
    // Ticks keep coming while a read runs, and one read at a time: about 1200 / 400
    check("QD34: a tick still reading skips the next", bridgeHook->calls <= 1200 / kSlowMs + 1);
    const int before = bridgeHook->calls;
    spin(400);   // the read in flight ends; the hook is gone from the next one
    check("QD34: the hook is gone with the next read", bridgeHook->calls == before);
    check("QD34: the status still polls", waitFor(1000, [b]() { return b->cpu0() >= 0; }));

    // The Stats window's 2 s counter reads
    st->setWatching(false);
    auto statsHook = std::make_shared<SlowHook>();
    st->setSlowReadHook([statsHook]() { statsHook->run(kSlowMs); });
    QElapsedTimer t;
    t.start();
    st->setWatching(true);
    const qint64 opened = t.elapsed();
    check("QD34: opening Stats does not wait for its reads", opened < kSlowMs - 150);
    check("QD34: the counter read ran", waitFor(2000, [statsHook]() { return statsHook->calls >= 1; }));
    const qint64 statsGap = longestGap(2400);   // takes in the next 2 s read
    printf("smoke: QD34 counter reads %d, opened in %lld ms, longest GUI gap %lld ms\n", statsHook->calls.load(),
           qint64(opened), qint64(statsGap));
    check("QD34: on the worker", !statsHook->onGui);
    check("QD34: a slow counter read leaves the GUI turning", statsGap < kSlowMs - 150);
    check("QD34: the counters arrive", waitFor(1000, [st]() { return !st->info().isEmpty(); }));
    check("QD34: the fills arrive", waitFor(1000, [st]() { return !st->buffers().isEmpty(); }));
    st->setWatching(false);
    st->setSlowReadHook({});
    spin(kSlowMs + 100);
}
