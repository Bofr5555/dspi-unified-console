// -- WP QD10 -- The fake hub process and what the Link smoke files share
// (SmokeDriverLink.cpp, SmokeDriverLinkFault.cpp); moved from SmokeDriverLink.cpp.
#ifndef LINKFAKEHUB_H
#define LINKFAKEHUB_H

extern "C" {
#include "dspi_core.h"
}

#include <QByteArray>
#include <QProcess>
#include <QQuickItem>
#include <QRegularExpression>
#include <QString>
#include <QStringList>
#include <QtNumeric>
#include <functional>
#include <memory>

namespace linksmoke {

// One dspi-link-fakehub process, with its faults on stdin
class FakeHubProcess {
public:
    bool start(const QStringList &args) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_FAKEHUB");
        if (binary.isEmpty()) return false;
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList { "--bind", "127.0.0.1:0" } + args);
        if (!m_proc.waitForStarted(5000) || !m_proc.waitForReadyRead(5000)) return false;
        // fake DSPi Link hub "NAME" (kind) on ADDR id ID auth AUTH
        const QString line = QString::fromUtf8(m_proc.readLine());
        const auto match = QRegularExpression(QStringLiteral(" on ([0-9.:]+) id ([^ ]+) ")).match(line);
        if (!match.hasMatch()) return false;
        address = match.captured(1);
        id = match.captured(2);
        url = QStringLiteral("ws://%1/dspi/v1").arg(address);
        return true;
    }
    // True once the hub says the fault has taken effect (-- WP QD8b --)
    bool fault(const QByteArray &command) {
        m_proc.write(command + "\n");
        if (!m_proc.waitForBytesWritten(5000)) return false;
        while (!m_proc.canReadLine())
            if (!m_proc.waitForReadyRead(10000)) return false;
        return m_proc.readLine().trimmed() == "ok";
    }
    void kill() {
        m_proc.kill();
        m_proc.waitForFinished(2000);
    }
    ~FakeHubProcess() { kill(); }
    QString address, id, url;

private:
    QProcess m_proc;
};

inline QQuickItem *findItem(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findItem(c, name)) return found;
    return nullptr;
}

// -- WP QD8b -- The analyser is polling: the hub's bands change, and the snapshot
// shows it. Comparing snapshot versions over time cannot say: the version moves only
// when the picture does, and the fake hub's signal is constant, so a version read
// after the first picture never moves again, whatever the worker does. Only the wait
// is a timeout here, for a worker that has really stopped.
inline bool analyserFollowsHub(FfiCore *core, FakeHubProcess &hub,
                               const std::function<bool(int, const std::function<bool()> &)> &waitFor) {
    static int nextLevel = 100;
    constexpr int kWaitMs = 20000;
    auto snapshot = std::make_unique<RtaSnapshot>();
    const auto level = [&] {
        dspi_rta_get_snapshot(core, snapshot.get());
        return snapshot->count > 0 ? snapshot->frames[0].avg_db[0] : qQNaN();
    };
    if (!waitFor(kWaitMs, [&] { return !qIsNaN(level()); })) return false;
    const float before = level();
    if (!hub.fault(QByteArray("rta-level ") + QByteArray::number(nextLevel++))) return false;
    return waitFor(kWaitMs, [&] {
        const float now = level();
        return !qIsNaN(now) && now != before;
    });
}

}

#endif
