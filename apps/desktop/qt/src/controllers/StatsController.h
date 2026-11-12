#ifndef STATSCONTROLLER_H
#define STATSCONTROLLER_H

#include <QObject>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <array>
#include <cstdint>

#include "Poller.h"
#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// -- WP QP9 -- Stats for nerbs (the Mac's StatsView in its "System Statistics"
// window), with the desktop's additions: clock diagnostics and the output
// ring's health.
//
// Two polls while the window shows (the Mac keeps polling hidden; here the
// history simply pauses with the window):
// - every 2 s the counters (dspi_fetch_stat), the ring health, the S/PDIF
//   receiver, LG Sound Sync, the ADAT output, the I2S slave clock and the clock
//   diagnostics;
// - every 60 ms the buffers and the per-output fills (dspi_fetch_stats_tick,
//   one transfer), which also feed a 256-sample history per fill row.
//
// Sections the device lacks are left out: ring health when the device does not
// report it, the S/PDIF input without the input-source probe, LG Sound Sync
// until a read answers (then it stays), ADAT on RP2350 only while its read
// answers, the I2S slave clock only in slave mode, clock diagnostics only on
// firmware built with them. USB error statistics have no export in the core
// yet; `usbErrors` is the seam for them (empty: the section stays hidden).
//
// Everything QML shows is prepared here as text (`*.text` keys) with its
// colour band ("good", "warn", "bad", "" for none).
class StatsController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool windowShown READ windowShown WRITE setWindowShown NOTIFY windowShownChanged)
    Q_PROPERTY(bool connected READ connected NOTIFY slowChanged)
    Q_PROPERTY(bool writable READ writable NOTIFY slowChanged)
    // Device: platform, firmware, serial, reconnects (texts)
    Q_PROPERTY(QVariantMap device READ device NOTIFY slowChanged)
    // System: clock, coreVoltage, sampleRate, temperature (texts)
    Q_PROPERTY(QVariantMap system READ system NOTIFY slowChanged)
    // Over/under counters: usbRing, spdif, pdmRing, pdmDma as {over, under} (-1 when not counted)
    Q_PROPERTY(QVariantMap counters READ counters NOTIFY slowChanged)
    // Starvation: total, delta, slots [n], lastEventMs, previousEventMs (0 for none)
    Q_PROPERTY(QVariantMap starvation READ starvation NOTIFY slowChanged)
    // Rows {label, text, band, tip}; empty when the device does not report it
    Q_PROPERTY(QVariantList ringHealth READ ringHealth NOTIFY slowChanged)
    Q_PROPERTY(QVariantList spdifInput READ spdifInput NOTIFY slowChanged)
    Q_PROPERTY(QVariantList channelStatus READ channelStatus NOTIFY slowChanged)
    Q_PROPERTY(QVariantList spdifDebug READ spdifDebug NOTIFY slowChanged)
    Q_PROPERTY(QVariantList lgSoundSync READ lgSoundSync NOTIFY slowChanged)
    Q_PROPERTY(QVariantList adatOutput READ adatOutput NOTIFY slowChanged)
    Q_PROPERTY(QVariantList i2sSlave READ i2sSlave NOTIFY slowChanged)
    Q_PROPERTY(QVariantList clockDiagnostics READ clockDiagnostics NOTIFY slowChanged)
    Q_PROPERTY(QVariantList usbErrors READ usbErrors NOTIFY slowChanged)
    // Buffers: streaming, pdmActive and the fill rows
    // {title, fill, min, max, text, range, band, colour ("output N" or "pdm"), hollow, dashed, history}
    Q_PROPERTY(bool streaming READ streaming NOTIFY buffersChanged)
    Q_PROPERTY(bool pdmActive READ pdmActive NOTIFY buffersChanged)
    Q_PROPERTY(QVariantList fillRows READ fillRows NOTIFY buffersChanged)
