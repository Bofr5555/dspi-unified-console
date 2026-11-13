// -- WP QP9 -- Stats for nerbs' controller (StatsController.h).
#include "StatsController.h"

#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "Snapshot.h"

#include <QCoreApplication>
#include <QDateTime>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <algorithm>
#include <cmath>
#include <limits>

static const Registrar::Controller<StatsController> statsController;
static const Registrar::Window statsWindow("tools.stats", "StatsWindow");

namespace {

QVariantMap row(const QString &label, const QString &text, const QString &band = QString(),
                const QString &tip = QString()) {
    return { { QStringLiteral("label"), label }, { QStringLiteral("text"), text },
             { QStringLiteral("band"), band }, { QStringLiteral("tip"), tip } };
}
// A row with a state dot: `band` colours the dot ("good", "warn", "bad", "off")
QVariantMap stateRow(const QString &label, const QString &text, const QString &band) {
    QVariantMap r = row(label, text, band);
    r.insert(QStringLiteral("dot"), true);
    return r;
}

QString yesNo(bool v) { return v ? QCoreApplication::translate("StatsController", "Yes") : QCoreApplication::translate("StatsController", "No"); }
QString dash() { return QStringLiteral("-"); }
QString khz(uint32_t hz) { return QStringLiteral("%1 kHz").arg(hz / 1000.0, 0, 'f', 1); }
QString unknown(int n) { return QCoreApplication::translate("StatsController", "Unknown (%1)").arg(n); }

QJsonObject json(const QByteArray &bytes) { return QJsonDocument::fromJson(bytes).object(); }

constexpr int kStatCount = 23;

} // namespace

StatsController::StatsController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    for (auto &h : m_history) h.fill(std::numeric_limits<double>::quiet_NaN(), kHistory);
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &StatsController::onSnapshot);
    if (ctx.poller) {
        m_slowPoll = ctx.poller->add(
            QStringLiteral("stats"), kSlowMs, this, [](FfiCore *c) { return readSlow(c); },
            [this](const Slow &s) { applySlow(s); });
        m_fastPoll = ctx.poller->add(
            QStringLiteral("stats buffers"), kFastMs, this, [](FfiCore *c) { return readFast(c); },
            [this](const Fast &f) { applyFast(f); });
        updatePolls();
    }
}

// ---- Reads (worker) ----

