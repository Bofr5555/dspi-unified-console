//! The DSPi behind the tunnel: each UART request becomes the USB vendor
//! request it wraps (`control_interfaces_spec.md` 1.1: one command surface,
//! three transports), sent through a [`Transport`] (the demo device or the
//! real one over USB). What only the UART path has, the tunnel plays itself.

use dspi_proto::VENDOR_INTERFACE;
use dspi_proto::frame::{Frame, status};
use dspi_transport::{Transport, TransportError};

use crate::uart::status_reply;

/// `REQ_SET_UART_CONFIG`, `REQ_SET_I2C_CONFIG`: USB only, `BLOCKED` over UART
/// (spec 1.2).
const REQ_SET_UART_CONFIG: u8 = 0xF5;
const REQ_SET_I2C_CONFIG: u8 = 0xF7;
/// `REQ_GET_UART_CONFIG` and `REQ_GET_CTRL_IFACE_STATUS`: answered from the
/// tunnel's own UART, since the device's USB answer describes the device's
/// real UART (on a K340 at the desk, probably off).
const REQ_GET_UART_CONFIG: u8 = 0xF6;
const REQ_GET_CTRL_IFACE_STATUS: u8 = 0xF9;
/// `REQ_GET_ALL_PARAMS`: a GET with `wLength` 0 ("full, uncapped") is the
/// whole bulk image.
const REQ_GET_ALL_PARAMS: u8 = 0xA0;
/// What "uncapped" reads at most: more than any bulk image.
const UNCAPPED: u16 = 8192;
/// The UART pins the tunnel reports (a legal RP2350 UART1 pair, spec 2.1).
const TX_PIN: u8 = 4;
const RX_PIN: u8 = 5;
/// `CtrlIfaceStatus.proto_version`.
const PROTO_VERSION: u8 = 1;

/// Whether `wIndex` is one the firmware ignores: 0, as the UART spec says
/// to send it (3.2), or the vendor interface, which Link clients send since
/// the hub forwards it unchanged. [`Transport`] has no wIndex (it always sends
/// the vendor interface), so a request using it as a parameter cannot be
/// passed on; none of the bridge's or our clients' requests does.
fn plain_index(index: u16) -> bool {
    index == 0 || index == VENDOR_INTERFACE
}

/// How the tunnel answers.
#[derive(Debug, Clone)]
pub struct DeviceOptions {
    /// The baud the tunnel's UART runs at, reported in `REQ_GET_UART_CONFIG`.
    pub baud: u32,
    /// `notify_enable` in `REQ_GET_UART_CONFIG`; off means no `0x40` frames.
    pub notify: bool,
    /// Refuse every SET with `ERROR` before it reaches the device: the K340
    /// rule (nothing written) made structural.
    pub read_only: bool,
}

/// Counters for the summary.
#[derive(Debug, Default, Clone)]
pub struct DeviceStats {
    pub gets: u64,
    pub sets: u64,
    pub refused_sets: u64,
    pub stalls: u64,
    pub timeouts: u64,
    pub nonzero_windex: u64,
}

pub struct Device {
    transport: Box<dyn Transport>,
    pub options: DeviceOptions,
    pub stats: DeviceStats,
}

impl Device {
    pub fn new(transport: Box<dyn Transport>, options: DeviceOptions) -> Self {
        Self {
            transport,
            options,
            stats: DeviceStats::default(),
        }
    }

    /// The answer to one request, or `None` when the device gave none (a USB
    /// timeout or a vanished device: the bridge then times out, as on a
    /// silent DSPi).
    pub fn answer(&mut self, request: &Frame) -> Option<Frame> {
        match *request {
            Frame::Get {
                request,
                value,
                index,
                length,
            } => {
                self.stats.gets += 1;
                if !plain_index(index) {
                    self.stats.nonzero_windex += 1;
                    return Some(status_reply(true, status::ERROR));
                }
                if let Some(local) = self.local_get(request) {
                    let want = if length == 0 {
                        local.len()
                    } else {
                        usize::from(length)
                    };
                    let mut payload = local;
                    payload.truncate(want);
                    return Some(Frame::GetResponse {
                        status: status::OK,
                        payload,
                    });
                }
                let cap = if length == 0 && request == REQ_GET_ALL_PARAMS {
                    UNCAPPED
                } else {
                    length
                };
                let got = self.transport.control_in_upto(request, value, cap);
                self.reply(
                    true,
                    got.map(|payload| Frame::GetResponse {
                        status: status::OK,
                        payload,
                    }),
                )
            }
            Frame::Set {
                request,
                value,
                index,
                ref payload,
            } => {
                self.stats.sets += 1;
                if request == REQ_SET_UART_CONFIG || request == REQ_SET_I2C_CONFIG {
                    return Some(status_reply(false, status::BLOCKED));
                }
                if !plain_index(index) {
                    self.stats.nonzero_windex += 1;
                    return Some(status_reply(false, status::ERROR));
                }
                if self.options.read_only {
                    self.stats.refused_sets += 1;
                    return Some(status_reply(false, status::ERROR));
                }
                let done = self.transport.control_out(request, value, payload);
                self.reply(false, done.map(|()| crate::uart::set_ok()))
            }
            // Responses and notifications never come from the bridge; the
            // parser only lets requests through.
            _ => None,
        }
    }

