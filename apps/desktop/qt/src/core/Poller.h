#ifndef POLLER_H
#define POLLER_H

#include <QHash>
#include <QObject>
#include <QPointer>
#include <QTimer>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <functional>
#include <memory>

class Core;
class DeviceModel;
class QQmlEngine;
class QJSEngine;
struct FfiCore;

// The one scheduler of the device's periodic reads. Each poll has a timer on
// the GUI thread and reads on the core's worker; its result is applied back on
// the GUI thread. A tick is skipped while the poll's previous read is still in
// flight, so a slow link never queues reads. Nothing polls while no device is
// open; everything resumes when one opens. Polls are never paused for any
// other reason (a bind, a dialog): the meters keep moving.
//
// Two polls are the base's (the Mac console's rates):
// - meters, every 60 ms: dspi_fetch_status's peaks and clip flags
//   (`peaks`, `clipFlags`, metersChanged);
// - status, every 1 s: the CPU loads and the active input count
//   (`cpu0`, `cpu1`, `activeInputs`, statusChanged).
// Over a slow transport (a Link bridge; LinkController sets slowTransport)
// every poll runs at 2 Hz at most.
//
// -- WP MDQT1 -- It reads the active device: after a switch the next tick is the
// new one's, a read of the old one still in flight is dropped, and the meters
// start over. The other open devices are read once a second by their sessions
// (DeviceSession).
//
// A work package adds its own poll instead of a timer of its own:
//
//     m_poll = ctx.poller->add(QStringLiteral("subharmonic meter"), 100, this,
//         [](FfiCore *c) { float v = 0; return dspi_fetch_subharm_meter(c, &v) ? v : NAN; },  // worker
//         [this](float v) { if (!std::isnan(v)) setMeter(v); });                              // GUI
//     ctx.poller->setEnabled(m_poll, windowOpen);   // e.g. only while its window shows
//
// `apply` runs only while `ctx` exists; remove() drops a poll for good.
class Poller : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QVariantList peaks READ peaks NOTIFY metersChanged)
    Q_PROPERTY(uint clipFlags READ clipFlags NOTIFY metersChanged)
    // -- WP QP18 -- the core's ballistics (P0M): the held peak per wire
    // channel (the hold marker), the highest since the connection opened, and
    // the clip hold (one bit per wire channel), which the clip marker shows
    Q_PROPERTY(QVariantList peakHold READ peakHold NOTIFY metersChanged)
    Q_PROPERTY(QVariantList peakMax READ peakMax NOTIFY metersChanged)
    Q_PROPERTY(uint clipHold READ clipHold NOTIFY metersChanged)
    Q_PROPERTY(int cpu0 READ cpu0 NOTIFY statusChanged)
    Q_PROPERTY(int cpu1 READ cpu1 NOTIFY statusChanged)
    Q_PROPERTY(int activeInputs READ activeInputs NOTIFY statusChanged)
    Q_PROPERTY(bool running READ running NOTIFY runningChanged)
    Q_PROPERTY(bool slowTransport READ slowTransport WRITE setSlowTransport NOTIFY slowTransportChanged)
public:
    using Id = int;
    // A poll's counts, for tests: ticks that posted a read, reads applied,
    // ticks skipped because a read was in flight
    struct Counts {
        int posted = 0;
        int completed = 0;
        int skipped = 0;
    };
    // The slowest a poll runs over a slow transport
    static constexpr int kSlowIntervalMs = 500;

    Poller(Core *core, DeviceModel *device, QObject *parent = nullptr);
    ~Poller() override;
    static Poller *instance();
    static Poller *create(QQmlEngine *, QJSEngine *);

    // Adds a poll every `intervalMs`: `read(FfiCore *)` on the worker returns a
    // value, `apply(value)` gets it on the GUI thread while `ctx` lives.
    template <class Read, class Apply>
    Id add(const QString &name, int intervalMs, QObject *ctx, Read read, Apply apply) {
        Runner run = [read = std::move(read), apply = std::move(apply)](FfiCore *core) mutable -> std::function<void()> {
            auto value = read(core);
            return [apply, value]() mutable { apply(value); };
        };
        return addRunner(name, intervalMs, ctx, std::move(run));
    }
    void remove(Id id);
    void setEnabled(Id id, bool enabled);
    void setInterval(Id id, int intervalMs);
    // -- fixup fs1 -- one read now, if the poll is enabled and a device is
    // open (its timer keeps its own cadence): for a window that would
    // otherwise show nothing until the first interval has passed
    void readNow(Id id);
    Counts counts(Id id) const;
    Id meterPoll() const { return m_meterPoll; }
    Id statusPoll() const { return m_statusPoll; }

    QVariantList peaks() const { return m_peaks; }
    uint clipFlags() const { return m_clipFlags; }
    QVariantList peakHold() const { return m_peakHold; }   // -- WP QP18 --
    QVariantList peakMax() const { return m_peakMax; }
    uint clipHold() const { return m_clipHold; }
    int cpu0() const { return m_cpu0; }
    int cpu1() const { return m_cpu1; }
    int activeInputs() const { return m_activeInputs; }
    bool running() const { return m_running; }
    bool slowTransport() const { return m_slow; }
    void setSlowTransport(bool slow);

signals:
    void metersChanged();
    void statusChanged();
    void runningChanged();
    void slowTransportChanged();

private:
    using Runner = std::function<std::function<void()>(FfiCore *)>;
    struct Poll {
        QString name;
        int interval = 0;
        bool hasCtx = false;
        QPointer<QObject> ctx;
        Runner run;
        QTimer timer;
        bool enabled = true;
        bool inFlight = false;
        Counts counts;
        // -- WP QPI3 -- checks it dies on its timer's thread (the smoke build
        // stops there, naming the poll, instead of a later crash)
        ~Poll();
    };
    Id addRunner(const QString &name, int intervalMs, QObject *ctx, Runner run);
    void tick(Id id);
    void schedule(Poll &poll);
    void updateRunning();
    void rebind();   // -- WP MDQT1 --

    Core *m_core;
    DeviceModel *m_device;
    QHash<Id, std::shared_ptr<Poll>> m_polls;
    Id m_nextId = 1;
    Id m_meterPoll = 0;
    Id m_statusPoll = 0;
    bool m_running = false;
    bool m_slow = false;
    quint64 m_epoch = 0;   // -- WP MDQT1 -- counts the switches of the active device
    QVariantList m_peaks;
    uint m_clipFlags = 0;
    QVariantList m_peakHold;   // -- WP QP18 --
    QVariantList m_peakMax;
    uint m_clipHold = 0;
    int m_cpu0 = 0;
    int m_cpu1 = 0;
    int m_activeInputs = 0;
};

#endif // POLLER_H
