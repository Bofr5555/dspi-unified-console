#ifndef EXTCONTROLLER_H
#define EXTCONTROLLER_H

#include <QByteArray>
#include <QObject>
#include <QString>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <atomic>
#include <functional>
#include <utility>

extern "C" {
#include "dspi_core.h"
}

class DSPiBridge;

// The extension channel, as QML sees it: `ext`. Today the crossfeed pair map:
// which two outputs each crossfeed pair processes (firmware with
// EXT_CROSSFEED_PAIR_MAP). hasPairMap is the core's connect probe, so firmware
// that stalls the channel never shows any of this.
//
// Every write sends the whole map, since pairs must stay disjoint, and what is
// shown is the map the device reports. Device calls run on one worker, in order;
// their results come back on the GUI thread. All writes go through applyMap().
class ExtController : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool hasPairMap READ hasPairMap NOTIFY changed)
    Q_PROPERTY(int pairCount READ pairCount NOTIFY changed)
    // One { left, right } per pair, 0-based outputs
    Q_PROPERTY(QVariantList pairs READ pairs NOTIFY changed)
    // "default", "k340" or "custom": the preset the map matches
    Q_PROPERTY(QString choice READ choice NOTIFY changed)
    Q_PROPERTY(bool busy READ busy NOTIFY changed)          // a write is on its way
    Q_PROPERTY(QString lastError READ lastError NOTIFY changed)
    // Both pairs: the factory mask has pair 1 only, the woofers alone on a K340
    Q_PROPERTY(int k340PairMask READ k340PairMask CONSTANT)
    // The window is open: follow changes from elsewhere (a preset load, a restore)
    Q_PROPERTY(bool watching READ watching WRITE setWatching NOTIFY watchingChanged)

public:
    explicit ExtController(DSPiBridge *bridge, QObject *parent = nullptr);
    ~ExtController() override;

    bool hasPairMap() const { return m_hasPairMap; }
    int pairCount() const { return m_pairCount; }
    QVariantList pairs() const;
    QString choice() const;
    bool busy() const { return m_writes > 0; }
    QString lastError() const { return m_lastError; }
    int k340PairMask() const { return 0x03; }
    bool watching() const { return m_watching; }
    void setWatching(bool on);

    // Whether the device would take these pairs (one { left, right } per pair)
    Q_INVOKABLE bool pairsValid(const QVariantList &pairs) const;
    // False refuses it here, with lastError set and nothing sent. True means it is
    // on its way; applyFinished reports how it went.
    Q_INVOKABLE bool applyPairs(const QVariantList &pairs);
    Q_INVOKABLE bool applyPreset(const QString &name);
    Q_INVOKABLE void clearError();
    Q_INVOKABLE void refresh();

    // Stands in for the core's dspi_ext_status reply; empty goes back to the core.
    // The demo device always has the channel, and the smoke driver needs one without.
    void setStatusOverride(const QByteArray &json);
    // For the smoke driver: runs on the worker before each write, to hold it there.
    // False refuses that write, as the device would. Set it with no write under way.
    void setWriteHook(std::function<bool()> hook) { m_writeHook = std::move(hook); }

signals:
    void changed();
    void watchingChanged();
    void applyFinished(bool ok);

private:
    static constexpr int kMapLen = 4;

    void syncStatus();
    void refreshSoon();
    void applyMap(const QByteArray &map);
    void setMap(const QByteArray &map);
    void setError(const QString &error);
    QByteArray pairsToMap(const QVariantList &pairs, bool *ok) const;
    // Runs on the worker; `done` runs on the GUI thread, unless the device changed
    void onWorker(std::function<bool(uint8_t *)> job, std::function<void(bool, QByteArray)> done);

    DSPiBridge *m_bridge;
    FfiCore *m_core;
    QThreadPool m_worker;
    QTimer m_refreshTimer;
    QString m_serial;
    QByteArray m_statusOverride;
    QByteArray m_map;
    QString m_lastError;
    // Bumped when the device changes: older writes don't go out, older results are dropped
    std::atomic<quint64> m_generation { 0 };
    std::function<bool()> m_writeHook;
    int m_pairCount = 1;
    int m_writes = 0;
    bool m_hasPairMap = false;
    bool m_watching = false;
    bool m_refreshPending = false;
};

#endif // EXTCONTROLLER_H
