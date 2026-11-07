// WP HB7: the Swift console's Link client (`apps/apple/DSPi Console/Link/LinkClient.swift`
// and the files it needs, unmodified) through the shared scenario list
// (../README.md), against `serve::conformance_serve_for_other_clients`.
// Built with swiftc in the macOS VM by ../run-clients.sh, which answers the
// `@ctl` lines. Shims.swift stands in for the app types the client only names.
//
// Environment: HB7_URL, HB7_HUB_ID.
// Output: one `PASS <id>` or `FAIL <id>: <why>` per scenario, then `DONE`.

import Foundation

setvbuf(stdout, nil, _IOLBF, 0)

let env = ProcessInfo.processInfo.environment
let hubURL = URL(string: env["HB7_URL"] ?? "")!
let hubID = env["HB7_HUB_ID"] ?? ""

let REQ_GET_PLATFORM: UInt8 = 0x7F
let REQ_GET_STATUS: UInt8 = 0x50
let REQ_SET_MASTER_VOLUME: UInt8 = 0xD2
let waitSeconds: TimeInterval = 10

struct Failure: Error, CustomStringConvertible { let description: String }

func expect(_ cond: Bool, _ why: @autoclosure () -> String) throws {
    if !cond { throw Failure(description: why()) }
}

// ---- The driver ----

/// Ask the driver for what only the hub's side can do.
func ctl(_ command: String) throws -> [String: Any] {
    print("@ctl \(command)")
    guard let line = readLine(),
          let object = try JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any]
    else { throw Failure(description: "the driver went away") }
    return object
}

func newPin() throws -> String {
    guard let pin = try ctl("pin")["pin"] as? String else { throw Failure(description: "no PIN") }
    return pin
}

// ---- Helpers. The client calls back on the main thread: spin its run loop. ----

func spin(until done: () -> Bool, timeout: TimeInterval = waitSeconds) -> Bool {
    let end = Date().addingTimeInterval(timeout)
    while !done() && Date() < end {
        RunLoop.main.run(until: Date().addingTimeInterval(0.01))
    }
    return done()
}

/// Waits for a completion-style call.
func call<T>(_ what: String, _ start: (@escaping (T) -> Void) -> Void) throws -> T {
    var result: T?
    start { result = $0 }
    guard spin(until: { result != nil }) else { throw Failure(description: "\(what): no answer") }
    return result!
}

func settled(_ c: LinkClient) -> Bool {
    switch c.state {
    case .ready, .awaitingAuth, .failed: return true
    default: return false
    }
}

func connect(_ name: String, token: String? = nil) throws -> LinkClient {
    let c = LinkClient()
    c.connect(to: hubURL, clientName: name, token: token)
    try expect(spin(until: { settled(c) }), "connect: state \(c.state)")
    return c
}

func pair(_ c: LinkClient, _ pin: String, _ name: String, _ role: LinkRole) throws -> Result<String, LinkClientError> {
    try call("pair") { done in c.pair(pin: pin, clientName: name, role: role, completion: done) }
}

func paired(_ name: String, _ role: LinkRole) throws -> (LinkClient, String) {
    let c = try connect(name)
    let token = try pair(c, try newPin(), name, role).get()
    // The published state follows the completion by one main-thread hop.
    try expect(spin(until: { c.state == .ready }), "state \(c.state) after pairing")
    return (c, token)
}

func command(_ c: LinkClient, _ r: LinkCmdRequest) throws -> LinkCmdResponse {
    try call("command") { done in c.command(r, completion: done) }
}

func platform(_ c: LinkClient, handle: UInt8 = 0) throws -> LinkCmdResponse {
    try command(c, LinkCmdRequest(tag: 0, handle: handle, direction: .get, bRequest: REQ_GET_PLATFORM,
                                  wValue: 0, wIndex: 2, wLength: 7))
}

