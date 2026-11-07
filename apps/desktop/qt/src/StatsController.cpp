#include "StatsController.h"
#include "DSPiBridge.h"

#include <QDateTime>
#include <QHash>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRunnable>
#include <QPainter>
#include <QPainterPath>
#ifdef Q_OS_MACOS
#include "MacSystemColors.h"
#endif
#include <algorithm>

namespace {
QString inputSourceName(int s) {
    switch (s) {
    case 1: return "S/PDIF 1";
    case 2: return "I2S";
    case 3: return "ADAT";
    case 4: return "S/PDIF 2";
    case 5: return "S/PDIF 3";
    case 6: return "S/PDIF 4";
    default: return "USB";
    }
}

QString categoryName(quint8 c) {
    switch (c) {
    case 0x00: return "General";
    case 0x01: return "CD Player";
    case 0x02: return "DAT";
    case 0x03: return "DCC";
    case 0x04: return "MiniDisc";
    case 0x06: return "Synthesizer";
    case 0x08: return "Broadcast Receiver";
    case 0x09: return "Musical Instrument";
    case 0x0A: return "A/D Converter";
    case 0x0C: return "Mixer";
    case 0x0D: return "Rate Converter";
    case 0x0E: return "Sampler";
    case 0x0F: return "Digital Signal Processor";
    default: return QString("0x%1").arg(c, 2, 16, QChar('0')).toUpper().replace("0X", "0x");
    }
}

QString wordLengthName(quint8 b) {
    switch (b & 0x0F) {
    case 0x00: return "Not indicated";
    case 0x02: return "16-bit";
    case 0x04: return "20-bit";
    case 0x08: return "17-bit";
    case 0x0A: return "22-bit";
    case 0x0B: return "24-bit";
    default: return "-";
    }
}

// -- WP QD5 --
QString mhz(qint64 hz) { return QString::number(hz / 1e6, 'f', 3) + " MHz"; }
QString ppm(qint64 centiPpm) {
    return (centiPpm > 0 ? "+" : "") + QString::number(centiPpm / 100.0, 'f', 2) + " ppm";
}

// The packet is a bench instrument of some 90 fields (clock_diag.h); these answer
// "is the clock steady, and is the loop tracking?". A field an older packet lacks
// leaves its row out.
QVariantList clockDiagRows(const QJsonObject &reply) {
    QHash<QString, QString> f;
    const QJsonArray fields = reply.value(QLatin1String("fields")).toArray();
    for (const QJsonValue &v : fields) {
        const QJsonObject o = v.toObject();
        f.insert(o.value(QLatin1String("name")).toString(), o.value(QLatin1String("value")).toString());
    }
    QVariantList rows;
    const auto row = [&rows](const QString &title, const QString &value) {
        rows.append(QVariantMap { { "title", title }, { "value", value } });
    };
    const auto has = [&f](const char *name) { return f.contains(QLatin1String(name)); };
    const auto num = [&f](const char *name) { return f.value(QLatin1String(name)).toLongLong(); };

    if (has("pll_lock")) row("PLL", num("pll_lock") ? "Locked" : "Unlocked");
    if (has("sysclk_last_hz")) row("System Clock", mhz(num("sysclk_last_hz")));
    if (has("sysclk_min_hz") && has("sysclk_max_hz")) row("System Clock Range", mhz(num("sysclk_min_hz")) + " – " + mhz(num("sysclk_max_hz")));
    if (has("vcxo_parked") && num("vcxo_parked")) row("VCXO Trim", "Parked");
    else if (has("vcxo_last_cppm")) row("VCXO Trim", ppm(num("vcxo_last_cppm")));
    if (has("vcxo_min_cppm") && has("vcxo_max_cppm")) row("VCXO Range", ppm(num("vcxo_min_cppm")) + " – " + ppm(num("vcxo_max_cppm")));
    if (has("servo_total_cppm")) row("Servo Output", ppm(num("servo_total_cppm")));
    if (has("loop_gapmax_us")) row("Longest Loop Gap", QString::number(num("loop_gapmax_us")) + " µs");
    if (has("fb_q16")) row("USB Feedback", QString::number(num("fb_q16") / 65536.0, 'f', 4) + " samples/frame");
    return rows;
}

// -- WP QD29 --
// A JSON read, or what the smoke driver gave instead. Runs on the worker.
QJsonObject readJson(const std::function<QByteArray()> &hook, uint32_t (*read)(FfiCore *, char *, uint32_t), FfiCore *core) {
    if (hook) return QJsonDocument::fromJson(hook()).object();
    QByteArray buf(4096, '\0');
    const uint32_t needed = read(core, buf.data(), uint32_t(buf.size()));
    if (needed == 0 || needed > uint32_t(buf.size())) return {};
    return QJsonDocument::fromJson(buf.left(int(needed - 1))).object();
}

// A fill row's levels. The trace reads `fill` too, where 0xFF means absent;
// firmware without the ring's scale can pass 100, so the value is held below that.
QVariantList outputFillRows(const QJsonArray &fills) {
    QVariantList rows;
    for (const QJsonValue &v : fills) {
        const QJsonObject o = v.toObject();
        rows.append(QVariantMap {
            { "fill", qBound(0, qRound(o.value(QLatin1String("fillPct")).toDouble()), StatsController::Absent - 1) },
            { "min", o.value(QLatin1String("minPct")).toInt() },
            { "max", o.value(QLatin1String("maxPct")).toInt() },
            { "band", o.value(QLatin1String("band")).toString() },
        });
    }
    return rows;
}
}

