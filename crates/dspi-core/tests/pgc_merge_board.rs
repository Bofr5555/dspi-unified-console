//! -- PG-C merge -- PGZERO line 5 (PLAN-PG section 1.9, S6): the core's
//! device info names the board, from the platform's byte 7 and, on board 0,
//! the USB descriptor (a paged Zero answers a 7-byte `SYS.PLATFORM`).

use std::sync::Arc;

use dspi_core::{DspiCore, DspiError};
use dspi_transport::fake::{Capture, FakeBoard, FakeGeneration, FakeOptions};
use dspi_transport::{FakeDevice, Transport, paged};

fn connect(dev: FakeDevice) -> Arc<DspiCore> {
    let serial = dev.descriptor().serial.clone();
    let t = paged::open(Box::new(dev)).expect("open");
    let core = DspiCore::new();
    core.open_transport(t, serial, 0)
        .expect("the connect sequence");
    core
}

fn zero(generation: Option<FakeGeneration>) -> FakeDevice {
    FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            board: FakeBoard::Zero,
            generation,
            ..FakeOptions::default()
        },
    )
}

#[test]
fn pgc_merge_the_device_info_names_the_board() {
    // A Pico of every generation
    for generation in FakeGeneration::ALL {
        let core = connect(FakeDevice::demo_generation(generation));
        let p = core.device_protocol().unwrap();
        assert_eq!(
            (p.board_id, p.board_name.as_str()),
            (0, "DSPi"),
            "{generation:?}"
        );
        core.disconnect();
        assert_eq!(core.device_protocol(), Err(DspiError::Disconnected));
    }
    // The legacy Zero by byte 7, the paged one by its descriptor
    for generation in [None, Some(FakeGeneration::STOCK117)] {
        let core = connect(zero(generation));
        let p = core.device_protocol().unwrap();
        assert_eq!(
            (p.board_id, p.board_name.as_str()),
            (1, "DSPi Zero"),
            "{generation:?}"
        );
        assert_eq!(core.board().unwrap().id(), 1);
    }
}