StatsController::Slow StatsController::readSlow(FfiCore *c) {
    Slow s;
    // The counters the window shows: 3..8, 13..16, 17 (starvation) and 22
    for (int which : { STAT_PDM_RING_OVERRUNS, STAT_PDM_RING_UNDERRUNS, STAT_PDM_DMA_OVERRUNS, STAT_PDM_DMA_UNDERRUNS,
                       STAT_SPDIF_OVERRUNS, STAT_SPDIF_UNDERRUNS, STAT_USB_RING_OVERRUNS, STAT_CLOCK_HZ, STAT_CORE_MV,
                       STAT_SAMPLE_RATE, STAT_TEMPERATURE, STAT_STARVATION_TOTAL }) {
        uint32_t v = 0;
        s.statOk[which] = dspi_fetch_stat(c, uint16_t(which), &v);
        s.stat[which] = v;
        s.ok = s.ok || s.statOk[which];
    }
    if (s.stat[STAT_STARVATION_TOTAL] > 0) {
        for (int i = 0; i < 4; ++i) dspi_fetch_stat(c, uint16_t(STAT_STARVATION_FIRST + i), &s.starvation[i]);
    }
    const QJsonObject ring = json(readSized([c](char *b, uint32_t n) { return dspi_fetch_ring_health(c, b, n); }));
    s.ringRead = ring.value(QLatin1String("ok")).toBool();
    if (s.ringRead && ring.contains(QLatin1String("targetFrames"))) {
        s.ringPresent = true;
        s.ring = ring.toVariantMap();
    }
    s.inputSource = dspi_feature_present(c, "input_source");
    if (s.inputSource) {
        SpdifRxStatus rx{};
        s.rxOk = dspi_fetch_spdif_rx_status(c, &rx);
        if (s.rxOk) {
            s.rxState = rx.state;
            s.rxSource = rx.input_source;
            s.rxLock = rx.lock_count;
            s.rxLoss = rx.loss_count;
            s.rxRate = rx.sample_rate;
            s.rxParity = rx.parity_errors;
            s.rxFifo = rx.fifo_fill_pct;
            s.rxLib = rx.lib_state;
            s.rxStable = rx.stable_callbacks;
            s.rxLost = rx.lost_callbacks;
            // The active receiver's pin: S/PDIF 2..4 are sources 4..6, the rest receiver 0
            const int ext = int(rx.input_source) - 4 + 1;
            const uint8_t index = ext >= 1 && ext <= 3 ? uint8_t(ext) : 0;
            uint8_t pin = 0;
            if (!dspi_fetch_spdif_rx_pin(c, index, &pin) || pin == 0) dspi_fetch_spdif_rx_pin(c, 0, &pin);
            s.rxPin = pin;
            if (rx.state == 2) dspi_fetch_spdif_rx_channel_status(c, s.channelStatus.data());
        }
    }
    LgStatus lg{};
    s.lgOk = dspi_fetch_lg_status(c, &lg);
    s.lgEnabled = lg.enabled;
    s.lgPresent = lg.present;
    s.lgVolume = lg.volume;
    s.lgMuted = lg.muted;
    AdatOutStatus adat{};
    s.adatOk = dspi_fetch_adat_out_status(c, &adat);
    s.adatEnabled = adat.enabled;
    s.adatActive = adat.active;
    s.adatRateOk = adat.rate_ok;
    s.adatPin = adat.pin;
    s.adatResync = adat.resync_count;
    s.adatSlip = adat.slip_count;
    InputLockStatus lock{};
    s.i2sOk = dspi_fetch_input_lock(c, 0, &lock);
    s.i2sState = lock.state;
    s.i2sMode = lock.clock_mode;
    s.i2sDetected = lock.detected_rate;
    s.i2sMeasured = lock.measured_hz;
    s.i2sLock = lock.lock_count;
    s.i2sLoss = lock.loss_count;
    const QJsonObject diag = json(readSized([c](char *b, uint32_t n) { return dspi_fetch_clock_diag(c, b, n); }));
    if (diag.value(QLatin1String("ok")).toBool() && diag.value(QLatin1String("available")).toBool())
        s.clockDiag = diag.value(QLatin1String("fields")).toArray().toVariantList();
    s.usb = json(readSized([c](char *b, uint32_t n) { return dspi_fetch_usb_error_stats(c, b, n); })).toVariantMap();
    return s;
}

StatsController::Fast StatsController::readFast(FfiCore *c) {
    Fast f;
    const QJsonObject tick = json(readSized([c](char *b, uint32_t n) { return dspi_fetch_stats_tick(c, b, n); }));
    if (!tick.value(QLatin1String("ok")).toBool()) return f;
    f.ok = true;
    f.buffers = tick.value(QLatin1String("buffers")).toObject().toVariantMap();
    const QJsonValue fills = tick.value(QLatin1String("fills"));
    f.fillsOk = fills.isArray();
    if (f.fillsOk) f.fills = fills.toArray().toVariantList();
    return f;
}

// ---- Apply (GUI) ----

void StatsController::applySlow(const Slow &s) {
    if (!s.ok) return;
    // A failed read leaves that value as it was
    Slow next = s;
    if (m_haveSlow) {
        for (int i = 0; i < kStatCount; ++i)
            if (!s.statOk[i]) {
                next.stat[i] = m_slow.stat[i];
                next.statOk[i] = m_slow.statOk[i];
            }
        if (next.stat[STAT_STARVATION_TOTAL] > 0 && !s.statOk[STAT_STARVATION_TOTAL])
            std::copy(std::begin(m_slow.starvation), std::end(m_slow.starvation), std::begin(next.starvation));
        // -- WP FIXS1 -- a ring-health read that failed keeps its section
        if (!s.ringRead) {
            next.ringRead = m_slow.ringRead;
            next.ringPresent = m_slow.ringPresent;
            next.ring = m_slow.ring;
        }
    }
    const uint32_t total = next.stat[STAT_STARVATION_TOTAL];
    m_delta = m_haveTotal ? total - m_lastTotal : 0;   // wraps as the counter does
    if (m_delta > 0) {
        m_eventBefore = m_eventAt;
        m_eventAt = QDateTime::currentMSecsSinceEpoch();
    }
    m_lastTotal = total;
    m_haveTotal = true;
    m_lgSeen = m_lgSeen || s.lgOk;
    m_targetFrames = next.ringPresent ? next.ring.value(QStringLiteral("targetFrames")).toInt() : 0;
    m_batchFrames = next.ringPresent ? next.ring.value(QStringLiteral("batchFrames")).toInt() : 0;
    m_clockDiag = s.clockDiag;
    m_slow = next;
    m_haveSlow = true;
    emit slowChanged();
}

