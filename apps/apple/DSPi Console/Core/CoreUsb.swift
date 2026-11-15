//
//  CoreUsb.swift
//  DSPi Console
//
//  -- WP SW-M1 -- The Mac's local DSPi on USB, through the Rust core's `usb`
//  feature (nusb, IOKit underneath: `macos_iokit`) instead of the app's own
//  IOKit driver.  It replaces USBDevice and USBNotificationReader, with the
//  same surface: the device list and hot-plug, selection and auto-connect,
//  generation-bound control transfers, and the notification stream.
//
//  The in-process DSPi Link hub (LinkHub) owns it: the app's own core is the
//  hub's session 1 (CoreBackedTransport over HubTransport, CG26) and remote
//  consoles are later sessions, so every session's bytes go out unchanged.
//  That is why this is the core's plain pipe (`UsbPipe`) and not a second
//  DspiCore: a core's raw calls refuse the bulk and voltage opcodes, and its
//  echo filter would take every session's writes as its own.  At SW-M3 the
//  Rust hub (`dspi-hub`, which owns USB the same way) replaces the Swift hub
//  and this file goes.
//
//  Hot-plug: the bus is enumerated every half second (`usbPipeScan`, which
//  opens nothing), as the core's own hotplug poll does; a notification stream
//  that ends (the device went away) polls at once.
//

#if os(macOS)
import Foundation
import Combine
import dspi_core

// MARK: - The bus, behind a seam for tests

/// Where CoreUsb finds and opens DSPis: the core's USB (CoreUsbCoreBus), or a
/// scripted bus in tests.
protocol CoreUsbBus: AnyObject {
    /// Every DSPi on the bus, opening none.  Called off the main thread.
    func scan() -> [DSPiDevice]
    /// Open and claim one.  Called on CoreUsb's transfer queue.
    func open(serial: String) throws -> UsbPipeProtocol
}

/// The core's USB: `usbPipeScan()` and `UsbPipe.open(serial:)`.
final class CoreUsbCoreBus: CoreUsbBus {
    func scan() -> [DSPiDevice] {
        usbPipeScan().map { DSPiDevice(serial: $0.serial, locationID: $0.locationId) }
    }

    func open(serial: String) throws -> UsbPipeProtocol {
        try UsbPipe.open(serial: serial)
    }
}

// MARK: - Notification packet fields (notification_protocol_v2_spec.md)

private let NOTIFY_EVT_IDLE: UInt8 = 0x00
private let NOTIFY_V2_VERSION: UInt8 = 0x02
private let NOTIFY_EVT_PARAM_CHANGED: UInt8 = 0x02
private let NOTIFY_EVT_BULK_INVALIDATED: UInt8 = 0x03
private let NOTIFY_EVT_CS_AUX: UInt8 = 0x0C
private let PARAM_SRC_HOST_SET: UInt8 = 1
private let PARAM_SRC_BULK_SET: UInt8 = 2

// MARK: - CoreUsb

final class CoreUsb: ObservableObject {
    @Published var isConnected: Bool = false
    @Published var errorMessage: String?
    @Published var availableDevices: [DSPiDevice] = []
    @Published var selectedDevice: DSPiDevice? = nil

    /// The USB host is session 1 on its own link; a DSPi Link hub hands out
    /// higher ids to its sessions.
    let session: LinkSessionID = 1

    /// The notification stream, shared by every subscriber (the hub, and the
    /// view model and monitor when they drive USB directly).
    let notifications = NotificationFanout()

    private let bus: CoreUsbBus
    private let pollInterval: TimeInterval
    /// Opens, closes and control transfers, one at a time.
    private let transferQueue = DispatchQueue(label: "se.bofr.dspi.coreusb.transfer")
    /// The hot-plug poll.
    private let pollQueue = DispatchQueue(label: "se.bofr.dspi.coreusb.poll")
    private var pollTimer: DispatchSourceTimer?

    /// The open device; touched on transferQueue only.
    private var pipe: UsbPipeProtocol?
    /// The serials the last poll saw (pollQueue only); nil before the first.
    private var lastSeen: [String]?