public:
    static constexpr int kSlowMs = 2000;
    static constexpr int kFastMs = 60;
    static constexpr int kHistory = 256;
    // The output ring: frames, and those the firmware erases ahead
    static constexpr int kRingFrames = 512;
    static constexpr int kEraseAhead = 64;

    StatsController(const AppContext &ctx, QObject *parent = nullptr);
    static StatsController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<StatsController>(); }

    bool windowShown() const { return m_shown; }
    void setWindowShown(bool shown);
    bool connected() const;
    bool writable() const;
    QVariantMap device() const;
    QVariantMap system() const;
    QVariantMap counters() const;
    QVariantMap starvation() const;
    QVariantList ringHealth() const;
    QVariantList spdifInput() const;
    QVariantList channelStatus() const;
    QVariantList spdifDebug() const;
    QVariantList lgSoundSync() const;
    QVariantList adatOutput() const;
    QVariantList i2sSlave() const;
    QVariantList clockDiagnostics() const { return m_clockDiag; }
    QVariantList usbErrors() const { return {}; }
    bool streaming() const { return m_streaming; }
    bool pdmActive() const { return m_pdmActive; }
    QVariantList fillRows() const;

    // Reset Watermarks (a device write: refused on a read-only device)
    Q_INVOKABLE void resetBufferWatermarks();
    // "0m 07s", "1h 02m 03s"
    Q_INVOKABLE static QString formatElapsed(qint64 ms);
    // The fill bands the core does not rate: PDM DMA and PDM ring; S/PDIF
    // without the ring's target
    Q_INVOKABLE static QString pdmDmaBand(double pct);
    Q_INVOKABLE static QString pdmRingBand(double pct);
    Q_INVOKABLE static QString spdifBand(double pct);
    // The ring's band from its target and batch, for a fill in percent of the ring
    Q_INVOKABLE static QString ringBand(double pct, int targetFrames, int batchFrames);
    static QString gapText(uint32_t us);
    static QString latencyText(uint32_t us);

    // For tests
    Poller::Id slowPoll() const { return m_slowPoll; }
    Poller::Id fastPoll() const { return m_fastPoll; }

    // One 2 s read, worker side
    struct Slow {
        bool ok = false;
        uint32_t stat[23] = {};
        bool statOk[23] = {};
        uint32_t starvation[4] = {};
        bool ringPresent = false;
        QVariantMap ring;
        bool inputSource = false;
        bool rxOk = false;
        uint8_t rxState = 0, rxSource = 0, rxLock = 0, rxLoss = 0, rxLib = 0, rxStable = 0, rxLost = 0;
        uint32_t rxRate = 0, rxParity = 0;
        uint16_t rxFifo = 0;
        uint8_t rxPin = 0;
        std::array<uint8_t, 24> channelStatus{};
        bool lgOk = false;
        bool lgEnabled = false, lgPresent = false, lgMuted = false;
        uint8_t lgVolume = 0xFF;
        bool adatOk = false;
        bool adatEnabled = false, adatActive = false, adatRateOk = false;
        uint8_t adatPin = 0;
        uint16_t adatResync = 0, adatSlip = 0;
        bool i2sOk = false;
        uint8_t i2sState = 0, i2sMode = 0, i2sLock = 0, i2sLoss = 0;
        uint32_t i2sDetected = 0, i2sMeasured = 0;
        QVariantList clockDiag;
    };
    // One 60 ms read, worker side
    struct Fast {
        bool ok = false;
        QVariantMap buffers;
        bool fillsOk = false;
        QVariantList fills;
    };

signals:
    void windowShownChanged();
    void slowChanged();
    void buffersChanged();

private:
    static Slow readSlow(FfiCore *core);
    static Fast readFast(FfiCore *core);
    void applySlow(const Slow &s);
    void applyFast(const Fast &f);
    void onSnapshot();
    void updatePolls();

    AppContext m_ctx;
    bool m_shown = false;
    Poller::Id m_slowPoll = 0;
    Poller::Id m_fastPoll = 0;
    Slow m_slow;
    bool m_haveSlow = false;
    bool m_lgSeen = false;
    // Starvation: the previous total, the delta of the last read, event times
    bool m_haveTotal = false;
    uint32_t m_lastTotal = 0;
    uint32_t m_delta = 0;
    qint64 m_eventAt = 0;
    qint64 m_eventBefore = 0;
    // Connections: the generation seen, how many after the first
    uint64_t m_generation = 0;
    bool m_seenConnect = false;
    int m_connects = 0;
    QVariantList m_clockDiag;
    // Buffers
    bool m_streaming = false;
    bool m_pdmActive = false;
    int m_numSpdif = 0;
    QVariantMap m_buffers;
    QVariantList m_fills;
    // History per series: slots 0..3, PDM DMA, PDM ring; NaN for no sample
    std::array<QList<double>, 6> m_history;
    // Ring target and batch from the last ring-health read (0 without it)
    int m_targetFrames = 0;
    int m_batchFrames = 0;
};

#endif // STATSCONTROLLER_H