void StatsController::applyFast(const Fast &f) {
    if (!f.ok) return;
    m_buffers = f.buffers;
    m_streaming = f.buffers.value(QStringLiteral("streaming")).toBool();
    m_pdmActive = f.buffers.value(QStringLiteral("pdmActive")).toBool();
    m_numSpdif = std::clamp(f.buffers.value(QStringLiteral("numSpdif")).toInt(), 0, 4);
    if (f.fillsOk) m_fills = f.fills;
    const QVariantMap pdm = f.buffers.value(QStringLiteral("pdm")).toMap();
    const double nan = std::numeric_limits<double>::quiet_NaN();
    std::array<double, 6> sample{ nan, nan, nan, nan, nan, nan };
    for (int i = 0; i < m_numSpdif && i < int(m_fills.size()); ++i)
        sample[size_t(i)] = std::min(100.0, m_fills.at(i).toMap().value(QStringLiteral("fillPct")).toDouble());
    if (m_pdmActive) {
        sample[4] = pdm.value(QStringLiteral("dmaFillPct")).toDouble();
        sample[5] = pdm.value(QStringLiteral("ringFillPct")).toDouble();
    }
    for (size_t i = 0; i < m_history.size(); ++i) {
        m_history[i].removeFirst();
        m_history[i].append(sample[i]);
    }
    emit buffersChanged();
}

void StatsController::onSnapshot() {
    const DeviceSnapshot &snap = m_ctx.device->snapshot();
    if (snap.connected && snap.generation != m_generation) {
        // Every connect after the first counts as a reconnect
        if (m_seenConnect) ++m_connects;
        m_seenConnect = true;
        m_generation = snap.generation;
        // -- WP FIXS1 -- the same device back: what it last said stays until
        // its next read (SHOTS1: a reset here showed zeros until then)
        const QString serial = m_ctx.device->serial().trimmed();
        const bool same = !serial.isEmpty() && serial == m_serial;
        m_serial = serial;
        if (same) {
            emit slowChanged();
            return;
        }
        // Another device: its sections and counters start over
        m_haveSlow = false;
        m_haveTotal = false;
        m_lgSeen = false;
        m_slow = Slow{};
        m_clockDiag.clear();
        m_fills.clear();
        m_numSpdif = 0;
        for (auto &h : m_history) h.fill(std::numeric_limits<double>::quiet_NaN(), kHistory);
    }
    emit slowChanged();
}

void StatsController::setWindowShown(bool shown) {
    if (m_shown == shown) return;
    m_shown = shown;
    updatePolls();
    emit windowShownChanged();
}

void StatsController::updatePolls() {
    if (!m_ctx.poller) return;
    m_ctx.poller->setEnabled(m_slowPoll, m_shown);
    m_ctx.poller->setEnabled(m_fastPoll, m_shown);
}

void StatsController::resetBufferWatermarks() {
    m_ctx.device->write(QStringLiteral("reset watermarks"), [](FfiCore *c) { return dspi_reset_buffer_stats(c); });
}

bool StatsController::connected() const { return m_ctx.device->connected(); }
bool StatsController::writable() const { return connected() && !m_ctx.device->readOnly(); }

// ---- Sections ----

QVariantMap StatsController::device() const {
    const DspState *st = m_ctx.device->state();
    QString platform = dash();
    if (st) {
        switch (st->platform_id) {
        case 1: platform = QStringLiteral("RP2350"); break;
        case 2: platform = QStringLiteral("STM32H723"); break;
        default: platform = QStringLiteral("RP2040"); break;
        }
    }
    const QString fw = m_ctx.device->firmwareVersion();
    const QString serial = m_ctx.device->serial().trimmed();
    return { { QStringLiteral("platform"), platform },
             { QStringLiteral("firmware"), st && !fw.isEmpty() ? QStringLiteral("v") + fw : dash() },
             { QStringLiteral("serial"), serial.isEmpty() ? dash() : serial },
             { QStringLiteral("reconnects"), QString::number(m_connects) } };
}

