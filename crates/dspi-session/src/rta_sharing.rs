//! Sharing the device's one analyser with other consoles: the rules a client
//! needs when it drives [`RtaEngine`] itself, without `dspi-core`.
//!
//! The device holds one configuration, and the last console to push one
//! drives it; every other console follows (`rta_sharing_spec.md` section 3,
//! `SpectrumAnalyser.swift`, `case following(origin:)`). An `RTA_CONFIG` event
//! from someone else, or on firmware without the event a readback that stops
//! agreeing, moves a [`Sharing`] to following. The client then feeds the engine
//! the device's configuration instead of its own, and [`Sharing::tick`] drops
//! the engine's push of it, so nothing is written over the other console.
//! [`Sharing::take_control`], or the client's own choice changing, ends that.
//!
//! This is the one home of these rules. `dspi-core` applies them too (WP CG19),
//! but polls over DSPi Link with its lock released, so a change can overtake a
//! poll. It therefore uses the two halves of [`Sharing::tick`] directly:
//! [`Sharing::begin_poll`] and [`Sharing::finish_poll`] around a poll through
//! [`Held`], with the [`Sharing`] left free meanwhile. The epoch in [`PollShare`]
//! keeps a reading taken before such a change from being applied after it.

use dspi_transport::clock::Instant;

use dspi_proto::generated::notify as nfy;
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::{RtaCaps, RtaConfig};
use dspi_transport::{DeviceDescriptor, Transport, TransportCaps, TransportKind};

use crate::notify::{Event, Notification};
use crate::rta::{Budget, Options, RtaEngine, TAP_INPUT, TAP_OUTPUT, TickReport};

/// `NOTIFY_EVT_RTA_CONFIG`, which the decoder delivers as `Event::Unknown`
/// (payload from byte 4: the applied `RtaConfig`, then the source).
const EVT_RTA_CONFIG: u8 = nfy::NOTIFY_EVT_RTA_CONFIG as u8;

/// Another console's configuration, while this one follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Followed {
    pub config: RtaConfig,
    /// The session that pushed it; 0 when the writer is unknown.
    pub origin: u16,
}

impl Followed {
    /// The device-side options the engine must poll with while following.
    pub fn options(&self) -> Options {
        Options {
            fft_order: Some(self.config.fft_order),
            avg_ms: self.config.avg_ms,
            peak_decay_db_s: self.config.peak_decay_db_s,
        }
    }
}

/// What this client knows about the other consoles on one connection.
#[derive(Debug, Clone, Default)]
pub struct Sharing {
    following: Option<Followed>,
    /// What the client last asked for, kept while nothing watches, so asking
    /// for the same again is told apart from a new choice and an event that
    /// arrives meanwhile is judged against it.
    last_own: Option<RtaConfig>,
    /// The device held `last_own` since the last push. Only then does a
    /// disagreeing readback mean another host, rather than a refusal.
    agreed: bool,
    /// Someone else chose what we want while we followed: the engine will
    /// push this, and must not, since the device already holds it.
    absorb: Option<RtaConfig>,
    // -- WP CG19 --
    /// Bumped by anything that changes ownership, so a readback taken before
    /// the change is not applied after it.
    epoch: u64,
}

// -- WP CG19 --
/// The rules for one poll, and what the poll saw. Taken from
/// [`Sharing::begin_poll`] and handed back to [`Sharing::finish_poll`], so the
/// poll itself needs no access to the [`Sharing`].
#[derive(Debug, Clone, Default)]
pub struct PollShare {
    /// Following: never write over the device's configuration.
    hold: bool,
    absorb: Option<RtaConfig>,
    epoch: u64,
    /// The configuration the device reported.
    readback: Option<RtaConfig>,
    /// A push really went out.
    pushed: bool,
    /// `absorb` was dropped instead of pushed.
    absorbed: bool,
}

impl PollShare {
    /// The device reported `config` during this poll. [`Held`] calls this for
    /// every configuration read.
    pub fn note_readback(&mut self, config: RtaConfig) {
        self.readback = Some(config);
    }
}