StatsController::StatsController(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge), m_core(bridge->core())
{
    for (auto &h : m_history) h.fill(Absent);
    m_worker.setMaxThreadCount(1);
    m_slow.setInterval(2000);
    m_fast.setInterval(60);
    connect(&m_slow, &QTimer::timeout, this, &StatsController::pollSlow);
    connect(&m_fast, &QTimer::timeout, this, &StatsController::pollFast);
    connect(m_bridge, &DSPiBridge::devicesChanged, this, &StatsController::onDeviceChanged);
    // -- WP QD5 -- A disconnect comes as stateChanged only: no rows of a device that's gone
    connect(m_bridge, &DSPiBridge::stateChanged, this, &StatsController::onDeviceChanged);
    onDeviceChanged();
}

StatsController::~StatsController() {
    m_generation++;
    m_worker.waitForDone();
}

// Counts reconnections after the first connection this session
void StatsController::onDeviceChanged() {
    // -- WP QD6 -- Read-only firmware is still read
    const QString serial = m_bridge->connected() ? m_bridge->selectedSerial() : QString();
    if (serial == m_serial) return;
    m_serial = serial;
    // Nothing carries over from the previous device
    m_haveStarvation = false;
    m_lgSupported = false;
    m_lastEvent = m_previousEvent = 0;
    for (auto &h : m_history) h.fill(Absent);
    m_buffers.clear();
    emit buffersChanged();
    m_generation++;
    setClockDiag({});
    // -- WP QD29 --
    setRingHealth({});
    m_outputFills.clear();
    m_outputFillsAnswered = false;
    if (!serial.isEmpty()) {
        if (m_seenDevice) m_reconnects++;
        m_seenDevice = true;
    }
    updateTimers();
    if (m_watching) { pollSlow(); pollFast(); }
}

void StatsController::setWatching(bool w) {
    if (w == m_watching) return;
    m_watching = w;
    emit watchingChanged();
    // Starvations while the window was closed have no time: start counting again
    if (w) m_haveStarvation = false;
    updateTimers();
    if (w) { pollSlow(); pollFast(); }
}

void StatsController::updateTimers() {
    const bool on = m_watching && !m_serial.isEmpty();
    if (on) { if (!m_slow.isActive()) m_slow.start(); if (!m_fast.isActive()) m_fast.start(); }
    else { m_slow.stop(); m_fast.stop(); }
}