QVariantMap StatsController::system() const {
    const auto &v = m_slow.stat;
    // -- WP FIXS1 -- a value not read yet is a dash, never a zero
    const auto known = [this](int which, const QString &text) { return m_slow.statOk[which] ? text : dash(); };
    return { { QStringLiteral("clock"), known(STAT_CLOCK_HZ, QStringLiteral("%1 MHz").arg(v[STAT_CLOCK_HZ] / 1e6, 0, 'f', 1)) },
             { QStringLiteral("coreVoltage"),
               known(STAT_CORE_MV, QStringLiteral("%1 V").arg(v[STAT_CORE_MV] / 1000.0, 0, 'f', 2)) },
             { QStringLiteral("sampleRate"), known(STAT_SAMPLE_RATE, khz(v[STAT_SAMPLE_RATE])) },
             { QStringLiteral("temperature"),
               known(STAT_TEMPERATURE, QStringLiteral("%1 °C").arg(int32_t(v[STAT_TEMPERATURE]) / 100.0, 0, 'f', 1)) } };
}

QVariantMap StatsController::counters() const {
    const auto &v = m_slow.stat;
    const auto &ok = m_slow.statOk;
    // -1: not read yet, so not shown (WP FIXS1)
    const auto count = [&](int which) { return ok[which] ? qint64(v[which]) : qint64(-1); };
    const auto pair = [](qint64 over, qint64 under) {
        return QVariantMap{ { QStringLiteral("over"), over }, { QStringLiteral("under"), under } };
    };
    return { { QStringLiteral("usbRing"), pair(count(STAT_USB_RING_OVERRUNS), -1) },
             { QStringLiteral("spdif"), pair(count(STAT_SPDIF_OVERRUNS), count(STAT_SPDIF_UNDERRUNS)) },
             { QStringLiteral("pdmRing"), pair(count(STAT_PDM_RING_OVERRUNS), count(STAT_PDM_RING_UNDERRUNS)) },
             { QStringLiteral("pdmDma"), pair(count(STAT_PDM_DMA_OVERRUNS), count(STAT_PDM_DMA_UNDERRUNS)) } };
}

QVariantMap StatsController::starvation() const {
    QVariantList perSlot;
    const int n = std::max(m_numSpdif, 2);
    for (int i = 0; i < n && i < 4; ++i)
        perSlot.append(QVariantMap{ { QStringLiteral("label"), QCoreApplication::translate("StatsController", "Out %1/%2")
                                                                   .arg(2 * i + 1)
                                                                   .arg(2 * i + 2) },
                                  { QStringLiteral("count"), qint64(m_slow.starvation[i]) } });
    return { { QStringLiteral("known"), m_slow.statOk[STAT_STARVATION_TOTAL] },
             { QStringLiteral("total"), qint64(m_slow.stat[STAT_STARVATION_TOTAL]) },
             { QStringLiteral("delta"), qint64(m_delta) },
             { QStringLiteral("slots"), perSlot },
             { QStringLiteral("lastEventMs"), m_eventAt },
             { QStringLiteral("previousEventMs"), m_eventBefore } };
}

QVariantList StatsController::ringHealth() const {
    if (!m_slow.ringPresent) return {};
    const QVariantMap &r = m_slow.ring;
    const auto num = [&](const char *key) { return r.value(QLatin1String(key)).toLongLong(); };
    const QVariant lowest = r.value(QStringLiteral("lowestFill"));
    return {
        row(QCoreApplication::translate("StatsController", "Fill Target"),
            QCoreApplication::translate("StatsController", "%1 frames (%2)").arg(num("targetFrames")).arg(latencyText(uint32_t(num("targetUs")))),
            QString(), QCoreApplication::translate("StatsController", "How full the output rings are kept, set by Latency in Settings")),
        row(QCoreApplication::translate("StatsController", "Underrun Episodes"), QString::number(num("underrunEpisodes")),
            num("underrunEpisodes") > 0 ? QStringLiteral("bad") : QString(),
            QCoreApplication::translate("StatsController", "Times the outputs ran dry while a source owed audio. A paused stream does not count.")),
        row(QCoreApplication::translate("StatsController", "Refused Blocks"), QString::number(num("refusedBlocks")),
            num("refusedBlocks") > 0 ? QStringLiteral("warn") : QString(),
            QCoreApplication::translate("StatsController", "Audio blocks dropped because the rings had no room for them")),
        row(QCoreApplication::translate("StatsController", "Lowest Fill"),
            lowest.isNull() || !lowest.isValid() ? dash()
                                                 : QCoreApplication::translate("StatsController", "%1 frames").arg(lowest.toLongLong()),
            QString(),
            QCoreApplication::translate("StatsController", "The emptiest any output ring got just before new audio arrived, since the watermarks were reset. Near zero means the margin is thin.")),
        row(QCoreApplication::translate("StatsController", "Worst Input Gap"), gapText(uint32_t(num("worstInputGapUs"))), QString(),
            QCoreApplication::translate("StatsController", "Longest wait between input deliveries since the watermarks were reset")),
        row(QCoreApplication::translate("StatsController", "Worst USB Gap"), gapText(uint32_t(num("worstUsbGapUs"))), QString(),
            QCoreApplication::translate("StatsController", "Longest gap between USB audio packets since the watermarks were reset. Long gaps are a paused stream.")),
        row(QCoreApplication::translate("StatsController", "Over-Budget Steps"), QString::number(num("overBudgetSteps")),
            num("overBudgetSteps") > 0 ? QStringLiteral("warn") : QString(),
            QCoreApplication::translate("StatsController", "Background tasks that ran past their time slice in the device's main loop")),
    };
}