/// The same tap, channels and transform size. Exact: a device analysing more
/// channels than asked for rotates through them. It clamps averaging and decay
/// rather than refusing them, so those never decide.
fn same_choice(a: &RtaConfig, b: &RtaConfig) -> bool {
    a.tap == b.tap && a.channel_mask == b.channel_mask && a.fft_order == b.fft_order
}

/// Whether this device could hold `c`: what the firmware reports has passed
/// `config_valid` and been masked by `apply_config` (rta.c), so its tap and order
/// are in range and its mask is non-empty and within `tap_valid_mask`, which is
/// the tap's width in bits. Anything else in an event or a readback is damage
/// and must not reach the views.
fn followable(c: &RtaConfig, caps: &RtaCaps) -> bool {
    let width = match c.tap {
        TAP_INPUT => caps.input_channels,
        TAP_OUTPUT => caps.output_channels,
        _ => return false,
    };
    let valid = 1u32
        .checked_shl(u32::from(width.min(16)))
        .map_or(u32::MAX, |b| b.saturating_sub(1));
    (caps.order_min..=caps.order_max).contains(&c.fft_order)
        && c.channel_mask != 0
        && u32::from(c.channel_mask) & !valid == 0
}

impl Sharing {
    /// The configuration being followed; `None` while driving.
    pub fn following(&self) -> Option<Followed> {
        self.following
    }

    // -- WP CG19 --
    /// Whether the device held what was asked for since the last push.
    pub fn agreed(&self) -> bool {
        self.agreed
    }

    /// A new connection: what the other consoles did belongs to the old one,
    /// and what we last asked for does not.
    pub fn reset(&mut self) {
        *self = Self {
            last_own: self.last_own,
            epoch: self.epoch.wrapping_add(1),
            ..Self::default()
        };
    }

    /// Stop following; the engine then wants the client's own configuration
    /// and pushes it. While driving this only outdates a poll in flight.
    pub fn take_control(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.following.take().is_some() {
            self.agreed = false;
            self.absorb = None;
        }
    }

    /// Judge what the client's views ask for now; `None` while nothing is
    /// asked for. A change takes the analyser back. The same thing asked for
    /// again, or nothing asked for, does not: hiding a display and showing it
    /// again is not a choice.
    pub fn judge_own(&mut self, own: Option<RtaConfig>) {
        let Some(mine) = own else {
            return;
        };
        if self.last_own != Some(mine) {
            self.take_control();
            self.agreed = false;
            self.absorb = None;
            self.last_own = Some(mine);
        }
    }

    /// The device holds what we want: driving, with nothing to push.
    fn holds_ours(&mut self, mine: RtaConfig) {
        if self.following.take().is_some() {
            self.absorb = Some(mine);
        }
        self.agreed = true;
    }

    /// Take a notification, if it is an `RTA_CONFIG` event. `kind` and
    /// `session` are the transport's, for the echo rule; `caps` is `None`
    /// until the analyser was probed, which ignores the event. Returns
    /// whether it was one.
    pub fn note(
        &mut self,
        note: &Notification,
        kind: TransportKind,
        session: Option<u16>,
        caps: Option<&RtaCaps>,
    ) -> bool {
        let Event::Unknown { id, bytes } = &note.event else {
            return false;
        };
        if *id != EVT_RTA_CONFIG {
            return false;
        }
        let source = bytes.get(RtaConfig::SIZE).copied().unwrap_or(0);
        let own = dspi_transport::is_own_write(kind, source, note.origin, session, false);
        self.note_config(bytes, own, note.origin.unwrap_or(0), caps);
        true
    }

