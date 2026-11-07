//! Settings > Overview: the pin map.
//!
//! Read-only, and deliberately so: the pin pickers on the other pages each
//! answer "is this one free?", and this answers "what does the whole header
//! look like right now". Only *held* pins are listed, which is the Console's
//! rule and the firmware's: a feature that is configured but switched off
//! reserves nothing, so a disabled optional S/PDIF input, a stopped ADAT
//! stream and a control interface that never came up appear nowhere.
//!
//! [`claims_from_state`] is the fallback the page draws with before Settings
//! has been able to ask the device: everything the bulk snapshot alone can
//! prove. It is deliberately a subset of `PinMap::build`, which additionally
//! reads the control interfaces and the control-surface bindings; those need a
//! session, and a page never has one.
//!
//! Above the map, on firmware that lists `EXT_CORE_VREG`, sits the core-voltage
//! status (the Console's `CoreVoltageSection`). It only reads: the writes stay
//! in the CLI, `dspi ext vreg`.

use dspi_proto::ext::{CoreVregStatus, vreg_mv};
use dspi_session::Session;
use dspi_session::ext::read_core_vreg as read_status;
use dspi_session::ext_state::ExtState;
use dspi_session::pins::{PinClaim, PinRole as ClaimRole, valid_pins};

use crate::widgets::{Action, KeyHelp, PinCell, PinRole};

use super::{Cx, PageEvent, Row, SettingsPage};

// -- WP CG20 -- Moved to the session, where the core's pin pickers use it too.
pub use dspi_session::pins::claims_from_state;

/// The core-voltage status when Settings opens: `EXT_LIST`, then the read if
/// it is listed. `None` on stock firmware, which stalls the list, and for a
/// DSPi Link role that may not use the extension channel.
pub(super) fn read_core_vreg(session: &mut Session) -> Option<CoreVregStatus> {
    session
        .with_transport(|t| ExtState::probe(t))
        .ok()?
        .core_vreg
}

/// Re-read a status the device has already shown it has. A failed read keeps
/// what we had, as [`super::SettingsData::adopt`] does, and a device without
/// the status is not asked again.
pub(super) fn poll_core_vreg(data: &mut super::SettingsData, session: &mut Session) {
    if data.core_vreg.is_some()
        && let Ok(status) = read_status(session.transport_mut())
    {
        data.core_vreg = Some(status);
    }
}

/// `1.20 V`, or the raw code when it is outside the range the firmware uses.
fn volts(code: u8) -> String {
    match vreg_mv(code) {
        Some(mv) => format!("{}.{:02} V", mv / 1000, (mv % 1000) / 10),
        None => format!("code 0x{code:02X}"),
    }
}

/// The Console's `formatUptime`: the two largest units that matter.
fn uptime(seconds: u32) -> String {
    let (d, h) = (seconds / 86_400, (seconds % 86_400) / 3_600);
    let (m, s) = ((seconds % 3_600) / 60, seconds % 60);
    if d > 0 {
        format!("{d} d {h} h")
    } else if h > 0 {
        format!("{h} h {m} min")
    } else if m > 0 {
        format!("{m} min {s} s")
    } else {
        format!("{s} s")
    }
}

/// The Core Voltage section, with the Console's rows and wording.
fn core_voltage_rows(s: &CoreVregStatus) -> Vec<Row> {
    let raised = s.flags & CoreVregStatus::FLAG_AUTO_RAISED != 0;
    let auto_off = s.flags & CoreVregStatus::FLAG_AUTO_DISABLED != 0;
    let mut rows = vec![
        Row::section("Core Voltage"),
        Row::pair(
            "Running at",
            format!(
                "{}{}",
                volts(s.live_vreg),
                if s.trial_active { " (trial)" } else { "" }
            ),
        ),
    ];
    if s.trial_active && s.trial_vreg != s.live_vreg {
        rows.push(Row::pair("Trial voltage", volts(s.trial_vreg)));
    }
    rows.push(Row::pair(
        "Stored",
        if s.stored_sel == 0 {
            "Build default".to_string()
        } else {
            format!(
                "{}{}",
                volts(s.stored_sel),
                if raised { ", raised by the device" } else { "" }
            )
        },
    ));
    rows.push(Row::pair("Build default", volts(s.default_vreg)));
    rows.push(Row::pair(
        "Automatic raising",
        if auto_off { "Off" } else { "On" },
    ));
    rows.push(Row::pair("Hangs this session", s.hang_count.to_string()));
    rows.push(Row::pair(
        "Last failed trial",
        if s.failed_trial_vreg == 0 {
            "None".to_string()
        } else {
            format!(
                "{}, hung after {}",
                volts(s.failed_trial_vreg),
                uptime(s.failed_trial_uptime_s)
            )
        },
    ));
    rows.push(Row::pair("Uptime", uptime(s.uptime_s)));
    rows.push(Row::note(
        "At its 307.2 MHz clock the device raises its stored voltage one step by itself after \
         two watchdog hangs in one power session, up to 1.25 V. To change it, use `dspi ext \
         vreg`.",
    ));
    rows.push(Row::Blank);
    rows
}

