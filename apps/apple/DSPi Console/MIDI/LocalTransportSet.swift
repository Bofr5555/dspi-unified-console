//
//  LocalTransportSet.swift
//  DSPi Console
//
//  Several local transports behind one: the iOS app's DSPi on USB MIDI and
//  its demo device in one device list.  `CompositeTransport.local` is a
//  single transport, so this sits in that slot.  Selection goes to whichever
//  member lists the device; the connection, selection, error and
//  notification streams follow the member in use.
//
//  Members are in priority order.  A member that selects a device by itself
//  (a DSPi plugged in) takes over from a lower-priority member (the demo
//  device), or from any member with nothing selected.
//
//  -- WP SWF2 -- On macOS the set is the DSPi on its vendor interface, then
//  the same DSPi on its USB MIDI port (one serial, one row in the list).  A
//  member that fails to open a device it lists (an error, nothing selected)
//  hands the device to the next member that lists it, so a vendor path that
//  cannot be opened falls back to MIDI instead of leaving a red badge.
//

import Foundation
import Combine

final class LocalTransportSet: DeviceTransport {
    private let members: [any DeviceTransport]
    private var active: any DeviceTransport
    private let lock = NSLock()
    /// Each member's list as last emitted (a publisher may fire before the
    /// member's property holds the new value).
    private var lists: [[DSPiDevice]]

    private let connectedSubject = CurrentValueSubject<Bool, Never>(false)
    private let devicesSubject = CurrentValueSubject<[DSPiDevice], Never>([])
    private let selectedSubject = CurrentValueSubject<DSPiDevice?, Never>(nil)
    private let errorSubject = CurrentValueSubject<String?, Never>(nil)
    private let fanout = NotificationFanout()
    private var memberCancellables = Set<AnyCancellable>()
    private var activeCancellables = Set<AnyCancellable>()

    init(_ members: [any DeviceTransport]) {
        precondition(!members.isEmpty)
        self.members = members
        self.active = members[0]
        self.lists = members.map { $0.availableDevices }
        attach(to: members[0])
        for (i, member) in members.enumerated() {
            member.availableDevicesPublisher
                .sink { [weak self] devices in self?.listChanged(member: i, devices) }
                .store(in: &memberCancellables)
            member.selectedDevicePublisher
                .sink { [weak self] device in self?.memberSelected(i, device) }
                .store(in: &memberCancellables)
            // -- WP SWF2 --
            member.errorMessagePublisher
                .sink { [weak self] error in self?.memberFailed(i, error) }
                .store(in: &memberCancellables)
        }
    }

    // -- WP SWF2 --
    /// The member in use reported an error with nothing selected: it could
    /// not open its device.  The first lower-priority member that lists one
    /// of its devices and has nothing selected opens that device instead.
    private func memberFailed(_ i: Int, _ error: String?) {
        guard error != nil else { return }
        let member = members[i]
        guard member === current, member.selectedDevice == nil, !member.isConnected else { return }
        lock.lock(); let failedList = lists[i]; let later = Array(lists.enumerated().dropFirst(i + 1)); lock.unlock()
        for (j, list) in later {
            let fallback = members[j]
            guard fallback.selectedDevice == nil,
                  let device = failedList.first(where: { list.contains($0) }),
                  let own = list.first(where: { $0 == device }) else { continue }
            attach(to: fallback)
            fallback.selectDevice(own)
            return
        }
    }

    private func listChanged(member: Int, _ devices: [DSPiDevice]) {
        lock.lock()
        lists[member] = devices
        var merged: [DSPiDevice] = []
        for list in lists { merged += list.filter { d in !merged.contains(d) } }
        lock.unlock()
        devicesSubject.send(merged)
    }

    private func index(of transport: any DeviceTransport) -> Int {
        members.firstIndex { $0 === transport } ?? members.count
    }

    private func memberSelected(_ i: Int, _ device: DSPiDevice?) {
        guard device != nil else { return }
        lock.lock(); let current = active; lock.unlock()
        let member = members[i]
        guard member !== current else { return }
        if current.selectedDevice == nil || i < index(of: current) {
            current.disconnect()
            attach(to: member)
        } else {
            // The user's choice stands; the newcomer lets go.
            member.disconnect()
        }
    }

    private func attach(to transport: any DeviceTransport) {
        lock.lock(); active = transport; lock.unlock()
        activeCancellables.removeAll()
        transport.isConnectedPublisher.sink { [weak self] in self?.connectedSubject.send($0) }
            .store(in: &activeCancellables)
        transport.selectedDevicePublisher.sink { [weak self] in self?.selectedSubject.send($0) }
            .store(in: &activeCancellables)
        transport.errorMessagePublisher.sink { [weak self] in self?.errorSubject.send($0) }
            .store(in: &activeCancellables)
        transport.addNotificationObserver { [weak self] in self?.fanout.publish($0) }
            .store(in: &activeCancellables)
    }

    private var current: any DeviceTransport { lock.lock(); defer { lock.unlock() }; return active }

    // MARK: - DeviceTransport

    var session: LinkSessionID { current.session }
    var generation: UInt64 { current.generation }
    var isConnected: Bool { connectedSubject.value }
    var isConnectedPublisher: AnyPublisher<Bool, Never> { connectedSubject.eraseToAnyPublisher() }
    var availableDevices: [DSPiDevice] { devicesSubject.value }
    var availableDevicesPublisher: AnyPublisher<[DSPiDevice], Never> { devicesSubject.eraseToAnyPublisher() }
    var selectedDevice: DSPiDevice? { selectedSubject.value }
    var selectedDevicePublisher: AnyPublisher<DSPiDevice?, Never> { selectedSubject.eraseToAnyPublisher() }
    var errorMessage: String? { errorSubject.value }
    var errorMessagePublisher: AnyPublisher<String?, Never> { errorSubject.eraseToAnyPublisher() }

    func selectDevice(_ device: DSPiDevice) {
        lock.lock(); let owner = lists.firstIndex { $0.contains(device) }; lock.unlock()
        guard let i = owner else { return }
        let member = members[i]
        let previous = current
        if member !== previous {
            previous.disconnect()
            attach(to: member)
        }
        member.selectDevice(device)
    }

    func reconnect() { current.reconnect() }
    func disconnect() { current.disconnect() }
    func markDisconnected() { current.markDisconnected() }

    func sendControlRequest(request: UInt8, value: UInt16, index: UInt16, data: Data) {
        current.sendControlRequest(request: request, value: value, index: index, data: data)
    }

    func getControlResult(request: UInt8, value: UInt16, index: UInt16, length: UInt16) -> Result<Data, LinkStatus> {
        current.getControlResult(request: request, value: value, index: index, length: length)
    }

    func addNotificationObserver(_ handler: @escaping (LinkNotification) -> Void) -> AnyCancellable {
        fanout.add(handler)
    }

    // -- WP SWF2 -- The member in use holds the lock: on macOS the hub's
    // (HubTransport).  Without these the protocol's no-op default would win.
    func acquireDeviceLock(reason: String, timeout: TimeInterval) -> Bool {
        current.acquireDeviceLock(reason: reason, timeout: timeout)
    }

    func releaseDeviceLock() { current.releaseDeviceLock() }
}

// -- WP SW8 -- The core of the member in use, if it is driven through one
// (CoreMirror.swift).
extension LocalTransportSet: CoreStateSourceProviding {
    var providedCoreStateSource: CoreStateSource? { current.coreStateSource }
}