void StatsController::pollSlow() {
    QVariantMap m;
    if (m_serial.isEmpty()) {
        if (!m_info.isEmpty()) { m_info.clear(); emit infoChanged(); }
        return;
    }
    auto stat = [this](uint16_t which) -> QVariant {
        uint32_t v = 0;
        return dspi_fetch_stat(m_core, which, &v) ? QVariant(qulonglong(v)) : QVariant();
    };
    QVariant clock = stat(STAT_CLOCK_HZ), mv = stat(STAT_CORE_MV), rate = stat(STAT_SAMPLE_RATE), temp = stat(STAT_TEMPERATURE);
    if (clock.isValid()) m["clockMHz"] = clock.toDouble() / 1e6;
    if (mv.isValid()) m["coreVolts"] = mv.toDouble() / 1000.0;
    if (rate.isValid()) m["sampleKHz"] = rate.toDouble() / 1000.0;
    if (temp.isValid()) m["tempC"] = qint16(quint16(temp.toULongLong() & 0xFFFF)) / 100.0;
    m["usbRingOver"] = stat(STAT_USB_RING_OVERRUNS);
    m["spdifOver"] = stat(STAT_SPDIF_OVERRUNS);
    m["spdifUnder"] = stat(STAT_SPDIF_UNDERRUNS);
    m["pdmRingOver"] = stat(STAT_PDM_RING_OVERRUNS);
    m["pdmRingUnder"] = stat(STAT_PDM_RING_UNDERRUNS);
    m["pdmDmaOver"] = stat(STAT_PDM_DMA_OVERRUNS);
    m["pdmDmaUnder"] = stat(STAT_PDM_DMA_UNDERRUNS);

    // S/PDIF DMA starvation: the delta since the last poll, and when
    uint32_t total = 0;
    if (dspi_fetch_stat(m_core, STAT_STARVATION_TOTAL, &total)) {
        quint32 delta = 0;
        if (m_haveStarvation) delta = total >= m_starvationTotal ? total - m_starvationTotal : total;   // reset counts as new
        m_starvationTotal = total;
        m_haveStarvation = true;
        if (delta > 0) {
            m_previousEvent = m_lastEvent;
            m_lastEvent = QDateTime::currentMSecsSinceEpoch();
        }
        QVariantList per;
        for (int i = 0; i < 4; i++) {
            uint32_t v = 0;
            per.append(total > 0 && dspi_fetch_stat(m_core, STAT_STARVATION_FIRST + i, &v) ? qulonglong(v) : 0ull);
        }
        m["starvationTotal"] = qulonglong(total);
        m["starvationDelta"] = qulonglong(delta);
        m["starvationPer"] = per;
    }
    m["lastEventMs"] = m_lastEvent;
    m["previousEventMs"] = m_previousEvent;

    // S/PDIF receiver
    SpdifRxStatus rx {};
    if (dspi_fetch_spdif_rx_status(m_core, &rx)) {
        QVariantMap s;
        s["state"] = rx.state;
        s["source"] = inputSourceName(rx.input_source);
        s["locked"] = rx.state == 2;
        s["rateKHz"] = rx.state == 2 ? rx.sample_rate / 1000.0 : 0.0;
        s["lockCount"] = rx.lock_count;
        s["lossCount"] = rx.loss_count;
        s["parityErrors"] = qulonglong(rx.parity_errors);
        s["fifoPct"] = rx.fifo_fill_pct;
        s["libState"] = rx.lib_state;
        s["stableCallbacks"] = rx.stable_callbacks;
        s["lostCallbacks"] = rx.lost_callbacks;
        int index = rx.input_source == 1 ? 0 : (rx.input_source >= 4 && rx.input_source <= 6) ? rx.input_source - 3 : 0;
        quint8 pin = 0;
        if (dspi_fetch_spdif_rx_pin(m_core, quint8(index), &pin) && pin) s["pin"] = pin;
        quint8 cs[24];
        if (rx.state == 2 && dspi_fetch_spdif_rx_channel_status(m_core, cs)) {
            s["consumer"] = (cs[0] & 0x01) == 0;
            s["pcm"] = (cs[0] & 0x02) == 0;
            s["copyPermitted"] = (cs[0] & 0x04) != 0;
            s["category"] = categoryName(cs[1]);
            s["wordLength"] = wordLengthName(cs[4]);
        }
        m["spdifRx"] = s;
    }

    // LG Sound Sync (shown once the device has answered)
    QVariantMap lg = m_bridge->fetchLgStatus();
    if (!lg.isEmpty()) m_lgSupported = true;
    if (m_lgSupported && !lg.isEmpty()) m["lg"] = lg;

    // ADAT bulk output (RP2350)
    if (m_bridge->platformName() == "RP2350") {
        QVariantMap adat = m_bridge->fetchAdatOutStatus();
        if (!adat.isEmpty()) m["adatOut"] = adat;
    }

    // I2S input as clock slave
    InputLockStatus i2s {};
    if (dspi_fetch_input_lock(m_core, 0, &i2s) && i2s.clock_mode == 1) {
        m["i2sSlave"] = QVariantMap{
            { "state", i2s.state }, { "detectedKHz", i2s.detected_rate / 1000.0 },
            { "measuredKHz", i2s.measured_hz / 1000.0 }, { "lockCount", i2s.lock_count }, { "lossCount", i2s.loss_count },
        };
    }

    if (m != m_info) { m_info = m; emit infoChanged(); }
    pollClockDiag();
    pollRingHealth();   // -- WP QD29 --
}

