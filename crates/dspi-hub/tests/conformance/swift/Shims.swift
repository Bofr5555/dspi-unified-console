// WP HB7: the app types that the Link client's files name and the conformance run
// never reaches (DeviceTransport.swift's device protocol and notification sources).
// The values are the firmware's (crates/dspi-proto/firmware/notify.h).

import Foundation

final class DSPiDevice {}

let NOTIFY_EVT_RTA_CONFIG: UInt8 = 0x0D
let NOTIFY_EVT_INPUT_SOURCE_NAME: UInt8 = 0x0F
let NOTIFY_EVT_DEVICE_NAME: UInt8 = 0x10
