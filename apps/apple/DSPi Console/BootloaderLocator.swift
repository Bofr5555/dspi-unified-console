import Foundation
import dspi_core
#if os(macOS)
import AppKit
#endif

// -- WP SW-M2 -- The bootloader on the bus and its drive come from the core
// (C11's `firmware.rs`: `bootloaderCounts`, `bootloaderDrives`); this file
// only pairs them and schedules the scans.

/// One board in BOOTSEL mode.
struct BootloaderBoard: Equatable {
    enum Chip: Equatable, CaseIterable {
        case rp2040, rp2350

        /// Volume the ROM loader mounts to receive a UF2.
        var volumeName: String {
            switch self {
            case .rp2040: return "RPI-RP2"
            case .rp2350: return "RP2350"
            }
        }

        /// Token in a release asset's filename, e.g. DSPi-RP2350-v1.1.7.uf2.
        var assetToken: String {
            switch self {
            case .rp2040: return "RP2040"
            case .rp2350: return "RP2350"
            }
        }

        var displayName: String {
            switch self {
            case .rp2040: return "RP2040 (Pico)"
            case .rp2350: return "RP2350 (Pico 2)"
            }
        }

        /// -- WP SW-M2 -- The core's `BootloaderDrive.platform` /
        /// `Uf2Info.platform` value of this chip.
        var platform: Int8 {
            switch self {
            case .rp2040: return 0
            case .rp2350: return 1
            }
        }
    }

    let chip: Chip

    /// -- WP SW-M2 -- The core's record of the drive to write the UF2 to (the
    /// core's `flashConfirmed` reads its `INFO_UF2.TXT` again before any
    /// byte goes out).  nil when the board is on the USB bus but its drive
    /// has not appeared - either the mount is still a moment away, or macOS
    /// refused us access to removable volumes.  Callers must not treat that
    /// as "no board"; the distinction is what lets the UI say something
    /// useful about a denied permission prompt.
    let drive: BootloaderDrive?

    init(chip: Chip, drive: BootloaderDrive?) {
        self.chip = chip
        self.drive = drive
    }

    /// A board whose drive is mounted at `volumeURL` (tests, previews).
    init(chip: Chip, volumeURL: URL?) {
        self.init(chip: chip,
                  drive: volumeURL.map { BootloaderDrive(path: $0.path, platform: chip.platform, info: "") })
    }

    /// Where the drive is mounted.
    var volumeURL: URL? { drive.map { URL(fileURLWithPath: $0.path) } }

    /// -- WP SW-M2 -- The boards the core sees: each chip on the USB bus in
    /// BOOTSEL (`counts`), the first of each kind paired with the first
    /// drive of that chip (`drives`).  With two identical boards attached
    /// only the first pairs: the installer refuses to act on more than one
    /// board anyway, and it needs the true count to say so.  A drive with no
    /// board on the bus is not reported: the USB match is what says a boot
    /// ROM is there now.
    static func pair(counts: BootloaderCounts, drives: [BootloaderDrive]) -> [BootloaderBoard] {
        Chip.allCases.flatMap { chip -> [BootloaderBoard] in
            let count = Int(chip == .rp2040 ? counts.rp2040 : counts.rp2350)
            guard count > 0 else { return [] }
            let drive = drives.first { $0.platform == chip.platform }
            return (0..<count).map { BootloaderBoard(chip: chip, drive: $0 == 0 ? drive : nil) }
        }
    }
}

/// Finds boards in BOOTSEL mode.  The protocol exists so tests and the
/// developer overrides can drive the installer with no hardware attached.
protocol BootloaderLocating: AnyObject {
    func currentBoards() -> [BootloaderBoard]
    var onChange: (([BootloaderBoard]) -> Void)? { get set }
    func start()
    func stop()
}

