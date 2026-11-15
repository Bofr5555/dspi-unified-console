//! WP HB2: the voltage rule for the app, through a real `DspiCore` on the hub. The core
//! judges the word; the hub lets the result through and refuses the network's.
#![allow(clippy::arithmetic_side_effects)]

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use dspi_core::{DspiCore, DspiError};
use dspi_hub::app::AppLink;
use dspi_hub::{Completion, Hub, HubConfig, Role, Session};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::{Dir, status};
use dspi_proto::ext::{CoreVregStatus, VregOp};
use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{FakeDevice, FakeRemote, Transport};

const V120: u8 = 0x0D;
const V125: u8 = 0x0E;
const V130: u8 = 0x0F;
/// `core_vreg_op`'s `op` for a `SET`, and for a `TRIAL`.
const SET: u8 = 0;
const TRIAL: u8 = 1;
const WORD: &str = "OVERVOLT";

fn app_at(code: u8) -> (Hub, Arc<DspiCore>, AppLink, FakeRemote) {
    let hub = Hub::start(HubConfig::in_process("hb2")).unwrap();
    let device = FakeDevice::demo();
    let (serial, remote) = (device.descriptor().serial.clone(), device.remote());
    remote.vreg_set_build_default(code);
    hub.attach(Box::new(device), 0x2E8B).unwrap();
    let core = DspiCore::new();
    let link = AppLink::attach(&hub.handle(), Arc::clone(&core)).unwrap();
    core.select_device(serial).unwrap();
    (hub, core, link, remote)
}

fn command(hub: &Hub, s: &Session, dir: Dir, value: u16, payload: Vec<u8>) -> CmdResponse {
    let (tx, rx) = mpsc::channel();
    let req = CmdRequest {
        tag: 1,
        handle: 0,
        dir,
        b_request: op::REQ_EXTENSION,
        w_value: value,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: if dir == Dir::Get {
            CoreVregStatus::SIZE as u16
        } else {
            payload.len() as u16
        },
        payload,
    };
    hub.submit(s, req, Completion::new(1, move |r| tx.send(r).unwrap()));
    rx.recv_timeout(Duration::from_secs(10)).unwrap()
}

/// What the unit has now, as an admin on the network reads it.
fn unit(hub: &Hub, s: &Session) -> CoreVregStatus {
    let reply = command(hub, s, Dir::Get, g::EXT_CORE_VREG, Vec::new());
    assert_eq!(reply.status, status::OK);
    CoreVregStatus::decode(&reply.payload).unwrap()
}

/// -- WP P1TZF -- a SET stores the voltage: it takes a token (as a client
/// mints it after its dialog); the other operations take none.
fn tok(core: &dspi_core::DspiCore, op: u8) -> Option<String> {
    (op == SET).then(|| core.confirm_token(dspi_core::ConfirmOp::VregSave))
}

/// Someone at the machine typed the word: sharing being on makes no difference, and the
/// owner's example from 1.25 V to 1.30 V goes through too.
#[test]
fn wp_hb2_the_apps_raise_with_the_word_reaches_the_unit_through_the_hub() {
    let (hub, core, _link, _remote) = app_at(V120);
    let admin = hub.open_session(Role::Admin, None).unwrap();
    core.core_vreg_op_confirmed(SET, V125, Some(WORD.into()), tok(&core, SET))
        .unwrap();
    assert_eq!(unit(&hub, &admin).stored_sel, V125);
    core.core_vreg_op_confirmed(SET, V130, Some(WORD.into()), tok(&core, SET))
        .unwrap();
    assert_eq!(unit(&hub, &admin).stored_sel, V130);
}

/// No word, a wrong word, or the raw path: refused by the core, and nothing is written.
#[test]
fn wp_hb2_the_apps_raise_without_the_word_is_refused_by_its_core_through_the_hub() {
    let (hub, core, _link, _remote) = app_at(V120);
    let admin = hub.open_session(Role::Admin, None).unwrap();
    for (op_code, code, word) in [
        (SET, V125, None),
        (TRIAL, V125, None),
        (SET, V125, Some("overvolt".to_owned())),
        (TRIAL, V130, Some(String::new())),
    ] {
        let refused = core.core_vreg_op_confirmed(op_code, code, word.clone(), tok(&core, op_code));
        assert!(
            matches!(refused, Err(DspiError::OvervoltWordRequired { .. })),
            "{op_code} {code:#04x} {word:?}: {refused:?}"
        );
    }
    let raw = core.raw_set(
        op::REQ_EXTENSION,
        g::EXT_CORE_VREG,
        VregOp::Set(V125).encode().to_vec(),
    );
    assert!(
        matches!(raw, Err(DspiError::InvalidArgument { .. })),
        "{raw:?}"
    );
    let s = unit(&hub, &admin);
    assert_eq!((s.live_vreg, s.stored_sel), (V120, 0));
    assert!(!s.trial_active);
}

/// The exemption is the app's alone: on the same hub, at the same moment, a network
/// admin's raise is still refused, and it can still do what the rule does not ask about.
#[test]
fn wp_hb2_a_network_admin_stays_refused_beside_the_app() {
    let (hub, core, _link, _remote) = app_at(V120);
    let admin = hub.open_session(Role::Admin, None).unwrap();
    let set = |code: u8| VregOp::Set(code).encode().to_vec();
    let answer = command(&hub, &admin, Dir::Set, g::EXT_CORE_VREG, set(V125));
    assert_eq!(answer.status, status::DENIED);
    assert_eq!(unit(&hub, &admin).stored_sel, 0);

    core.core_vreg_op_confirmed(SET, V125, Some(WORD.into()), tok(&core, SET))
        .unwrap();
    let answer = command(&hub, &admin, Dir::Set, g::EXT_CORE_VREG, set(V130));
    assert_eq!(answer.status, status::DENIED);
    assert_eq!(unit(&hub, &admin).stored_sel, V125);
    let answer = command(&hub, &admin, Dir::Set, g::EXT_CORE_VREG, set(V125));
    assert_eq!(answer.status, status::OK, "level");
}
