import Foundation
import dspi_core

// MARK: - Bundled images

/// A UF2 image shipped inside the app bundle.
///
/// Console bundles the firmware of its own version so the pair can never be
/// mismatched by a user who updates one and not the other.  See CLAUDE.md >
/// Releases for how the images get here.
struct FirmwareImage: Equatable {
    let chip: BootloaderBoard.Chip
    let url: URL
    let version: FirmwareVersion

    /// Locates the bundled image for a chip.
    ///
    /// Throws `bundledImageStale` when the image's version does not match the
    /// app's, which catches a release that bumped `MARKETING_VERSION` and
    /// forgot to refresh the `.uf2` files.  Failing loudly in development is
    /// the entire point: a stale image would otherwise ship and quietly
    /// downgrade every device it touched.
    static func bundled(for chip: BootloaderBoard.Chip,
                        in bundle: Bundle = .main) throws -> FirmwareImage {
        let candidates = (bundle.urls(forResourcesWithExtension: "uf2", subdirectory: "Firmware") ?? [])
            + (bundle.urls(forResourcesWithExtension: "uf2", subdirectory: nil) ?? [])

        guard let url = candidates.first(where: {
            $0.lastPathComponent.contains("-\(chip.assetToken)-")
        }) else {
            throw FirmwareInstallError.imageMissing(chip.displayName)
        }

        guard let version = versionFromAssetName(url.lastPathComponent) else {
            throw FirmwareInstallError.imageMissing(chip.displayName)
        }

        if let expected = FirmwareVersion.expected, version != expected {
            throw FirmwareInstallError.bundledImageStale(bundled: version.description,
                                                         expected: expected.description)
        }

        return FirmwareImage(chip: chip, url: url, version: version)
    }

    /// Pulls "1.1.7" out of "DSPi-RP2350-v1.1.7.uf2".
    static func versionFromAssetName(_ name: String) -> FirmwareVersion? {
        guard let marker = name.range(of: "-v", options: .backwards) else { return nil }
        let tail = name[marker.upperBound...].replacingOccurrences(of: ".uf2", with: "")
        return FirmwareVersion(tail)
    }
}

// MARK: - Errors

enum FirmwareInstallError: Error, Equatable {
    case noBoardFound
    case multipleBoards(Int)
    case volumeNotMounted(String)
    case imageMissing(String)
    case bundledImageStale(bundled: String, expected: String)
    case writeFailed(String)
    case deviceDidNotReturn
    case versionMismatch(expected: String, got: String)
    /// Another DSPi Link session held the device lock, so the device was
    /// never asked to restart.
    case deviceLocked
    /// -- WP P1SW2F -- (review p1sw2 F3) Refused before anything was
    /// written: the installer's lock (the core's notice row) or a write that
    /// no person confirmed.
    case refused(String)
    /// -- WP SW-M2 -- The core's firmware guard (CG8) did not let the image
    /// on: its own sentence (an image that is not firmware, a risk nobody
    /// accepted).  Nothing was written.
    case guardRefused(String)

    /// One sentence fit to show a user.
    var message: String {
        switch self {
        case .noBoardFound:
            return "No board in bootloader mode was found. Hold the BOOTSEL button while plugging the board in."
        case .multipleBoards(let count):
            return "\(count) boards are in bootloader mode. Disconnect all but the one you want to update."
        case .volumeNotMounted(let name):
            return "The board is connected but its \(name) drive has not appeared. If macOS asked for permission to access removable volumes, allow it and try again."
        case .imageMissing(let chip):
            return "This build of uConsole for DSPi does not include firmware for the \(chip)."
        case .bundledImageStale(let bundled, let expected):
            return "The bundled firmware is version \(bundled) but this Console expects \(expected). The app was built incorrectly; do not install it."
        case .writeFailed(let detail):
            return "Writing the firmware failed: \(detail)"
        case .deviceDidNotReturn:
            return "The firmware was written but the device did not reappear. Unplug it, plug it back in, and check whether it works."
        case .versionMismatch(let expected, let got):
            return "The device came back running firmware \(got) instead of \(expected)."
        case .deviceLocked:
            return "\(DeviceLock.lockedMessage). Try again when it has finished."
        case .refused(let words):
            return words
        case .guardRefused(let words):
            return words
        }
    }
}

