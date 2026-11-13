#ifndef COREWORKER_H
#define COREWORKER_H

#include <QMutex>
#include <QObject>
#include <QPointer>
#include <QThread>
#include <atomic>
#include <deque>
#include <functional>
#include <memory>
#include <type_traits>
#include <utility>

struct FfiCore;

// The one thread that calls the core. Every blocking dspi_* call (fetches,
// setters, hot-plug polls, scans, files, reading the state record) runs here,
// in the order it was posted, so the GUI thread never waits on a device.
//
//     worker->call(this,
//         [](FfiCore *c) { return dspi_fetch_core1_mode(c); },     // worker thread
//         [this](uint8_t mode) { ... });                            // GUI thread
//
// `done` runs on the GUI thread, and only while `ctx` is alive. A job must not
// capture QObjects it touches on the worker thread; it gets the core and
// returns values.
class CoreWorker : public QObject {
    Q_OBJECT
public:
    explicit CoreWorker(FfiCore *core, QObject *parent = nullptr);
    ~CoreWorker() override;

    // Runs `job` on the worker thread after everything posted before it.
    void post(std::function<void(FfiCore *)> job);

    // Runs `job` there, then `done(result)` on the GUI thread if `ctx` still exists.
    template <class Job, class Done>
    void call(QObject *ctx, Job job, Done done) {
        QPointer<QObject> guard(ctx);
        QObject *home = m_home;
        post([job = std::move(job), done = std::move(done), guard, home](FfiCore *core) mutable {
            // -- WP QPI2 -- `done` and the result cross to the GUI thread in a
            // move-only box, so nothing they hold can be copied and left behind
            // on this thread; they are destroyed where `done` runs
            struct Box {
                Done done;
                std::decay_t<decltype(job(core))> result;
            };
            auto box = std::make_unique<Box>(Box{ std::move(done), job(core) });
            QMetaObject::invokeMethod(
                home,
                [box = std::move(box), guard]() mutable {
                    Q_ASSERT(!guard || QThread::currentThread() == guard->thread());
                    if (guard) box->done(std::move(box->result));
                    box.reset();
                },
                Qt::QueuedConnection);
        });
    }

    // Jobs posted and not finished yet (the smoke driver waits for 0).
    int pending() const { return m_pending.load(); }

    // Lets the running job finish, drops the rest and joins the thread. Called
    // once, on the GUI thread, before the core is freed; post() does nothing
    // afterwards. The jobs that never ran are destroyed on the caller's thread,
    // as every job that ran is (runOne sends it home, QPI4).
    void stop();

private:
    using Job = std::function<void(FfiCore *)>;
    // On the worker thread: the oldest job, unless stopped
    void runOne();

    FfiCore *m_core;
    QThread m_thread;
    QObject *m_runner = nullptr;   // lives on m_thread
    QObject *m_home = nullptr;     // lives on the GUI thread, delivers results
    std::atomic<int> m_pending{0};
    std::atomic<bool> m_stopped{false};
    // -- WP QPI3 -- the jobs wait here, not in the runner's event queue, so
    // stop() can take back the ones that never ran (and the `done` closures
    // they hold) and destroy them at home
    QMutex m_lock;
    std::deque<Job> m_queue;
};

#endif // COREWORKER_H
