//! The voltage rule end to end (HUB.md section 9, HB2): a raise past 1200 mV from the
//! network is refused by the hub, on a real socket, while the app beside it, whose core
//! judges the word, goes through. Loopback only.
#![allow(clippy::arithmetic_side_effects)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use dspi_core::DspiCore;
use dspi_hub::app::AppLink;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_link::socket::TcpConnector;
use dspi_link::{ConnectOptions, Dir, LinkClient, status};
use dspi_proto::ext::{CoreVregStatus, VregOp};
use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{FakeDevice, Transport};

const V120: u8 = 0x0D;
const V125: u8 = 0x0E;
const V130: u8 = 0x0F;
const SET: u8 = 0;
const WORD: &str = "OVERVOLT";

fn stored(c: &LinkClient) -> u8 {
    let len = CoreVregStatus::SIZE as u16;
    let reply = c.command(
        0,
        Dir::Get,
        op::REQ_EXTENSION,
        g::EXT_CORE_VREG,
        2,
        len,
        &[],
    );
    assert_eq!(reply.status, status::OK);
    CoreVregStatus::decode(&reply.payload).unwrap().stored_sel
}

fn set(c: &LinkClient, code: u8) -> u8 {
    let payload = VregOp::Set(code).encode();
    c.command(
        0,
        Dir::Set,
        op::REQ_EXTENSION,
        g::EXT_CORE_VREG,
        2,
        payload.len() as u16,
        &payload,
    )
    .status
}

#[test]
fn wp_hb1_a_network_raise_is_refused_on_the_wire_and_the_apps_goes_through() {
    let dir = std::env::temp_dir()
        .join("wp-hb1-vreg")
        .join(std::process::id().to_string());
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir)
        }),
        ..HubConfig::in_process("hb1")
    })
    .unwrap();
    let device = FakeDevice::demo();
    let (serial, remote) = (device.descriptor().serial.clone(), device.remote());
    remote.vreg_set_build_default(V120);
    hub.attach(Box::new(device), 0x2E8B).unwrap();
    let core = DspiCore::new();
    let _link = AppLink::attach(&hub.handle(), Arc::clone(&core)).unwrap();
    core.select_device(serial).unwrap();

    let port = hub.network().unwrap().port();
    let options = ConnectOptions {
        client_name: "admin on the LAN".into(),
        app: "wp_hb1".into(),
        app_version: "0".into(),
        token: None,
    };
    let url = format!("ws://127.0.0.1:{port}/dspi/v1");
    let (net, _) = LinkClient::connect(&TcpConnector::default(), &url, &options, None).unwrap();
    assert_eq!(net.role(), "admin", "the most a network client can be");

    assert_eq!(
        set(&net, V125),
        status::DENIED,
        "no word can be typed over the wire"
    );
    assert_eq!(stored(&net), 0, "nothing was written");

    core.core_vreg_op(SET, V125, Some(WORD.into())).unwrap();
    assert_eq!(stored(&net), V125, "the app's raise reached the unit");

    assert_eq!(set(&net, V130), status::DENIED);
    assert_eq!(stored(&net), V125);
    assert_eq!(set(&net, V125), status::OK, "level is not a raise");
    assert_eq!(set(&net, V120), status::OK, "and neither is down");
    assert_eq!(stored(&net), V120);
    net.close();
    hub.stop();
}
