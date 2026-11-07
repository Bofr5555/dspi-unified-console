#include "HostAudioFormat.h"

#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QProcessEnvironment>
#include <QRegularExpression>
#include <QStandardPaths>
#include <algorithm>

#ifdef Q_OS_LINUX

namespace HostAudio {

namespace {
constexpr int kMaxChannels = 64;
constexpr qint64 kMaxProcFile = 16 * 1024;
// pw-dump --monitor ends when PipeWire restarts. One that ended sooner than this
// isn't going to work, as on a host with the tool but no daemon: no retries.
constexpr int kMonitorRestartMs = 5000;

int channelsFrom(int n) { return n >= 1 && n <= kMaxChannels ? n : 0; }

// pw-dump writes numbers as numbers or as strings, by version
int intOf(const QJsonValue &v) {
    if (v.isDouble()) return v.toInt();
    return v.toString().toInt();
}

int nodeChannels(const QJsonObject &info, const QJsonObject &props) {
    if (const int n = channelsFrom(intOf(props.value(QLatin1String("audio.channels"))))) return n;
    const QJsonArray formats = info.value(QLatin1String("params")).toObject().value(QLatin1String("Format")).toArray();
    if (!formats.isEmpty()) {
        const QJsonValue v = formats.first().toObject().value(QLatin1String("channels"));
        if (const int n = channelsFrom(intOf(v))) return n;
    }
    const QStringList positions = props.value(QLatin1String("audio.position")).toString()
        .split(QRegularExpression(QStringLiteral("[,\\s]+")), Qt::SkipEmptyParts);
    return channelsFrom(int(positions.size()));
}

// Read a small text file; /proc files report size 0
QByteArray readSmall(const QString &path) {
    QFile f(path);
    return f.open(QIODevice::ReadOnly) ? f.read(kMaxProcFile) : QByteArray();
}

int openChannels(const QString &cardDir) {
    static const QRegularExpression channels(QStringLiteral("^channels:\\s*(\\d+)"), QRegularExpression::MultilineOption);
    const QDir dir(cardDir);
    const QStringList pcms = dir.entryList({ QStringLiteral("pcm*p") }, QDir::Dirs);
    for (const QString &pcm : pcms) {
        const QDir pcmDir(dir.filePath(pcm));
        const QStringList subs = pcmDir.entryList({ QStringLiteral("sub*") }, QDir::Dirs);
        for (const QString &sub : subs) {
            const QString text = QString::fromUtf8(readSmall(pcmDir.filePath(sub) + QStringLiteral("/hw_params")));
            const QRegularExpressionMatch m = channels.match(text);
            if (m.hasMatch()) return channelsFrom(m.captured(1).toInt());
        }
    }
    return 0;
}

// Run from a package like an AppImage, the bundled libraries must not reach the
// host's tool
QProcessEnvironment toolEnvironment() {
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.remove(QStringLiteral("LD_LIBRARY_PATH"));
    env.remove(QStringLiteral("LD_PRELOAD"));
    return env;
}

class LinuxBackend : public Backend {
public:
    explicit LinuxBackend(const LinuxPaths &paths) : m_paths(paths) {
        m_restart.setSingleShot(true);
        m_restart.setInterval(kMonitorRestartMs);
        QObject::connect(&m_restart, &QTimer::timeout, &m_restart, [this]() { startMonitor(); });
    }
    ~LinuxBackend() override { LinuxBackend::stopWatching(); }

    std::vector<Endpoint> read() override {
        bool ok = false;
        std::vector<Endpoint> sinks = parsePipeWireDump(runDump(), &ok);
        if (ok && std::any_of(sinks.begin(), sinks.end(), isDspi)) return sinks;
        return readAlsa(m_paths.procAsound, m_paths.sysClassSound);
    }

    void startWatching(std::function<void()> changed) override {
        m_changed = std::move(changed);
        m_watching = true;
        startMonitor();
    }

    void stopWatching() override {
        m_watching = false;
        m_restart.stop();
        if (!m_monitor) return;
        m_monitor->disconnect();
        m_monitor->kill();
        m_monitor->waitForFinished(500);
        m_monitor.reset();
    }

private:
    QString program() const { return QStandardPaths::findExecutable(m_paths.pwDump); }

    QByteArray runDump() const {
        const QString tool = program();
        if (tool.isEmpty()) return {};
        QProcess p;
        p.setProcessEnvironment(toolEnvironment());
        p.setStandardErrorFile(QProcess::nullDevice());
        p.start(tool, QStringList());
        if (!p.waitForFinished(m_paths.timeoutMs)) {
            p.kill();
            p.waitForFinished(500);
            return {};
        }
        if (p.exitStatus() != QProcess::NormalExit || p.exitCode() != 0) return {};
        return p.readAllStandardOutput();
    }