    /// Delays between open attempts, in seconds: the device is published a
    /// moment before it can be claimed, most visibly on the re-enumeration
    /// after a firmware flash.
    static let openRetryDelays: [TimeInterval] = [0.1, 0.2, 0.4, 0.8, 1.0]
    /// How long one notification read waits, so a close never waits longer.
    static let notifyWaitMs: UInt32 = 100

    // MARK: Connection generation

    /// Bumped on every open and close.  Transfers capture it when called and
    /// check it when they run, so work issued for device A never reaches a
    /// device B opened in between.
    private var connectionGeneration: UInt64 = 0
    private let generationLock = NSLock()

    var generation: UInt64 {
        generationLock.lock(); defer { generationLock.unlock() }
        return connectionGeneration
    }

    private func bumpGeneration() {
        generationLock.lock(); connectionGeneration &+= 1; generationLock.unlock()
    }

    /// Why the last notification reader ended, for the monitor window.
    private(set) var notificationReaderError: String?

    /// - Parameters:
    ///   - startMonitoring: false gives an inert instance that never scans or
    ///     opens hardware (tests that need one only as a dependency).
    ///   - bus: the core's USB, or a scripted bus.
    ///   - pollInterval: the hot-plug poll's period.
    init(startMonitoring: Bool = true, bus: CoreUsbBus = CoreUsbCoreBus(), pollInterval: TimeInterval = 0.5) {
        self.bus = bus
        self.pollInterval = pollInterval
        if startMonitoring { startPolling() }
    }

    deinit {
        pollTimer?.cancel()
    }

    // MARK: Hot-plug

    private func startPolling() {
        let timer = DispatchSource.makeTimerSource(queue: pollQueue)
        timer.schedule(deadline: .now(), repeating: pollInterval, leeway: .milliseconds(50))
        timer.setEventHandler { [weak self] in self?.poll() }
        pollTimer = timer
        timer.resume()
    }

    /// Poll now, off schedule (a stream that ended, `reconnect`).
    func pollNow() {
        pollQueue.async { [weak self] in self?.poll() }
    }

    /// One poll: diff the bus against the last one, then apply it on main
    /// (where the published list lives).  pollQueue only.
    /// `force`: apply even if the bus looks unchanged (a stream that ended).
    private func poll(force: Bool = false) {
        let found = bus.scan()
        let serials = found.map(\.serial).sorted()
        let first = lastSeen == nil
        guard force || first || serials != lastSeen else { return }
        let before = Set(lastSeen ?? [])
        lastSeen = serials
        let arrived = found.filter { !before.contains($0.serial) }
        DispatchQueue.main.async { self.apply(present: found, arrived: arrived) }
    }

    /// The device list changed: publish it, auto-connect, and follow a
    /// departure of the selected device.  Main thread.
    private func apply(present: [DSPiDevice], arrived: [DSPiDevice]) {
        let present = present.sorted { $0.serial < $1.serial }
        // Keep the known order, add new devices at the end.
        var list = availableDevices.filter { d in present.contains { $0.serial == d.serial } }
        for d in present where !list.contains(where: { $0.serial == d.serial }) { list.append(d) }
        if list != availableDevices || list.map(\.locationID) != availableDevices.map(\.locationID) {
            availableDevices = list
        }

        if let selected = selectedDevice {
            if !present.contains(where: { $0.serial == selected.serial }) {
                // The selected device left: close it, keep the selection for
                // the auto-reconnect on replug, and move to another DSPi if
                // one is still there so the user is not left on a phantom.
                transferQueue.sync { closePipe() }
                isConnected = false
                errorMessage = "Device Removed"
                if let next = list.first { selectDevice(next) }
            } else if !isConnected, let back = arrived.first(where: { $0.serial == selected.serial }) {
                // Replugged: reconnect.
                selectDevice(back)
            }
        } else if let first = arrived.first {
            selectDevice(first)
        }
        if list.isEmpty { selectedDevice = nil }
    }

    // MARK: Selection

    func selectDevice(_ device: DSPiDevice) {
        transferQueue.async {
            self.closePipe()
            self.open(device, attempt: 0)
        }
    }

