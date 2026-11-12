#include "Poller.h"

#include "Core.h"
#include "CoreApi.h"
#include "DeviceModel.h"

#include <QJSEngine>
#include <QThread>

namespace {
Poller *s_instance = nullptr;

// One status read, with whether it worked. Worker thread.
struct StatusRead {
    bool ok = false;
    SystemStatus status{};
};
StatusRead readStatus(FfiCore *c) {
    StatusRead r;
    r.ok = dspi_fetch_status(c, &r.status);
    return r;
}
} // namespace

Poller::Poller(Core *core, DeviceModel *device, QObject *parent) : QObject(parent), m_core(core), m_device(device) {
    s_instance = this;
    m_meterPoll = add(QStringLiteral("meters"), 60, this, &readStatus, [this](const StatusRead &r) {
        if (!r.ok) return;
        const int n = qMin(int(r.status.num_channels), MAX_CHANNELS);
        QVariantList peaks;
        peaks.reserve(n);
        for (int i = 0; i < n; ++i) peaks.append(double(r.status.peaks[i]));
        m_peaks = peaks;
        m_clipFlags = r.status.clip_flags;
        emit metersChanged();
    });
    m_statusPoll = add(QStringLiteral("status"), 1000, this, &readStatus, [this](const StatusRead &r) {
        if (!r.ok) return;
        const int cpu0 = r.status.cpu0, cpu1 = r.status.cpu1, inputs = r.status.active_input_channels;
        if (cpu0 == m_cpu0 && cpu1 == m_cpu1 && inputs == m_activeInputs) return;
        m_cpu0 = cpu0;
        m_cpu1 = cpu1;
        m_activeInputs = inputs;
        emit statusChanged();
    });
    connect(device, &DeviceModel::snapshotApplied, this, &Poller::updateRunning);
    updateRunning();
}

Poller::Poll::~Poll() {
    if (QThread::currentThread() == timer.thread()) return;
#ifdef DSPI_SMOKE
    qFatal("Poller: the poll \"%s\" is destroyed on thread %p, its timer lives on %p", qPrintable(name),
           static_cast<void *>(QThread::currentThread()), static_cast<void *>(timer.thread()));
#else
    qWarning("Poller: the poll \"%s\" is destroyed off its timer's thread", qPrintable(name));
#endif
}

Poller::~Poller() {
    if (s_instance == this) s_instance = nullptr;
}

Poller *Poller::instance() { return s_instance; }

Poller *Poller::create(QQmlEngine *, QJSEngine *) {
    QJSEngine::setObjectOwnership(s_instance, QJSEngine::CppOwnership);
    return s_instance;
}

Poller::Id Poller::addRunner(const QString &name, int intervalMs, QObject *ctx, Runner run) {
    const Id id = m_nextId++;
    auto poll = std::make_shared<Poll>();
    poll->name = name;
    poll->interval = qMax(10, intervalMs);
    poll->hasCtx = ctx != nullptr;
    poll->ctx = ctx;
    poll->run = std::move(run);
    connect(&poll->timer, &QTimer::timeout, this, [this, id]() { tick(id); });
    m_polls.insert(id, poll);
    schedule(*poll);
    return id;
}

void Poller::remove(Id id) {
    // A read in flight finds no poll when it comes back, and is dropped.
    // -- WP QPI2 -- the timer stops here, on the GUI thread, whoever still
    // holds the poll
    if (const std::shared_ptr<Poll> poll = m_polls.take(id)) poll->timer.stop();
}

void Poller::setEnabled(Id id, bool enabled) {
    if (const std::shared_ptr<Poll> poll = m_polls.value(id)) {
        poll->enabled = enabled;
        schedule(*poll);
    }
}

void Poller::setInterval(Id id, int intervalMs) {
    if (const std::shared_ptr<Poll> poll = m_polls.value(id)) {
        poll->interval = qMax(10, intervalMs);
        schedule(*poll);
    }
}

Poller::Counts Poller::counts(Id id) const {
    const std::shared_ptr<Poll> poll = m_polls.value(id);
    return poll ? poll->counts : Counts{};
}

void Poller::setSlowTransport(bool slow) {
    if (slow == m_slow) return;
    m_slow = slow;
    for (const std::shared_ptr<Poll> &poll : std::as_const(m_polls)) schedule(*poll);
    emit slowTransportChanged();
}

void Poller::schedule(Poll &poll) {
    if (!m_running || !poll.enabled) {
        poll.timer.stop();
        return;
    }
    poll.timer.setInterval(m_slow ? qMax(poll.interval, kSlowIntervalMs) : poll.interval);
    if (!poll.timer.isActive()) poll.timer.start();
}

void Poller::updateRunning() {
    const bool running = m_device->connected() && !m_core->isShutDown();
    if (running == m_running) return;
    m_running = running;
    for (const std::shared_ptr<Poll> &poll : std::as_const(m_polls)) schedule(*poll);
    emit runningChanged();
}

void Poller::tick(Id id) {
    const std::shared_ptr<Poll> poll = m_polls.value(id);
    if (!poll || !m_running) return;
    if (poll->inFlight) {
        ++poll->counts.skipped;
        return;
    }
    poll->inFlight = true;
    ++poll->counts.posted;
    Runner run = poll->run;
    // -- WP QPI2 -- the read's closures carry the poll's id, never the poll:
    // only m_polls (the GUI thread) owns a Poll and its timer
    m_core->worker()->call(
        this, [run](FfiCore *c) mutable { return run(c); },
        [this, id](std::function<void()> apply) {
            const std::shared_ptr<Poll> poll = m_polls.value(id);
            if (!poll) return;
            poll->inFlight = false;
            ++poll->counts.completed;
            if (poll->hasCtx && !poll->ctx) return;
            apply();
        });
}
