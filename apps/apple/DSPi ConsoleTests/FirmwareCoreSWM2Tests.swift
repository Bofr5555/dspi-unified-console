import XCTest
import dspi_core
@testable import DSPi_Console

/// WP SW-M2: the firmware install through the core. The write is the core's
/// guarded `flashConfirmed` (C11 behind CG8, with a P1TK token) onto a fake
/// bootloader drive: a directory holding the boot ROM's `INFO_UF2.TXT`. No
/// device, no real drive, nothing flashed. Runs in the macOS and iOS bundles.
final class FirmwareCoreSWM2Tests: XCTestCase {

    // MARK: - The fake drive gets the image, through the core

    /// A stock device is connected; its restart brings the board (review
    /// batch 2 F1: only that board is judged by the device's record).
    func testSWM2AFakeDriveGetsTheImageThroughTheCore() throws {
        let f = try SWM2Fixture(self)
        let installer = f.installer(board: SWM2Fixture.stockBoard, chip: .rp2350)
        f.locator.boards = []
        installer.beginWatching()
        XCTAssertEqual(installer.state, .waitingForBoard)

        var restarts = 0
        XCTAssertTrue(installer.installWhenReady(confirmed: .firmwareFlash(), restart: { restarts += 1 }))
        XCTAssertTrue(SWM2Fixture.spin(2) { restarts == 1 }, "the stock device is restarted without a question")
        f.locator.boards = [f.board]
        f.locator.emit()
        SWM2Fixture.settle(installer)
        XCTAssertNil(installer.question, "a stock board and an unstamped image: no risk, no question")
        XCTAssertEqual(installer.state, .verified(SWM2Fixture.version))
        XCTAssertEqual(try Data(contentsOf: f.written), f.bytes, "the core wrote the image as firmware.uf2")
    }

    /// The locator pairs the core's USB counts with the core's drives.
    func testSWM2TheLocatorPairsTheCoresCountsAndDrives() {
        let rp2350 = BootloaderDrive(path: "/Volumes/RP2350", platform: 1, info: "")
        let rp2040 = BootloaderDrive(path: "/Volumes/RPI-RP2", platform: 0, info: "")
        XCTAssertEqual(BootloaderBoard.pair(counts: BootloaderCounts(rp2040: 0, rp2350: 0), drives: [rp2350]), [],
                       "a drive with no boot ROM on the bus is not a board")
        XCTAssertEqual(BootloaderBoard.pair(counts: BootloaderCounts(rp2040: 0, rp2350: 1), drives: []),
                       [BootloaderBoard(chip: .rp2350, drive: nil)], "on the bus, drive not mounted yet")
        XCTAssertEqual(BootloaderBoard.pair(counts: BootloaderCounts(rp2040: 1, rp2350: 1), drives: [rp2350, rp2040]),
                       [BootloaderBoard(chip: .rp2040, drive: rp2040), BootloaderBoard(chip: .rp2350, drive: rp2350)])
        XCTAssertEqual(BootloaderBoard.pair(counts: BootloaderCounts(rp2040: 0, rp2350: 2), drives: [rp2350]),
                       [BootloaderBoard(chip: .rp2350, drive: rp2350), BootloaderBoard(chip: .rp2350, drive: nil)],
                       "two of a kind: the true count, one paired")
        XCTAssertEqual(BootloaderBoard(chip: .rp2350, volumeURL: URL(fileURLWithPath: "/Volumes/RP2350")).drive,
                       rp2350)
    }

    // MARK: - The guard cannot be skipped

    /// A board already in BOOTSEL cannot be asked: the guard asks for the
    /// words before anything is written, and only the exact words go on.
    func testSWM2ABoardInTheBootloaderIsAskedBeforeAnyByteIsWritten() throws {
        let f = try SWM2Fixture(self)
        let installer = f.installer(board: nil)   // InBootloaderBoardReader
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        XCTAssertTrue(SWM2Fixture.spin { installer.question != nil }, "the guard asks")
        let phrase = try XCTUnwrap(installer.question).phrase
        XCTAssertFalse(phrase.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path), "nothing written while it asks")
        guard case .ready = installer.state else { return XCTFail("not writing while it asks: \(installer.state)") }