// MARK: - The board's needs (WP SW-M2)

/// What the firmware guard judges a board by: the core's record of its
/// core-voltage needs (CG8), and its chip when known.
struct FirmwareGuardSubject: Equatable {
    var board: FirmwareGuardBoard
    var chip: BootloaderBoard.Chip?

    /// A board already in BOOTSEL cannot be asked: unknown, so the guard asks.
    static var inBootloader: FirmwareGuardSubject {
        FirmwareGuardSubject(board: firmwareGuardBoardInBootloader(), chip: nil)
    }
}

/// Reads the connected board's needs, once per install and before it is
/// restarted into the bootloader (the board cannot be asked afterwards).
/// Called off the main thread.
protocol FirmwareBoardReading {
    func readBoard() -> FirmwareGuardSubject
}

/// No device to ask: every board is judged as one already in BOOTSEL.
struct InBootloaderBoardReader: FirmwareBoardReading {
    func readBoard() -> FirmwareGuardSubject { .inBootloader }
}

/// The view model's device, through the core (`firmwareGuardBoard`).  A
/// device not driven through a core (`-CoreTransport NO`) cannot be read, so
/// it counts as unknown and the guard asks.
struct ViewModelBoardReader: FirmwareBoardReading {
    let vm: DSPViewModel

    func readBoard() -> FirmwareGuardSubject {
        dispatchPrecondition(condition: .notOnQueue(.main))
        let (connected, source, platform): (Bool, CoreStateSource?, String) = DispatchQueue.main.sync {
            (vm.isDeviceConnected, vm.transport.coreStateSource, vm.platformName)
        }
        guard connected else { return .inBootloader }
        let chip = BootloaderBoard.Chip.allCases.first { $0.assetToken == platform }
        guard let source, case .success(let board) = source.call({ $0.firmwareGuardBoard() }) else {
            return FirmwareGuardSubject(board: FirmwareGuardBoard(requiredMv: 0, source: .noDevice), chip: chip)
        }
        return FirmwareGuardSubject(board: board, chip: chip)
    }
}

/// The guard's question to the person: its sentence, and the words to type
/// to go ahead anyway (the core's, compared exactly).
struct FirmwareGuardQuestion: Equatable {
    let message: String
    let phrase: String
}

// MARK: - The core that writes (WP SW-M2)

/// The only firmware write in the app: the core's guarded `flashConfirmed`
/// (C11 behind CG8, with a P1TK token).  Its own `DspiCore`, with no device
/// open: the board has left by the time its drive is there.  The client
/// read-only mode (`-ReadOnly YES`) holds here as on the app's cores.
///
/// Before any byte goes out the core validates the image, reads the drive's
/// `INFO_UF2.TXT` again, checks the image's chip against it, checks the room
/// on the drive, and runs the guard with the board record and the typed
/// words; without a token minted from the person's confirmation it refuses.
final class FirmwareFlashCore {
    private let core: DspiCore
    private let queue = DispatchQueue(label: "com.dspi.firmware-core", qos: .userInitiated)

    init(clientReadOnly: Bool = CoreBackedTransport.readOnlyLaunchFlag) {
        core = DspiCore()
        if clientReadOnly { try? core.setClientReadOnly(on: true) }
    }