// -- WP QD5 --
void StatsController::pollClockDiag() {
    if (m_clockDiagBusy) return;
    m_clockDiagBusy = true;
    const unsigned generation = m_generation;
    FfiCore *core = m_core;
    m_worker.start(QRunnable::create([this, core, generation]() {
        QByteArray reply;
        if (m_clockDiagHook) {
            reply = m_clockDiagHook();
        } else {
            QByteArray buf(16 * 1024, '\0');
            const uint32_t needed = dspi_fetch_clock_diag(core, buf.data(), uint32_t(buf.size()));
            if (needed > 0 && needed <= uint32_t(buf.size())) reply = buf.left(int(needed - 1));
        }
        const QJsonObject o = QJsonDocument::fromJson(reply).object();
        // A stall reads as not available: no section
        QVariantList rows;
        if (o.value(QLatin1String("ok")).toBool() && o.value(QLatin1String("available")).toBool())
            rows = clockDiagRows(o);
        QMetaObject::invokeMethod(this, [this, rows, generation]() {
            m_clockDiagBusy = false;
            if (m_generation == generation) setClockDiag(rows);
        }, Qt::QueuedConnection);
    }));
}

void StatsController::setClockDiag(const QVariantList &rows) {
    if (rows == m_clockDiag) return;
    m_clockDiag = rows;
    emit clockDiagChanged();
}

// -- WP QD29 -- Seven transfers: the slow rate. A failed read says nothing about the
// rings, so it keeps the last values; only {"value":null} means no such status.
void StatsController::pollRingHealth() {
    if (m_ringHealthBusy) return;
    m_ringHealthBusy = true;
    const unsigned generation = m_generation;
    m_worker.start(QRunnable::create([this, core = m_core, hook = m_ringHealthHook, generation]() {
        const QJsonObject o = readJson(hook, dspi_fetch_ring_health, core);
        const bool answered = o.value(QLatin1String("ok")).toBool();
        QVariantMap health;
        if (answered && o.contains(QLatin1String("targetFrames"))) {
            health = o.toVariantMap();
            health.remove(QStringLiteral("ok"));
            if (o.value(QLatin1String("lowestFill")).isNull()) health.remove(QStringLiteral("lowestFill"));
        }
        QMetaObject::invokeMethod(this, [this, answered, health, generation]() {
            m_ringHealthBusy = false;
            if (answered && m_generation == generation) setRingHealth(health);
        }, Qt::QueuedConnection);
    }));
}

void StatsController::setRingHealth(const QVariantMap &health) {
    if (health == m_ringHealth) return;
    m_ringHealth = health;
    emit ringHealthChanged();
}

// Two transfers. pollFast merges the result on its next tick. A failed read keeps the
// last fills; with none yet, the rows show the buffer read's own numbers.
void StatsController::pollOutputFills() {
    if (m_outputFillsBusy) return;
    m_outputFillsBusy = true;
    const unsigned generation = m_generation;
    m_worker.start(QRunnable::create([this, core = m_core, hook = m_outputFillsHook, generation]() {
        const QJsonObject o = readJson(hook, dspi_fetch_output_fills, core);
        const bool ok = o.value(QLatin1String("ok")).toBool();
        const QVariantList rows = ok ? outputFillRows(o.value(QLatin1String("value")).toArray()) : QVariantList();
        QMetaObject::invokeMethod(this, [this, ok, rows, generation]() {
            m_outputFillsBusy = false;
            if (m_generation != generation) return;
            m_outputFillsAnswered = true;
            if (ok) m_outputFills = rows;
        }, Qt::QueuedConnection);
    }));
}