    // -- WP CG19 --
    /// Take the payload of an `RTA_CONFIG` event: the configuration the device
    /// just applied. `own`: the echo rule says we wrote it. `origin`: the session
    /// the hub attributes it to, 0 when unknown. `caps` as in [`Self::note`].
    /// Returns whether the event held a configuration this device could hold;
    /// otherwise nothing changed.
    pub fn note_config(
        &mut self,
        payload: &[u8],
        own: bool,
        origin: u16,
        caps: Option<&RtaCaps>,
    ) -> bool {
        let Some(caps) = caps else {
            return false;
        };
        let Some(config) = RtaConfig::decode(payload)
            .ok()
            .filter(|c| followable(c, caps))
        else {
            return false;
        };
        self.epoch = self.epoch.wrapping_add(1);
        // While released this is judged against what we last asked for, so a
        // later request follows a change made in the meantime.
        if let Some(mine) = self.last_own {
            if same_choice(&config, &mine) {
                // Our echo, or someone else choosing exactly what we want.
                self.holds_ours(mine);
            } else if !own {
                self.following = Some(Followed { config, origin });
            }
            // An echo of ours that no longer matches is an older push,
            // superseded by one still in flight.
        }
        true
    }

    /// One poll of `engine`: pushes are held back while following, and the
    /// configuration the device reports is judged afterwards. Firmware without
    /// the event shows another host only there; with the event this just gets
    /// there first, and the event then supplies the origin.
    pub fn tick(
        &mut self,
        engine: &mut RtaEngine,
        t: &mut dyn Transport,
        now: Instant,
        budget: Budget,
    ) -> TickReport {
        let mut share = self.begin_poll();
        let report = engine.tick(&mut Held::new(t, &mut share), now, budget);
        self.finish_poll(&share, engine.caps());
        report
    }

    // -- WP CG19 --
    /// What the poll about to start must do about pushes. The poll runs through
    /// [`Held`]; anything may happen to this [`Sharing`] until
    /// [`Self::finish_poll`].
    pub fn begin_poll(&self) -> PollShare {
        PollShare {
            hold: self.following.is_some(),
            absorb: self.absorb,
            epoch: self.epoch,
            ..PollShare::default()
        }
    }

    // -- WP CG19 --
    /// Judge what a finished poll saw. `caps` as in [`Self::note`].
    pub fn finish_poll(&mut self, share: &PollShare, caps: Option<&RtaCaps>) {
        if share.pushed {
            self.agreed = false;
        }
        if share.absorbed && self.absorb == share.absorb {
            self.absorb = None;
        }
        // Anything that changed ownership mid-poll outranks this reading.
        let Some(read) = share.readback.filter(|_| share.epoch == self.epoch) else {
            return;
        };
        if let Some(mine) = self.last_own {
            self.judge_readback(read, mine, caps);
        }
    }

    fn judge_readback(&mut self, read: RtaConfig, mine: RtaConfig, caps: Option<&RtaCaps>) {
        if same_choice(&read, &mine) {
            self.holds_ours(mine);
            return;
        }
        let another_host = match self.following {
            Some(f) => !same_choice(&read, &f.config),
            // It held ours and now holds something else, which is not a
            // refusal. Never agreed: the engine's own give-up count decides.
            None => self.agreed,
        };
        if another_host && caps.is_some_and(|c| followable(&read, c)) {
            self.following = Some(Followed {
                config: read,
                origin: 0,
            });
        }
    }
}

/// The transport as one poll sees it: drops the engine's push when the rules
/// say the device must keep what it holds, and notes what it reports.
pub struct Held<'a> {
    t: &'a mut dyn Transport,
    share: &'a mut PollShare,
}

impl<'a> Held<'a> {
    // -- WP CG19 --
    /// `t` as one poll with `share` should see it.
    pub fn new(t: &'a mut dyn Transport, share: &'a mut PollShare) -> Self {
        Self { t, share }
    }
}