    /// Writes `image` to `drive`, minting the token from `confirmed` now,
    /// right before the call (the board is in BOOTSEL by then).  Blocking:
    /// call off the main thread.
    func flash(_ image: Data, to drive: BootloaderDrive, board: FirmwareGuardBoard,
               typed: String, confirmed: UserConfirmation) -> Result<Void, DspiError> {
        dispatchPrecondition(condition: .notOnQueue(.main))
        return queue.sync { () -> Result<Void, DspiError> in
            do {
                try core.flashConfirmed(image: image, drive: drive, board: board, typedConfirmation: typed,
                                        token: confirmed.token(core, for: .firmwareFlash))
                return .success(())
            } catch let error as DspiError {
                return .failure(error)
            } catch {
                return .failure(.Unsupported)
            }
        }
    }

    #if DEBUG
    /// Tests: the core itself, to show what it refuses when called directly.
    func withCoreForTesting<T>(_ body: (DspiCore) throws -> T) rethrows -> T { try queue.sync { try body(core) } }
    #endif
}

// MARK: - Verification

/// Waits for a DSPi to enumerate after a flash and reports the version it
/// claims.  A protocol so tests can verify without hardware.
protocol FirmwareVerifying {
    func awaitDeviceVersion(timeout: TimeInterval) -> FirmwareVersion?
}

/// Polls the live view model.  Called from the installer's background queue,
/// so each read hops to the main thread where the published state is written.
struct ViewModelFirmwareVerifier: FirmwareVerifying {
    let vm: DSPViewModel

    func awaitDeviceVersion(timeout: TimeInterval) -> FirmwareVersion? {
        // The main.sync below deadlocks instantly if this ever runs on the
        // main queue; make that mistake loud rather than a hang.
        dispatchPrecondition(condition: .notOnQueue(.main))
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            let version: FirmwareVersion? = DispatchQueue.main.sync {
                guard vm.isDeviceConnected, let v = vm.firmwareVersion else { return nil }
                return FirmwareVersion(v.major, v.minor, v.patch, v.beta)
            }
            if let version { return version }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return nil
    }
}

// MARK: - Installer

enum FirmwareInstallState: Equatable {
    case idle
    /// Nothing in BOOTSEL yet.
    case waitingForBoard
    /// Board on the bus but its drive has not mounted.  Ordinary for a second
    /// or two after it appears, so this is a waiting state, not a failure.
    case waitingForVolume(BootloaderBoard.Chip)
    /// Exactly one board, volume mounted, ready to be written when the user says so.
    case ready(BootloaderBoard)
    /// Writing.  -- WP SW-M2 -- The core writes the image in one call, so
    /// the fraction is 0 for the whole write (the card shows no percentage).
    case writing(Double)
    /// Bytes are out and the board is rebooting; waiting for it to enumerate.
    case waitingForDevice
    case verified(FirmwareVersion)
    case failed(FirmwareInstallError)
}

/// Detects a board in BOOTSEL, has the core write the bundled UF2 to it, and
/// confirms the device came back running what was written.
///
/// -- WP SW-M2 -- UI over the core: the locator's boards come from the core,
/// the guard's verdict is the core's (`firmwareGuardCheck`, CG8), and the
/// write is the core's guarded `flashConfirmed` with a token minted from the
/// person's confirmation (`FirmwareFlashCore`).  Swift writes no byte of
/// firmware.  The installer's lock (`gate`) is asked when the install is
/// armed and again before the write; neither it nor the guard can be left
/// out, since both are fixed at init and the only way in is
/// `installWhenReady(confirmed:)`.
///
/// Deliberately does none of the deciding: it never flashes on its own, never
/// chooses between two attached boards, and never touches presets.  Callers
/// own the confirmation UI.  Used by the Getting Started wizard, by
/// Tools > Firmware Update, and by the version-mismatch banner.
final class FirmwareInstaller: ObservableObject {
    @Published private(set) var state: FirmwareInstallState = .idle
    /// -- WP SW-M2 -- The guard's question, while it waits for the person's
    /// typed answer (`answer(_:)`) or for them to decline it.
    @Published private(set) var question: FirmwareGuardQuestion?