void StatsController::pollFast() {
    if (m_serial.isEmpty()) {
        if (!m_buffers.isEmpty()) { m_buffers.clear(); emit buffersChanged(); }
        return;
    }
    // -- WP QD29 -- No rows until the first reply: at the lowest latency the buffer
    // read's own percentages read as an empty ring.
    pollOutputFills();
    if (!m_outputFillsAnswered) return;

    BufferStats b {};
    if (!dspi_fetch_buffer_stats(m_core, &b)) return;
    const auto liveFill = [this, &b](int i) {   // -- WP QD29 --
        return i < m_outputFills.size() ? quint8(m_outputFills[i].toMap().value("fill").toInt()) : b.spdif[i].fill_pct;
    };

    // History: one sample per poll and series, newest at the end
    quint8 sample[SeriesCount];
    for (int i = 0; i < 4; i++) sample[i] = i < b.num_spdif ? liveFill(i) : Absent;
    sample[PdmDma] = b.pdm_active ? b.pdm.dma_fill_pct : Absent;
    sample[PdmRing] = b.pdm_active ? b.pdm.ring_fill_pct : Absent;
    for (int s = 0; s < SeriesCount; s++) {
        std::rotate(m_history[s].begin(), m_history[s].begin() + 1, m_history[s].end());
        m_history[s].back() = sample[s];
    }

    QVariantMap m;
    m["numSpdif"] = b.num_spdif;
    m["pdmActive"] = b.pdm_active;
    m["streaming"] = b.streaming;
    QVariantList rows;
    for (int i = 0; i < b.num_spdif; i++) {
        // -- WP QD29 --
        if (i < m_outputFills.size()) rows.append(m_outputFills[i]);
        else rows.append(QVariantMap{ { "fill", b.spdif[i].fill_pct }, { "min", b.spdif[i].min_pct }, { "max", b.spdif[i].max_pct } });
    }
    m["spdif"] = rows;
    m["pdmDma"] = QVariantMap{ { "fill", b.pdm.dma_fill_pct }, { "min", b.pdm.dma_min_pct }, { "max", b.pdm.dma_max_pct } };
    m["pdmRing"] = QVariantMap{ { "fill", b.pdm.ring_fill_pct }, { "min", b.pdm.ring_min_pct }, { "max", b.pdm.ring_max_pct } };
    m_buffers = m;
    // Every sample moves the traces
    emit buffersChanged();
}

void StatsController::resetWatermarks() {
    if (!m_serial.isEmpty() && m_bridge->writable()) dspi_reset_buffer_stats(m_core);   // -- WP QD6 --
    pollFast();
    if (!m_serial.isEmpty()) pollRingHealth();  // -- WP QD29 -- Its watermarks reset too
}

// ── BufferTraceItem ──

BufferTraceItem::BufferTraceItem(QQuickItem *parent) : QQuickPaintedItem(parent) {
    setAntialiasing(true);
    connect(this, &BufferTraceItem::changed, this, [this]() { update(); });
}

void BufferTraceItem::setController(QObject *c) {
    auto *s = qobject_cast<StatsController *>(c);
    if (s == m_stats) return;
    if (m_stats) disconnect(m_stats, nullptr, this, nullptr);
    m_stats = s;
    if (m_stats) connect(m_stats, &StatsController::buffersChanged, this, [this]() { if (isVisible()) update(); });
    update();
}

void BufferTraceItem::paint(QPainter *p) {
    if (!m_stats || width() < 4 || height() < 4) return;
    p->setRenderHint(QPainter::Antialiasing);
    const qreal w = width(), h = height();
    auto yFor = [h](qreal pct) { return h - qBound<qreal>(0, pct, 100) / 100.0 * h; };

    // Watermark band
    if (m_bandMax >= m_bandMin) {
        QColor band = m_color;
        band.setAlphaF(0.14);
        p->fillRect(QRectF(0, yFor(m_bandMax), w, yFor(m_bandMin) - yFor(m_bandMax)), band);
    }
    // 50 % guide
#ifdef Q_OS_MACOS
    // BufferTraceView: the 50% guide in separatorColor
    static const QColor separator = [] {
        const QVariant v = macSystemColors().value(QStringLiteral("separator"));
        return v.isValid() ? v.value<QColor>() : QColor::fromRgbF(1, 1, 1, 0.098);
    }();
    QPen guide(separator, 0.5, Qt::CustomDashLine);
#else
    QPen guide(QColor(255, 255, 255, 40), 0.5, Qt::CustomDashLine);
#endif
    guide.setDashPattern({ 4, 6 });
    p->setPen(guide);
    p->drawLine(QPointF(0, yFor(50)), QPointF(w, yFor(50)));

    // Trace, oldest on the left; absent samples lift the pen
    const auto &hist = m_stats->history(qBound(0, m_series, StatsController::SeriesCount - 1));
    const int n = StatsController::HistoryLength;
    QPainterPath path;
    bool pen = false;
    const qreal step = w / (n - 1);
    for (int i = 0; i < n; i++) {
        quint8 v = hist[i];
        if (v == StatsController::Absent) { pen = false; continue; }
        QPointF pt(i * step, yFor(v));
        if (!pen) { path.moveTo(pt); pen = true; }
        else path.lineTo(pt);
    }
    QPen line(m_color, 1.25, m_dashed ? Qt::CustomDashLine : Qt::SolidLine, Qt::FlatCap, Qt::RoundJoin);
    if (m_dashed) line.setDashPattern({ 2.4, 1.6 });
    p->setPen(line);
    p->setBrush(Qt::NoBrush);
    p->drawPath(path);
}