/// The kit's tint role for a session claim role.
fn tint(role: ClaimRole) -> PinRole {
    match role {
        ClaimRole::Output => PinRole::Output,
        ClaimRole::Clock => PinRole::Clock,
        ClaimRole::Input => PinRole::Input,
        ClaimRole::Control => PinRole::Control,
        ClaimRole::Utility => PinRole::Utility,
    }
}

/// The Console's section order: Outputs, Clocks, Inputs, Control, Other.
const ROLE_ORDER: [ClaimRole; 5] = [
    ClaimRole::Output,
    ClaimRole::Clock,
    ClaimRole::Input,
    ClaimRole::Control,
    ClaimRole::Utility,
];

#[derive(Debug, Default)]
pub struct OverviewPage {
    cursor: usize,
    grid_cursor: usize,
}

const KEYS: &[KeyHelp] = &[
    KeyHelp::new("← →", "Walk the pin map"),
    KeyHelp::new("↑ ↓", "Scroll"),
    KeyHelp::new("Tab", "Sidebar, page, save bar"),
    KeyHelp::new("[ ]", "Back, forward"),
    KeyHelp::new("Esc", "Leave Settings"),
];

impl OverviewPage {
    fn cells(&self, cx: &Cx<'_>) -> Vec<PinCell> {
        let claims = cx.data.pin_claims(cx.state);
        valid_pins(cx.platform())
            .into_iter()
            .map(|gpio| PinCell {
                gpio,
                owner: claims
                    .iter()
                    .find(|c| c.gpio == gpio)
                    .map(|c| (tint(c.role), c.owner.clone())),
            })
            .collect()
    }
}

impl SettingsPage for OverviewPage {
    fn rows(&self, cx: &Cx<'_>) -> Vec<Row> {
        if !cx.connected {
            return vec![
                Row::pair("No Device Connected", String::new()),
                Row::note(
                    "Pin assignments live on the device. Connect a DSPi to see which GPIOs are \
                     in use.",
                ),
            ];
        }
        let mut rows = cx
            .data
            .core_vreg
            .as_ref()
            .map(core_voltage_rows)
            .unwrap_or_default();
        let claims = cx.data.pin_claims(cx.state);
        if claims.is_empty() {
            rows.push(Row::note("No GPIOs are currently claimed."));
            return rows;
        }
        let total = valid_pins(cx.platform()).len();
        let free = total.saturating_sub(claims.len());
        rows.extend([
            Row::pair(
                format!("{} of {total} GPIOs in use", claims.len()),
                if free == 0 {
                    "none free".to_string()
                } else {
                    format!("{free} free")
                },
            ),
            Row::Grid {
                cells: self.cells(cx),
                cursor: self.grid_cursor,
            },
        ]);
        for role in ROLE_ORDER {
            let rows_for: Vec<&PinClaim> = claims.iter().filter(|c| c.role == role).collect();
            if rows_for.is_empty() {
                continue;
            }
            rows.push(Row::Blank);
            rows.push(Row::section(tint(role).name()));
            for c in rows_for {
                rows.push(Row::pair(format!("GP{}", c.gpio), c.owner.clone()));
            }
        }
        rows
    }

    fn act(&mut self, _index: usize, action: Action, cx: &Cx<'_>) -> PageEvent {
        if let Action::Selected(i) = action {
            self.grid_cursor = i;
            let cells = self.cells(cx);
            return match cells.get(i) {
                Some(PinCell {
                    gpio,
                    owner: Some((role, label)),
                }) => PageEvent::Status(format!("GP{gpio}: {label} ({})", role.name())),
                Some(PinCell { gpio, owner: None }) => {
                    PageEvent::Status(format!("GP{gpio} - available"))
                }
                None => PageEvent::Handled,
            };
        }
        PageEvent::Handled
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_cursor(&mut self, i: usize) {
        self.cursor = i;
    }

    fn keys(&self) -> &'static [KeyHelp] {
        KEYS
    }
}