    /// How long to wait for the board to come back after a write.  Covers the
    /// reboot, USB re-enumeration, and the app's own reconnect and platform
    /// fetch; generous on purpose, because timing out after a flash that
    /// actually worked tells the user their device is broken when it is not.
    static let verifyTimeout: TimeInterval = 30

    /// How long a board may sit on the USB bus with no drive before we call it
    /// a failure.  The mount always lags enumeration; treating that gap as an
    /// error meant a board plugged in while the window was open was rejected
    /// a moment before its drive appeared.
    static let volumeWaitTimeout: TimeInterval = 8

    static let unconfirmedWords = "The firmware was not written: the install was not confirmed."
    /// -- WP SW-M2 -- The person declined the guard's question.
    static let declinedWords = "The firmware was not written: the core voltage risk was not accepted."

    private let locator: BootloaderLocating
    private let verifier: FirmwareVerifying
    private let imageProvider: (BootloaderBoard.Chip) throws -> FirmwareImage
    private let now: () -> Date
    private let boardReader: FirmwareBoardReading
    private let flashCore: FirmwareFlashCore
    /// -- WP P1SW2F -- F3: what refuses a write before anything reaches the
    /// drive (the installer's lock, in the core's words); nil lets it go.
    private let gate: () -> FirmwareInstallError?
    private let queue = DispatchQueue(label: "com.foxdac.firmware-install")

    /// Set once `install` begins and cleared only by `reset()`.  While it is
    /// set, detection stops touching the state: mid-write because the board
    /// disappearing is the expected reboot, and afterwards because the outcome
    /// - `.verified` or a `.failed` - must stay on screen rather than being
    /// overwritten by the next poll tick.  Before an install, detection stays
    /// live: a failure it reported is only ever a description of what is
    /// plugged in right now, and must give way when that changes.
    private var installing = false

    /// When the current board was first seen without a drive.
    private var volumeWaitStarted: Date?

    /// The user has committed to an update, so the next ready board is written
    /// without asking again.  Kept here rather than in the view because the
    /// board may be ready before the user commits, or commit before the board
    /// is ready, and only one of those two orders is a state *change*.
    private var armed = false

    /// The person's confirmation the armed install was made with.
    private(set) var flashConfirmation: UserConfirmation?
    /// -- WP SW-M2 -- The board's needs for this run, read once when armed
    /// (before any restart); nil until read.  A write waits for it.
    private(set) var subject: FirmwareGuardSubject?
    /// What the person typed for the guard's question ("" none).
    private var typed = ""
    /// What runs once the question is answered.
    private var afterAnswer: (() -> Void)?
    /// Counts runs: a reply for an earlier one (reset or abandoned since) is dropped.
    private var run = 0

    init(locator: BootloaderLocating,
         verifier: FirmwareVerifying,
         imageProvider: @escaping (BootloaderBoard.Chip) throws -> FirmwareImage
            = { try FirmwareImage.bundled(for: $0) },
         now: @escaping () -> Date = Date.init,
         boardReader: FirmwareBoardReading = InBootloaderBoardReader(),
         flashCore: FirmwareFlashCore = FirmwareFlashCore(),
         gate: @escaping () -> FirmwareInstallError? = { FirmwareFlashGate.refusal() }) {
        self.locator = locator
        self.verifier = verifier
        self.imageProvider = imageProvider
        self.now = now
        self.boardReader = boardReader
        self.flashCore = flashCore
        self.gate = gate
    }

    // MARK: Detection

    /// Begin watching for boards.  `state` tracks what is attached until
    /// `install` is called.
    func beginWatching() {
        locator.onChange = { [weak self] boards in
            self?.applyBoards(boards)
        }
        locator.start()
        applyBoards(locator.currentBoards())
    }

    func stopWatching() {
        locator.onChange = nil
        locator.stop()
    }