#if os(macOS)
/// Locates real boards through the core (-- WP SW-M2 --, C11's
/// `firmware.rs`): `bootloaderCounts()` for which chips are on the USB bus in
/// BOOTSEL, `bootloaderDrives()` for the mounted drives whose `INFO_UF2.TXT`
/// names an RP2040 or RP2350 boot ROM.  This class only pairs the two and
/// schedules the scans; it reads no USB registry and no mount table itself.
///
/// USB tells us *which chip* with certainty; the drive tells us *where to
/// write*.  The mount lags enumeration by a moment, so a board is reported
/// with a nil drive until its drive shows up rather than being hidden.
final class SystemBootloaderLocator: BootloaderLocating {
    var onChange: (([BootloaderBoard]) -> Void)?

    private var pollTimer: Timer?
    private var observers: [NSObjectProtocol] = []

    /// Scanning happens here, never on the main thread: the core's drive scan
    /// opens `INFO_UF2.TXT` on each candidate volume, and a volume a board
    /// yanked away as it rebooted can stall that.
    private let scanQueue = DispatchQueue(label: "com.foxdac.bootloader-scan")

    /// Last completed scan.  Main thread only; `currentBoards()` hands this
    /// back rather than scanning inline, so no caller can block on a scan.
    private var lastScan: [BootloaderBoard] = []

    /// A scan is under way (main thread).  A stalled scan must not let the
    /// one-second poll pile scans up behind it on `scanQueue`.
    private var scanning = false

    func start() {
        let centre = NSWorkspace.shared.notificationCenter
        for name in [NSWorkspace.didMountNotification, NSWorkspace.didUnmountNotification] {
            observers.append(centre.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                self?.rescan()
            })
        }

        // Mount notifications cover the volume appearing, but nothing tells us
        // a board reached the USB bus without mounting (the permission-denied
        // case, and the moment between enumeration and mount).  A one-second
        // poll fills that gap.  It only runs while a flashing UI is on screen,
        // so the cost is bounded.
        let timer = Timer(timeInterval: 1.0, repeats: true) { [weak self] _ in self?.rescan() }
        RunLoop.main.add(timer, forMode: .common)
        pollTimer = timer

        rescan()
    }

    func stop() {
        pollTimer?.invalidate()
        pollTimer = nil
        let centre = NSWorkspace.shared.notificationCenter
        observers.forEach { centre.removeObserver($0) }
        observers.removeAll()
    }

    deinit { stop() }

    /// The most recent scan, not a fresh one.  Deliberate: see `scanQueue`.
    func currentBoards() -> [BootloaderBoard] { lastScan }

    /// Reports on every tick rather than only on change: a board sitting on
    /// the bus with no drive yet looks identical scan to scan, and the
    /// installer needs those ticks to decide the drive is not coming.  The
    /// installer drops states that did not change, so this costs no UI churn.
    private func rescan() {
        guard !scanning else { return }
        scanning = true
        scanQueue.async { [weak self] in
            guard let self else { return }
            let boards = BootloaderBoard.pair(counts: bootloaderCounts(), drives: bootloaderDrives())
            DispatchQueue.main.async {
                self.scanning = false
                self.lastScan = boards
                self.onChange?(boards)
            }
        }
    }
}
#else
/// iOS cannot see a board in BOOTSEL: the core is built without USB there
/// (`bootloaderCounts` is 0/0) and has no drive to find (`bootloaderDrives` is
/// empty), so the locator reports no boards without asking (WP SW18: there is
/// no firmware window on iOS).
final class SystemBootloaderLocator: BootloaderLocating {
    var onChange: (([BootloaderBoard]) -> Void)?
    func currentBoards() -> [BootloaderBoard] { [] }
    func start() { onChange?([]) }
    func stop() {}
}
#endif

/// Test and developer-override double.  Set `boards`, then call `emit()` to
/// drive the installer through any state without hardware.
final class FakeBootloaderLocator: BootloaderLocating {
    var onChange: (([BootloaderBoard]) -> Void)?
    var boards: [BootloaderBoard] = []
    private(set) var started = false

    init(boards: [BootloaderBoard] = []) { self.boards = boards }

    func currentBoards() -> [BootloaderBoard] { boards }
    func start() { started = true; onChange?(boards) }
    func stop() { started = false }
    func emit() { onChange?(boards) }
}
