#ifndef BOOTLOADERLOCATOR_H
#define BOOTLOADERLOCATOR_H

#include <QList>
#include <QObject>
#include <QSet>
#include <QString>
#include <QTimer>
#include <functional>

class Core;

// -- WP QP16 -- The boards sitting in their USB bootloader (BOOTSEL), and
// where to write to each (the Mac's SystemBootloaderLocator). Two signals are
// paired: the core's bus count says which chip and how many
// (dspi_bootloader_counts), the core's drive search says where to write
// (dspi_bootloader_drives: the drives whose INFO_UF2.TXT names a boot ROM;
// Linux mountinfo and /run/media, Windows removable drives by label). A board
// whose drive has not appeared yet is reported without one, so the installer
// can tell "no board" from "no drive yet".
//
// On Linux a board on the bus with no mounted drive is mounted by its volume
// label (/dev/disk/by-label/RPI-RP2 or RP2350, `udisksctl mount`), once per
// arrival. Windows mounts the drive itself.
//
// Scans run on the core's worker once a second while started (only while the
// firmware window shows). The base's Poller is not used: it stops while no
// device is open, which is exactly when a board is in its bootloader.
class BootloaderLocator : public QObject {
    Q_OBJECT
public:
    // Chips, as the core numbers them (BootloaderDrive.platform)
    enum Chip { RP2040 = 0, RP2350 = 1 };
    struct Board {
        int chip = RP2040;
        QString drive;   // the drive's root, "" while it has not appeared
        bool operator==(const Board &o) const { return chip == o.chip && drive == o.drive; }
    };
    struct Drive {
        QString path;
        int chip = -1;
    };
    struct Scan {
        int rp2040 = 0;
        int rp2350 = 0;
        QList<Drive> drives;
    };
    // A scan, on the worker thread: the core's by default
    using ScanFn = std::function<Scan()>;

    explicit BootloaderLocator(Core *core, QObject *parent = nullptr);

    void start();
    void stop();
    bool running() const { return m_timer.isActive(); }
    // The last scan's boards, never a fresh one
    QList<Board> boards() const { return m_boards; }

    // The chip's volume label (RPI-RP2, RP2350) and its name for people
    static QString volumeName(int chip);
    static QString displayName(int chip);
    // Boards from a scan: each chip's bus count, at least as many as its
    // drives; the first ones get the drives
    static QList<Board> pair(const Scan &scan);

    // The smoke driver's fake bus (no USB, no mounts); null restores the core's
    void setScanForTest(ScanFn scan);

signals:
    // After every scan, changed or not: the installer counts the ticks a
    // board waits for its drive
    void scanned(const QList<BootloaderLocator::Board> &boards);

private:
    void tick();
    void mountByLabel(const Scan &scan);

    Core *m_core;
    QTimer m_timer;
    bool m_inFlight = false;
    QList<Board> m_boards;
    ScanFn m_scanForTest;
    QSet<int> m_mountTried;   // chips asked to mount since they arrived
};

#endif // BOOTLOADERLOCATOR_H