    void startMonitor() {
        const QString tool = program();
        if (!m_watching || tool.isEmpty() || m_monitor) return;
        m_monitor = std::make_unique<QProcess>();
        QProcess *p = m_monitor.get();
        p->setProcessEnvironment(toolEnvironment());
        p->setStandardErrorFile(QProcess::nullDevice());
        QObject::connect(p, &QProcess::readyReadStandardOutput, p, [this, p]() {
            p->readAllStandardOutput();     // that something changed is all it says here
            m_changed();
        });
        QObject::connect(p, QOverload<int, QProcess::ExitStatus>::of(&QProcess::finished), p, [this, p]() {
            const bool lasted = m_monitorAge.elapsed() >= kMonitorRestartMs;
            p->disconnect();
            p->deleteLater();
            (void)m_monitor.release();
            m_changed();
            if (lasted && m_watching) m_restart.start();
        });
        QObject::connect(p, &QProcess::errorOccurred, p, [this, p](QProcess::ProcessError error) {
            if (error != QProcess::FailedToStart) return;
            p->disconnect();
            p->deleteLater();
            (void)m_monitor.release();
        });
        m_monitorAge.start();
        p->start(tool, { QStringLiteral("--monitor") });
    }

    LinuxPaths m_paths;
    std::function<void()> m_changed;
    std::unique_ptr<QProcess> m_monitor;
    QElapsedTimer m_monitorAge;
    QTimer m_restart;
    bool m_watching = false;
};
}

std::vector<Endpoint> parsePipeWireDump(const QByteArray &dump, bool *ok) {
    std::vector<Endpoint> out;
    const QJsonDocument doc = QJsonDocument::fromJson(dump);
    if (ok) *ok = doc.isArray();
    const QJsonArray objects = doc.array();
    for (const QJsonValue &v : objects) {
        const QJsonObject object = v.toObject();
        if (object.value(QLatin1String("type")).toString() != QLatin1String("PipeWire:Interface:Node")) continue;
        const QJsonObject info = object.value(QLatin1String("info")).toObject();
        const QJsonObject props = info.value(QLatin1String("props")).toObject();
        if (props.value(QLatin1String("media.class")).toString() != QLatin1String("Audio/Sink")) continue;
        // A filter or loopback named after the DSPi is not the DSPi
        const bool alsa = props.contains(QLatin1String("api.alsa.path"))
            || props.value(QLatin1String("device.api")).toString() == QLatin1String("alsa")
            || props.value(QLatin1String("factory.name")).toString().startsWith(QLatin1String("api.alsa"));
        if (!alsa) continue;
        const auto prop = [&props](const char *key) { return props.value(QLatin1String(key)).toString(); };
        Endpoint e;
        e.text = QStringList { prop("node.name"), prop("node.description"), prop("node.nick"),
                               prop("alsa.card_name"), prop("alsa.long_card_name") }.join(QLatin1Char(' '));
        // node.name is alsa_output.usb-<vendor>_<product>_<serial>-00.<profile>
        e.serialText = prop("node.name") + QLatin1Char(' ') + prop("device.serial");
        e.channels = nodeChannels(info, props);
        out.push_back(e);
    }
    return out;
}

std::vector<Endpoint> readAlsa(const QString &procRoot, const QString &sysClassSound) {
    std::vector<Endpoint> out;
    static const QRegularExpression card(QStringLiteral("^\\s*(\\d+)\\s*\\[(.*)$"), QRegularExpression::MultilineOption);
    const QString cards = QString::fromUtf8(readSmall(procRoot + QStringLiteral("/cards")));
    QRegularExpressionMatchIterator it = card.globalMatch(cards);
    while (it.hasNext()) {
        const QRegularExpressionMatch m = it.next();
        const QString index = m.captured(1);
        const QString dir = procRoot + QStringLiteral("/card") + index;
        if (!QFile::exists(dir + QStringLiteral("/usbid"))) continue;   // the USB cards
        Endpoint e;
        e.text = m.captured(2);
        // sound/cardN/device is the USB interface; its parent is the USB device
        e.serialText = QString::fromUtf8(readSmall(sysClassSound + QStringLiteral("/card") + index
                                                   + QStringLiteral("/device/../serial"))).trimmed();
        e.channels = openChannels(dir);
        out.push_back(e);
    }
    return out;
}

std::unique_ptr<Backend> makeLinuxBackend(const LinuxPaths &paths) {
    return std::make_unique<LinuxBackend>(paths);
}

std::unique_ptr<Backend> makeBackend() { return makeLinuxBackend(LinuxPaths()); }

} // namespace HostAudio

#endif // Q_OS_LINUX
