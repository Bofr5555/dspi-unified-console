// WP SW-M3: one more app name the HB7 client's files mention and its run never
// reaches, beside crates/dspi-hub/tests/conformance/swift/Shims.swift:
// DeviceTransport.swift's RefusedWrite takes two sentences from the core's notice
// rows (ConsoleNotice, Core/NoticeCenter.swift, since P1SW3).

import Foundation

struct ConsoleNotice {
    let title: String
    let sentence: String
    static func row(_ id: String) -> ConsoleNotice { ConsoleNotice(title: id, sentence: id) }
}
