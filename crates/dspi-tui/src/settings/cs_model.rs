//! The TUI's side of the control-surface editing model.
//!
//! The model itself (the wire vocabulary, the name tables, the link laws,
//! [`CsData`] and the pickers) lives in [`dspi_session::cs_model`], where
//! every GUI shares it, and is re-exported here so the pages keep calling it
//! `cs_model::*`. What stays is what only a terminal has: the [`CsContext`]
//! adapter over the Settings screen's `Cx`, the `SessionReply` wrappers
//! around [`apply`] and [`apply_code`], the placeholder rows, and the demo
//! device as the Settings screen and the gallery see it.

pub use dspi_session::cs_model::*;

use dspi_session::DeviceState;
use dspi_session::surfaces::{CsStatusPacket, Surfaces};
use dspi_session::{Session, surfaces};

use super::Cx;
use crate::shell::SessionReply;

impl CsContext for Cx<'_> {
    fn state(&self) -> &DeviceState {
        self.state
    }

    fn pin_owner(&self, gpio: u8, excluding: &str) -> Option<String> {
        self.data.owner_of(self.state, gpio, excluding)
    }

    fn control_i2c_sda(&self) -> Option<u8> {
        if self.data.iface.as_ref().is_some_and(|s| s.i2c_live) {
            self.data.i2c.as_ref().map(|c| c.sda_pin)
        } else {
            None
        }
    }

    fn preset_name(&self, slot: usize) -> String {
        self.data
            .preset_names
            .get(slot)
            .cloned()
            .unwrap_or_default()
    }

    fn valid_pins(&self) -> Vec<u8> {
        dspi_session::pins::valid_pins(self.platform())
    }
}

/// [`apply`] as the pages' reply: "Applied", or the device's reason.
pub(crate) fn run(
    session: &mut Session,
    f: impl FnOnce(&mut Surfaces<'_>) -> surfaces::Result<CsStatusPacket>,
) -> SessionReply {
    match apply(session, f) {
        Ok(()) => SessionReply::Ok("Applied".into()),
        Err(e) => SessionReply::Err(e.to_string()),
    }
}

/// [`apply_code`] as the pages' reply: `ok`, or the device's reason.
pub(crate) fn run_code(
    session: &mut Session,
    ok: &str,
    f: impl FnOnce(&mut Surfaces<'_>) -> surfaces::Result<u8>,
) -> SessionReply {
    match apply_code(session, f) {
        Ok(()) => SessionReply::Ok(ok.into()),
        Err(e) => SessionReply::Err(e.to_string()),
    }
}

/// The Console's placeholder while the capability tables are missing: either
/// the device is still being read ([`AWAITING`]), or there is no device to
/// read.
pub(crate) fn placeholder_rows(cx: &Cx<'_>) -> Vec<super::Row> {
    if cx.connected {
        vec![super::Row::note(AWAITING)]
    } else {
        vec![empty_state(DISCONNECTED_TITLE, DISCONNECTED_BODY)]
    }
}

/// The Console's centred empty state, as a terminal can draw one: a title and
/// its explanation, in the kit's info banner rather than a section header,
/// which is uppercased and would shout it.
pub(crate) fn empty_state(title: &str, body: &str) -> super::Row {
    super::Row::Banner(
        crate::widgets::BannerKind::Info,
        title.to_string(),
        body.to_string(),
    )
}

/// The demo device as Settings and the gallery show it: the session's
/// control-surface demo on top of the Settings demo's wiring.
pub mod demo {
    use dspi_session::DeviceState;
    pub use dspi_session::cs_model::demo::{caps, data};

    /// The Settings demo device, with the control-surface capabilities the
    /// three Control pages need.
    pub fn state() -> DeviceState {
        dspi_session::cs_model::demo::with_cs(super::super::demo::state())
    }

    /// The Settings demo data, with the control surfaces filled in.
    pub fn settings_data() -> super::super::SettingsData {
        super::super::SettingsData {
            cs: Some(data()),
            ..super::super::demo::data()
        }
    }
}

#[cfg(test)]
mod tests_wp_s4 {
    use super::*;
    use crate::settings::{AppConfig, SettingsData};

    fn parts() -> (DeviceState, SettingsData, AppConfig) {
        (demo::state(), demo::settings_data(), AppConfig::default())
    }

    /// The adapter hands the session the Settings screen's own preset names
    /// and pin owners.
    #[test]
    fn wp_s4_the_cx_adapter_reads_the_settings_data() {
        let (st, mut d, cfg) = parts();
        d.preset_names = vec!["Living Room".into(), String::new()];
        let cx = Cx {
            state: &st,
            data: &d,
            config: &cfg,
            connected: true,
            global_dirty: false,
        };
        let cs = demo::data();
        assert_eq!(
            enum_value_label(&cx, &cs, noun::PRESET, 0),
            "Preset 1 - Living Room"
        );
        assert_eq!(enum_value_label(&cx, &cs, noun::PRESET, 1), "Preset 2");
        // GPIO 5 is the first output pin in the Settings demo.
        assert!(CsContext::pin_owner(&cx, 5, "nobody").is_some());
        assert_eq!(
            CsContext::pin_owner(&cx, 5, "nobody"),
            d.owner_of(&st, 5, "nobody")
        );
        assert_eq!(CsContext::control_i2c_sda(&cx), None, "interface idle");
    }

    /// A live control interface blocks its I2C instance for the display.
    #[test]
    fn wp_s4_a_live_i2c_interface_blocks_its_instance() {
        let (st, mut d, cfg) = parts();
        let mut iface = d.iface.clone().unwrap_or_default();
        iface.i2c_live = true;
        d.iface = Some(iface);
        let mut i2c = d.i2c.clone().unwrap_or_default();
        i2c.sda_pin = 2;
        d.i2c = Some(i2c);
        let cx = Cx {
            state: &st,
            data: &d,
            config: &cfg,
            connected: true,
            global_dirty: false,
        };
        assert_eq!(CsContext::control_i2c_sda(&cx), Some(2));
        let cs = demo::data();
        let slot = cs
            .bindings
            .iter()
            .position(|b| b.component == ty::DISPLAY)
            .expect("demo display");
        // SDA 2 is on instance 1; no candidate may share that instance.
        let sdas = i2c_sda_candidates(&cx, &cs.bindings, slot);
        assert!(!sdas.is_empty());
        assert!(sdas.iter().all(|sda| (sda >> 1) & 1 != 1));
    }
}