#[cfg(test)]
mod tests {
    use super::super::Page;
    use super::super::tests::{data, frame, key, scratch, screen, state};
    use super::super::{AppConfig, SettingsData, SettingsScreen};
    use super::*;
    use crate::shell::{Screen, ScreenEvent};
    use crossterm::event::KeyCode;
    use dspi_session::ext::{VregOp, core_vreg_op};
    use dspi_transport::{Capture, FakeDevice, FakeOptions, Transport};

    #[test]
    fn the_overview_summarises_and_maps_every_gpio() {
        let (mut s, st) = screen(Page::Overview);
        for (w, h) in [(120u16, 40u16), (80, 24)] {
            let f = frame(&mut s, &st, w, h);
            assert_eq!(f.lines().count(), h as usize);
            assert!(f.contains("GPIOs in use"), "{w}x{h}: {f}");
            assert!(f.contains("GP0"), "the map: {w}x{h}\n{f}");
        }
        let f = frame(&mut s, &st, 120, 40);
        assert!(f.contains("free"), "{f}");
        assert!(f.contains("OUTPUTS"), "{f}");
        assert!(f.contains("CLOCKS"), "{f}");
        assert!(f.contains("INPUTS"), "{f}");
    }

    #[test]
    fn the_claims_are_the_firmwares_rules() {
        let st = state();
        let claims = claims_from_state(&st);
        let owner = |g: u8| {
            claims
                .iter()
                .find(|c| c.gpio == g)
                .map(|c| c.owner.clone())
                .unwrap_or_default()
        };
        assert_eq!(owner(6), "Output 1");
        assert_eq!(owner(10), "Subwoofer");
        assert_eq!(owner(14), "I2S BCK");
        assert_eq!(owner(15), "I2S LRCLK");
        assert_eq!(owner(13), "I2S MCK");
        assert_eq!(owner(5), "S/PDIF 1 RX");
        assert_eq!(owner(21), "S/PDIF 2 RX");
        assert_eq!(owner(12), "ADAT Output");
        assert_eq!(owner(11), "DAC Mute");
        // An unclaimed pin is unclaimed: GPIO 0 is nobody's.
        assert!(claims.iter().all(|c| c.gpio != 0));
    }

    #[test]
    fn a_claimed_pin_is_marked_in_the_map_and_named_on_the_echo_line() {
        let mut st = state();
        st.caps.features.clear();
        let cx = Cx {
            state: &st,
            data: &data(),
            config: &super::super::AppConfig::default(),
            connected: true,
            global_dirty: false,
        };
        let page = OverviewPage::default();
        let cells = page.cells(&cx);
        let cell = cells.iter().find(|c| c.gpio == 6).expect("GP6");
        assert_eq!(
            cell.owner,
            Some((PinRole::Output, "Output 1".to_string())),
            "a claimed pin carries its owner and role"
        );
        assert!(cells.iter().find(|c| c.gpio == 0).unwrap().owner.is_none());

        // And walking the grid says who holds the pin under the cursor.
        let (mut s, st) = screen(Page::Overview);
        s.handle(key(KeyCode::Tab), &st);
        let mut last = ScreenEvent::Handled;
        for _ in 0..6 {
            last = s.handle(key(KeyCode::Right), &st);
        }
        assert_eq!(
            last,
            ScreenEvent::Status("GP6: Output 1 (Outputs)".into()),
            "the echo line names the owner"
        );
    }

    #[test]
    fn a_disconnected_overview_says_where_the_pins_live() {
        let st = state();
        let mut s =
            super::super::SettingsScreen::new(&st, data(), super::super::AppConfig::default())
                .connected(false)
                .open(Page::Overview, &st);
        let f = frame(&mut s, &st, 120, 40);
        assert!(f.contains("No Device Connected"), "{f}");
        assert!(f.contains("Pin assignments live on the device."), "{f}");
    }

    // -- WP TM2 --

    /// A real session on a fake device, with Settings open on `page` and the
    /// data read the way the runner reads it.
    fn on_device(
        fake: FakeDevice,
        page: Page,
    ) -> (SettingsScreen, dspi_session::DeviceState, Session) {
        let mut t: Box<dyn Transport> = Box::new(fake);
        let caps = dspi_session::probe(t.as_mut()).unwrap();
        let mut session = Session::new(t, caps.clone()).unwrap();
        let bulk = session.snapshot().unwrap();
        let st = dspi_session::DeviceState::new(caps, bulk);
        let read = SettingsData::read(&mut session);
        let s = SettingsScreen::new(&st, read, AppConfig::default())
            .config_path(scratch("tm2"))
            .open(page, &st);
        (s, st, session)
    }

