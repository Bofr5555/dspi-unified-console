#include "HostAudioFormat.h"

#ifdef Q_OS_LINUX

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QRegularExpression>

namespace HostAudio {

namespace {

QString readText(const QString &path) {
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) return QString();
    return QString::fromUtf8(f.readAll()).trimmed();
}

// ALSA: each sound card of a USB device. The serial is in the USB device's
// directory above the card's interface; the channel count is in the playback
// stream's hw_params while something has it open ("closed" otherwise, when
// the OS gives no answer). PipeWire and PulseAudio open the card through ALSA,
// so this sees their format too while they play.
class AlsaBackend : public Backend {
public:
    std::vector<Endpoint> read() override {
        std::vector<Endpoint> out;
        const QDir cards(QStringLiteral("/sys/class/sound"));
        static const QRegularExpression cardName(QStringLiteral("^card(\\d+)$"));
        static const QRegularExpression channelsLine(QStringLiteral("^channels:\\s*(\\d+)"),
                                                     QRegularExpression::MultilineOption);
        for (const QString &name : cards.entryList(QDir::Dirs | QDir::NoDotAndDotDot)) {
            const auto m = cardName.match(name);
            if (!m.hasMatch()) continue;
            // card -> the USB interface -> the USB device, which holds the serial
            const QString device = QFileInfo(cards.filePath(name) + QStringLiteral("/device")).canonicalFilePath();
            if (device.isEmpty() || !device.contains(QLatin1String("/usb"))) continue;
            Endpoint e;
            e.serialText = readText(device + QStringLiteral("/../serial"));
            e.text = readText(cards.filePath(name) + QStringLiteral("/id")) + QLatin1Char(' ')
                     + readText(device + QStringLiteral("/../product"));
            const QString proc = QStringLiteral("/proc/asound/card%1/").arg(m.captured(1));
            for (const QString &pcm : QDir(proc).entryList({ QStringLiteral("pcm*p") }, QDir::Dirs)) {
                const auto c = channelsLine.match(readText(proc + pcm + QStringLiteral("/sub0/hw_params")));
                if (c.hasMatch()) {
                    e.channels = c.captured(1).toInt();
                    break;
                }
            }
            out.push_back(e);
        }
        return out;
    }
};

} // namespace

std::unique_ptr<Backend> makeBackend() { return std::make_unique<AlsaBackend>(); }

} // namespace HostAudio

#elif !defined(Q_OS_WIN)

namespace HostAudio {
// No backend here: the device's own count is used
std::unique_ptr<Backend> makeBackend() {
    struct None : Backend {
        std::vector<Endpoint> read() override { return {}; }
    };
    return std::make_unique<None>();
}
} // namespace HostAudio

#endif
