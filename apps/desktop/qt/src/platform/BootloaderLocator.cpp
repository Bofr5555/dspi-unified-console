// -- WP QP16 -- Boards in BOOTSEL and their drives (BootloaderLocator.h)
#include "BootloaderLocator.h"

#include "Core.h"
#include "Snapshot.h"

#include <QFileInfo>
#include <QJsonArray>
#include <QJsonObject>
#include <QProcess>

namespace {
constexpr int kScanMs = 1000;

// The core's view of the bus and the drives. Worker thread.
BootloaderLocator::Scan coreScan() {
    BootloaderLocator::Scan s;
    uint32_t pico = 0, pico2 = 0;
    dspi_bootloader_counts(&pico, &pico2);
    s.rp2040 = int(pico);
    s.rp2350 = int(pico2);
    const QJsonArray drives =
        jsonValue(readSized([](char *b, uint32_t n) { return dspi_bootloader_drives(b, n); })).toArray();
    for (const QJsonValue &v : drives) {
        const QJsonObject d = v.toObject();
        s.drives.append({ d.value(QStringLiteral("path")).toString(), d.value(QStringLiteral("platform")).toInt(-1) });
    }
    return s;
}
} // namespace

BootloaderLocator::BootloaderLocator(Core *core, QObject *parent) : QObject(parent), m_core(core) {
    m_timer.setInterval(kScanMs);
    connect(&m_timer, &QTimer::timeout, this, &BootloaderLocator::tick);
}

QString BootloaderLocator::volumeName(int chip) {
    return chip == RP2350 ? QStringLiteral("RP2350") : QStringLiteral("RPI-RP2");
}

QString BootloaderLocator::displayName(int chip) {
    return chip == RP2350 ? QStringLiteral("RP2350 (Pico 2)") : QStringLiteral("RP2040 (Pico)");
}

QList<BootloaderLocator::Board> BootloaderLocator::pair(const Scan &scan) {
    QList<Board> out;
    for (int chip : { int(RP2040), int(RP2350) }) {
        QStringList paths;
        for (const Drive &d : scan.drives)
            if (d.chip == chip) paths.append(d.path);
        const int count = qMax(chip == RP2350 ? scan.rp2350 : scan.rp2040, int(paths.size()));
        for (int n = 0; n < count; ++n) out.append({ chip, paths.value(n) });
    }
    return out;
}

void BootloaderLocator::start() {
    if (!m_timer.isActive()) {
        m_mountTried.clear();
        m_timer.start();
        tick();
    }
}

void BootloaderLocator::stop() {
    m_timer.stop();
}

void BootloaderLocator::setScanForTest(ScanFn scan) {
    m_scanForTest = std::move(scan);
}

void BootloaderLocator::tick() {
    if (m_inFlight) return;
    m_inFlight = true;
    ScanFn fake = m_scanForTest;
    m_core->worker()->call(
        this, [fake](FfiCore *) { return fake ? fake() : coreScan(); },
        [this, real = !fake](Scan scan) {
            m_inFlight = false;
            if (!m_timer.isActive()) return;
            if (real) mountByLabel(scan);
            m_boards = pair(scan);
            emit scanned(m_boards);
        });
}

// A chip on the bus with no drive: ask udisks to mount its labelled block
// device, once until the chip leaves the bus. A failure only means the drive
// keeps not appearing, which the installer reports in time.
void BootloaderLocator::mountByLabel(const Scan &scan) {
#ifdef Q_OS_LINUX
    for (int chip : { int(RP2040), int(RP2350) }) {
        const int count = chip == RP2350 ? scan.rp2350 : scan.rp2040;
        bool mounted = false;
        for (const Drive &d : scan.drives) mounted = mounted || d.chip == chip;
        if (!count) {
            m_mountTried.remove(chip);
            continue;
        }
        if (mounted || m_mountTried.contains(chip)) continue;
        const QString device = QStringLiteral("/dev/disk/by-label/") + volumeName(chip);
        if (!QFileInfo::exists(device)) continue;
        m_mountTried.insert(chip);
        // Left to run on its own: the next scans see the drive, or not
        if (!QProcess::startDetached(QStringLiteral("udisksctl"),
                                     { QStringLiteral("mount"), QStringLiteral("--no-user-interaction"), QStringLiteral("-b"), device }))
            qInfo("BootloaderLocator: udisksctl could not be started for %s", qPrintable(device));
    }
#else
    Q_UNUSED(scan)
#endif
}