impl Transport for Held<'_> {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        let r = self.t.control_in(opcode, value, len);
        if opcode == op::REQ_RTA_GET_CONFIG
            && let Ok(b) = &r
            && let Ok(config) = RtaConfig::decode(b)
        {
            self.share.note_readback(config);
        }
        r
    }

    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.t.control_in_upto(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        if opcode != op::REQ_RTA_SET_CONFIG {
            return self.t.control_out(opcode, value, data);
        }
        if self.share.hold {
            return Ok(());
        }
        if self.share.absorb.is_some() && RtaConfig::decode(data).ok() == self.share.absorb {
            self.share.absorbed = true;
            return Ok(());
        }
        let r = self.t.control_out(opcode, value, data);
        self.share.pushed |= r.is_ok();
        r
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.t.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.t.max_transfer()
    }

    fn caps(&self) -> TransportCaps {
        self.t.caps()
    }

    fn session_id(&self) -> Option<u16> {
        self.t.session_id()
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_tm5 {
    use std::time::Duration;

    use dspi_transport::MockTransport;
    use dspi_transport::mock::{LogHandle, Reply};

    use super::*;
    use crate::rta::Request;

    fn caps() -> RtaCaps {
        RtaCaps {
            input_channels: 8,
            output_channels: 9,
            order_min: 8,
            order_max: 10,
            order_default: 10,
            bass_bands: 14,
            max_bands: 37,
            level_zero: 243,
            ..RtaCaps::default()
        }
    }

    fn config(mask: u16) -> RtaConfig {
        RtaConfig {
            tap: TAP_OUTPUT,
            channel_mask: mask,
            fft_order: 10,
            avg_ms: 300,
            peak_decay_db_s: 12,
            flags: 0,
        }
    }

    /// The event as the decoder delivers it, attributed to `origin` on Link.
    fn event(c: &RtaConfig, origin: u16) -> Notification {
        let mut bytes = c.encode().to_vec();
        bytes.push(1);
        Notification {
            seq: 1,
            event: Event::Unknown {
                id: EVT_RTA_CONFIG,
                bytes,
            },
            lost: false,
            origin: Some(origin),
            received: Instant::now(),
        }
    }

    /// A client on session 7, asking for `mask`, with a device that answers
    /// `readbacks` in turn and then goes on with the last.
    struct Rig {
        sharing: Sharing,
        engine: RtaEngine,
        mock: MockTransport,
        log: LogHandle,
        view: crate::rta::ViewId,
        now: Instant,
    }

    impl Rig {
        fn new(mask: u16, readbacks: Vec<RtaConfig>) -> Self {
            let mock = MockTransport::new()
                .reply(
                    op::REQ_RTA_GET_CONFIG,
                    Reply::Sequence(
                        readbacks
                            .iter()
                            .map(|c| Reply::Data(c.encode().to_vec()))
                            .collect(),
                    ),
                )
                .data(op::REQ_RTA_GET_STATUS, vec![0; 24]);
            let log = mock.log_handle();
            let mut engine = RtaEngine::new();
            assert!(engine.load(caps(), vec![100; 37]));
            engine.set_options(
                Followed {
                    config: config(mask),
                    origin: 0,
                }
                .options(),
            );
            let view = engine.subscribe(Request {
                tap: TAP_OUTPUT,
                mask,
                wants_bins: false,
            });
            let mut sharing = Sharing::default();
            sharing.judge_own(Some(config(mask)));
            Self {
                sharing,
                engine,
                mock,
                log,
                view,
                now: Instant::now(),
            }
        }

        fn note(&mut self, n: &Notification) -> bool {
            let caps = caps();
            self.sharing
                .note(n, TransportKind::Link, Some(7), Some(&caps))
        }

        /// One poll, with the client feeding the engine what it should
        /// analyse, as the terminal's analyser state does.
        fn tick(&mut self, own: u16) {
            let (mask, options) = match self.sharing.following() {
                Some(f) => (f.config.channel_mask, f.options()),
                None => (
                    own,
                    Followed {
                        config: config(own),
                        origin: 0,
                    }
                    .options(),
                ),
            };
            self.engine.set_options(options);
            self.engine.update(
                self.view,
                Request {
                    tap: TAP_OUTPUT,
                    mask,
                    wants_bins: false,
                },
            );
            self.now += Duration::from_secs(1);
            self.sharing.tick(
                &mut self.engine,
                &mut self.mock,
                self.now,
                Budget::default(),
            );
        }

        fn pushes(&self) -> Vec<RtaConfig> {
            self.log
                .lock()
                .unwrap()
                .iter()
                .filter(|x| x.opcode == op::REQ_RTA_SET_CONFIG)
                .map(|x| RtaConfig::decode(&x.payload).unwrap())
                .collect()
        }
    }

    #[test]
    fn wp_tm5_our_echo_changes_nothing_and_another_session_is_followed() {
        let mut r = Rig::new(0b01, vec![config(0b01)]);
        r.tick(0b01);
        assert_eq!(r.pushes(), [config(0b01)]);
        assert!(r.note(&event(&config(0b01), 7)));
        // An older push of ours that no longer matches is not someone else.
        assert!(r.note(&event(&config(0b10), 7)));
        assert_eq!(r.sharing.following(), None);

        assert!(r.note(&event(&config(0b10), 9)));
        let f = r.sharing.following().unwrap();
        assert_eq!((f.origin, f.config.channel_mask), (9, 0b10));
        // Polls what the device analyses, and never pushes over it.
        r.tick(0b01);
        r.tick(0b01);
        assert_eq!(r.pushes(), [config(0b01)]);
        assert_eq!(r.engine.wanted().unwrap().channel_mask, 0b10);
        assert!(!r.engine.rejected());
    }

    #[test]
    fn wp_tm5_take_control_pushes_ours_again() {
        let mut r = Rig::new(0b01, vec![config(0b01)]);
        r.tick(0b01);
        r.note(&event(&config(0b10), 9));
        r.tick(0b01);
        r.sharing.take_control();
        assert_eq!(r.sharing.following(), None);
        r.tick(0b01);
        assert_eq!(r.pushes(), [config(0b01), config(0b01)]);
    }

    #[test]
    fn wp_tm5_only_a_new_choice_takes_the_analyser_back() {
        let mut r = Rig::new(0b01, vec![config(0b01)]);
        r.note(&event(&config(0b10), 9));
        // Nothing asked for, and the same again: not a choice.
        r.sharing.judge_own(None);
        r.sharing.judge_own(Some(config(0b01)));
        assert!(r.sharing.following().is_some());
        // Other channels, or other averaging, is.
        r.sharing.judge_own(Some(RtaConfig {
            avg_ms: 50,
            ..config(0b01)
        }));
        assert_eq!(r.sharing.following(), None);
    }

    #[test]
    fn wp_tm5_someone_choosing_what_we_want_ends_following_without_a_push() {
        let mut r = Rig::new(0b01, vec![config(0b01)]);
        r.tick(0b01);
        r.note(&event(&config(0b10), 9));
        r.tick(0b01);
        // Their averaging differs: our choice all the same.
        let theirs = RtaConfig {
            avg_ms: 100,
            ..config(0b01)
        };
        r.note(&event(&theirs, 9));
        assert_eq!(r.sharing.following(), None);
        r.tick(0b01);
        r.tick(0b01);
        assert_eq!(r.pushes(), [config(0b01)], "the device already holds it");
    }

    #[test]
    fn wp_tm5_a_readback_that_stops_agreeing_means_another_host() {
        let mut r = Rig::new(0b01, vec![config(0b01), config(0b10)]);
        r.tick(0b01); // the push
        r.tick(0b01); // agrees
        assert_eq!(r.sharing.following(), None);
        r.tick(0b01); // does not any more
        let f = r.sharing.following().unwrap();
        assert_eq!((f.origin, f.config.channel_mask), (0, 0b10));
    }

    #[test]
    fn wp_tm5_a_refused_push_is_not_following() {
        // The device goes on holding something else from the first read.
        let mut r = Rig::new(0b01, vec![config(0b10)]);
        for _ in 0..8 {
            r.tick(0b01);
        }
        assert_eq!(r.sharing.following(), None);
        assert!(r.engine.rejected(), "the engine's give-up count decides");
    }

    #[test]
    fn wp_tm5_what_no_device_could_send_is_ignored() {
        let mut r = Rig::new(0b01, vec![config(0b01)]);
        let bad = [
            config(0),      // no channels
            config(1 << 9), // past the outputs
            RtaConfig {
                tap: 7,
                ..config(0b10)
            },
            RtaConfig {
                fft_order: 14,
                ..config(0b10)
            },
        ];
        for c in &bad {
            assert!(r.note(&event(c, 9)), "{c:?}");
            assert_eq!(r.sharing.following(), None, "{c:?}");
        }
        // A short one, and one that is not this event at all.
        let mut short = event(&config(0b10), 9);
        short.event = Event::Unknown {
            id: EVT_RTA_CONFIG,
            bytes: vec![3, 1],
        };
        assert!(r.note(&short));
        short.event = Event::Unknown {
            id: EVT_RTA_CONFIG + 1,
            bytes: config(0b10).encode().to_vec(),
        };
        assert!(!r.note(&short));
        assert_eq!(r.sharing.following(), None);
        // Caps unknown: nothing can be judged.
        let n = event(&config(0b10), 9);
        assert!(r.sharing.note(&n, TransportKind::Link, Some(7), None));
        assert_eq!(r.sharing.following(), None);
    }

    #[test]
    fn wp_tm5_a_new_connection_forgets_the_others_and_keeps_our_choice() {
        let mut r = Rig::new(0b01, vec![config(0b01)]);
        r.note(&event(&config(0b10), 9));
        r.sharing.reset();
        assert_eq!(r.sharing.following(), None);
        // Asking for the same on the new connection is not a new choice, and
        // an event is still judged against it.
        r.note(&event(&config(0b10), 9));
        assert!(r.sharing.following().is_some());
    }
}

/// WP CG19: the rules as `dspi-core` drives them, a poll at a time with the
/// `Sharing` free meanwhile, and the check against the firmware.
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg19 {
    use super::*;

    fn caps() -> RtaCaps {
        RtaCaps {
            input_channels: 2,
            output_channels: 5,
            order_min: 8,
            order_max: 10,
            ..RtaCaps::default()
        }
    }

    fn config(mask: u16) -> RtaConfig {
        RtaConfig {
            tap: TAP_OUTPUT,
            channel_mask: mask,
            fft_order: 10,
            avg_ms: 300,
            peak_decay_db_s: 12,
            flags: 0,
        }
    }

    /// Driving 0b01 with the device agreeing.
    fn agreed() -> Sharing {
        let mut s = Sharing::default();
        s.judge_own(Some(config(0b01)));
        let mut p = s.begin_poll();
        p.note_readback(config(0b01));
        s.finish_poll(&p, Some(&caps()));
        assert!(s.agreed());
        s
    }

    fn event(s: &mut Sharing, mask: u16, origin: u16) -> bool {
        s.note_config(&config(mask).encode(), false, origin, Some(&caps()))
    }

    /// `apply_config` and `config_valid` (rta.c) on the channel mask, for a
    /// device with `width` channels at this tap: what a SET leaves in `rta.cfg`,
    /// which is what events and readbacks report; `None` when it is refused.
    fn firmware_applies(mask: u16, width: u8) -> Option<u16> {
        let valid = ((1u32 << width) - 1) as u16;
        (mask & valid != 0).then_some(mask & valid)
    }

    #[test]
    fn wp_cg19_followable_is_what_the_firmware_can_report() {
        for (input, output) in [(2, 5), (8, 9), (1, 16), (0, 3)] {
            let caps = RtaCaps {
                input_channels: input,
                output_channels: output,
                ..caps()
            };
            for (tap, width) in [(TAP_INPUT, input), (TAP_OUTPUT, output)] {
                for mask in 0..=u16::MAX {
                    let c = RtaConfig {
                        tap,
                        ..config(mask)
                    };
                    // Exactly the masks a SET can leave behind: the result of
                    // applying them is itself, and is not refused.
                    let reportable = firmware_applies(mask, width) == Some(mask);
                    assert_eq!(followable(&c, &caps), reportable, "{tap} {width} {mask:#x}");
                }
            }
        }
    }

    #[test]
    fn wp_cg19_tap_and_order_are_held_to_the_firmwares_range() {
        for tap in 0..=u8::MAX {
            let c = RtaConfig { tap, ..config(1) };
            assert_eq!(
                followable(&c, &caps()),
                tap == TAP_INPUT || tap == TAP_OUTPUT,
                "{tap}"
            );
        }
        for fft_order in 0..=u8::MAX {
            let c = RtaConfig {
                fft_order,
                ..config(1)
            };
            assert_eq!(
                followable(&c, &caps()),
                (8..=10).contains(&fft_order),
                "{fft_order}"
            );
        }
    }

    #[test]
    fn wp_cg19_a_reading_overtaken_by_a_change_is_dropped() {
        // The reading says another host moved the device to 0b10.
        let overtaken = |change: &dyn Fn(&mut Sharing)| {
            let mut s = agreed();
            let mut p = s.begin_poll();
            p.note_readback(config(0b10));
            change(&mut s);
            let before = s.following();
            s.finish_poll(&p, Some(&caps()));
            assert_eq!(s.following(), before);
        };
        overtaken(&|s| s.take_control());
        overtaken(&|s| s.judge_own(Some(config(0b100))));
        overtaken(&|s| s.reset());
        overtaken(&|s| assert!(event(s, 0b100, 9)));
        overtaken(&|s| assert!(event(s, 0b01, 9)));

        // Nothing happened meanwhile: it counts.
        let mut s = agreed();
        let mut p = s.begin_poll();
        p.note_readback(config(0b10));
        s.finish_poll(&p, Some(&caps()));
        assert_eq!(s.following().map(|f| f.origin), Some(0));
    }

    #[test]
    fn wp_cg19_what_changes_nothing_does_not_outdate_a_reading() {
        let mut s = agreed();
        let mut p = s.begin_poll();
        p.note_readback(config(0b10));
        // The same choice again, nothing asked for, and a damaged event.
        s.judge_own(Some(config(0b01)));
        s.judge_own(None);
        assert!(!event(&mut s, 0, 9));
        assert!(!s.note_config(&[3, 1], false, 9, Some(&caps())));
        s.finish_poll(&p, Some(&caps()));
        assert!(s.following().is_some());
    }

    #[test]
    fn wp_cg19_a_reading_is_judged_after_a_release_in_flight() {
        // Released while the poll ran: the device was still seen to have moved,
        // as an event at that moment would have shown.
        let mut s = agreed();
        let mut p = s.begin_poll();
        p.note_readback(config(0b10));
        s.judge_own(None);
        s.finish_poll(&p, Some(&caps()));
        assert_eq!(s.following().unwrap().config.channel_mask, 0b10);
    }

    #[test]
    fn wp_cg19_only_the_absorb_the_poll_acted_on_is_cleared() {
        let mut s = agreed();
        assert!(event(&mut s, 0b10, 9));
        assert!(event(&mut s, 0b01, 9)); // absorb = 0b01
        let mut p = s.begin_poll();
        p.absorbed = true;
        // A new choice, followed by someone choosing it, while the poll ran.
        let next = config(0b100);
        s.judge_own(Some(next));
        assert!(event(&mut s, 0b10, 9));
        assert!(event(&mut s, 0b100, 9));
        s.finish_poll(&p, Some(&caps()));
        assert_eq!(s.absorb, Some(next));
        // The poll that really dropped it clears it.
        let mut p = s.begin_poll();
        p.absorbed = true;
        s.finish_poll(&p, Some(&caps()));
        assert_eq!(s.absorb, None);
    }

    #[test]
    fn wp_cg19_unknown_caps_decide_nothing() {
        let mut s = agreed();
        assert!(!s.note_config(&config(0b10).encode(), false, 9, None));
        let mut p = s.begin_poll();
        p.note_readback(config(0b10));
        s.finish_poll(&p, None);
        assert_eq!(s.following(), None);
    }
}