        XCTAssertFalse(installer.answer(phrase.lowercased()), "the words are compared exactly")
        XCTAssertFalse(installer.answer(""))
        XCTAssertNotNil(installer.question)
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))

        XCTAssertTrue(installer.answer(phrase))
        SWM2Fixture.settle(installer)
        XCTAssertEqual(installer.state, .verified(SWM2Fixture.version))
        XCTAssertEqual(try Data(contentsOf: f.written), f.bytes)
    }

    /// Review batch 2 F1 (the reviewer's probe): a board already in BOOTSEL
    /// when the install is armed is not the connected device's restart, so
    /// the connected device's record does not speak for it: the guard asks,
    /// nothing is written, and the connected device is not restarted.
    func testSWM2ABoardAlreadyInBootselIsNotJudgedByTheConnectedDevice() throws {
        let f = try SWM2Fixture(self)
        let installer = f.installer(board: SWM2Fixture.stockBoard, chip: .rp2350)
        installer.beginWatching()
        XCTAssertEqual(installer.state, .ready(f.board), "a board is in BOOTSEL before the click")
        var restarts = 0
        installer.installWhenReady(confirmed: .firmwareFlash(), restart: { restarts += 1 })
        SWM2Fixture.settle(installer)
        XCTAssertNotNil(installer.question, "CG8: a board already in the bootloader is unknown, so the guard asks")
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path),
                       "nothing written to a BOOTSEL board without the words")
        XCTAssertEqual(restarts, 0, "the BOOTSEL board is the target; the connected device need not restart")
    }

    /// An at-risk board (it needs 1.20 V) and an image that does not say it
    /// runs at that: asked while the device still runs, so the restart waits
    /// for the answer; declined, nothing is written and nothing restarted.
    func testSWM2AnAtRiskBoardIsAskedBeforeTheRestartAndDecliningWritesNothing() throws {
        let f = try SWM2Fixture(self)
        let installer = f.installer(board: SWM2Fixture.atRiskBoard, chip: .rp2350)
        f.locator.boards = []   // the restart brings the board
        installer.beginWatching()
        var restarts = 0
        installer.installWhenReady(confirmed: .firmwareFlash(), restart: { restarts += 1 })
        XCTAssertTrue(SWM2Fixture.spin { installer.question != nil }, "the guard asks at once")
        XCTAssertEqual(restarts, 0, "the device is not restarted before the answer")

        installer.declineQuestion()
        XCTAssertEqual(installer.state, .failed(.refused(FirmwareInstaller.declinedWords)))
        SWM2Fixture.spin(0.3) { false }
        XCTAssertEqual(restarts, 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))
        f.locator.emit()
        XCTAssertEqual(installer.state, .failed(.refused(FirmwareInstaller.declinedWords)), "frozen until reset")
    }

    func testSWM2AnAtRiskBoardGoesOnWithTheWordsAndRestartsOnce() throws {
        let f = try SWM2Fixture(self)
        let installer = f.installer(board: SWM2Fixture.atRiskBoard, chip: .rp2350)
        f.locator.boards = []   // the restart brings the board
        installer.beginWatching()
        var restarts = 0
        installer.installWhenReady(confirmed: .firmwareFlash(), restart: { restarts += 1 })
        XCTAssertTrue(SWM2Fixture.spin { installer.question != nil })
        XCTAssertTrue(installer.answer(try XCTUnwrap(installer.question).phrase))
        XCTAssertEqual(restarts, 1)
        f.locator.boards = [f.board]
        f.locator.emit()
        SWM2Fixture.settle(installer)
        XCTAssertNil(installer.question, "answered once, not asked again at the write")
        XCTAssertEqual(installer.state, .verified(SWM2Fixture.version))
        XCTAssertEqual(try Data(contentsOf: f.written), f.bytes)
    }

    /// Below the installer: the core itself refuses a write without a token
    /// minted from a confirmation, and an at-risk write without the words,
    /// whatever Swift does. Nothing reaches the drive either way.
    func testSWM2TheCoreRefusesAWriteWithoutATokenOrWithoutTheWords() throws {
        let f = try SWM2Fixture(self)
        let core = FirmwareFlashCore(clientReadOnly: false)
        let drive = try XCTUnwrap(f.board.drive)

        XCTAssertThrowsError(try core.withCoreForTesting {
            try $0.flashConfirmed(image: f.bytes, drive: drive, board: SWM2Fixture.stockBoard,
                                  typedConfirmation: "", token: nil)
        }) { error in
            guard case DspiError.ConfirmationRequired = error else { return XCTFail("\(error)") }
        }
        XCTAssertThrowsError(try core.withCoreForTesting {
            try $0.flashConfirmed(image: f.bytes, drive: drive, board: SWM2Fixture.stockBoard,
                                  typedConfirmation: "", token: $0.confirmToken(op: .savePreset))
        }, "a token for another operation")
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))

        for board in [SWM2Fixture.atRiskBoard, firmwareGuardBoardInBootloader()] {
            let r = SWM2Fixture.offMain {
                core.flash(f.bytes, to: drive, board: board, typed: "", confirmed: .firmwareFlash())
            }
            guard case .failure(.FirmwareGuardBlocked) = r else { return XCTFail("\(board): \(r)") }
            XCTAssertEqual(FirmwareInstaller.installError(.FirmwareGuardBlocked(msg: "m")), .guardRefused("m"))
        }
        let wrong = SWM2Fixture.offMain {
            core.flash(f.bytes, to: drive, board: SWM2Fixture.atRiskBoard, typed: "i accept the core voltage risk",
                       confirmed: .firmwareFlash())
        }
        guard case .failure(.FirmwareGuardBlocked) = wrong else { return XCTFail("\(wrong)") }
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))

        let phrase = firmwareGuardCheck(image: f.bytes, board: SWM2Fixture.atRiskBoard, typedConfirmation: "")
            .confirmationPhrase
        let ok = SWM2Fixture.offMain {
            core.flash(f.bytes, to: drive, board: SWM2Fixture.atRiskBoard, typed: phrase, confirmed: .firmwareFlash())
        }
        guard case .success = ok else { return XCTFail("\(ok)") }
        XCTAssertEqual(try Data(contentsOf: f.written), f.bytes)
    }

    /// The client read-only mode holds on the firmware core too.
    func testSWM2TheReadOnlyModeRefusesTheWrite() throws {
        let f = try SWM2Fixture(self)
        let core = FirmwareFlashCore(clientReadOnly: true)
        let drive = try XCTUnwrap(f.board.drive)
        let r = SWM2Fixture.offMain {
            core.flash(f.bytes, to: drive, board: SWM2Fixture.stockBoard, typed: "", confirmed: .firmwareFlash())
        }
        guard case .failure(.ReadOnly) = r else { return XCTFail("\(r)") }
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))
    }

    // MARK: - The lock cannot be skipped

    /// The installer's lock is asked when armed and again at the write: a
    /// lock that closed in between refuses the write.
    func testSWM2TheLockAskedAtTheWriteRefusesIt() throws {
        let f = try SWM2Fixture(self)
        var locked = false
        let row = "Locked by the installer"
        let installer = FirmwareInstaller(locator: f.locator, verifier: SWM2Verifier(version: SWM2Fixture.version),
                                          imageProvider: { _ in f.image },
                                          boardReader: SWM2BoardReader(subject: .init(board: SWM2Fixture.stockBoard,
                                                                                      chip: nil)),
                                          flashCore: FirmwareFlashCore(clientReadOnly: false),
                                          gate: { locked ? .refused(row) : nil })
        f.locator.boards = []
        installer.beginWatching()
        XCTAssertTrue(installer.installWhenReady(confirmed: .firmwareFlash()))
        SWM2Fixture.spin(0.3) { false }
        locked = true
        f.locator.boards = [f.board]
        f.locator.emit()
        XCTAssertEqual(installer.state, .failed(.refused(row)))
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))
    }

    func testSWM2TheLockRefusesBeforeAnythingIsArmed() throws {
        let f = try SWM2Fixture(self)
        let installer = FirmwareInstaller(locator: f.locator, verifier: SWM2Verifier(version: nil),
                                          imageProvider: { _ in f.image },
                                          gate: { .refused("locked") })
        installer.beginWatching()
        XCTAssertFalse(installer.installWhenReady(confirmed: .firmwareFlash()))
        XCTAssertFalse(installer.isArmed)
        XCTAssertEqual(installer.state, .failed(.refused("locked")))
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))
    }

    // MARK: - What the core checks at the drive

    func testSWM2AFileThatIsNotFirmwareIsRefusedByTheGuard() throws {
        let f = try SWM2Fixture(self, bytes: Data(repeating: 0xAB, count: 4096))
        let installer = f.installer(board: SWM2Fixture.stockBoard)
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        SWM2Fixture.settle(installer)
        guard case .failed(.guardRefused(let words)) = installer.state else {
            return XCTFail("refused by the guard: \(installer.state)")
        }
        XCTAssertTrue(words.hasPrefix("This is not a firmware file"), words)
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))
    }

    func testSWM2AnImageForTheOtherChipIsRefusedAtTheDrive() throws {
        let f = try SWM2Fixture(self, chip: .rp2040, bytes: SWM2Fixture.uf2(family: 0xE48B_FF59))
        let installer = f.installer(board: SWM2Fixture.stockBoard)
        installer.beginWatching()
        SWM2Fixture.installAnswering(installer)
        guard case .failed(.writeFailed) = installer.state else { return XCTFail("\(installer.state)") }
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))
    }

    /// The drive record is read again by the core: a drive that is gone, or
    /// a directory that is not a boot ROM's, gets nothing.
    func testSWM2ADriveThatIsNotABootROMsGetsNothing() throws {
        let f = try SWM2Fixture(self)
        try FileManager.default.removeItem(at: f.volume.appendingPathComponent("INFO_UF2.TXT"))
        let installer = f.installer(board: SWM2Fixture.stockBoard)
        installer.beginWatching()
        SWM2Fixture.installAnswering(installer)
        guard case .failed(.writeFailed) = installer.state else { return XCTFail("\(installer.state)") }
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))
    }
}