    /// Returns a finished installer to detection so another update can run.
    ///
    /// The outcome of an install - `.verified` or `.failed` - is deliberately
    /// frozen so the user gets to read it; this is the one door back out, and
    /// it is only opened by an explicit user action (Try Again, Update Another
    /// Board) or a fresh window.  It clears the commitment along with the
    /// freeze: a second board is a second decision, never covered by the
    /// first click.  Refused mid-install, where there is no outcome yet to
    /// dismiss and detection must stay off.
    func reset() {
        switch state {
        case .writing, .waitingForDevice: return
        default: break
        }
        run += 1
        installing = false
        armed = false
        volumeWaitStarted = nil
        flashConfirmation = nil
        subject = nil
        typed = ""
        afterAnswer = nil
        question = nil
        applyBoards(locator.currentBoards())
    }

    /// Maps what is attached onto a state.
    ///
    /// Stops entirely once an install has begun: mid-write the board
    /// vanishing is the expected reboot, and afterwards the outcome must hold
    /// until `reset()`.  Until then it always runs, including over a failure
    /// it reported itself, so a board whose drive shows up late recovers on
    /// its own.
    private func applyBoards(_ boards: [BootloaderBoard]) {
        guard !installing else { return }

        let next: FirmwareInstallState
        switch boards.count {
        case 0:
            volumeWaitStarted = nil
            next = .waitingForBoard
        case 1:
            if boards[0].drive != nil {
                volumeWaitStarted = nil
                next = .ready(boards[0])
            } else {
                // The drive mounts a moment after the board enumerates, so
                // hold in a waiting state and only call it a failure once it
                // is clear the drive is not coming.
                let started = volumeWaitStarted ?? now()
                volumeWaitStarted = started
                next = now().timeIntervalSince(started) >= Self.volumeWaitTimeout
                    ? .failed(.volumeNotMounted(boards[0].chip.volumeName))
                    : .waitingForVolume(boards[0].chip)
            }
        default:
            volumeWaitStarted = nil
            next = .failed(.multipleBoards(boards.count))
        }
        publish(next)

        // Committed before the board arrived: write it now that it is here.
        installIfReady()
    }

    // MARK: Install

    /// Records the user's decision to update, and writes as soon as there is a
    /// board to write to - immediately if one is already ready.
    ///
    /// This is the only way to start an install.  Waiting for `.ready` to
    /// *arrive* misses the common case where the board was plugged in first
    /// and the state is already `.ready` when the user clicks.
    ///
    /// -- WP SW-M2 -- The board's needs are read first (`boardReader`, off
    /// the main thread), and when its chip is known the guard is asked now,
    /// so a question comes while the device still runs; `restart` (the
    /// connected device's restart into the bootloader) runs only after that.
    /// -- WP P1SW2F -- False when refused (and failed with the reason).
    @discardableResult
    func installWhenReady(confirmed: UserConfirmation, restart: (() -> Void)? = nil) -> Bool {
        // -- WP P1SW2F -- F3/F5: only a person's FirmwareFlash confirmation
        // (the Install button) arms a write, and the gate (the installer's
        // lock) is asked now, while the DSPi is still connected.
        assert(confirmed.op == .firmwareFlash, "an install armed by \(confirmed.op)")
        guard confirmed.op == .firmwareFlash else {
            abandon(.refused(Self.unconfirmedWords))
            return false
        }
        if let refusal = gate() {
            abandon(refusal)
            return false
        }
        guard !installing, flashConfirmation == nil else { return false }
        flashConfirmation = confirmed
        armed = true

        // -- WP SW-M2 -- (review batch 2 F1) The connected device's record
        // speaks only for the board its restart brings.  A board already on
        // the bus in BOOTSEL is another board, or one nobody can ask: it is
        // judged as in the bootloader (unknown, so the guard asks), and the
        // connected device is not restarted.
        if boardOnBus {
            subject = .inBootloader
            installIfReady()
            return true
        }

        let run = self.run
        let reader = boardReader
        let imageProvider = self.imageProvider
        queue.async { [weak self] in
            let subject = reader.readBoard()
            // The connected chip's image, judged now; a board already in
            // BOOTSEL (no chip) is judged at the write.
            var verdict: FirmwareGuardVerdict?
            if let chip = subject.chip, let image = try? imageProvider(chip),
               let bytes = try? Data(contentsOf: image.url) {
                verdict = firmwareGuardCheck(image: bytes, board: subject.board, typedConfirmation: "")
            }
            DispatchQueue.main.async {
                guard let self, self.run == run, self.armed else { return }
                // A board that reached the bus while the record was read was
                // not brought by the restart either (F1).
                if self.boardOnBus {
                    self.subject = .inBootloader
                    self.installIfReady()
                    return
                }
                self.subject = subject
                let proceed = { [weak self] in
                    restart?()
                    self?.installIfReady()
                }
                guard let verdict, !verdict.allowed else { return proceed() }
                if verdict.decision == .needsConfirmation {
                    self.ask(verdict, then: proceed)
                } else {
                    self.end(.guardRefused(verdict.message))
                }
            }
        }
        return true
    }