    /// transferQueue only.
    private func open(_ device: DSPiDevice, attempt: Int) {
        do {
            let opened = try bus.open(serial: device.serial)
            pipe = opened
            bumpGeneration()
            let generation = self.generation
            startReader(opened, generation: generation)
            DispatchQueue.main.async {
                guard self.generation == generation else { return }
                self.selectedDevice = device
                self.isConnected = true
                self.errorMessage = nil
            }
        } catch {
            if case .Disconnected? = error as? DspiError {
                DispatchQueue.main.async {
                    self.isConnected = false
                    self.errorMessage = "Device not found"
                }
                return
            }
            if attempt < Self.openRetryDelays.count {
                transferQueue.asyncAfter(deadline: .now() + Self.openRetryDelays[attempt]) { [weak self] in
                    // Another open won meanwhile: leave it.
                    guard let self = self, self.pipe == nil else { return }
                    self.open(device, attempt: attempt + 1)
                }
                return
            }
            let message = Self.openFailureMessage(error)
            DispatchQueue.main.async {
                self.isConnected = false
                self.errorMessage = message
            }
        }
    }

    /// The status line for an open that failed after every retry.
    static func openFailureMessage(_ error: Error) -> String {
        if case .Transfer(let msg)? = error as? DspiError {
            let lower = msg.lowercased()
            if lower.contains("exclusive") || lower.contains("busy") {
                return "Device busy (another app has it open)"
            }
        }
        return "Could not open device"
    }

    /// Close the open device without publishing.  transferQueue only.
    private func closePipe() {
        guard let open = pipe else { return }
        pipe = nil
        bumpGeneration()
        open.close()
    }

    func disconnect() {
        transferQueue.sync { closePipe() }
        DispatchQueue.main.async { self.isConnected = false }
    }

    /// Disconnect and look at the bus again from scratch.
    func reconnect() {
        disconnect()
        pollQueue.async { [weak self] in
            self?.lastSeen = nil
            self?.poll()
        }
    }

    // MARK: Notifications

    /// One reader thread per open: it ends when the pipe closes or the device
    /// goes, and drops what it reads once a newer open has replaced it.
    private func startReader(_ pipe: UsbPipeProtocol, generation: UInt64) {
        let thread = Thread { [weak self] in
            while true {
                do {
                    let packet = try pipe.readNotification(timeoutMs: CoreUsb.notifyWaitMs)
                    guard let self = self, self.generation == generation else { return }
                    self.deliver(packet)
                } catch {
                    guard let self = self else { return }
                    if self.generation == generation {
                        self.notificationReaderError = "\(error)"
                        self.streamEnded(serial: pipe.serial(), generation: generation)
                    }
                    return
                }
            }
        }
        thread.name = "DSPi USB notifications"
        thread.qualityOfService = .userInitiated
        thread.start()
    }

    /// The open device's notification stream ended: it went away, even if
    /// the bus still lists it (a replug faster than one poll).  Close it, say
    /// so, and forget its serial so the next poll takes it as arrived and
    /// reconnects it if it is there (as the core's own hotplug poll does).
    private func streamEnded(serial: String, generation: UInt64) {
        transferQueue.async {
            guard self.generation == generation else { return }
            self.closePipe()
            DispatchQueue.main.async {
                self.isConnected = false
                self.errorMessage = "Device Removed"
                self.pollQueue.async {
                    self.lastSeen?.removeAll { $0 == serial }
                    self.poll(force: true)
                }
            }
        }
    }

    /// Publish one packet.  Timeouts (empty), idle keep-alives (0x00) and v1
    /// packets (every v1 event has a v2 twin) are dropped.
    private func deliver(_ bytes: Data) {
        guard !bytes.isEmpty else { return }
        if bytes.count == 1 && bytes.first == NOTIFY_EVT_IDLE { return }
        guard bytes.first == NOTIFY_V2_VERSION else { return }
        notifications.publish(LinkNotification(packet: bytes,
                                               origin: Self.attributedOrigin(bytes, session: session),
                                               receivedAt: Date()))
    }

