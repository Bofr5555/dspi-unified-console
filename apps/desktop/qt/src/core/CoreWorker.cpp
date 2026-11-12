#include "CoreWorker.h"

CoreWorker::CoreWorker(FfiCore *core, QObject *parent) : QObject(parent), m_core(core) {
    m_home = new QObject(this);
    m_runner = new QObject;
    m_runner->moveToThread(&m_thread);
    connect(&m_thread, &QThread::finished, m_runner, &QObject::deleteLater);
    m_thread.setObjectName(QStringLiteral("dspi-core-worker"));
    m_thread.start();
}

CoreWorker::~CoreWorker() { stop(); }

void CoreWorker::post(std::function<void(FfiCore *)> job) {
    if (m_stopped.load()) return;
    m_pending.fetch_add(1);
    {
        QMutexLocker hold(&m_lock);
        m_queue.push_back(std::move(job));
    }
    // One wake-up per job, so the order stays the order posted
    QMetaObject::invokeMethod(m_runner, [this]() { runOne(); }, Qt::QueuedConnection);
}

void CoreWorker::runOne() {
    Job job;
    {
        QMutexLocker hold(&m_lock);
        // Stopped: what is left goes back with stop(), on the GUI thread
        if (m_stopped.load() || m_queue.empty()) return;
        job = std::move(m_queue.front());
        m_queue.pop_front();
    }
    job(m_core);
    // -- WP QPI4 -- the job that ran goes home to die: every closure the
    // worker held (a poll's read and apply, a write's pending done) is
    // destroyed on the GUI thread, never here
    QMetaObject::invokeMethod(m_home, [job = std::move(job)] {}, Qt::QueuedConnection);
    m_pending.fetch_sub(1);
}

void CoreWorker::stop() {
    if (m_stopped.exchange(true)) return;
    Q_ASSERT(QThread::currentThread() == thread());
    m_thread.quit();
    m_thread.wait();
    // -- WP QPI3 -- the jobs that never ran die here, on this thread
    std::deque<Job> unrun;
    {
        QMutexLocker hold(&m_lock);
        unrun.swap(m_queue);
    }
    unrun.clear();
    m_pending.store(0);
}
