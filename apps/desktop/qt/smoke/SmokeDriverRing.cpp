// WP QD29: the Output Ring Health section and the ring-scaled fill rows on the demo
// device, and through hooks what the demo never says.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "StatsController.h"

#include <QColor>
#include <QElapsedTimer>
#include <QQuickItem>
#include <QQuickWindow>
#include <QThread>
#include <atomic>
#include <cstdio>

namespace {
QQuickItem *findItem(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findItem(c, name)) return found;
    return nullptr;
}

constexpr int kReal = 0, kFullValues = 1, kNoStatus = 2, kTransferError = 3;
constexpr int kFillsGood = 0, kFillsWarn = 1, kFillsBad = 2, kFillsError = 3;
}

void SmokeDriver::testRingHealth() {
    section("output ring health (QD29)");
    auto *st = m_.stats;
    auto *b = m_.bridge;
    FfiCore *core = b->core();

    QQuickWindow *statsWindow = nullptr;
    QQuickItem *ring = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>()) {
        if (QQuickItem *found = findItem(w->contentItem(), QStringLiteral("ringHealthSection"))) {
            statsWindow = w;
            ring = found;
        }
    }
    check("QD29: the section exists", statsWindow && ring);
    if (!statsWindow || !ring) return;
    statsWindow->setVisible(true);
    spin(300);

    const auto rowValue = [&](const char *name) {
        QQuickItem *row = findItem(ring, QLatin1String(name));
        return row ? row->property("value").toString() : QStringLiteral("<no row>");
    };
    const auto rowColor = [&](const char *name) {
        QQuickItem *row = findItem(ring, QLatin1String(name));
        return row ? row->property("valueColor").value<QColor>() : QColor();
    };
    const auto spdifRow = [st](int i) { return st->buffers().value("spdif").toList().value(i).toMap(); };
    const auto fillColor = [&]() {
        QQuickItem *row = findItem(statsWindow->contentItem(), QStringLiteral("outputFillRow0"));
        return row ? row->property("fillColor").value<QColor>() : QColor();
    };
    const QColor green = statsWindow->property("green").value<QColor>();
    const QColor yellow = statsWindow->property("yellow").value<QColor>();
    const QColor orange = statsWindow->property("orange").value<QColor>();
    const QColor red = statsWindow->property("red").value<QColor>();

    // ── The real reads, on the demo device ──
    check("QD29: the demo shows the section", waitFor(3000, [&]() { return !st->ringHealth().isEmpty() && ring->isVisible(); }));
    printf("smoke: QD29 demo: %s / %s / %s\n", qPrintable(rowValue("ringFillTarget")), qPrintable(rowValue("ringLowestFill")), qPrintable(rowValue("ringUsbGap")));
    check("QD29: the target is the 4 ms default", rowValue("ringFillTarget").endsWith(QLatin1String("frames (4 ms)")));
    check("QD29: no write yet, no lowest fill", rowValue("ringLowestFill") == QLatin1String("-"));
    check("QD29: the demo's fills sit in the good band",
          waitFor(2000, [&]() { return spdifRow(0).value("band").toString() == QLatin1String("good") && fillColor() == green; }));

    // ── 0.67 ms, where the old percentages read as an empty ring ──
    QVariantMap at067;
    // Stream-only: USB would run the first option without the flag, 2 ms
    check("QD29: a stream input", dspi_set_input_source(core, 1));
    check("QD29: the demo takes 0.67 ms", dspi_set_output_latency(core, 1));
    const bool settled = waitFor(6000, [&]() {
        at067 = spdifRow(0);
        return rowValue("ringFillTarget").endsWith(QLatin1String("(0.67 ms)")) && at067.value("fill").toInt() < 25;
    });
    printf("smoke: QD29 at 0.67 ms: %s, row fill %d%% band %s\n", qPrintable(rowValue("ringFillTarget")),
           at067.value("fill").toInt(), qPrintable(at067.value("band").toString()));
    check("QD29: the section follows the latency", settled);
    BufferStats raw {};
    check("QD29: the old percentage reads empty there", dspi_fetch_buffer_stats(core, &raw) && raw.spdif[0].fill_pct == 0);
    check("QD29: the row is on the ring's scale, not empty", at067.value("fill").toInt() == 6);
    check("QD29: and is good, by the live target", at067.value("band").toString() == QLatin1String("good")
          && waitFor(500, [&]() { return fillColor() == green; }));
    check("QD29: the trace is on the same scale", waitFor(500, [&]() { return st->history(0).back() == 6; }));
    statsWindow->grabWindow().save(m_out + QStringLiteral("/window-stats-ring-067.png"));
    check("QD29: back to USB and 4 ms", dspi_set_input_source(core, 0) && dspi_set_output_latency(core, 5));
    check("QD29: the section follows back", waitFor(6000, [&]() { return rowValue("ringFillTarget").endsWith(QLatin1String("(4 ms)")); }));

    // ── Hooks: every value, no status, a failed transfer ──
    std::atomic<int> health { kFullValues };
    std::atomic<int> fills { kFillsGood };
    std::atomic<int> healthReads { 0 };
    std::atomic<bool> offUiThread { true };
    QThread *const ui = QThread::currentThread();
    st->setRingHealthHook([&]() -> QByteArray {
        if (QThread::currentThread() == ui) offUiThread = false;
        healthReads++;
        switch (health.load()) {
        case kNoStatus: return R"({"ok":true,"value":null})";
        case kTransferError: return R"({"ok":false,"code":-1,"error":"transfer"})";
        default:
            return R"({"ok":true,"targetFrames":32,"targetUs":667,"batchFrames":48,"underrunEpisodes":3,"refusedBlocks":2,)"
                   R"("lowestFill":17,"worstInputGapUs":812,"worstUsbGapUs":12500,"overBudgetSteps":1})";
        }
    });
    std::atomic<int> tickReads { 0 };       // -- WP CG21 --
    st->setOutputFillsHook([&]() -> QByteArray {
        if (QThread::currentThread() == ui) offUiThread = false;
        tickReads++;
        switch (fills.load()) {
        case kFillsWarn: return R"({"ok":true,"value":[{"fillPct":70.4,"minPct":12,"maxPct":71,"band":"warn"}]})";
        case kFillsBad: return R"({"ok":true,"value":[{"fillPct":0.0,"minPct":0,"maxPct":71,"band":"bad"}]})";
        case kFillsError: return R"({"ok":false,"code":-1,"error":"transfer"})";
        default: return R"({"ok":true,"value":[{"fillPct":6.25,"minPct":5,"maxPct":8,"band":"good"}]})";
        }
    });
    check("QD29: a device's values show", waitFor(3000, [&]() { return rowValue("ringFillTarget") == QLatin1String("32 frames (0.67 ms)"); }));
    printf("smoke: QD29 rows: %s, %s, %s, %s, %s, %s, %s\n", qPrintable(rowValue("ringFillTarget")), qPrintable(rowValue("ringUnderruns")),
           qPrintable(rowValue("ringRefused")), qPrintable(rowValue("ringLowestFill")), qPrintable(rowValue("ringInputGap")),
           qPrintable(rowValue("ringUsbGap")), qPrintable(rowValue("ringOverBudget")));
    check("QD29: counts and frames", rowValue("ringUnderruns") == QLatin1String("3") && rowValue("ringRefused") == QLatin1String("2")
          && rowValue("ringLowestFill") == QLatin1String("17 frames") && rowValue("ringOverBudget") == QLatin1String("1"));
    check("QD29: gaps in µs below 10 ms, in ms above", rowValue("ringInputGap") == QString::fromUtf8("812 µs") && rowValue("ringUsbGap") == QLatin1String("12.5 ms"));
    check("QD29: counts above 0 colour their row", rowColor("ringUnderruns") == red && rowColor("ringRefused") == orange && rowColor("ringOverBudget") == orange);
    check("QD29: the reads ran off the UI thread", offUiThread.load());
    spin(100);
    statsWindow->grabWindow().save(m_out + QStringLiteral("/window-stats-ring.png"));

    // A failed read says nothing about the rings
    health = kTransferError;
    const int beforeFailed = healthReads;
    check("QD29: a failed read happens", waitFor(5000, [&]() { return healthReads > beforeFailed + 1; }));
    spin(100);
    check("QD29: and keeps the last values", ring->isVisible() && rowValue("ringUnderruns") == QLatin1String("3"));

    // A reset's watermarks show without waiting for the slow poll
    const int beforeReset = healthReads;
    st->resetWatermarks();
    check("QD29: a reset reads the health again", waitFor(300, [&]() { return healthReads > beforeReset; }));

    health = kNoStatus;
    check("QD29: no status hides the section", waitFor(3000, [&]() { return st->ringHealth().isEmpty() && !ring->isVisible(); }));
    health = kFullValues;
    check("QD29: and it comes back", waitFor(3000, [&]() { return ring->isVisible(); }));

    // ── The band comes from the core, whatever the percentage alone would say ──
    fills = kFillsWarn;     // 70 % is good by the old fixed bands
    check("QD29: a warn band shows yellow", waitFor(1000, [&]() { return spdifRow(0).value("fill").toInt() == 70 && fillColor() == yellow; }));
    check("QD29: the row has the read's watermarks", spdifRow(0).value("min").toInt() == 12 && spdifRow(0).value("max").toInt() == 71);
    fills = kFillsBad;
    check("QD29: a bad band shows red", waitFor(1000, [&]() { return spdifRow(0).value("fill").toInt() == 0 && fillColor() == red; }));
    fills = kFillsGood;
    check("QD29: a good band shows green", waitFor(1000, [&]() { return spdifRow(0).value("fill").toInt() == 6 && fillColor() == green; }));
    fills = kFillsError;
    spin(300);
    check("QD29: a failed fill read keeps the last", spdifRow(0).value("fill").toInt() == 6 && fillColor() == green);
    const QVariantList outputs = st->buffers().value("spdif").toList();
    check("QD29: outputs past the reply keep the buffer read's numbers", outputs.size() > 1 && !outputs[1].toMap().contains("band"));

    // -- WP CG21 -- A tick is one call, whose fills the hook stands in for, and it stays
    // off the UI thread. The transfers it costs are counted on the fake hub (dspi-core).
    fills = kFillsGood;
    const int ticksBefore = tickReads;
    QElapsedTimer ticking;
    ticking.start();
    spin(600);
    const int ticks = tickReads - ticksBefore;
    printf("smoke: CG21: %d reads in %lld ms\n", ticks, (long long)ticking.elapsed());
    check("CG21: at most one read per 60 ms tick", ticks >= 1 && ticks <= int(ticking.elapsed() / 60) + 1);
    check("CG21: the reads ran off the UI thread", offUiThread.load());

    // ── A change of device leaves nothing behind ──
    fills = kFillsGood;
    b->disconnectDevice();
    check("QD29: no device, no section", st->ringHealth().isEmpty() && !ring->isVisible());
    check("QD29: and no fill rows", st->buffers().isEmpty());
    b->rescan();
    check("QD29: back on the demo device", b->connected());
    st->setRingHealthHook(nullptr);
    st->setOutputFillsHook(nullptr);
    spin(200);      // the last hooked read is done before its captures go
    check("QD29: the real reads again", waitFor(3000, [&]() { return rowValue("ringFillTarget").endsWith(QLatin1String("(4 ms)")); }));
    statsWindow->setVisible(false);
    spin(200);
}
