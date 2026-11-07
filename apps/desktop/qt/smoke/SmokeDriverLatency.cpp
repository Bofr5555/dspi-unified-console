// WP QD5: the Output Latency row through the firmware's deferred apply on the demo
// device, the clock diagnostics section, and the C0 and C1 bars.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "OutputLatencyController.h"
#include "StatsController.h"

#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
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

template <class F>
QJsonObject coreJson(F call) {
    QByteArray buf(16 * 1024, '\0');
    const uint32_t needed = call(buf.data(), uint32_t(buf.size()));
    if (needed == 0 || needed > uint32_t(buf.size())) return {};
    buf.truncate(int(needed - 1));
    return QJsonDocument::fromJson(buf).object();
}

constexpr int kPrompt = 0, kDefer = 1, kHold = 2, kDrop = 3;
}

void SmokeDriver::testOutputLatency() {
    section("output latency: the row on the demo device (QD5)");
    auto *b = m_.bridge;
    FfiCore *core = b->core();
    auto *latency = qobject_cast<OutputLatencyController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("outputLatency")).value<QObject *>());
    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    QObject *ctx = settings ? settings->findChild<QObject *>(QStringLiteral("settingsContext")) : nullptr;
    QObject *reason = settings ? settings->findChild<QObject *>(QStringLiteral("saveBarReason")) : nullptr;
    check("QD5: the controller, the window and its context", latency && settings && ctx && reason);
    if (!latency || !settings || !ctx || !reason) return;

    settings->setVisible(true);
    spin(300);
    QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QStringLiteral("global")));
    check("QD5: the demo device has the setting", waitFor(2000, [&]() { return latency->supported(); }));
    QQuickItem *row = findItem(settings->contentItem(), QStringLiteral("latencyRow"));
    check("QD5: the row is on Global Parameters", row && row->isVisible());
    if (!row) { settings->setVisible(false); return; }
    const int shortHeight = settings->height();
    settings->setHeight(1000);
    spin(200);
    settings->grabWindow().save(m_out + QStringLiteral("/settings-latency.png"));
    settings->setHeight(shortHeight);

    // ── Options come from the device ──
    const QJsonObject table = coreJson([core](char *o, uint32_t n) { return dspi_fetch_output_latency_options(core, o, n); });
    const QJsonArray wire = table.value(QLatin1String("value")).toArray();
    const QVariantList options = row->property("options").toList();
    bool same = !wire.isEmpty() && wire.size() == options.size();
    for (int i = 0; same && i < wire.size(); i++)
        same = wire[i].toObject().value(QLatin1String("id")).toInt() == options[i].toMap().value("value").toInt();
    check("QD5: the options are the device's table, in its order", same);
    QStringList texts;
    for (const QVariant &o : options) texts << o.toMap().value("text").toString();
    printf("smoke: QD5 options: %s\n", qPrintable(texts.join(QStringLiteral(", "))));
    check("QD5: the labels are the table's values", options.size() == 7 && texts[0] == QLatin1String("0.67 ms") && texts[1] == QLatin1String("1 ms") && texts[4] == QLatin1String("4 ms (default)"));

    const auto stored = [core]() -> QMap<QString, QString> {
        QMap<QString, QString> m;
        const QJsonObject o = coreJson([core](char *c, uint32_t n) { return dspi_fetch_output_latency(core, c, n); });
        for (const QJsonValue &v : o.value(QLatin1String("fields")).toArray())
            m.insert(v.toObject().value(QLatin1String("name")).toString(), v.toObject().value(QLatin1String("value")).toString());
        return m;
    };
    const auto dirty = [ctx]() { return ctx->property("dirty").toBool(); };
    const auto shown = [row]() { return row->property("value").toInt(); };
    const auto sets = [core]() { return dspi_demo_latency_sets(core); };
    const auto save = [&]() { QMetaObject::invokeMethod(ctx, "save"); };
    const auto waitDone = [&](int ms) { return waitFor(ms, [latency]() { return !latency->applying(); }); };
    const auto timing = [core](int mode, int reads, int blackout) { return dspi_demo_latency_timing(core, uint8_t(mode), uint8_t(reads), uint8_t(blackout)); };
    check("QD5: the row shows what is stored", shown() == stored().value("requested_id").toInt() && shown() == 5);
    check("QD5: nothing to save at first", !dirty());

    // ── A draft until Save ──
    int before = sets();
    latency->edit(3);
    check("QD5: a choice shows on the row", shown() == 3);
    check("QD5: and is a draft", dirty() && stored().value("requested_id") == QLatin1String("5") && sets() == before);
    QMetaObject::invokeMethod(ctx, "revert");
    check("QD5: revert drops it", shown() == 5 && !dirty() && sets() == before);

    // ── On the worker, off the UI thread, through the one apply path ──
    std::atomic<bool> offUiThread { true };
    QThread *const ui = QThread::currentThread();
    latency->setWriteHook([&]() {
        if (QThread::currentThread() == ui) offUiThread = false;
        return true;
    });

    // ── Applied at once ──
    latency->edit(3);
    save();
    check("QD5: save finishes (prompt)", waitDone(2000));
    check("QD5: the device has it", stored().value("requested_id") == QLatin1String("3"));
    check("QD5: the row and the save bar settle", shown() == 3 && !dirty() && !latency->dirty());
    check("QD5: the write went once", sets() == before + 1);
    check("QD5: the write ran off the UI thread", offUiThread.load());

    // ── Deferred, as the firmware's main loop does it: the row must not snap back ──
    check("QD5: demo timing, deferred", timing(kDefer, 5, 0));
    before = sets();
    latency->edit(6);
    save();
    check("QD5: applying is on", latency->applying());
    int snapBacks = 0, samples = 0;
    bool sawPending = false;
    QElapsedTimer ran;
    ran.start();
    const bool settled = waitFor(4000, [&]() {
        samples++;
        if (shown() != 6) snapBacks++;
        return !latency->applying();
    });
    check("QD5: save finishes (deferred)", settled);
    printf("smoke: QD5 deferred apply took %lld ms over %d samples, %d snap-backs\n", ran.elapsed(), samples, snapBacks);
    check("QD5: it waited for the apply", ran.elapsed() >= 500);
    check("QD5: the row never showed the old value", samples > 1 && snapBacks == 0);
    check("QD5: the device has it after the wait", stored().value("requested_id") == QLatin1String("6") && shown() == 6 && !dirty());
    check("QD5: the deferred write went once", sets() == before + 1);

    // ── Through the flash window: stalled status reads end neither the wait nor the write ──
    check("QD5: demo timing, blacked out", timing(kDefer, 1, 3));
    before = sets();
    latency->edit(2);
    save();
    check("QD5: save finishes (status stalls)", waitDone(4500));
    check("QD5: the stalls did not fail it", stored().value("requested_id") == QLatin1String("2") && !dirty() && reason->property("text").toString().isEmpty());
    check("QD5: the write went once through the stalls", sets() == before + 1);

    // ── Held: the wait is bounded and nothing is sent again ──
    check("QD5: demo timing, held", timing(kHold, 0, 0));
    before = sets();
    latency->edit(4);
    save();
    sawPending = waitFor(1000, [&]() { return stored().value("pending") == QLatin1String("1"); });
    check("QD5: the device reports pending, with the old id", sawPending && stored().value("requested_id") == QLatin1String("2"));
    check("QD5: the row keeps the choice meanwhile", shown() == 4 && latency->applying());
    ran.restart();
    const bool ended = waitDone(6000);
    printf("smoke: QD5 held apply: wait ended after %lld more ms\n", ran.elapsed());
    check("QD5: the wait ends by itself", ended);
    spin(100);
    QString shownReason = reason->property("text").toString();
    printf("smoke: QD5 save bar shows: %s\n", qPrintable(shownReason));
    check("QD5: the save bar says so", reason->property("visible").toBool() && shownReason.startsWith(QLatin1String("Not saved: ")));
    check("QD5: the draft stays", shown() == 4 && dirty());
    check("QD5: nothing was sent again", sets() == before + 1);
    settings->setHeight(1000);
    spin(200);
    settings->grabWindow().save(m_out + QStringLiteral("/settings-latency-held.png"));
    settings->setHeight(shortHeight);
    // Still held at the one read after the wait: everything stays
    spin(3500);
    check("QD5: still pending at the re-read: the draft and message stay", shown() == 4 && dirty() && latency->requestedId() == 2 && reason->property("text").toString() == shownReason);
    check("QD5: the re-read sent nothing", sets() == before + 1);
    // Save again (the user's choice, so one more SET), and it lands after the wait.
    // A name draft keeps the save bar up, so the message must go by itself.
    latency->edit(4);
    save();
    check("QD5: the wait ends again", waitDone(6000));
    spin(100);
    check("QD5: the message is back", reason->property("text").toString() == shownReason && sets() == before + 2);
    QObject *names = m_.engine->rootContext()->contextProperty(QStringLiteral("deviceNames")).value<QObject *>();
    QMetaObject::invokeMethod(names, "editDevice", Q_ARG(QString, QStringLiteral("Draft")));
    check("QD5: demo timing, released", timing(kPrompt, 0, 0));
    check("QD5: the re-read settles the row by itself", waitFor(4500, [&]() { return !latency->dirty(); }) && shown() == 4 && latency->requestedId() == 4);
    spin(50);
    check("QD5: and clears the message", reason->property("text").toString().isEmpty());
    check("QD5: and sent nothing", sets() == before + 2);
    QMetaObject::invokeMethod(names, "revert");
    check("QD5: no draft left", waitFor(500, [&]() { return !dirty(); }));

    // ── Dropped: ACKed, nothing changes, nothing stays pending ──
    check("QD5: demo timing, dropped", timing(kDrop, 1, 0));
    before = sets();
    latency->edit(7);
    save();
    check("QD5: save finishes (dropped)", waitDone(4500));
    spin(100);
    check("QD5: a dropped request shows in the save bar", reason->property("text").toString().startsWith(QLatin1String("Not saved: ")));
    check("QD5: its draft stays, the device keeps its value", shown() == 7 && dirty() && stored().value("requested_id") == QLatin1String("4"));
    check("QD5: the dropped write went once", sets() == before + 1);
    QMetaObject::invokeMethod(ctx, "revert");
    check("QD5: revert clears it, and the message", shown() == 4 && !dirty() && reason->property("text").toString().isEmpty());
    check("QD5: demo timing, prompt", timing(kPrompt, 0, 0));

    // ── A refusal before the write ──
    before = sets();
    latency->setWriteHook([]() { return false; });
    latency->edit(3);
    save();
    check("QD5: save finishes (refused)", waitDone(2000));
    spin(100);
    check("QD5: a refusal shows, and nothing was sent", reason->property("text").toString().startsWith(QLatin1String("Not saved: ")) && dirty() && sets() == before);
    latency->setWriteHook(nullptr);
    QMetaObject::invokeMethod(ctx, "revert");

    // ── A change of device during the wait ──
    check("QD5: demo timing, held again", timing(kHold, 0, 0));
    latency->edit(7);
    save();
    check("QD5: held on the worker, the UI runs", waitFor(1000, [&]() { return latency->applying(); }));
    spin(300);
    b->disconnectDevice();
    printf("smoke: QD5 no device: connected %d, supported %d, row visible %d\n", b->connected(), latency->supported(), row->isVisible());
    check("QD5: no device: no draft", !dirty() && !latency->dirty());
    check("QD5: no device: no section", !latency->supported() && !row->isVisible());
    b->rescan();
    check("QD5: a fresh demo device comes back", b->connected());
    check("QD5: the wait stopped at the change", waitDone(1000));
    const bool back = waitFor(2000, [&]() { return latency->supported() && latency->shownId() == 5; });
    printf("smoke: QD5 new device: supported %d, shown %d, requested %d\n", latency->supported(), latency->shownId(), latency->requestedId());
    check("QD5: the controller shows the new device's value", back);
    check("QD5: the old device's choice did not reach the new one", stored().value("requested_id") == QLatin1String("5") && sets() == 0 && !dirty());
    // The page went with the device; the row comes back with it
    QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QStringLiteral("global")));
    spin(300);
    QQuickItem *again = findItem(settings->contentItem(), QStringLiteral("latencyRow"));
    check("QD5: the row is back, on the new device's value", again && again->isVisible() && again->property("value").toInt() == 5);
    settings->setVisible(false);
}