    fn status() -> CoreVregStatus {
        CoreVregStatus {
            live_vreg: 0x0D,
            stored_sel: 0,
            default_vreg: 0x0C,
            flags: 0,
            hang_count: 0,
            trial_active: false,
            trial_vreg: 0,
            failed_trial_vreg: 0,
            failed_trial_uptime_s: 0,
            uptime_s: 0,
        }
    }

    /// The `label value` rows of the section, in order.
    fn pairs(s: &CoreVregStatus) -> Vec<(String, String)> {
        core_voltage_rows(s)
            .into_iter()
            .filter_map(|r| match r {
                Row::Pair(l, v) => Some((l, v)),
                _ => None,
            })
            .collect()
    }

    fn value(s: &CoreVregStatus, label: &str) -> String {
        pairs(s)
            .into_iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("no row {label}"))
    }

    fn start_trial(session: &mut Session) {
        core_vreg_op(
            session.transport_mut(),
            VregOp::Trial(0x0E),
            Some("OVERVOLT"),
        )
        .unwrap();
    }

    #[test]
    fn tm2_the_demo_device_renders_its_core_voltage() {
        let (mut s, st, _session) = on_device(FakeDevice::demo(), Page::Overview);
        let f = frame(&mut s, &st, 120, 40);
        for want in [
            "CORE VOLTAGE",
            "Running at",
            "1.20 V",
            "Stored",
            "Build default",
            "Automatic raising",
            "Hangs this session",
            "Last failed trial",
            "None",
            "Uptime",
            "GPIOs in use",
        ] {
            assert!(f.contains(want), "{want}:\n{f}");
        }
        assert!(f.contains("dspi ext"), "where the writes are:\n{f}");
    }

    #[test]
    fn tm2_the_section_leaves_the_pin_map_reachable_on_a_small_terminal() {
        let (mut s, st, _session) = on_device(FakeDevice::demo(), Page::Overview);
        let f = frame(&mut s, &st, 80, 24);
        assert_eq!(f.lines().count(), 24);
        assert!(f.contains("CORE VOLTAGE"), "{f}");
        s.handle(key(KeyCode::Tab), &st);
        let f = frame(&mut s, &st, 80, 24);
        assert!(f.contains("GP0"), "focus brings the map into view:\n{f}");
    }

    #[test]
    fn tm2_stock_firmware_has_no_section() {
        let (mut s, st, _session) = on_device(
            FakeDevice::new(Capture::demo(), FakeOptions::default()),
            Page::Overview,
        );
        assert_eq!(s.data.core_vreg, None);
        let f = frame(&mut s, &st, 120, 40);
        assert!(!f.contains("CORE VOLTAGE"), "{f}");
        assert!(!f.contains("Running at"), "{f}");
        assert!(f.contains("GPIOs in use"), "the map is still there:\n{f}");
    }

    #[test]
    fn tm2_a_disconnected_overview_has_no_section() {
        let st = state();
        let mut d = data();
        d.core_vreg = Some(status());
        let mut s = SettingsScreen::new(&st, d, AppConfig::default())
            .config_path(scratch("tm2"))
            .connected(false)
            .open(Page::Overview, &st);
        let f = frame(&mut s, &st, 120, 40);
        assert!(!f.contains("CORE VOLTAGE"), "{f}");
        assert!(f.contains("No Device Connected"), "{f}");
    }

    #[test]
    fn tm2_the_section_shows_with_no_claimed_pins_and_offers_no_control() {
        let st = state();
        let mut d = data();
        d.core_vreg = Some(status());
        d.claims = Some(Vec::new());
        let cx = Cx {
            state: &st,
            data: &d,
            config: &AppConfig::default(),
            connected: true,
            global_dirty: false,
        };
        let rows = OverviewPage::default().rows(&cx);
        assert!(
            rows.iter()
                .any(|r| matches!(r, Row::Section(t) if t == "Core Voltage"))
        );
        assert!(
            rows.iter()
                .any(|r| matches!(r, Row::Note(t) if t.contains("No GPIOs")))
        );
        assert!(
            core_voltage_rows(&status()).iter().all(|r| !r.focusable()),
            "status only: nothing in the section can be acted on"
        );
    }

    #[test]
    fn tm2_the_rows_read_like_the_consoles() {
        let idle = status();
        assert_eq!(value(&idle, "Running at"), "1.20 V");
        assert_eq!(value(&idle, "Stored"), "Build default");
        assert_eq!(value(&idle, "Build default"), "1.15 V");
        assert_eq!(value(&idle, "Automatic raising"), "On");
        assert_eq!(value(&idle, "Hangs this session"), "0");
        assert_eq!(value(&idle, "Last failed trial"), "None");
        assert_eq!(value(&idle, "Uptime"), "0 s");
        assert!(pairs(&idle).iter().all(|(l, _)| l != "Trial voltage"));

        let busy = CoreVregStatus {
            live_vreg: 0x0E,
            stored_sel: 0x0D,
            flags: CoreVregStatus::FLAG_AUTO_RAISED | CoreVregStatus::FLAG_AUTO_DISABLED,
            hang_count: 3,
            failed_trial_vreg: 0x0C,
            failed_trial_uptime_s: 90,
            uptime_s: 3_660,
            ..status()
        };
        assert_eq!(value(&busy, "Stored"), "1.20 V, raised by the device");
        assert_eq!(value(&busy, "Automatic raising"), "Off");
        assert_eq!(value(&busy, "Hangs this session"), "3");
        assert_eq!(
            value(&busy, "Last failed trial"),
            "1.15 V, hung after 1 min 30 s"
        );
        assert_eq!(value(&busy, "Uptime"), "1 h 1 min");

        // A stored value that is not raised says nothing more.
        let stored = CoreVregStatus {
            stored_sel: 0x0D,
            ..status()
        };
        assert_eq!(value(&stored, "Stored"), "1.20 V");

        // The live voltage is the trial's: no second row.
        let same = CoreVregStatus {
            trial_active: true,
            trial_vreg: 0x0D,
            ..status()
        };
        assert_eq!(value(&same, "Running at"), "1.20 V (trial)");
        assert!(pairs(&same).iter().all(|(l, _)| l != "Trial voltage"));

        // A trial that differs from the live voltage gets its own row.
        let apart = CoreVregStatus {
            trial_active: true,
            trial_vreg: 0x0E,
            ..status()
        };
        assert_eq!(value(&apart, "Trial voltage"), "1.25 V");

        // Bytes from the device are never trusted: a code off the table shows
        // as itself, and the largest uptime formats.
        let odd = CoreVregStatus {
            live_vreg: 0xFF,
            uptime_s: u32::MAX,
            ..status()
        };
        assert_eq!(value(&odd, "Running at"), "code 0xFF");
        assert_eq!(value(&odd, "Uptime"), "49710 d 6 h");
        assert_eq!(uptime(86_400), "1 d 0 h");
    }

    #[test]
    fn tm2_the_poll_follows_a_trial_started_in_the_cli() {
        let (mut s, st, mut session) = on_device(FakeDevice::demo(), Page::Overview);
        start_trial(&mut session);
        // Not on screen until the once-a-second poll.
        assert!(!frame(&mut s, &st, 120, 40).contains("(trial)"));
        s.poll(&mut session, &st);
        let f = frame(&mut s, &st, 120, 40);
        assert!(f.contains("1.25 V (trial)"), "{f}");
    }

    #[test]
    fn tm2_only_the_overview_page_polls_and_a_failed_read_keeps_the_status() {
        // Another page: the status stays as it was opened.
        let (mut s, st, mut session) = on_device(FakeDevice::demo(), Page::About);
        let opened = s.data.core_vreg.clone();
        assert!(opened.is_some());
        start_trial(&mut session);
        s.poll(&mut session, &st);
        assert_eq!(s.data.core_vreg, opened);

        // The device leaves while the page is open: nothing panics, and the
        // last status stays readable.
        let fake = FakeDevice::demo();
        let remote = fake.remote();
        let (mut s, st, mut session) = on_device(fake, Page::Overview);
        let opened = s.data.core_vreg.clone();
        assert!(opened.is_some());
        remote.disconnect();
        s.poll(&mut session, &st);
        assert_eq!(s.data.core_vreg, opened);

        // A device without the status is not asked again.
        let mut plain = SettingsData::default();
        let mut t: Box<dyn Transport> =
            Box::new(FakeDevice::new(Capture::demo(), FakeOptions::default()));
        let caps = dspi_session::probe(t.as_mut()).unwrap();
        let mut session = Session::new(t, caps).unwrap();
        poll_core_vreg(&mut plain, &mut session);
        assert_eq!(plain.core_vreg, None);
    }
}
