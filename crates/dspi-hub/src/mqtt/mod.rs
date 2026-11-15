//! MQTT for the hub (PLAN-P2 I6, OD-P2c): a hand-written MQTT 3.1.1 client on std threads,
//! no async runtime and no new crate. No TLS (OD-P2f): a LAN broker over plain TCP.
//!
//! | Module | What | Owner |
//! |---|---|---|
//! | [`codec`] | the packets, pure encode and decode with length limits | P2MQC |
//! | [`client`] | one blocking client thread: keep-alive, reconnect, a bounded queue | P2MQC |
//! | [`service`] | the hub's broker setting (`mqtt.*`, `mqtt.json`) and what it publishes | P2MQ |
//!
//! P2MQ registers it as the `mqtt` service ([`service`], `mqtt.json`); P2HA adds Home Assistant.

pub mod client;
pub mod codec;
// -- WP P2MQ --
pub mod service;

pub use client::{MqttClient, MqttConfig, MqttError, MqttEvent, MqttStatus};
pub use codec::{QoS, Will};
