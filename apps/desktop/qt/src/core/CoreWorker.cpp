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
    QMetaObject::invokeMethod(
        m_runner,
        [this, job = std::move(job)]() {
            if (!m_stopped.load()) job(m_core);
            m_pending.fetch_sub(1);
        },
        Qt::QueuedConnection);
}

void CoreWorker::stop() {
    if (m_stopped.exchange(true)) return;
    m_thread.quit();
    m_thread.wait();
    m_pending.store(0);
}