    fn reply(&mut self, is_get: bool, got: Result<Frame, TransportError>) -> Option<Frame> {
        match got {
            Ok(frame) => Some(frame),
            // A STALL is the firmware refusing the request; over UART the same
            // refusal is CTRL_STATUS_ERROR (spec 3.4).
            Err(TransportError::Stalled { .. }) => {
                self.stats.stalls += 1;
                Some(status_reply(is_get, status::ERROR))
            }
            Err(TransportError::ShortRead { .. }) => Some(status_reply(is_get, status::ERROR)),
            Err(_) => {
                self.stats.timeouts += 1;
                None
            }
        }
    }

    /// The UART-only reads the tunnel answers itself.
    fn local_get(&self, request: u8) -> Option<Vec<u8>> {
        match request {
            REQ_GET_UART_CONFIG => {
                let mut c = vec![1, TX_PIN, RX_PIN, u8::from(self.options.notify)];
                c.extend_from_slice(&self.options.baud.to_le_bytes());
                Some(c)
            }
            REQ_GET_CTRL_IFACE_STATUS => Some(vec![0, 1, 0, 0, PROTO_VERSION, 0, 0, 0]),
            _ => None,
        }
    }

    pub fn transport(&self) -> &dyn Transport {
        self.transport.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dspi_transport::FakeDevice;

    fn device(read_only: bool) -> Device {
        Device::new(
            Box::new(FakeDevice::demo()),
            DeviceOptions {
                baud: 115_200,
                notify: true,
                read_only,
            },
        )
    }

    fn get(request: u8, value: u16, length: u16) -> Frame {
        Frame::Get {
            request,
            value,
            index: 0,
            length,
        }
    }

    #[test]
    fn wp_lk3_a_get_is_the_usb_in_request_it_wraps() {
        let mut d = device(false);
        let Some(Frame::GetResponse { status: s, payload }) = d.answer(&get(0x7E, 0, 32)) else {
            panic!("no answer")
        };
        assert_eq!(s, status::OK);
        assert!(payload.starts_with(b"DEMO0000DSPI0001"));
        // The bulk image, uncapped (wLength 0), as the bridge reads it.
        let Some(Frame::GetResponse { payload, .. }) = d.answer(&get(0xA0, 0, 0)) else {
            panic!("no bulk")
        };
        assert!(payload.len() > 3000, "{}", payload.len());
    }

    #[test]
    fn wp_lk3_the_uart_config_is_the_tunnels_own() {
        let mut d = device(false);
        let Some(Frame::GetResponse { payload, .. }) = d.answer(&get(0xF6, 0, 8)) else {
            panic!()
        };
        assert_eq!(payload, vec![1, 4, 5, 1, 0x00, 0xC2, 0x01, 0x00]);
        let Some(Frame::GetResponse { payload, .. }) = d.answer(&get(0xF9, 0, 8)) else {
            panic!()
        };
        assert_eq!(payload[1], 1, "uart_live");
        let set = Frame::Set {
            request: 0xF5,
            value: 0,
            index: 0,
            payload: vec![0; 8],
        };
        assert_eq!(
            d.answer(&set),
            Some(Frame::SetResponse {
                status: status::BLOCKED
            })
        );
    }

    #[test]
    fn wp_lk3_read_only_refuses_every_set_before_the_device() {
        let mut d = device(true);
        let before = d.answer(&get(0x57, 0, 32));
        let rename = Frame::Set {
            request: 0x56,
            value: 0,
            index: 0,
            payload: b"changed".to_vec(),
        };
        assert_eq!(
            d.answer(&rename),
            Some(Frame::SetResponse {
                status: status::ERROR
            })
        );
        assert_eq!(d.answer(&get(0x57, 0, 32)), before);
        assert_eq!(d.stats.refused_sets, 1);
    }

    #[test]
    fn wp_lk3_a_stall_is_error_and_windex_is_refused() {
        // An opcode the device does not implement: the mock stalls it.
        let mut d = Device::new(
            Box::new(dspi_transport::MockTransport::new().data(0x7F, vec![1, 2, 3, 4, 5, 6, 7])),
            DeviceOptions {
                baud: 115_200,
                notify: true,
                read_only: false,
            },
        );
        assert_eq!(
            d.answer(&get(0xEE, 0, 4)),
            Some(Frame::GetResponse {
                status: status::ERROR,
                payload: vec![]
            })
        );
        assert_eq!(d.stats.stalls, 1);
        let odd = Frame::Get {
            request: 0x7F,
            value: 0,
            index: 5,
            length: 7,
        };
        assert_eq!(
            d.answer(&odd),
            Some(Frame::GetResponse {
                status: status::ERROR,
                payload: vec![]
            })
        );
        assert_eq!(d.stats.nonzero_windex, 1);
    }
}
