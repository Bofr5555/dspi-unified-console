// WP SW25: the generated DspiConstants.swift seen from a user of its module
// (ci/compile-bindings.sh swift). The values come from the core; this checks
// that they arrive with the Swift types the app passes on the wire, and the few
// the Swift app hard-codes today.
import dspi_constants
import Foundation

var failures = 0
func expect(_ ok: Bool, _ what: String) {
    if !ok { print("constants_check_sw25: FAIL \(what)"); failures += 1 }
}

// Types: these assignments do not compile if a width changes.
let opcode: UInt8 = Opcodes.REQ_EXTENSION
let ext: UInt16 = DspiConstants.EXT_CORE_VREG
let flags: UInt32 = DspiConstants.NOTIFY_DISCONNECTED
let event: UInt8 = DspiConstants.NOTIFY_EVT_RTA_CONFIG
let linkStatus: UInt8 = LinkConstants.STATUS_LOCKED
let port: UInt16 = LinkConstants.DEFAULT_PORT
let channels: Int = DspiConstants.MAX_CHANNELS
let floorDb: Float = DspiConstants.RTA_FLOOR_DB

// The values the app hard-codes (CoreBackedTransport.notifyDisconnected,
// ExtensionChannel.swift, Constants.swift, LinkStatus, LinkDiscovery).
expect(flags == 1 << 9, "NOTIFY_DISCONNECTED")
expect(opcode == 0x00, "REQ_EXTENSION")
expect(DspiConstants.EXT_LIST == 0x0000 && DspiConstants.EXT_CROSSFEED_PAIR_MAP == 0x0001 && ext == 0x0002,
       "EXT_* sub-commands")
expect(event == 0x0D && DspiConstants.NOTIFY_EVT_DEVICE_NAME == 0x10, "NOTIFY_EVT_*")
expect(Opcodes.REQ_SET_EQ_PARAM == 0x42 && Opcodes.REQ_GET_ALL_PARAMS == 0xA0, "REQ_*")
expect(DspiConstants.VENDOR_ID == 0x2e8b && DspiConstants.PRODUCT_ID == 0xfeaa
       && DspiConstants.NOTIFY_ENDPOINT == 0x83, "USB ids")
expect(linkStatus == 0x85 && LinkConstants.STATUS_BAD_FRAME == 0x87 && port == 11915, "DSPi Link")
expect(channels > 0 && floorDb < 0, "sizes and floats")

// The NOTIFY_* result flags are distinct single bits.
let notifyFlags: [UInt32] = [
    DspiConstants.NOTIFY_STATE, DspiConstants.NOTIFY_CURVES, DspiConstants.NOTIFY_REFRESHED,
    DspiConstants.NOTIFY_INPUT_FORMAT, DspiConstants.NOTIFY_PRESET, DspiConstants.NOTIFY_SIGGEN,
    DspiConstants.NOTIFY_CS_AUX, DspiConstants.NOTIFY_IR_LEARN, DspiConstants.NOTIFY_NAMES,
    DspiConstants.NOTIFY_DISCONNECTED, DspiConstants.NOTIFY_STALE, DspiConstants.NOTIFY_STREAMS,
]
expect(notifyFlags.allSatisfy { $0.nonzeroBitCount == 1 }, "NOTIFY_* single bits")
expect(notifyFlags.reduce(0, |).nonzeroBitCount == notifyFlags.count, "NOTIFY_* distinct")

if failures > 0 { exit(1) }
print("constants_check_sw25: ok (\(notifyFlags.count) notify flags)")