void SmokeDriver::testClockDiagAndCpuBars() {
    section("clock diagnostics, C0 and C1 bars (QD5)");
    auto *st = m_.stats;
    auto *b = m_.bridge;

    QQuickWindow *statsWindow = nullptr;
    QQuickItem *clock = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>()) {
        if (QQuickItem *found = findItem(w->contentItem(), QStringLiteral("clockDiagSection"))) {
            statsWindow = w;
            clock = found;
        }
    }
    check("QD5: the section exists", statsWindow && clock);
    if (!statsWindow || !clock) return;
    statsWindow->setVisible(true);
    spin(300);

    // The real read: the demo stalls this opcode, as firmware without it does
    FfiCore *core = b->core();
    const QJsonObject real = coreJson([core](char *o, uint32_t n) { return dspi_fetch_clock_diag(core, o, n); });
    check("QD5: the demo stalls the request", real.value(QLatin1String("ok")).toBool() && !real.value(QLatin1String("available")).toBool());
    check("QD5: stats polls", waitFor(3000, [st]() { return !st->info().isEmpty(); }));
    spin(2300);     // past a slow poll, and its worker's reply
    check("QD5: a stall hides the section", st->clockDiag().isEmpty() && !clock->isVisible());

    // A device with them, through the same parsing: a stall afterwards hides it again
    std::atomic<int> replyKind { 1 };
    std::atomic<bool> offUiThread { true };
    QThread *const ui = QThread::currentThread();
    st->setClockDiagHook([&]() -> QByteArray {
        if (QThread::currentThread() == ui) offUiThread = false;
        if (replyKind == 0) return R"({"ok":true,"available":false,"raw":[],"fields":[]})";
        QJsonArray fields;
        const auto add = [&fields](const char *name, const char *value) {
            fields.append(QJsonObject { { "name", name }, { "value", value } });
        };
        add("pll_lock", "1");
        add("sysclk_last_hz", "307200000");
        add("sysclk_min_hz", "307199000");
        add("sysclk_max_hz", "307201000");
        if (replyKind == 2) {
            // An older packet stops early; the rows it lacks stay out
            return QJsonDocument(QJsonObject { { "ok", true }, { "available", true }, { "fields", fields } }).toJson(QJsonDocument::Compact);
        }
        add("vcxo_last_cppm", "-125");
        add("vcxo_min_cppm", "-200");
        add("vcxo_max_cppm", "50");
        add("vcxo_parked", "0");
        add("servo_total_cppm", "130");
        add("loop_gapmax_us", "412");
        add("fb_q16", "3145728");
        return QJsonDocument(QJsonObject { { "ok", true }, { "available", true }, { "fields", fields } }).toJson(QJsonDocument::Compact);
    });
    check("QD5: a device with them shows the section", waitFor(3000, [&]() { return !st->clockDiag().isEmpty() && clock->isVisible(); }));
    const auto value = [st](const char *title) {
        for (const QVariant &v : st->clockDiag())
            if (v.toMap().value("title").toString() == QLatin1String(title)) return v.toMap().value("value").toString();
        return QStringLiteral("<no row>");
    };
    printf("smoke: QD5 clock rows: %s / %s / %s / %s / %s\n", qPrintable(value("PLL")), qPrintable(value("System Clock")),
           qPrintable(value("VCXO Trim")), qPrintable(value("Longest Loop Gap")), qPrintable(value("USB Feedback")));
    check("QD5: the rows are decoded", value("PLL") == QLatin1String("Locked") && value("System Clock") == QLatin1String("307.200 MHz")
          && value("VCXO Trim") == QLatin1String("-1.25 ppm") && value("Servo Output") == QLatin1String("+1.30 ppm")
          && value("USB Feedback") == QLatin1String("48.0000 samples/frame"));
    check("QD5: the read ran off the UI thread", offUiThread.load());
    spin(100);
    statsWindow->grabWindow().save(m_out + QStringLiteral("/window-stats-clock.png"));
    replyKind = 2;
    check("QD5: an older packet leaves its rows out", waitFor(3000, [&]() { return value("VCXO Trim") == QLatin1String("<no row>") && value("PLL") == QLatin1String("Locked"); }));
    replyKind = 0;
    check("QD5: a stall later hides it again", waitFor(3000, [&]() { return st->clockDiag().isEmpty() && !clock->isVisible(); }));
    replyKind = 1;
    check("QD5: and it comes back", waitFor(3000, [&]() { return !st->clockDiag().isEmpty(); }));
    b->disconnectDevice();
    check("QD5: no device, no section", st->clockDiag().isEmpty() && !clock->isVisible());
    b->rescan();
    check("QD5: back on the demo device", b->connected());
    st->setClockDiagHook(nullptr);
    statsWindow->setVisible(false);
    spin(200);      // the last hooked read is done before its captures go

    // ── Both bars, each with its own core's load ──
    auto *main = qobject_cast<QQuickWindow *>(root());
    QQuickItem *c0 = main ? findItem(main->contentItem(), QStringLiteral("cpuCore0")) : nullptr;
    QQuickItem *c1 = main ? findItem(main->contentItem(), QStringLiteral("cpuCore1")) : nullptr;
    check("QD5: both bars exist and show", c0 && c1 && c0->isVisible() && c1->isVisible());
    if (!c0 || !c1) return;
    check("QD5: the bars follow the bridge", c0->property("load").toInt() == b->cpu0() && c1->property("load").toInt() == b->cpu1());
    QQuickItem *section = c0->parentItem();
    section->setProperty("cpu0", 25);
    section->setProperty("cpu1", 75);
    spin(50);
    printf("smoke: QD5 bars: C0 %s (%g of %g px), C1 %s (%g of %g px), section %g wide\n",
           qPrintable(c0->property("valueText").toString()), c0->property("fillWidth").toReal(), c0->property("trackWidth").toReal(),
           qPrintable(c1->property("valueText").toString()), c1->property("fillWidth").toReal(), c1->property("trackWidth").toReal(), section->width());
    check("QD5: each bar shows its own core", c0->property("valueText").toString() == QLatin1String("25%") && c1->property("valueText").toString() == QLatin1String("75%"));
    const qreal track = c0->property("trackWidth").toReal();
    check("QD5: each fill is its core's share", track > 0 && qFuzzyCompare(c0->property("fillWidth").toReal(), track * 0.25) && qFuzzyCompare(c1->property("fillWidth").toReal(), track * 0.75));
    const auto noOverlap = [&]() { return c0->x() + c0->width() <= c1->x(); };
    check("QD5: the bars don't overlap", noOverlap());
    const qreal wide = section->width();
    section->setWidth(220);     // the narrowest sidebar
    spin(50);
    check("QD5: nor in the narrowest sidebar", noOverlap() && c0->x() >= 0 && c1->x() + c1->width() <= 220);
    section->setWidth(wide);
    main->grabWindow().save(m_out + QStringLiteral("/window-main-cpu-bars.png"));

    // Back to the bridge, for what runs after this
    QQmlExpression rebind(m_.engine->rootContext(), section, QStringLiteral(
        "cpu0 = Qt.binding(function() { return bridge.cpu0 }); cpu1 = Qt.binding(function() { return bridge.cpu1 })"));
    rebind.evaluate();
    check("QD5: the bars are back on the bridge", !rebind.hasError() && section->property("cpu0").toInt() == b->cpu0());
}