QVariantList StatsController::spdifInput() const {
    if (!m_slow.inputSource || !m_slow.rxOk) return {};
    const Slow &s = m_slow;
    QString state, band;
    switch (s.rxState) {
    case 0: state = QCoreApplication::translate("StatsController", "Inactive"); band = QStringLiteral("off"); break;
    case 1: state = QCoreApplication::translate("StatsController", "Acquiring"); band = QStringLiteral("warn"); break;
    case 2: state = QCoreApplication::translate("StatsController", "Locked"); band = QStringLiteral("good"); break;
    case 3: state = QCoreApplication::translate("StatsController", "Relocking"); band = QStringLiteral("warn"); break;
    default: state = unknown(s.rxState); band = QStringLiteral("off"); break;
    }
    QString source;
    switch (s.rxSource) {
    case 1: source = QStringLiteral("S/PDIF 1"); break;
    case 4: source = QStringLiteral("S/PDIF 2"); break;
    case 5: source = QStringLiteral("S/PDIF 3"); break;
    case 6: source = QStringLiteral("S/PDIF 4"); break;
    case 2: source = QStringLiteral("I2S"); break;
    case 3: source = QStringLiteral("ADAT"); break;
    default: source = QStringLiteral("USB"); break;
    }
    const bool locked = s.rxState == 2;
    return {
        stateRow(QCoreApplication::translate("StatsController", "State"), state, band),
        row(QCoreApplication::translate("StatsController", "Active Source"), source),
        row(QCoreApplication::translate("StatsController", "Sample Rate"), locked && s.rxRate > 0 ? khz(s.rxRate) : dash()),
        row(QCoreApplication::translate("StatsController", "Lock Count"), QString::number(s.rxLock)),
        row(QCoreApplication::translate("StatsController", "Loss Count"), QString::number(s.rxLoss)),
        row(QCoreApplication::translate("StatsController", "Parity Errors"), locked ? QString::number(s.rxParity) : dash()),
        row(QCoreApplication::translate("StatsController", "FIFO Fill"), locked ? QStringLiteral("%1%").arg(s.rxFifo) : dash()),
        row(QCoreApplication::translate("StatsController", "RX Pin"), QStringLiteral("GPIO %1").arg(s.rxPin)),
    };
}