    /// On USB this app is the only host, so a change the firmware tags as a
    /// host or bulk write can only have been ours.  Everything else (GPIO,
    /// preset, UAC1, UART, I2C, internal) belongs to someone else.
    static func attributedOrigin(_ packet: Data, session: LinkSessionID) -> LinkSessionID {
        guard packet.count >= 5 else { return 0 }
        let sourceOffset: Int
        switch packet[packet.startIndex + 1] {
        case NOTIFY_EVT_PARAM_CHANGED, NOTIFY_EVT_CS_AUX: sourceOffset = 8
        case NOTIFY_EVT_BULK_INVALIDATED: sourceOffset = 4
        case NOTIFY_EVT_RTA_CONFIG: sourceOffset = 16
        case NOTIFY_EVT_INPUT_SOURCE_NAME, NOTIFY_EVT_DEVICE_NAME: sourceOffset = 5
        default: return 0
        }
        guard packet.count > sourceOffset else { return 0 }
        let source = packet[packet.startIndex + sourceOffset]
        return (source == PARAM_SRC_HOST_SET || source == PARAM_SRC_BULK_SET) ? session : 0
    }

    // MARK: Control transfers

    /// The Link status for a failed transfer.
    static func linkStatus(_ error: Error) -> LinkStatus {
        switch error as? DspiError {
        case .Disconnected?: return .noDevice
        case .Link(let code, _)? where code == "timeout": return .timeout
        default: return .error   // a stall (Refused 0) and anything else
        }
    }

    func sendControlRequest(request: UInt8, value: UInt16, index: UInt16, data: Data) {
        let expected = generation
        transferQueue.async {
            guard expected == self.generation, let pipe = self.pipe else { return }
            try? pipe.controlOut(opcode: request, value: value, data: data)
        }
    }

    func getControlRequest(request: UInt8, value: UInt16, index: UInt16, length: UInt16) -> Data? {
        if case .success(let data) = getControlResult(request: request, value: value, index: index, length: length) {
            return data
        }
        return nil
    }

    /// Synchronous OUT transfer that reports the outcome, for the hub.
    /// `.ok` certifies dispatch, not apply.  `index` is not sent: the core
    /// addresses the vendor interface (wIndex 2), as every DSPi request does.
    func sendControlResult(request: UInt8, value: UInt16, index: UInt16, data: Data) -> LinkStatus {
        let expected = generation
        return transferQueue.sync {
            guard expected == self.generation, let pipe = self.pipe else { return .noDevice }
            do {
                try pipe.controlOut(opcode: request, value: value, data: data)
                return .ok
            } catch {
                return Self.linkStatus(error)
            }
        }
    }

    func getControlResult(request: UInt8, value: UInt16, index: UInt16, length: UInt16) -> Result<Data, LinkStatus> {
        let expected = generation
        return transferQueue.sync {
            guard expected == self.generation, let pipe = self.pipe else { return .failure(.noDevice) }
            do {
                return .success(try pipe.controlIn(opcode: request, value: value, len: length))
            } catch {
                return .failure(Self.linkStatus(error))
            }
        }
    }
}

// MARK: - DeviceTransport

extension CoreUsb: DeviceTransport {
    var isConnectedPublisher: AnyPublisher<Bool, Never> { $isConnected.eraseToAnyPublisher() }
    var availableDevicesPublisher: AnyPublisher<[DSPiDevice], Never> { $availableDevices.eraseToAnyPublisher() }
    var selectedDevicePublisher: AnyPublisher<DSPiDevice?, Never> { $selectedDevice.eraseToAnyPublisher() }
    var errorMessagePublisher: AnyPublisher<String?, Never> { $errorMessage.eraseToAnyPublisher() }

    func markDisconnected() {
        DispatchQueue.main.async { self.isConnected = false }
    }

    func addNotificationObserver(_ handler: @escaping (LinkNotification) -> Void) -> AnyCancellable {
        notifications.add(handler)
    }

    /// Driven directly, with no hub, there is no other session to lock out.
    func acquireDeviceLock(reason: String, timeout: TimeInterval) -> Bool { true }
    func releaseDeviceLock() {}
}
#endif