    /// Whether an update has been committed to but not yet started.
    var isArmed: Bool { armed }

    /// -- WP SW-M2 -- A board in BOOTSEL is on the bus now (drive mounted or not).
    private var boardOnBus: Bool {
        switch state {
        case .ready, .waitingForVolume, .failed(.multipleBoards): return true
        default: return false
        }
    }

    /// -- WP SW-M2 -- The person's answer to the guard's question.  True when
    /// it is the question's words exactly, and the run goes on with them; the
    /// core checks them again at the write.  False leaves the question up.
    @discardableResult
    func answer(_ words: String) -> Bool {
        guard let q = question, words == q.phrase else { return false }
        typed = words
        question = nil
        let next = afterAnswer
        afterAnswer = nil
        next?()
        return true
    }

    /// -- WP SW-M2 -- The person declined the guard's question: nothing is
    /// written, and the run ends until `reset()`.
    func declineQuestion() {
        guard question != nil else { return }
        question = nil
        afterAnswer = nil
        end(.refused(Self.declinedWords))
    }

    /// Abandons a committed update before anything is written, reporting why
    /// (the device could not be restarted into the bootloader).  Frozen like
    /// any other failure until `reset()`.
    func abandon(_ error: FirmwareInstallError) {
        guard !installing else { return }
        end(error)
    }

    /// Ends the run before anything is written: frozen until `reset()`.
    private func end(_ error: FirmwareInstallError) {
        run += 1
        armed = false
        installing = true
        question = nil
        afterAnswer = nil
        publish(.failed(error))
    }

    private func ask(_ verdict: FirmwareGuardVerdict, then next: @escaping () -> Void) {
        afterAnswer = next
        question = FirmwareGuardQuestion(message: verdict.message, phrase: verdict.confirmationPhrase)
    }

    /// Writes when armed, the board's needs are read, no question is up, and
    /// a board is ready.
    private func installIfReady() {
        guard armed, subject != nil, question == nil, case .ready(let board) = state else { return }
        install(board)
    }