// MARK: - Fixture (shared with FirmwareInstallerTests)

/// A fake bootloader drive (a directory with the boot ROM's `INFO_UF2.TXT`)
/// and a small valid UF2 standing in for the bundled image.
struct SWM2Fixture {
    static let version = FirmwareVersion(1, 1, 7)
    /// A stock board: 1.15 V, no risk.
    static let stockBoard = FirmwareGuardBoard(requiredMv: 1150, source: .extCoreVreg)
    /// A board that needs 1.20 V (the K340).
    static let atRiskBoard = FirmwareGuardBoard(requiredMv: 1200, source: .extCoreVreg)

    let root: URL
    let volume: URL
    let image: FirmwareImage
    let bytes: Data
    let board: BootloaderBoard
    let locator: FakeBootloaderLocator
    /// Where the core writes the image.
    var written: URL { volume.appendingPathComponent("firmware.uf2") }

    init(_ test: XCTestCase, chip: BootloaderBoard.Chip = .rp2350, bytes: Data? = nil) throws {
        root = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("dspi-swm2-\(UUID().uuidString)")
        volume = root.appendingPathComponent(chip.volumeName)
        try FileManager.default.createDirectory(at: volume, withIntermediateDirectories: true)
        let root = self.root
        test.addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        let model = chip == .rp2040 ? "Raspberry Pi RP2" : "Raspberry Pi RP2350"
        try "UF2 Bootloader v1.0\nModel: \(model)\nBoard-ID: \(chip.volumeName)\n"
            .write(to: volume.appendingPathComponent("INFO_UF2.TXT"), atomically: true, encoding: .utf8)
        self.bytes = bytes ?? Self.uf2()
        let url = root.appendingPathComponent("DSPi-\(chip.assetToken)-v\(Self.version).uf2")
        try self.bytes.write(to: url)
        image = FirmwareImage(chip: chip, url: url, version: Self.version)
        board = BootloaderBoard(chip: chip, volumeURL: volume)
        locator = FakeBootloaderLocator(boards: [board])
    }