QVariantList StatsController::channelStatus() const {
    if (!m_slow.inputSource || !m_slow.rxOk || m_slow.rxState != 2) return {};
    const auto &raw = m_slow.channelStatus;
    QString category;
    switch (raw[1]) {
    case 0x00: category = QCoreApplication::translate("StatsController", "General"); break;
    case 0x01: category = QCoreApplication::translate("StatsController", "CD Player"); break;
    case 0x02: category = QStringLiteral("DAT"); break;
    case 0x03: category = QStringLiteral("DCC"); break;
    case 0x04: category = QStringLiteral("MiniDisc"); break;
    case 0x06: category = QCoreApplication::translate("StatsController", "Synthesizer"); break;
    case 0x08: category = QCoreApplication::translate("StatsController", "Broadcast Receiver"); break;
    case 0x09: category = QCoreApplication::translate("StatsController", "Musical Instrument"); break;
    case 0x0A: category = QCoreApplication::translate("StatsController", "A/D Converter"); break;
    case 0x0C: category = QCoreApplication::translate("StatsController", "Mixer"); break;
    case 0x0D: category = QCoreApplication::translate("StatsController", "Rate Converter"); break;
    case 0x0E: category = QCoreApplication::translate("StatsController", "Sampler"); break;
    case 0x0F: category = QCoreApplication::translate("StatsController", "Digital Signal Processor"); break;
    default: category = QStringLiteral("0x") + QString::number(raw[1], 16).toUpper().rightJustified(2, QLatin1Char('0')); break;
    }
    QString word;
    switch (raw[4] & 0x0F) {
    case 0x0: word = QCoreApplication::translate("StatsController", "Not indicated"); break;
    case 0x2: word = QStringLiteral("16-bit"); break;
    case 0x4: word = QStringLiteral("20-bit"); break;
    case 0x8: word = QStringLiteral("17-bit"); break;
    case 0xA: word = QStringLiteral("22-bit"); break;
    case 0xB: word = QStringLiteral("24-bit"); break;
    default: word = dash(); break;
    }
    return {
        row(QCoreApplication::translate("StatsController", "Format"),
            raw[0] & 1 ? QCoreApplication::translate("StatsController", "Professional") : QCoreApplication::translate("StatsController", "Consumer")),
        row(QCoreApplication::translate("StatsController", "Audio"), raw[0] & 2 ? QStringLiteral("Non-PCM") : QStringLiteral("PCM")),
        row(QCoreApplication::translate("StatsController", "Category"), category),
        row(QCoreApplication::translate("StatsController", "Word Length"), word),
        row(QCoreApplication::translate("StatsController", "Copy"),
            raw[0] & 4 ? QCoreApplication::translate("StatsController", "Permitted") : QCoreApplication::translate("StatsController", "Prohibited")),
    };
}

QVariantList StatsController::spdifDebug() const {
    if (!m_slow.inputSource || !m_slow.rxOk || m_slow.rxState == 0) return {};
    QString lib;
    switch (m_slow.rxLib) {
    case 0: lib = QCoreApplication::translate("StatsController", "No Signal"); break;
    case 1: lib = QCoreApplication::translate("StatsController", "Waiting Stable"); break;
    case 2: lib = QCoreApplication::translate("StatsController", "Stable"); break;
    default: lib = unknown(m_slow.rxLib); break;
    }
    return {
        row(QCoreApplication::translate("StatsController", "Library State"), lib),
        row(QCoreApplication::translate("StatsController", "Stable Callbacks"), QString::number(m_slow.rxStable)),
        row(QCoreApplication::translate("StatsController", "Lost Callbacks"), QString::number(m_slow.rxLost)),
    };
}

QVariantList StatsController::lgSoundSync() const {
    if (!m_lgSeen) return {};
    const Slow &s = m_slow;
    return {
        row(QCoreApplication::translate("StatsController", "Enabled"), yesNo(s.lgEnabled)),
        stateRow(QCoreApplication::translate("StatsController", "Present"), yesNo(s.lgPresent),
                 s.lgPresent ? QStringLiteral("good") : QStringLiteral("off")),
        row(QCoreApplication::translate("StatsController", "TV Volume"),
            s.lgVolume == 0xFF ? dash() : QStringLiteral("%1 / 100").arg(s.lgVolume)),
        row(QCoreApplication::translate("StatsController", "TV Mute"),
            s.lgMuted ? QCoreApplication::translate("StatsController", "On") : QCoreApplication::translate("StatsController", "Off")),
    };
}

QVariantList StatsController::adatOutput() const {
    const DspState *st = m_ctx.device->state();
    if (!m_slow.adatOk || !st || st->platform_id != 1) return {};
    const Slow &s = m_slow;
    QString state, band;
    if (!s.adatEnabled) { state = QCoreApplication::translate("StatsController", "Disabled"); band = QStringLiteral("off"); }
    else if (s.adatActive) { state = QCoreApplication::translate("StatsController", "Streaming"); band = QStringLiteral("good"); }
    else if (!s.adatRateOk) { state = QCoreApplication::translate("StatsController", "Suspended (rate above 48 kHz)"); band = QStringLiteral("warn"); }
    else { state = QCoreApplication::translate("StatsController", "Enabled"); band = QStringLiteral("warn"); }
    return {
        stateRow(QCoreApplication::translate("StatsController", "State"), state, band),
        row(QCoreApplication::translate("StatsController", "Streaming"), yesNo(s.adatActive)),
        row(QCoreApplication::translate("StatsController", "Rate Supported"), yesNo(s.adatRateOk)),
        row(QCoreApplication::translate("StatsController", "Data Pin"), QStringLiteral("GPIO %1").arg(s.adatPin)),
        row(QCoreApplication::translate("StatsController", "Resync Count"), QString::number(s.adatResync)),
        row(QCoreApplication::translate("StatsController", "Slip Count"), QString::number(s.adatSlip)),
    };
}