    /// Has the core write the bundled image for `board`, then verifies the
    /// result.  Only from `installIfReady`: armed by a person's confirmation.
    private func install(_ board: BootloaderBoard) {
        guard !installing else { return }

        // The commitment is consumed here, success or not.  Left standing, a
        // pre-write failure would retrigger the install on every poll tick,
        // and a later board would be flashed on a decision made about an
        // earlier one.
        armed = false

        // Frozen from the first possible failure, not from the first byte:
        // detection would otherwise overwrite an error like a missing image
        // with `.ready` on the next tick, since the board is still attached.
        installing = true

        guard let confirmation = flashConfirmation else {
            publish(.failed(.refused(Self.unconfirmedWords)))
            return
        }
        // -- WP P1SW2F -- F3: the gate again, right before the write.
        if let refusal = gate() {
            publish(.failed(refusal))
            return
        }
        guard let drive = board.drive else {
            publish(.failed(.volumeNotMounted(board.chip.volumeName)))
            return
        }

        let image: FirmwareImage
        do {
            image = try imageProvider(board.chip)
        } catch let error as FirmwareInstallError {
            publish(.failed(error))
            return
        } catch {
            publish(.failed(.writeFailed(error.localizedDescription)))
            return
        }

        let subject = self.subject ?? .inBootloader
        let run = self.run
        queue.async { [weak self] in
            guard let bytes = try? Data(contentsOf: image.url) else {
                self?.publish(.failed(.writeFailed("could not read the bundled image")))
                return
            }
            DispatchQueue.main.async {
                guard let self, self.run == run else { return }
                self.check(bytes, image: image, drive: drive, board: subject.board, confirmation: confirmation)
            }
        }
    }

    /// The guard on the image now going out (main thread): the write when it
    /// allows it, the question when it needs the person's words.
    private func check(_ bytes: Data, image: FirmwareImage, drive: BootloaderDrive,
                       board: FirmwareGuardBoard, confirmation: UserConfirmation) {
        let verdict = firmwareGuardCheck(image: bytes, board: board, typedConfirmation: typed)
        if verdict.allowed {
            write(bytes, image: image, drive: drive, board: board, confirmation: confirmation)
        } else if verdict.decision == .needsConfirmation {
            ask(verdict) { [weak self] in
                self?.check(bytes, image: image, drive: drive, board: board, confirmation: confirmation)
            }
        } else {
            publish(.failed(.guardRefused(verdict.message)))
        }
    }

    private func write(_ bytes: Data, image: FirmwareImage, drive: BootloaderDrive,
                       board: FirmwareGuardBoard, confirmation: UserConfirmation) {
        publish(.writing(0))
        let typed = self.typed
        queue.async { [weak self] in
            guard let self else { return }
            // The core's guarded write; `Ok` means only that the write ended
            // or the drive went away during it (the board rebooting).
            if case .failure(let error) = self.flashCore.flash(bytes, to: drive, board: board,
                                                                typed: typed, confirmed: confirmation) {
                self.publish(.failed(Self.installError(error)))
                return
            }

            self.publish(.waitingForDevice)

            // The copy returning proves nothing.  Success is the device coming
            // back and telling us it is running what we wrote.
            guard let reported = self.verifier.awaitDeviceVersion(timeout: Self.verifyTimeout) else {
                self.publish(.failed(.deviceDidNotReturn))
                return
            }
            guard reported == image.version else {
                self.publish(.failed(.versionMismatch(expected: image.version.description,
                                                      got: reported.description)))
                return
            }
            self.publish(.verified(reported))
        }
    }

    /// The core's refusal or failure as the installer reports it.
    static func installError(_ error: DspiError) -> FirmwareInstallError {
        switch error {
        case .FirmwareGuardBlocked(let msg), .OvervoltWordRequired(let msg): return .guardRefused(msg)
        case .ConfirmationRequired, .InstallerLock, .ReadOnly:
            return .refused(CoreSnapshots.words(error, clientReadOnly: true))
        default:
            return .writeFailed(CoreSnapshots.words(error))
        }
    }

    #if DEBUG
    /// Test hook: drives the state machine directly, and can mark a write as
    /// begun, so transitions that only happen mid-install are reachable
    /// without actually writing anything.
    func setStateForTesting(_ next: FirmwareInstallState, installing: Bool = false) {
        state = next
        self.installing = installing
    }
    #endif

    private func publish(_ next: FirmwareInstallState) {
        if Thread.isMainThread {
            guard state != next else { return }
            state = next
        } else {
            DispatchQueue.main.async { self.state = next }
        }
    }
}