    /// An installer over this drive; `board` nil reads every board as one in
    /// the bootloader (unknown), as the app does with no device.
    func installer(board record: FirmwareGuardBoard?, chip: BootloaderBoard.Chip? = nil,
                   verified: FirmwareVersion? = SWM2Fixture.version) -> FirmwareInstaller {
        let reader: FirmwareBoardReading = record.map { SWM2BoardReader(subject: .init(board: $0, chip: chip)) }
            ?? InBootloaderBoardReader()
        let image = self.image
        return FirmwareInstaller(locator: locator, verifier: SWM2Verifier(version: verified),
                                 imageProvider: { _ in image }, boardReader: reader,
                                 flashCore: FirmwareFlashCore(clientReadOnly: false), gate: { nil })
    }

    /// A valid UF2 of `blocks` 256-byte pages at the start of flash.
    static func uf2(blocks: Int = 64, family: UInt32 = 0xE48B_FF59) -> Data {
        var out = Data()
        for i in 0..<blocks {
            var b = [UInt8](repeating: 0, count: 512)
            func put(_ v: UInt32, _ at: Int) {
                withUnsafeBytes(of: v.littleEndian) { for (k, x) in $0.enumerated() { b[at + k] = x } }
            }
            put(0x0A32_4655, 0); put(0x9E5D_5157, 4); put(0x0000_2000, 8)
            put(0x1000_0000 + UInt32(i) * 256, 12); put(256, 16); put(UInt32(i), 20)
            put(UInt32(blocks), 24); put(family, 28)
            for k in 0..<256 { b[32 + k] = UInt8(truncatingIfNeeded: i &+ k) }
            put(0x0AB1_6F30, 508)
            out.append(contentsOf: b)
        }
        return out
    }