QVariantList StatsController::usbErrorRows(const QVariantMap &reply) {
    if (!reply.value(QStringLiteral("ok")).toBool() || reply.value(QStringLiteral("total")).toUInt() == 0) return {};
    const auto count = [&reply](const char *key) { return QString::number(reply.value(QLatin1String(key)).toUInt()); };
    return {
        row(QCoreApplication::translate("StatsController", "Total"), count("total"), QStringLiteral("bad")),
        row(QCoreApplication::translate("StatsController", "CRC Errors"), count("crc")),
        row(QCoreApplication::translate("StatsController", "Bit Stuffing Errors"), count("bitstuff")),
        row(QCoreApplication::translate("StatsController", "Receive Overflows"), count("rxOverflow")),
        row(QCoreApplication::translate("StatsController", "Receive Timeouts"), count("rxTimeout")),
        row(QCoreApplication::translate("StatsController", "Data Sequence Errors"), count("dataSeq")),
    };
}

QVariantList StatsController::i2sSlave() const {
    if (!m_slow.i2sOk || m_slow.i2sMode != 1) return {};
    const Slow &s = m_slow;
    QString state, band;
    switch (s.i2sState) {
    case 0: state = QCoreApplication::translate("StatsController", "Inactive"); band = QStringLiteral("off"); break;
    case 1: state = QCoreApplication::translate("StatsController", "Acquiring"); band = QStringLiteral("warn"); break;
    case 2: state = QCoreApplication::translate("StatsController", "Relocking"); band = QStringLiteral("warn"); break;
    case 3: state = QCoreApplication::translate("StatsController", "Locked"); band = QStringLiteral("good"); break;
    default: state = unknown(s.i2sState); band = QStringLiteral("off"); break;
    }
    return {
        stateRow(QCoreApplication::translate("StatsController", "State"), state, band),
        row(QCoreApplication::translate("StatsController", "Detected Rate"), s.i2sState == 3 && s.i2sDetected > 0 ? khz(s.i2sDetected) : dash()),
        row(QCoreApplication::translate("StatsController", "Measured Rate"), s.i2sMeasured > 0 ? khz(s.i2sMeasured) : dash()),
        row(QCoreApplication::translate("StatsController", "Lock Count"), QString::number(s.i2sLock)),
        row(QCoreApplication::translate("StatsController", "Loss Count"), QString::number(s.i2sLoss)),
    };
}

