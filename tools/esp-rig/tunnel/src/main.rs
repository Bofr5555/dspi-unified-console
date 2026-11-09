//! WP LK3: `esp-rig-tunnel`, a DSPi on the UART of the emulated ESP32 bridge.
//!
//! Connects to the emulator's UART (QEMU's `-serial unix:PATH,server`), reads
//! the bridge's request frames, answers each through a DSPi behind the
//! [`dspi_transport::Transport`] seam (the demo device, or the real one over
//! USB) and sends the answers and the device's notifications back at the pace
//! of a real 8N1 line at the chosen baud, with optional wire faults.
//! See tools/esp-rig/README.md.

// -- WP LK4 -- (review c73 F1) Unix only: the workspace's Windows build
// builds every member's binaries, and QEMU's UART here is a Unix socket.
#[cfg(unix)]
mod device;
#[cfg(unix)]
mod tunnel;
#[cfg(unix)]
mod uart;

#[cfg(unix)]
fn main() {
    tunnel::main();
}

#[cfg(not(unix))]
fn main() {
    eprintln!("esp-rig-tunnel: Unix only (the emulator's UART is a Unix socket)");
    std::process::exit(2);
}