    /// `body` on a thread of its own (the core's write refuses the main
    /// thread), waited for here.
    static func offMain<T>(_ body: @escaping () -> T) -> T {
        var result: T?
        let done = DispatchSemaphore(value: 0)
        Thread {
            result = body()
            done.signal()
        }.start()
        done.wait()
        return result!
    }

    /// Spins the main run loop until `done` or `seconds` pass.
    @discardableResult
    static func spin(_ seconds: TimeInterval = 5, until done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if done() { return true }
            RunLoop.current.run(until: Date().addingTimeInterval(0.02))
        }
        return done()
    }

    /// Arms the install for a board already in BOOTSEL and gives the guard's
    /// words when it asks (an unknown board), then waits for the outcome.
    static func installAnswering(_ installer: FirmwareInstaller) {
        installer.installWhenReady(confirmed: .firmwareFlash())
        settle(installer)
        if let q = installer.question {
            installer.answer(q.phrase)
            settle(installer)
        }
    }

    /// Until the install ends, or the guard asks.
    static func settle(_ installer: FirmwareInstaller) {
        spin {
            if installer.question != nil { return true }
            switch installer.state {
            case .verified, .failed: return true
            default: return false
            }
        }
    }
}

struct SWM2Verifier: FirmwareVerifying {
    let version: FirmwareVersion?
    func awaitDeviceVersion(timeout: TimeInterval) -> FirmwareVersion? { version }
}

struct SWM2BoardReader: FirmwareBoardReading {
    let subject: FirmwareGuardSubject
    func readBoard() -> FirmwareGuardSubject { subject }
}