QVariantList StatsController::fillRows() const {
    QVariantList rows;
    const DspState *st = m_ctx.device->state();
    const auto historyOf = [this](int series) {
        QVariantList h;
        h.reserve(kHistory);
        for (double v : m_history[size_t(series)]) h.append(v);
        return h;
    };
    const auto make = [&](const QString &title, double fill, int min, int max, const QString &band, const QString &colour,
                          bool hollow, bool dashed, int series) {
        return QVariantMap{ { QStringLiteral("title"), title },
                            { QStringLiteral("fill"), fill },
                            { QStringLiteral("min"), min },
                            { QStringLiteral("max"), max },
                            { QStringLiteral("text"), QStringLiteral("%1%").arg(qRound(fill)) },
                            { QStringLiteral("range"), QStringLiteral("%1–%2%").arg(min).arg(max) },
                            { QStringLiteral("band"), band },
                            { QStringLiteral("colour"), colour },
                            { QStringLiteral("hollow"), hollow },
                            { QStringLiteral("dashed"), dashed },
                            { QStringLiteral("history"), historyOf(series) } };
    };
    const QVariantList spdif = m_buffers.value(QStringLiteral("spdif")).toList();
    for (int i = 0; i < m_numSpdif; ++i) {
        const bool i2s = st && i < 4 && st->output_types[i] == 1;
        const QString title = i2s ? QCoreApplication::translate("StatsController", "Out %1/%2 (I2S)").arg(2 * i + 1).arg(2 * i + 2)
                                  : QCoreApplication::translate("StatsController", "Out %1/%2 (SPDIF)").arg(2 * i + 1).arg(2 * i + 2);
        double fill = 0;
        int min = 0, max = 0;
        QString band;
        if (i < m_fills.size()) {
            // The core's fills: on the ring's scale, rated by its target
            const QVariantMap f = m_fills.at(i).toMap();
            fill = f.value(QStringLiteral("fillPct")).toDouble();
            min = f.value(QStringLiteral("minPct")).toInt();
            max = f.value(QStringLiteral("maxPct")).toInt();
            band = f.value(QStringLiteral("band")).toString();
        } else if (i < spdif.size()) {
            const QVariantMap b = spdif.at(i).toMap();
            fill = b.value(QStringLiteral("fillPct")).toDouble();
            min = b.value(QStringLiteral("minPct")).toInt();
            max = b.value(QStringLiteral("maxPct")).toInt();
            band = spdifBand(fill);
        }
        rows.append(make(title, fill, min, max, band, QStringLiteral("output %1").arg(2 * i), false, false, i));
    }
    if (m_pdmActive) {
        const QVariantMap pdm = m_buffers.value(QStringLiteral("pdm")).toMap();
        const double dma = pdm.value(QStringLiteral("dmaFillPct")).toDouble();
        const double ring = pdm.value(QStringLiteral("ringFillPct")).toDouble();
        rows.append(make(QCoreApplication::translate("StatsController", "PDM DMA"), dma, pdm.value(QStringLiteral("dmaMinPct")).toInt(),
                         pdm.value(QStringLiteral("dmaMaxPct")).toInt(), pdmDmaBand(dma), QStringLiteral("pdm"), false, false, 4));
        rows.append(make(QCoreApplication::translate("StatsController", "PDM Ring"), ring, pdm.value(QStringLiteral("ringMinPct")).toInt(),
                         pdm.value(QStringLiteral("ringMaxPct")).toInt(), pdmRingBand(ring), QStringLiteral("pdm"), true, true, 5));
    }
    return rows;
}

// ---- Formats and bands ----

QString StatsController::formatElapsed(qint64 ms) {
    const qint64 s = std::max<qint64>(0, ms / 1000);
    const qint64 h = s / 3600, m = (s % 3600) / 60, sec = s % 60;
    if (h > 0) return QStringLiteral("%1h %2m %3s").arg(h).arg(m, 2, 10, QLatin1Char('0')).arg(sec, 2, 10, QLatin1Char('0'));
    return QStringLiteral("%1m %2s").arg(m).arg(sec, 2, 10, QLatin1Char('0'));
}

QString StatsController::gapText(uint32_t us) {
    if (us < 10000) return QStringLiteral("%1 µs").arg(us);
    return QStringLiteral("%1 ms").arg(us / 1000.0, 0, 'f', 1);
}

QString StatsController::latencyText(uint32_t us) {
    if (us % 1000 == 0) return QStringLiteral("%1 ms").arg(us / 1000);
    return QStringLiteral("%1 ms").arg(us / 1000.0, 0, 'f', 2);
}

QString StatsController::spdifBand(double pct) {
    const int v = int(std::floor(std::clamp(pct, 0.0, 100.0)));
    if (v == 0 || v == 100) return QStringLiteral("bad");
    if (v < 25 || v > 75) return QStringLiteral("warn");
    return QStringLiteral("good");
}

QString StatsController::pdmDmaBand(double pct) {
    const int v = int(std::floor(std::clamp(pct, 0.0, 100.0)));
    if (v > 50) return QStringLiteral("bad");
    if (v < 5 || v > 30) return QStringLiteral("warn");
    return QStringLiteral("good");
}

QString StatsController::pdmRingBand(double pct) {
    const int v = int(std::floor(std::clamp(pct, 0.0, 100.0)));
    if (v > 50) return QStringLiteral("bad");
    if (v > 20) return QStringLiteral("warn");
    return QStringLiteral("good");
}

QString StatsController::ringBand(double pct, int targetFrames, int batchFrames) {
    const double frames = std::clamp(pct, 0.0, 100.0) / 100.0 * kRingFrames;
    if (frames < 1 || frames >= kRingFrames - kEraseAhead) return QStringLiteral("bad");
    if (frames < targetFrames / 4.0 || frames > targetFrames + 2.0 * batchFrames) return QStringLiteral("warn");
    return QStringLiteral("good");
}