func setVolume(_ c: LinkClient, _ db: Float) throws -> LinkStatus {
    var bits = db.bitPattern.littleEndian
    let payload = withUnsafeBytes(of: &bits) { Data($0) }
    return try command(c, LinkCmdRequest(tag: 0, handle: 0, direction: .set, bRequest: REQ_SET_MASTER_VOLUME,
                                         wValue: 0, wIndex: 2, payload: payload)).status
}

func failedReason(_ c: LinkClient) -> String? {
    if case .failed(let reason) = c.state { return reason }
    return nil
}

// ---- The scenarios ----

var open: [LinkClient] = []
func keep(_ c: LinkClient) -> LinkClient { open.append(c); return c }

let scenarios: [(String, () throws -> Void)] = [
    ("C1 hello", {
        let c = keep(try connect("swift c1"))
        try expect(c.state == .awaitingAuth(needsPairing: true), "state \(c.state)")
        try expect(c.hubInfo?.id == hubID, "hub id \(String(describing: c.hubInfo?.id))")
        try expect(c.capabilities.contains("cmd") && !c.capabilities.contains("fw_install"),
                   "caps \(c.capabilities)")
        try expect((c.limits?.maxFrame ?? 0) > 0, "no max_frame")
    }),
    ("C2 a wrong PIN is typed and a right one then works", {
        let right = try newPin()
        let c = keep(try connect("swift c2"))
        let wrong = right == "000000" ? "111111" : "000000"
        switch try pair(c, wrong, "swift c2", .control) {
        case .failure(.rejected(code: "unauthenticated", _)): break
        case let other: throw Failure(description: "wrong PIN gave \(other)")
        }
        try expect(c.state == .awaitingAuth(needsPairing: true), "state \(c.state)")
        _ = try pair(c, right, "swift c2", .control).get()
        try expect(spin(until: { c.state == .ready }), "state \(c.state)")
    }),
    ("C3 pair lists the device", {
        let (c, _) = try paired("swift c3", .admin)
        _ = keep(c)
        try expect(c.role == .admin, "role \(String(describing: c.role))")
        try expect(spin(until: { !c.devices.isEmpty }), "no devices")
        let d = c.devices
        try expect(d.count == 1 && d[0].handle == 0 && d[0].state == .online && !d[0].serial.isEmpty, "\(d)")
    }),
    ("C4 the token reconnects without a PIN", {
        let (c, token) = try paired("swift c4", .control)
        _ = keep(c)
        let again = keep(try connect("swift c4", token: token))
        try expect(again.state == .ready, "state \(again.state)")
        try expect(again.sessionID != c.sessionID, "the same session id")
        try expect(again.role == .control, "role \(String(describing: again.role))")
    }),
    ("C5 commands", {
        let (c, _) = try paired("swift c5", .control)
        _ = keep(c)
        let p = try platform(c)
        try expect(p.status == .ok && p.payload.count == 7, "status \(p.status)")
        try expect(try platform(c, handle: 9).status == .noDevice, "not NO_DEVICE")
    }),
    ("C6 a write is heard with its origin", {
        let (c, token) = try paired("swift c6", .control)
        _ = keep(c)
        let other = keep(try connect("swift c6", token: token))
        var echo: LinkNotifyFrame?
        var heard: LinkNotifyFrame?
        c.onNotify = { if echo == nil { echo = $0 } }
        other.onNotify = { if heard == nil { heard = $0 } }
        try expect(try setVolume(c, -12) == .ok, "write refused")
        try expect(spin(until: { echo != nil && heard != nil }), "no notifications")
        try expect(echo!.origin == c.sessionID, "echo origin \(echo!.origin), session \(c.sessionID)")
        try expect(heard!.origin == c.sessionID, "foreign origin \(heard!.origin)")
        try expect(heard!.packet.first == 2, "not a v2 packet")
    }),
    ("C7 a change at the device has origin 0", {
        let (c, _) = try paired("swift c7", .viewer)
        _ = keep(c)
        var heard: LinkNotifyFrame?
        c.onNotify = { if heard == nil { heard = $0 } }
        _ = try ctl("change")
        try expect(spin(until: { heard != nil }), "no notification")
        try expect(heard!.origin == 0, "origin \(heard!.origin)")
    }),
    ("C8 snapshot", {
        let (c, _) = try paired("swift c8", .viewer)
        _ = keep(c)
        let snap = try call("snapshot") { done in c.snapshot(handle: 0, completion: done) }.get()
        let bulk = snap.bulkB64.flatMap { Data(base64Encoded: $0) }
        try expect(snap.handle == 0 && (bulk?.count ?? 0) > 0, "bulk \(String(describing: bulk?.count))")
    }),
    ("C9 polls arrive and stop", {
        let (c, _) = try paired("swift c9", .viewer)
        _ = keep(c)
        var count = 0
        c.onPoll = { p in if p.handle == 0 && p.slot == 1 && !p.payload.isEmpty { count += 1 } }
        let spec = LinkPollSpec(slot: 1, req: Int(REQ_GET_STATUS), val: 9, idx: 2, len: 41, hz: 10)
        let grants = try call("poll.subscribe") { done in c.subscribePolls(handle: 0, polls: [spec], completion: done) }.get()
        try expect(grants.count == 1 && grants[0].slot == 1, "\(grants)")
        try expect(spin(until: { count >= 3 }), "\(count) polls")
        c.unsubscribePolls(handle: 0, slots: [1])
        _ = spin(until: { false }, timeout: 0.3)
        let before = count
        _ = spin(until: { false }, timeout: 0.5)
        try expect(count == before, "polls after the unsubscribe")
    }),
    ("C10 a lock holds others off", {
        let (c, token) = try paired("swift c10", .control)
        _ = keep(c)
        let other = keep(try connect("swift c10", token: token))
        try call("lock.acquire") { done in c.acquireLock(handle: 0, reason: "conformance", timeoutMs: 5000, completion: done) }.get()
        try expect(try setVolume(other, -20) == .locked, "not LOCKED")
        try expect(try setVolume(c, -20) == .ok, "the holder could not write")
        c.releaseLock(handle: 0)
        let end = Date().addingTimeInterval(waitSeconds)
        var status = LinkStatus.locked
        while status != .ok && Date() < end { status = try setVolume(other, -21) }
        try expect(status == .ok, "still locked")
    }),
    ("C11 a viewer reads and may not write", {
        let (c, _) = try paired("swift c11", .viewer)
        _ = keep(c)
        try expect(c.role == .viewer, "role \(String(describing: c.role))")
        try expect(try platform(c).status == .ok, "read refused")
        try expect(try setVolume(c, -6) == .denied, "not DENIED")
    }),
    ("C12 a revoke closes 4002 and the token is refused", {
        let (c, token) = try paired("swift c12 revoked", .viewer)
        try expect(try ctl("revoke swift c12 revoked")["revoked"] as? Bool == true, "not revoked")
        try expect(spin(until: { failedReason(c) != nil }), "not closed: \(c.state)")
        try expect(failedReason(c) == "access revoked", "closed: \(c.state)")
        let again = keep(try connect("swift c12 revoked", token: token))
        try expect(again.state == .awaitingAuth(needsPairing: true), "state \(again.state)")
    }),
    ("C13 a stop closes 4003", {
        let (c, _) = try paired("swift c13", .viewer)
        _ = try ctl("stop")
        try expect(spin(until: { failedReason(c) != nil }), "not closed: \(c.state)")
        try expect(failedReason(c) == "hub shutting down", "closed: \(c.state)")
    }),
]

var failed = 0
for (id, run) in scenarios {
    do {
        try run()
        print("PASS \(id)")
    } catch {
        failed += 1
        print("FAIL \(id): \(error)")
    }
    for c in open { c.disconnect() }
    open.removeAll()
}
print("DONE \(scenarios.count - failed)/\(scenarios.count)")
exit(failed == 0 ? 0 : 1)
