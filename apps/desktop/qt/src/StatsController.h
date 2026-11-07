#ifndef STATSCONTROLLER_H
#define STATSCONTROLLER_H

#include <QObject>
#include <QTimer>
#include <QVariantMap>
#include <QQuickPaintedItem>
#include <QPointer>
#include <QColor>
#include <QThreadPool>
#include <array>
#include <atomic>
#include <functional>

extern "C" {
#include "dspi_core.h"
}

class DSPiBridge;

// System Statistics: device counters every 2 s and buffer fill levels every
// 60 ms (with a 15 s history for the traces), polled only while the window
// is open. infoChanged and buffersChanged fire only when a value changed.
// Clock diagnostics come every 2 s, off the UI thread. Firmware without them
// stalls the request, which leaves clockDiag empty and hides the section.
class StatsController : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool watching READ watching WRITE setWatching NOTIFY watchingChanged)
    Q_PROPERTY(QVariantMap info READ info NOTIFY infoChanged)
    Q_PROPERTY(QVariantMap buffers READ buffers NOTIFY buffersChanged)
    Q_PROPERTY(int reconnects READ reconnects NOTIFY infoChanged)
    // -- WP QD5 -- [{title, value}], empty when the device reports none
    Q_PROPERTY(QVariantList clockDiag READ clockDiag NOTIFY clockDiagChanged)

public:
    // Trace series: S/PDIF consumers 0-3, PDM DMA, PDM ring
    enum { SeriesCount = 6, PdmDma = 4, PdmRing = 5, HistoryLength = 256 };
    static constexpr quint8 Absent = 0xFF;

    explicit StatsController(DSPiBridge *bridge, QObject *parent = nullptr);
    ~StatsController() override;

    bool watching() const { return m_watching; }
    void setWatching(bool w);
    QVariantMap info() const { return m_info; }
    QVariantMap buffers() const { return m_buffers; }
    int reconnects() const { return m_reconnects; }
    QVariantList clockDiag() const { return m_clockDiag; }

    // For the smoke driver: runs on the worker instead of the read and returns
    // its JSON, as dspi_fetch_clock_diag would. The demo device has no clock
    // diagnostics to show.
    void setClockDiagHook(std::function<QByteArray()> hook) { m_clockDiagHook = std::move(hook); }

    // Oldest first; Absent where the buffer wasn't running
    const std::array<quint8, HistoryLength> &history(int series) const { return m_history[series]; }

    Q_INVOKABLE void resetWatermarks();

signals:
    void watchingChanged();
    void infoChanged();
    void buffersChanged();
    void clockDiagChanged();

private:
    void pollSlow();
    void pollFast();
    void updateTimers();
    void onDeviceChanged();
    void pollClockDiag();
    void setClockDiag(const QVariantList &rows);

    DSPiBridge *m_bridge;
    FfiCore *m_core;
    bool m_watching = false;
    QTimer m_slow, m_fast;
    QVariantMap m_info, m_buffers;
    std::array<std::array<quint8, HistoryLength>, SeriesCount> m_history;   // oldest first

    // Starvation baseline and event times (host clock, ms since epoch)
    bool m_haveStarvation = false;
    quint32 m_starvationTotal = 0;
    qint64 m_lastEvent = 0, m_previousEvent = 0;
    bool m_lgSupported = false;

    QString m_serial;
    bool m_seenDevice = false;
    int m_reconnects = 0;

    // -- WP QD5 --
    QVariantList m_clockDiag;
    bool m_clockDiagBusy = false;
    std::atomic<unsigned> m_generation { 0 };   // a read in flight is dropped at a change of device
    std::function<QByteArray()> m_clockDiagHook;
    QThreadPool m_worker;                       // one thread, joined by the destructor
};

// One buffer's fill over the last 15 s: the watermark band, a 50 % guide
// and the trace (dashed for the PDM ring), oldest on the left.
class BufferTraceItem : public QQuickPaintedItem
{
    Q_OBJECT
    Q_PROPERTY(QObject *controller READ controller WRITE setController)
    Q_PROPERTY(int series MEMBER m_series NOTIFY changed)
    Q_PROPERTY(QColor color MEMBER m_color NOTIFY changed)
    Q_PROPERTY(bool dashed MEMBER m_dashed NOTIFY changed)
    Q_PROPERTY(int bandMin MEMBER m_bandMin NOTIFY changed)
    Q_PROPERTY(int bandMax MEMBER m_bandMax NOTIFY changed)

public:
    explicit BufferTraceItem(QQuickItem *parent = nullptr);
    QObject *controller() const { return m_stats; }
    void setController(QObject *c);
    void paint(QPainter *p) override;

signals:
    void changed();

private:
    QPointer<StatsController> m_stats;
    int m_series = 0;
    QColor m_color = Qt::white;
    bool m_dashed = false;
    int m_bandMin = 100, m_bandMax = 0;
};

#endif // STATSCONTROLLER_H
