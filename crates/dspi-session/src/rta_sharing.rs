//! Sharing the device's one analyser with other consoles: the rules a client
//! needs when it drives [`RtaEngine`] itself, without `dspi-core`.
//!
//! The device holds one configuration, and the last console to push one
//! drives it; every other console follows (`rta_sharing_spec.md` section 3,
//! `SpectrumAnalyser.swift`, `case following(origin:)`). An `RTA_CONFIG` event
//! from someone else, or on firmware without the event a readback that stops
//! agreeing, moves a [`Sharing`] to following (but see "the last choice made
//! wins" below). The client then feeds the engine
//! the device's configuration instead of its own, and [`Sharing::tick`] drops
//! the engine's push of it, so nothing is written over the other console.
//! [`Sharing::take_control`], or the client's own choice changing, ends that.
//!
//! The last choice made wins (owner, 2026-11-14; WP ANLAST). Which push is
//! later is the order the device applied them in, which is the order of the
//! `RTA_CONFIG` events: the hub forwards every console's SET to the one device
//! and relays each event to every session under one lock, so every console
//! drains the same sequence. No clock is compared. A new choice of ours is
//! therefore pending until its own echo comes through that sequence: another
//! console's event drained before it was applied before ours, and does not make
//! us follow even though it is drained after our choice. One drained after our
//! echo does. Two consoles choosing at once thus agree on the winner: the one
//! whose push the device applied last. (Before ANLAST any event drained after
//! our choice made us follow, so a console could follow another straight after
//! choosing while its own push was held back.) Without the event (older
//! firmware) nothing is pending in effect: readbacks decide as before.
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
    // -- WP ANLAST --
    /// A choice of ours whose echo has not come through the event sequence yet.
    /// Never set while following.
    pending: Option<Pending>,
    // -- WP CORENITS --
    /// Our pushes whose echo has not been drained yet, oldest first, at most
    /// [`OWN_PUSHES_MAX`]. An event matching one is ours even when the hub
    /// attributes it to nobody (ANLAST N1), and one the device is known to
    /// have left since is stale and ignored (N2).
    own_pushes: Vec<OwnPush>,
}

// -- WP CORENITS --
/// A push of ours not yet echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OwnPush {
    config: RtaConfig,
    /// A readback showed the device holding another console's configuration
    /// after this went out: its echo, when drained, is older than that.
    left: bool,
}

// -- WP CORENITS --
/// Unechoed pushes kept; only firmware without the event (no echo ever) or
/// lost echoes fill it, and the oldest goes first.
const OWN_PUSHES_MAX: usize = 4;

// -- WP ANLAST --
/// A choice of ours the device has not been seen to apply yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Pending {
    /// Our push of it went out.
    pushed: bool,
    // -- WP ANLASTF -- F1
    /// The engine tried to push it, whether the device or hub took it or not
    /// (a STALL, `LOCKED`, `DENIED`): the engine's give-up count is ours.
    attempted: bool,
    /// The latest configuration another console pushed meanwhile, or the one
    /// followed when the choice was made. The device applied it before ours,
    /// so it counts only if ours never is: our push refused, or the choice
    /// released before it was pushed.
    earlier: Option<Followed>,
    // -- WP ANLASTQ --
    /// Made by [`Sharing::take_control`], the person's explicit "stop
    /// following", not by a change of what the views ask for. With nothing
    /// of ours to push (nothing asked for), it ends in driving nothing rather
    /// than in following again; a refusal still gives way to `earlier`.
    explicit: bool,
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
    // -- WP ANLAST --
    /// What really went out, when a push did.
    pushed_config: Option<RtaConfig>,
    // -- WP ANLASTF --
    /// What the engine tried to push, accepted or not (F1).
    attempted_config: Option<RtaConfig>,
    /// The engine read the bands: it was watching, so a poll without a push
    /// means it already agrees with the device (F2).
    polled: bool,
}

impl PollShare {
    /// The device reported `config` during this poll. [`Held`] calls this for
    /// every configuration read.
    pub fn note_readback(&mut self, config: RtaConfig) {
        self.readback = Some(config);
    }
}

// -- WP ANLASTF --
/// Everything the engine compares before it pushes (`RtaEngine::agrees`):
/// equal, and there is nothing to push.
fn same_push(a: &RtaConfig, b: &RtaConfig) -> bool {
    same_choice(a, b) && a.avg_ms == b.avg_ms && a.peak_decay_db_s == b.peak_decay_db_s
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
    /// and pushes it, which is a choice made now (pending until its echo).
    /// While driving this only outdates a poll in flight.
    pub fn take_control(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if let Some(f) = self.following.take() {
            self.agreed = false;
            self.absorb = None;
            // -- WP ANLAST -- / ANLASTF F1: if ours never lands, the device
            // still holds the one followed.
            self.pending = Some(Pending {
                earlier: Some(f),
                explicit: true, // -- WP ANLASTQ --
                ..Pending::default()
            });
        }
    }

    // -- WP ANLAST --
    /// Whether a choice of ours is waiting for its echo. While it waits, this
    /// console drives: other consoles' events drained meanwhile are older.
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    // -- WP ANLAST --
    /// Our pending choice will never be applied: follow whoever pushed last
    /// meanwhile, if anyone did.
    fn abandon_pending(&mut self) {
        if let Some(earlier) = self.pending.take().and_then(|p| p.earlier) {
            // A change of ownership: outdates a reading in flight.
            self.epoch = self.epoch.wrapping_add(1);
            self.following = Some(earlier);
            self.agreed = false;
            self.absorb = None;
        }
    }

    // -- WP ANLAST --
    /// The engine gave up on what we push (the device keeps refusing it). Call
    /// after each poll with `RtaEngine::rejected`; [`Self::tick`] does. A
    /// choice whose push went out and was refused is not applied, so another
    /// console's push seen meanwhile drives after all.
    ///
    /// WP ANLASTF F1: a push the device STALLs or the hub refuses (`LOCKED`,
    /// `DENIED`) counts as pushed and refused, so this holds for it too.
    pub fn engine_gave_up(&mut self, rejected: bool) {
        if rejected && self.pending.is_some_and(|p| p.attempted) {
            self.abandon_pending();
        }
    }

    /// Judge what the client's views ask for now; `None` while nothing is
    /// asked for. A change takes the analyser back. The same thing asked for
    /// again, or nothing asked for, does not: hiding a display and showing it
    /// again is not a choice.
    ///
    /// A choice is pending until its echo (WP ANLAST). Released before its
    /// push went out, it never reached the device, so another console's push
    /// seen meanwhile is followed after all.
    pub fn judge_own(&mut self, own: Option<RtaConfig>) {
        let Some(mine) = own else {
            // -- WP ANLAST --
            if self.pending.is_some_and(|p| !p.pushed) {
                // -- WP ANLASTQ -- the person took control with nothing chosen
                // (a side switch to an empty side, Clear): as before ANLAST,
                // that stops following; the next push of anyone's is followed.
                if self.pending.is_some_and(|p| p.explicit) {
                    self.pending = None;
                } else {
                    self.abandon_pending();
                }
            }
            return;
        };
        if self.last_own != Some(mine) {
            // -- WP ANLASTF --
            let before = self.pending.take();
            let was_following = self.following;
            self.take_control();
            self.agreed = false;
            self.absorb = None;
            self.last_own = Some(mine);
            if was_following.is_some_and(|f| same_push(&f.config, &mine)) {
                // F2: what the device holds already. Made now and confirmed at
                // once: driving, and a later push makes us follow.
                self.pending = None;
                self.agreed = true;
                return;
            }
            // F3: a choice changed again before its push keeps what was seen
            // meanwhile; once a push of ours went out, what came before is older.
            let earlier = match before {
                Some(p) if p.pushed => None,
                Some(p) => p.earlier,
                None => was_following,
            };
            self.pending = Some(Pending {
                earlier,
                ..Pending::default()
            }); // -- WP ANLAST --
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
        // -- WP CORENITS -- an echo of a push of ours: the echo rule says so,
        // or the hub attributes it to nobody (its window passed) and we sent
        // it. One the device has left since is stale and changes nothing (N2);
        // one attributed to nobody is still ours (N1).
        let echoed = if own || origin == 0 {
            self.take_own_push(&config)
        } else {
            None
        };
        if echoed == Some(true) {
            return true;
        }
        let own = own || echoed.is_some();
        self.epoch = self.epoch.wrapping_add(1);
        // While released this is judged against what we last asked for, so a
        // later request follows a change made in the meantime.
        if let Some(mine) = self.last_own {
            if same_choice(&config, &mine) {
                // Our echo, or someone else choosing exactly what we want:
                // from here on, the device applied ours (WP ANLAST).
                self.pending = None;
                self.holds_ours(mine);
            } else if !own {
                let theirs = Followed { config, origin };
                match &mut self.pending {
                    // -- WP ANLAST -- applied before our pending choice: older.
                    Some(p) => p.earlier = Some(theirs),
                    None => self.following = Some(theirs),
                }
            } else if let Some(p) = &mut self.pending {
                // -- WP ANLASTF -- an older push of ours was applied: what
                // another console pushed before it is older still.
                p.earlier = None;
            }
            // An echo of ours that no longer matches is an older push,
            // superseded by one still in flight.
        }
        true
    }

    // -- WP CORENITS --
    /// Drop the oldest unechoed push matching `config`, with every older one
    /// (events come in push order, so their echoes are gone); `Some(left)` if
    /// there was one.
    fn take_own_push(&mut self, config: &RtaConfig) -> Option<bool> {
        let at = self
            .own_pushes
            .iter()
            .position(|p| same_choice(&p.config, config))?;
        let left = self.own_pushes.get(at).map(|p| p.left);
        self.own_pushes.drain(..=at);
        left
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
        self.engine_gave_up(engine.rejected()); // -- WP ANLAST --
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
        // -- WP CORENITS -- remember it until its echo. A config pushed again
        // replaces its older entry: an echo cannot tell the two apart.
        if let Some(sent) = share.pushed_config {
            self.own_pushes.retain(|p| !same_choice(&p.config, &sent));
            if self.own_pushes.len() >= OWN_PUSHES_MAX {
                self.own_pushes.remove(0);
            }
            self.own_pushes.push(OwnPush {
                config: sent,
                left: false,
            });
        }
        // -- WP ANLAST -- our pending choice really went out.
        if let (Some(p), Some(sent), Some(mine)) =
            (&mut self.pending, share.pushed_config, self.last_own)
            && same_choice(&sent, &mine)
        {
            p.pushed = true;
        }
        // -- WP ANLASTF -- F1: or was tried, and refused.
        if let (Some(p), Some(tried), Some(mine)) =
            (&mut self.pending, share.attempted_config, self.last_own)
            && same_choice(&tried, &mine)
        {
            p.attempted = true;
        }
        // -- WP ANLASTF -- F2: the engine watched and had nothing to push: it
        // already agrees with the device, so no echo will come. Confirmed now;
        // a push seen since the choice is later than it.
        if share.polled
            && share.attempted_config.is_none()
            && !share.absorbed
            && share.epoch == self.epoch
            && let (Some(p), Some(mine)) = (self.pending, self.last_own)
            && !p.attempted
        {
            self.pending = None;
            match p.earlier.filter(|e| !same_choice(&e.config, &mine)) {
                Some(e) => {
                    self.epoch = self.epoch.wrapping_add(1);
                    self.following = Some(e);
                    self.agreed = false;
                }
                None => self.agreed = true,
            }
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
            // The device moved away from ours after holding it, or from the
            // one followed: a push later than ours either way (WP ANLAST).
            // -- WP ANLASTF -- F4: the event seen meanwhile names who.
            let origin = self
                .pending
                .take()
                .and_then(|p| p.earlier)
                .filter(|e| same_choice(&e.config, &read))
                .map_or(0, |e| e.origin);
            // -- WP CORENITS -- N2: the device left every push of ours made
            // before this reading; their echoes still queued are stale.
            for p in &mut self.own_pushes {
                p.left = true;
            }
            self.following = Some(Followed {
                config: read,
                origin,
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
        self.share.polled |= opcode == op::REQ_RTA_GET_BANDS; // -- WP ANLASTF --
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
        // -- WP ANLASTF --
        self.share.polled |= opcode == op::REQ_RTA_GET_BANDS_ALL || opcode == op::REQ_RTA_GET_BANDS;
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
        // -- WP ANLASTF -- F1: tried, whatever the answer.
        self.share.attempted_config = RtaConfig::decode(data).ok();
        let r = self.t.control_out(opcode, value, data);
        self.share.pushed |= r.is_ok();
        // -- WP ANLAST --
        if r.is_ok() {
            self.share.pushed_config = RtaConfig::decode(data).ok();
        }
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

    // -- WP PGCORE -- the paged calls pass: the engine's own pushes are
    // 1.1.6-shaped (`REQ_RTA_SET_CONFIG`, held above), planned below this
    // by the generation wrapper; a paged call here is someone's raw one.
    fn carries_pages(&self) -> bool {
        self.t.carries_pages()
    }

    fn control_in_paged(
        &mut self,
        id: dspi_transport::CmdId,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.t.control_in_paged(id, value, max_len)
    }

    fn control_out_paged(
        &mut self,
        id: dspi_transport::CmdId,
        value: u16,
        data: &[u8],
    ) -> dspi_transport::Result<()> {
        self.t.control_out_paged(id, value, data)
    }

    fn control_in_batch_paged(
        &mut self,
        reads: &[dspi_transport::PagedRead],
    ) -> Vec<dspi_transport::Result<Vec<u8>>> {
        self.t.control_in_batch_paged(reads)
    }

    fn generation(&self) -> Option<dspi_transport::Generation> {
        self.t.generation()
    }

    fn paged_discovery(&self) -> Option<dspi_transport::paged::Discovery> {
        self.t.paged_discovery()
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

        // -- WP ANLAST --
        /// Our own push of `mask` comes back through the event sequence.
        fn echo(&mut self, mask: u16) {
            assert!(self.note(&event(&config(mask), 7)));
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
        r.echo(0b01); // WP ANLAST: 9's push comes after ours
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
        r.tick(0b01);
        r.echo(0b01); // WP ANLAST: 9's push comes after ours
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
        r.echo(0b01); // WP ANLAST: 9's push comes after ours
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
        r.tick(0b01);
        r.echo(0b01); // WP ANLAST: 9's push comes after ours
        r.note(&event(&config(0b10), 9));
        assert!(r.sharing.following().is_some());
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

    /// Driving 0b01 with the device agreeing. WP ANLAST: our push's echo came
    /// through too, so the choice is no longer pending and another console's
    /// event after this is a later push.
    fn agreed() -> Sharing {
        let mut s = Sharing::default();
        s.judge_own(Some(config(0b01)));
        assert!(s.note_config(&config(0b01).encode(), true, 7, Some(&caps())));
        assert!(!s.pending());
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
        // WP ANLAST: its echo, so 9's push after it is followed.
        assert!(s.note_config(&next.encode(), true, 7, Some(&caps())));
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

/// WP ANLAST: the last choice made wins, ordered by the device's event
/// sequence (which the hub relays to every console in one order), not by when
/// each console drains it. Before ANLAST, any event drained after our choice
/// made us follow, even one for a push the device applied before ours.
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_anlast {
    use super::*;

    fn caps() -> RtaCaps {
        RtaCaps {
            input_channels: 2,
            output_channels: 16,
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

    /// One poll whose push of `c` went out.
    fn pushed(s: &mut Sharing, c: RtaConfig) {
        let mut p = s.begin_poll();
        assert!(!p.hold, "a pending choice is never held back");
        p.pushed = true;
        p.pushed_config = Some(c);
        p.attempted_config = Some(c); // -- WP ANLASTF -- as `Held` records it
        s.finish_poll(&p, Some(&caps()));
    }

    /// The device's event for `c`, as console `me` hears it, pushed by `by`.
    fn heard(s: &mut Sharing, c: RtaConfig, by: u16, me: u16) {
        assert!(s.note_config(&c.encode(), by == me, by, Some(&caps())));
    }

    /// Console B (session 9), driving 0b01 with its echo seen.
    fn driving() -> Sharing {
        let mut s = Sharing::default();
        s.judge_own(Some(config(0b01)));
        pushed(&mut s, config(0b01));
        heard(&mut s, config(0b01), 9, 9);
        assert!(!s.pending());
        assert_eq!(s.following(), None);
        s
    }

    #[test]
    fn wp_anlast_a_choice_made_while_an_earlier_push_is_unread_drives() {
        // The An34 race: A (session 3) pushed 0b10, and its event still waits
        // in B's queue when B chooses 0b100.
        let mut b = driving();
        b.judge_own(Some(config(0b100)));
        heard(&mut b, config(0b10), 3, 9);
        // Old rule: B followed A here and its push was held back.
        assert_eq!(b.following(), None, "A's push was applied before ours");
        assert!(b.pending());
        pushed(&mut b, config(0b100));
        heard(&mut b, config(0b100), 9, 9);
        assert!(!b.pending());
        assert_eq!(b.following(), None, "B drives");
    }

    #[test]
    fn wp_anlast_a_push_after_our_choice_is_followed() {
        let mut b = driving();
        b.judge_own(Some(config(0b100)));
        pushed(&mut b, config(0b100));
        heard(&mut b, config(0b100), 9, 9);
        // A pushes after B's echo: later than B's choice on the device.
        heard(&mut b, config(0b10), 3, 9);
        let f = b.following().unwrap();
        assert_eq!((f.origin, f.config.channel_mask), (3, 0b10));
    }

    #[test]
    fn wp_anlast_take_control_is_a_choice_too() {
        let mut b = driving();
        heard(&mut b, config(0b10), 3, 9);
        assert!(b.following().is_some());
        b.take_control();
        assert!(b.pending());
        // A's next push was applied before ours: B still drives.
        heard(&mut b, config(0b1000), 3, 9);
        assert_eq!(b.following(), None);
        pushed(&mut b, config(0b01));
        heard(&mut b, config(0b01), 9, 9);
        assert_eq!(b.following(), None);
        // And one after it is followed.
        heard(&mut b, config(0b10), 3, 9);
        assert_eq!(b.following().unwrap().origin, 3);
    }

    #[test]
    fn wp_anlast_a_choice_released_before_its_push_follows_the_push_seen_meanwhile() {
        let mut b = driving();
        b.judge_own(Some(config(0b100)));
        heard(&mut b, config(0b10), 3, 9);
        assert_eq!(b.following(), None);
        // Nothing asked for any more: our choice never reached the device.
        b.judge_own(None);
        assert!(!b.pending());
        assert_eq!(b.following().unwrap().config.channel_mask, 0b10);
    }

    #[test]
    fn wp_anlast_a_released_choice_whose_push_went_out_waits_for_its_echo() {
        let mut b = driving();
        b.judge_own(Some(config(0b100)));
        pushed(&mut b, config(0b100));
        heard(&mut b, config(0b10), 3, 9);
        b.judge_own(None);
        assert_eq!(b.following(), None, "ours is on its way to the device");
        heard(&mut b, config(0b100), 9, 9);
        assert_eq!(b.following(), None);
    }

    #[test]
    fn wp_anlast_a_refused_choice_follows_the_push_seen_meanwhile() {
        let mut b = driving();
        b.judge_own(Some(config(0b100)));
        heard(&mut b, config(0b10), 3, 9);
        // The engine's count from the previous choice decides nothing.
        b.engine_gave_up(true);
        assert!(b.pending());
        pushed(&mut b, config(0b100));
        b.engine_gave_up(false);
        assert!(b.pending());
        // The device keeps refusing ours: no echo will ever come.
        b.engine_gave_up(true);
        assert!(!b.pending());
        assert_eq!(b.following().unwrap().origin, 3);
    }

    #[test]
    fn wp_anlast_a_lost_echo_still_sees_a_later_push_by_readback() {
        let mut b = driving();
        b.judge_own(Some(config(0b100)));
        pushed(&mut b, config(0b100));
        // The echo is lost; the device is read to hold ours, then A's.
        let mut p = b.begin_poll();
        p.note_readback(config(0b100));
        b.finish_poll(&p, Some(&caps()));
        assert_eq!(b.following(), None);
        let mut p = b.begin_poll();
        p.note_readback(config(0b10));
        b.finish_poll(&p, Some(&caps()));
        assert!(!b.pending());
        assert_eq!(b.following().unwrap().config.channel_mask, 0b10);
        // A's next event updates what is followed.
        heard(&mut b, config(0b1000), 3, 9);
        assert_eq!(b.following().unwrap().origin, 3);
    }

    #[test]
    fn wp_anlast_two_consoles_choosing_at_once_converge() {
        for a_first in [true, false] {
            let (mut a, mut b) = (Sharing::default(), Sharing::default());
            a.judge_own(Some(config(0b01)));
            b.judge_own(Some(config(0b10)));
            pushed(&mut a, config(0b01));
            pushed(&mut b, config(0b10));
            // The device applies them in the hub's order; both hear that order.
            let stream = if a_first {
                [(config(0b01), 3), (config(0b10), 9)]
            } else {
                [(config(0b10), 9), (config(0b01), 3)]
            };
            for (c, by) in stream {
                heard(&mut a, c, by, 3);
                heard(&mut b, c, by, 9);
            }
            let (winner, loser) = if a_first { (&b, &a) } else { (&a, &b) };
            assert_eq!(winner.following(), None, "{a_first}");
            let f = loser.following().unwrap();
            assert_eq!(f.config, stream[1].0, "{a_first}");
            assert_eq!(f.origin, stream[1].1, "{a_first}");
        }
    }

    /// A model of N consoles on one device: the device appends each push's
    /// event to one sequence, every console drains it in that order at its own
    /// pace, and chooses, pushes or drains in any interleaving.
    struct World {
        consoles: Vec<(u16, Sharing, Option<RtaConfig>, usize)>,
        stream: Vec<(RtaConfig, u16)>,
    }

    impl World {
        fn new(n: u16) -> Self {
            Self {
                consoles: (1..=n)
                    .map(|id| (id, Sharing::default(), None, 0))
                    .collect(),
                stream: Vec::new(),
            }
        }

        fn choose(&mut self, i: usize, mask: u16) {
            let (_, s, own, _) = &mut self.consoles[i];
            *own = Some(config(mask));
            s.judge_own(*own);
        }

        /// The engine's push: only when not following and our choice is
        /// pending unpushed (absorbed otherwise).
        fn push(&mut self, i: usize) -> bool {
            let (id, s, own, _) = &mut self.consoles[i];
            let Some(c) = *own else { return false };
            let due = s.following().is_none() && s.pending.is_some_and(|p| !p.pushed);
            if due {
                pushed(s, c);
                self.stream.push((c, *id));
            }
            due
        }

        fn drain(&mut self, i: usize) -> bool {
            let (id, s, _, at) = &mut self.consoles[i];
            let Some(&(c, by)) = self.stream.get(*at) else {
                return false;
            };
            *at += 1;
            heard(s, c, by, *id);
            true
        }

        fn settle(&mut self) {
            loop {
                let mut moved = false;
                for i in 0..self.consoles.len() {
                    while self.drain(i) {
                        moved = true;
                    }
                    moved |= self.push(i);
                }
                if !moved {
                    return;
                }
            }
        }

        /// Exactly the last pusher drives, and every other console follows it.
        fn check(&self, seed: u64) {
            let Some(&(last, by)) = self.stream.last() else {
                return;
            };
            for (id, s, own, _) in &self.consoles {
                if own.is_none() {
                    // Never asked for anything: nothing to judge against.
                    continue;
                }
                if *id == by {
                    assert_eq!(s.following(), None, "seed {seed}: {id} drives");
                } else {
                    let f = s.following();
                    assert_eq!(
                        f.map(|f| (f.config, f.origin)),
                        Some((last, by)),
                        "seed {seed}: {id} follows {by}"
                    );
                }
                assert!(!s.pending(), "seed {seed}: {id}");
            }
        }
    }

    #[test]
    fn wp_anlast_any_interleaving_converges_on_the_last_push() {
        for seed in 0..2000u64 {
            let mut rng = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let mut next = |n: u64| {
                rng = rng
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (rng >> 33) % n
            };
            let n = 2 + next(2) as usize;
            let mut w = World::new(n as u16);
            let mut mask = 0u16;
            for _ in 0..(4 + next(20)) {
                let i = next(n as u64) as usize;
                match next(3) {
                    0 => {
                        // A unique mask per choice, so nobody chooses another's.
                        mask += 1;
                        w.choose(i, mask);
                    }
                    1 => {
                        w.push(i);
                    }
                    _ => {
                        w.drain(i);
                    }
                }
            }
            w.settle();
            w.check(seed);
        }
    }
}

/// WP ANLASTF: the review's probes (review/anlast-probes a8cbc4b6), kept as
/// tests, with the fixups' own: each asserts what the rule promises on a way
/// out of a pending choice (F1-F5), with the real engine where it decides.
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_anlastf {
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

    /// Session 7's console: engine + sharing over a mock, as the Terminal runs them.
    struct Rig {
        sharing: Sharing,
        engine: RtaEngine,
        mock: MockTransport,
        log: LogHandle,
        view: crate::rta::ViewId,
        now: Instant,
    }

    impl Rig {
        fn new(mask: u16, readbacks: Vec<RtaConfig>, set: Reply) -> Self {
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
                .reply(op::REQ_RTA_SET_CONFIG, set)
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

        /// One poll as the client runs it: judge our choice, feed the engine.
        fn tick(&mut self, own: u16) {
            self.sharing.judge_own(Some(config(own)));
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

        fn pushes(&self) -> Vec<u16> {
            self.log
                .lock()
                .unwrap()
                .iter()
                .filter(|x| x.opcode == op::REQ_RTA_SET_CONFIG)
                .map(|x| RtaConfig::decode(&x.payload).unwrap().channel_mask)
                .collect()
        }

        fn state(&self) -> String {
            format!(
                "pending={} following={:?} rejected={} pushes={:?}",
                self.sharing.pending(),
                self.sharing
                    .following()
                    .map(|f| (f.origin, f.config.channel_mask)),
                self.engine.rejected(),
                self.pushes()
            )
        }
    }

    /// R1: the firmware STALLs an invalid SET (vendor_commands.c:2128, config.h:195),
    /// and the hub answers LOCKED/DENIED the same way: the push is an Err. The engine
    /// gives up after three; the rule says we then follow the push seen meanwhile.
    #[test]
    fn wp_anlastf_r1_a_push_the_device_stalls_gives_way_to_the_push_seen_meanwhile() {
        let mut r = Rig::new(0b01, vec![config(0b01), config(0b10)], Reply::Stall);
        // Driving 0b01 with its echo seen (the push itself is not the point here).
        assert!(r.note(&event(&config(0b01), 7)));
        assert!(!r.sharing.pending());
        // Our new choice; its push STALLs. Console 9's push lands meanwhile.
        r.tick(0b100);
        assert_eq!(r.pushes(), [0b100], "{}", r.state());
        assert!(r.sharing.pending(), "{}", r.state());
        assert!(r.note(&event(&config(0b10), 9)));
        assert_eq!(
            r.sharing.following(),
            None,
            "9's push was applied before ours"
        );
        for _ in 0..12 {
            r.tick(0b100);
            if r.engine.rejected() {
                break;
            }
        }
        assert!(r.engine.rejected(), "the engine gave up: {}", r.state());
        assert!(r.sharing.following().is_some(), "STUCK: {}", r.state());
    }

    /// R1b: after the stall, console 9 keeps pushing: we never follow any of it.
    #[test]
    fn wp_anlastf_r1b_after_a_stalled_push_later_pushes_are_followed() {
        let mut r = Rig::new(0b01, vec![config(0b01), config(0b10)], Reply::Stall);
        assert!(r.note(&event(&config(0b01), 7)));
        r.tick(0b100);
        for _ in 0..12 {
            r.tick(0b100);
        }
        assert!(r.engine.rejected(), "{}", r.state());
        for m in [0b10u16, 0b1000, 0b11] {
            assert!(r.note(&event(&config(m), 9)));
            r.tick(0b100);
        }
        assert!(
            r.sharing.following().is_some(),
            "STUCK after 3 later pushes by 9: {}",
            r.state()
        );
    }

    /// R2 (F2, decided: confirmed at once): a choice identical to what the device holds (we follow 9's 0b10 and now
    /// choose 0b10 ourselves) makes no SET, so no echo ever comes. 9 then chooses
    /// 0b1000, later than our choice: the last choice made is 9's.
    #[test]
    fn wp_anlastf_r2_a_choice_the_device_already_holds_is_confirmed_at_once() {
        let mut r = Rig::new(
            0b01,
            // ticks: 1 push (no read); 2 swallowed push (no read); 3 read 0b10
            // (confirmed); 4 our identical choice, read 0b1000 (9's later push).
            vec![config(0b10), config(0b1000)],
            Reply::Data(vec![]),
        );
        r.tick(0b01);
        assert!(r.note(&event(&config(0b01), 7)));
        assert!(r.note(&event(&config(0b10), 9)));
        assert_eq!(r.sharing.following().unwrap().origin, 9);
        r.tick(0b01);
        r.tick(0b01);
        assert_eq!(
            r.pushes(),
            [0b01],
            "nothing pushed while following: {}",
            r.state()
        );
        assert_eq!(r.engine.wanted().unwrap().channel_mask, 0b10);
        // We choose exactly what the device holds: a choice, no SET needed.
        r.tick(0b10);
        assert_eq!(
            r.pushes(),
            [0b01],
            "no SET: the device already holds 0b10: {}",
            r.state()
        );
        // WP ANLASTF F2 (decided): made now and confirmed at once, nothing pending.
        // This tick's readback already shows 9's later 0b1000, so we may follow it.
        assert!(!r.sharing.pending(), "{}", r.state());
        // 9 chose 0b1000 later than we chose 0b10 (its event, and its readback above).
        assert!(r.note(&event(&config(0b1000), 9)));
        let after_event = r.state();
        r.tick(0b10);
        r.tick(0b10);
        assert!(
            r.pushes() == [0b01] && r.sharing.following().is_some(),
            "9's later choice overridden. after 9's event: [{after_event}]; two ticks later: [{}]",
            r.state()
        );
    }

    /// R3: released before its push went out, a choice gives way to the push seen
    /// meanwhile (ANLAST). Also when the choice was changed once before release.
    #[test]
    fn wp_anlastf_r3_a_rechosen_then_released_choice_still_follows_the_push_seen_meanwhile() {
        let mut s = Sharing::default();
        s.judge_own(Some(config(0b01)));
        assert!(s.note_config(&config(0b01).encode(), true, 7, Some(&caps())));
        assert!(!s.pending());
        s.judge_own(Some(config(0b100)));
        assert!(s.note_config(&config(0b10).encode(), false, 9, Some(&caps())));
        assert_eq!(s.following(), None);
        s.judge_own(Some(config(0b1000))); // changed again, still unpushed
        s.judge_own(None); // and released
        assert_eq!(
            s.following().map(|f| (f.origin, f.config.channel_mask)),
            Some((9, 0b10)),
            "9's push, seen meanwhile, is what the device holds; pending={}",
            s.pending()
        );
    }

    /// R3b: the same without the re-choice passes (the WP's own case).
    #[test]
    fn wp_anlastf_r3b_released_without_a_rechoice_follows() {
        let mut s = Sharing::default();
        s.judge_own(Some(config(0b01)));
        assert!(s.note_config(&config(0b01).encode(), true, 7, Some(&caps())));
        s.judge_own(Some(config(0b100)));
        assert!(s.note_config(&config(0b10).encode(), false, 9, Some(&caps())));
        s.judge_own(None);
        assert_eq!(s.following().map(|f| f.origin), Some(9));
    }

    /// R4: firmware without the event: pending never clears after a push, and a
    /// later host is still followed by readback.
    #[test]
    fn wp_anlastf_r4_without_the_event_pending_stays_set_but_is_harmless() {
        let mut r = Rig::new(0b01, vec![config(0b01), config(0b10)], Reply::Data(vec![]));
        r.tick(0b01); // push, no read
        r.tick(0b01); // read 0b01: agreed
        assert!(r.sharing.agreed(), "{}", r.state());
        assert!(
            r.sharing.pending(),
            "no echo ever: still pending; {}",
            r.state()
        );
        r.tick(0b01); // read 0b10: another host
        assert_eq!(
            r.sharing.following().map(|f| f.origin),
            Some(0),
            "{}",
            r.state()
        );
        assert!(!r.sharing.pending());
    }

    /// R5: lost echo, but the other console's event was seen: the origin is known
    /// from it, yet the readback path reports 0.
    #[test]
    fn wp_anlastf_r5_a_lost_echo_keeps_the_origin_the_event_carried() {
        let mut s = Sharing::default();
        s.judge_own(Some(config(0b01)));
        assert!(s.note_config(&config(0b01).encode(), true, 7, Some(&caps())));
        s.judge_own(Some(config(0b100)));
        let mut p = s.begin_poll();
        p.pushed = true;
        p.pushed_config = Some(config(0b100));
        s.finish_poll(&p, Some(&caps()));
        assert!(s.note_config(&config(0b10).encode(), false, 9, Some(&caps())));
        let mut p = s.begin_poll();
        p.note_readback(config(0b100));
        s.finish_poll(&p, Some(&caps()));
        let mut p = s.begin_poll();
        p.note_readback(config(0b10));
        s.finish_poll(&p, Some(&caps()));
        assert_eq!(
            s.following().map(|f| f.origin),
            Some(9),
            "origin from the event seen"
        );
    }

    // -- WP ANLASTQ --
    /// Qt's side switch to a side with nothing chosen while following: the
    /// views ask for nothing, then Take control. The person took control: not
    /// following any more (before ANLASTQ the release revived the following
    /// at once, so the switch seemed to do nothing). A later push by anyone
    /// is followed, as before.
    #[test]
    fn wp_anlastq_take_control_with_nothing_chosen_stops_following() {
        for take_first in [false, true] {
            let mut s = Sharing::default();
            let caps = caps();
            s.judge_own(Some(config(0b01)));
            assert!(s.note_config(&config(0b01).encode(), true, 7, Some(&caps)));
            assert!(s.note_config(&config(0b10).encode(), false, 9, Some(&caps)));
            assert!(s.following().is_some());
            if take_first {
                s.take_control();
                s.judge_own(None);
            } else {
                s.judge_own(None);
                s.take_control();
                s.judge_own(None);
            }
            assert_eq!(s.following(), None, "take_first {take_first}");
            assert!(!s.pending(), "take_first {take_first}");
            assert!(s.note_config(&config(0b100).encode(), false, 9, Some(&caps)));
            assert_eq!(s.following().map(|f| f.origin), Some(9));
        }
    }

    /// The same release after a choice made by the views (not Take control)
    /// still gives way to the push seen meanwhile (ANLAST, R3b).
    #[test]
    fn wp_anlastq_a_views_choice_released_before_its_push_still_follows() {
        let mut s = Sharing::default();
        let caps = caps();
        s.judge_own(Some(config(0b01)));
        assert!(s.note_config(&config(0b01).encode(), true, 7, Some(&caps)));
        assert!(s.note_config(&config(0b10).encode(), false, 9, Some(&caps)));
        s.judge_own(Some(config(0b100)));
        s.judge_own(None);
        assert_eq!(s.following().map(|f| f.origin), Some(9));
    }

    /// F1 with the hub: a session that does not hold the device lock gets
    /// `LOCKED` for its push, which the engine counts like a STALL.
    #[test]
    fn wp_anlastf_a_push_the_hub_refuses_locked_gives_way_to_the_push_seen_meanwhile() {
        let mut r = Rig::new(
            0b01,
            vec![config(0b01), config(0b10)],
            Reply::Error(dspi_transport::TransportError::Locked),
        );
        assert!(r.note(&event(&config(0b01), 7)));
        r.tick(0b100);
        assert!(r.sharing.pending(), "{}", r.state());
        assert!(r.note(&event(&config(0b10), 9)));
        assert_eq!(r.sharing.following(), None);
        for _ in 0..12 {
            r.tick(0b100);
            if r.engine.rejected() {
                break;
            }
        }
        assert!(r.engine.rejected(), "{}", r.state());
        assert!(!r.sharing.pending(), "{}", r.state());
        assert_eq!(
            r.sharing
                .following()
                .map(|f| (f.origin, f.config.channel_mask)),
            Some((9, 0b10)),
            "{}",
            r.state()
        );
        // And 9's next push is followed in place, never pushed over.
        let n = r.pushes().len();
        assert!(r.note(&event(&config(0b1000), 9)));
        assert_eq!(r.sharing.following().unwrap().config.channel_mask, 0b1000);
        r.tick(0b100);
        r.tick(0b100);
        assert!(r.sharing.following().is_some(), "{}", r.state());
        assert_eq!(r.pushes().len(), n, "{}", r.state());
    }

    /// F1: refused while following, with nothing pushed meanwhile: the device
    /// still holds the configuration followed when the choice was made.
    #[test]
    fn wp_anlastf_a_refused_choice_made_while_following_follows_that_again() {
        let mut r = Rig::new(0b01, vec![config(0b10)], Reply::Stall);
        assert!(r.note(&event(&config(0b01), 7)));
        assert!(r.note(&event(&config(0b10), 9)));
        assert!(r.sharing.following().is_some());
        for _ in 0..12 {
            r.tick(0b100);
            if r.engine.rejected() {
                break;
            }
        }
        assert_eq!(
            r.sharing
                .following()
                .map(|f| (f.origin, f.config.channel_mask)),
            Some((9, 0b10)),
            "{}",
            r.state()
        );
    }

    /// F5: the firmware applies a staged SET from its main loop, so two SETs
    /// staged in one pass give one event, with the later configuration, which
    /// the hub attributes to the oldest writer. Here A (7) pushed 0b10, then
    /// B (9) pushed 0b100; one event 0b100 attributed to A. B, whose choice it
    /// carries, resolves at once. A takes it for an older push of its own,
    /// stays pending, and its engine sees the readback disagree and pushes
    /// again; that push's echo settles both, with A driving.
    #[test]
    fn wp_anlastf_two_sets_coalesced_into_one_event_still_settle() {
        // A: the real engine; the device reads 0b100, then (after A's second
        // push) 0b10.
        let mut a = Rig::new(0b01, vec![config(0b100), config(0b10)], Reply::Data(vec![]));
        assert!(a.note(&event(&config(0b01), 7)));
        a.tick(0b10); // A's push
        assert_eq!(a.pushes(), [0b10]);
        // B, as the session's rules alone.
        let mut b = Sharing::default();
        let caps = caps();
        b.judge_own(Some(config(0b01)));
        assert!(b.note_config(&config(0b01).encode(), true, 9, Some(&caps)));
        b.judge_own(Some(config(0b100)));
        let mut p = b.begin_poll();
        p.pushed = true;
        p.pushed_config = Some(config(0b100));
        p.attempted_config = Some(config(0b100));
        b.finish_poll(&p, Some(&caps));
        // The one event, attributed to A.
        assert!(a.note(&event(&config(0b100), 7)));
        assert!(b.note_config(&config(0b100).encode(), false, 7, Some(&caps)));
        assert!(!b.pending(), "the console whose config it carries resolves");
        assert_eq!(b.following(), None);
        assert!(a.sharing.pending(), "{}", a.state());
        // A is not stuck: the readback disagrees, the engine pushes again.
        a.tick(0b10);
        a.tick(0b10);
        assert_eq!(a.pushes(), [0b10, 0b10], "{}", a.state());
        assert!(a.note(&event(&config(0b10), 7)));
        assert!(b.note_config(&config(0b10).encode(), false, 7, Some(&caps)));
        assert!(!a.sharing.pending());
        assert_eq!(a.sharing.following(), None, "A drives");
        assert_eq!(b.following().map(|f| f.origin), Some(7), "B follows A");
    }

    // ---- the model --------------------------------------------------------

    /// The one device several consoles share: what it holds, and the event
    /// sequence every console drains in the same order. It STALLs a SET for
    /// any mask divisible by 7, as the firmware does for an invalid one.
    #[derive(Default)]
    struct Device {
        config: Option<RtaConfig>,
        stream: Vec<(RtaConfig, u16)>,
    }

    fn refused(c: &RtaConfig) -> bool {
        c.channel_mask.is_multiple_of(7)
    }

    /// One console's path to the device, as the hub gives it.
    struct Wire {
        device: std::sync::Arc<std::sync::Mutex<Device>>,
        id: u16,
        descriptor: DeviceDescriptor,
    }

    impl Transport for Wire {
        fn control_in(&mut self, opcode: u8, _: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
            match opcode {
                op::REQ_RTA_GET_CONFIG => {
                    let d = self.device.lock().unwrap();
                    d.config
                        .map(|c| c.encode().to_vec())
                        .ok_or(dspi_transport::TransportError::Stalled { opcode })
                }
                _ => Ok(vec![0; usize::from(len)]),
            }
        }

        fn control_out(&mut self, opcode: u8, _: u16, data: &[u8]) -> dspi_transport::Result<()> {
            if opcode == op::REQ_RTA_SET_CONFIG {
                let c = RtaConfig::decode(data).unwrap();
                if refused(&c) {
                    return Err(dspi_transport::TransportError::Stalled { opcode });
                }
                let mut d = self.device.lock().unwrap();
                d.config = Some(c);
                d.stream.push((c, self.id));
            }
            Ok(())
        }

        fn descriptor(&self) -> &DeviceDescriptor {
            &self.descriptor
        }

        fn session_id(&self) -> Option<u16> {
            Some(self.id)
        }

        fn caps(&self) -> TransportCaps {
            TransportCaps {
                kind: TransportKind::Link,
                chunked_bulk: false,
                remote: true,
            }
        }
    }

    struct Console {
        id: u16,
        sharing: Sharing,
        engine: RtaEngine,
        wire: Wire,
        view: Option<crate::rta::ViewId>,
        /// What it asks for; `None` while its displays are hidden.
        own: Option<u16>,
        /// The last thing it asked for, shown again on a re-show.
        last: u16,
        /// Events drained so far.
        at: usize,
        /// Reconnected, and nothing pushed or heard since.
        stale: bool,
    }

    impl Console {
        fn new(id: u16, device: &std::sync::Arc<std::sync::Mutex<Device>>, at: usize) -> Self {
            let mut engine = RtaEngine::new();
            assert!(engine.load(caps(), vec![100; 37]));
            Self {
                id,
                sharing: Sharing::default(),
                engine,
                wire: Wire {
                    device: device.clone(),
                    id,
                    descriptor: DeviceDescriptor {
                        serial: "MODEL".into(),
                        bus_id: String::new(),
                        address: 0,
                        ..Default::default()
                    },
                },
                view: None,
                own: None,
                last: 0,
                at,
                stale: false,
            }
        }

        /// One poll as the Terminal runs it: judge, feed the engine, tick.
        fn poll(&mut self, now: Instant) {
            self.sharing.judge_own(self.own.map(config));
            let mask = match (self.sharing.following(), self.own) {
                (_, None) => None,
                (Some(f), Some(_)) => Some((f.config.channel_mask, f.options())),
                (None, Some(m)) => Some((
                    m,
                    Followed {
                        config: config(m),
                        origin: 0,
                    }
                    .options(),
                )),
            };
            match mask {
                Some((mask, options)) => {
                    self.engine.set_options(options);
                    let req = Request {
                        tap: TAP_OUTPUT,
                        mask,
                        wants_bins: false,
                    };
                    match self.view {
                        Some(v) => self.engine.update(v, req),
                        None => self.view = Some(self.engine.subscribe(req)),
                    }
                }
                None => {
                    if let Some(v) = self.view.take() {
                        self.engine.release(v);
                    }
                }
            }
            let before = self.wire.device.lock().unwrap().stream.len();
            self.sharing
                .tick(&mut self.engine, &mut self.wire, now, Budget::default());
            if self.wire.device.lock().unwrap().stream.len() > before {
                self.stale = false;
            }
        }

        fn drain(&mut self) -> bool {
            let next = self
                .wire
                .device
                .lock()
                .unwrap()
                .stream
                .get(self.at)
                .copied();
            let Some((c, by)) = next else {
                return false;
            };
            self.at += 1;
            self.stale = false;
            let caps = caps();
            self.sharing
                .note_config(&c.encode(), by == self.id, by, Some(&caps));
            true
        }

        fn reconnect(&mut self) {
            self.sharing.reset();
            self.engine.disconnect();
            assert!(self.engine.load(caps(), vec![100; 37]));
            // A new session: what was relayed before is not replayed.
            self.at = self.wire.device.lock().unwrap().stream.len();
            self.stale = true;
        }
    }

    struct World {
        device: std::sync::Arc<std::sync::Mutex<Device>>,
        consoles: Vec<Console>,
        now: Instant,
    }

    impl World {
        fn new(n: u16) -> Self {
            let device = std::sync::Arc::new(std::sync::Mutex::new(Device::default()));
            let consoles = (1..=n).map(|id| Console::new(id, &device, 0)).collect();
            Self {
                device,
                consoles,
                now: Instant::now(),
            }
        }

        fn poll(&mut self, i: usize) {
            self.now += Duration::from_secs(1);
            let now = self.now;
            self.consoles[i].poll(now);
        }

        /// What holds after every step. Never pending and following at once.
        /// A console that has heard every event, waits for nothing, and asked
        /// for something, sees the device rightly: following what it holds,
        /// or driving with its own choice (or its own older push) there. The
        /// one exception is a refusal with nobody to follow (the engine shows
        /// it), as before ANLAST.
        fn check(&self, seed: u64, step: usize) {
            let d = self.device.lock().unwrap();
            for c in &self.consoles {
                let s = &c.sharing;
                assert!(
                    !(s.pending() && s.following().is_some()),
                    "seed {seed} step {step}: {} pending and following",
                    c.id
                );
                let Some(mine) = s.last_own else { continue };
                let Some(held) = d.config else { continue };
                let drained = c.at == d.stream.len();
                if !drained || s.pending() || c.stale {
                    continue;
                }
                // Refused with nobody to follow: the engine says so (as before
                // ANLAST). Refused while another console's push was seen: that
                // one is followed (F1).
                if c.engine.rejected() && s.following().is_none() {
                    continue;
                }
                let last_by = d.stream.last().map(|e| e.1);
                match s.following() {
                    Some(f) => assert!(
                        same_choice(&f.config, &held),
                        "seed {seed} step {step}: {} follows {f:?}, device holds {held:?}",
                        c.id
                    ),
                    None => assert!(
                        same_choice(&held, &mine) || last_by == Some(c.id),
                        "seed {seed} step {step}: {} drives {mine:?}, device holds {held:?} by {last_by:?}",
                        c.id
                    ),
                }
            }
        }

        /// Everyone shows its choice again, polls and drains until nothing moves.
        fn settle(&mut self, seed: u64) {
            for c in &mut self.consoles {
                if c.own.is_none() && c.last != 0 {
                    c.own = Some(c.last);
                }
            }
            // Long enough for a refused push's three attempts and their reads.
            for _ in 0..20 {
                for i in 0..self.consoles.len() {
                    while self.consoles[i].drain() {}
                    self.poll(i);
                    while self.consoles[i].drain() {}
                }
            }
            // Quiet: whoever the device announced last drives, everyone else
            // who asked for something follows it, unless refused.
            let d = self.device.lock().unwrap();
            let Some(&(held, by)) = d.stream.last() else {
                return;
            };
            assert!(same_choice(&d.config.unwrap(), &held));
            for c in &self.consoles {
                let s = &c.sharing;
                // A refused choice is given up too (F1): nothing waits for ever.
                assert!(
                    !s.pending(),
                    "seed {seed}: {} pending: {s:?} own {:?} stream {:?}",
                    c.id,
                    c.own,
                    d.stream
                );
                if c.own.is_none() || c.engine.rejected() {
                    continue;
                }
                match s.following() {
                    None => assert!(
                        same_choice(&held, &s.last_own.unwrap()),
                        "seed {seed}: {} drives, device holds {held:?} by {by}",
                        c.id
                    ),
                    Some(f) => assert!(
                        same_choice(&f.config, &held),
                        "seed {seed}: {} follows {f:?}, device holds {held:?}",
                        c.id
                    ),
                }
            }
        }
    }

    /// Two or three consoles: choices (new, refused, identical to what the
    /// device holds or another console asks for), releases and re-shows,
    /// polls with readbacks, drains and reconnects, in any interleaving, with
    /// the invariant checked after every step and convergence at the end.
    #[test]
    fn wp_anlastf_any_interleaving_keeps_the_invariant_and_converges() {
        for seed in 0..1500u64 {
            let mut rng = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let mut next = |n: u64| {
                rng = rng
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (rng >> 33) % n
            };
            let n = 2 + next(2) as usize;
            let mut w = World::new(n as u16);
            let mut fresh = 0u16;
            for step in 0..(6 + next(40) as usize) {
                let i = next(n as u64) as usize;
                match next(10) {
                    0 | 1 => {
                        fresh = (fresh + 1) % 0x1FF;
                        let m = fresh.max(1);
                        w.consoles[i].own = Some(m);
                        w.consoles[i].last = m;
                    }
                    2 => {
                        // Identical: what the device holds, or what another asks.
                        let other = w.consoles[next(n as u64) as usize].own;
                        let held = w.device.lock().unwrap().config.map(|c| c.channel_mask);
                        if let Some(m) = if next(2) == 0 { held } else { other } {
                            w.consoles[i].own = Some(m);
                            w.consoles[i].last = m;
                        }
                    }
                    3 => {
                        let c = &mut w.consoles[i];
                        c.own = match c.own {
                            Some(_) => None,
                            None if c.last != 0 => Some(c.last),
                            None => None,
                        };
                    }
                    4..=6 => w.poll(i),
                    7 | 8 => {
                        w.consoles[i].drain();
                    }
                    _ => w.consoles[i].reconnect(),
                }
                w.check(seed, step);
            }
            w.settle(seed);
        }
    }
}

/// WP CORENITS: the ANLAST re-review's nits N1 and N2, with its probes R9 and
/// R6 (review/anlast-probe2 73017a03) adopted as passing tests.
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_corenits {
    use super::*;

    fn caps() -> RtaCaps {
        RtaCaps {
            input_channels: 2,
            output_channels: 16,
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

    /// One poll whose push of `c` went out, as `Held` records it.
    fn pushed(s: &mut Sharing, c: RtaConfig) {
        let mut p = s.begin_poll();
        p.pushed = true;
        p.pushed_config = Some(c);
        p.attempted_config = Some(c);
        s.finish_poll(&p, Some(&caps()));
    }

    fn readback(s: &mut Sharing, c: RtaConfig) {
        let mut p = s.begin_poll();
        p.note_readback(c);
        s.finish_poll(&p, Some(&caps()));
    }

    /// Driving 0b01, our echo seen.
    fn driving() -> Sharing {
        let mut s = Sharing::default();
        s.judge_own(Some(config(0b01)));
        pushed(&mut s, config(0b01));
        assert!(s.note_config(&config(0b01).encode(), true, 7, Some(&caps())));
        assert!(!s.pending() && s.following().is_none());
        s
    }

    /// N1 / R9: an older push of ours whose echo the hub attributes to nobody
    /// (origin 0) is ours, not another console's: when the re-choice is
    /// refused, nothing of anyone else's is followed.
    #[test]
    fn wp_corenits_r9_own_older_echo_attributed_to_nobody_is_ours() {
        let caps = caps();
        let mut s = driving();
        s.judge_own(Some(config(0b100)));
        pushed(&mut s, config(0b100));
        s.judge_own(Some(config(0b1000))); // re-choice before the echo
        assert!(s.note_config(&config(0b100).encode(), false, 0, Some(&caps)));
        assert!(s.pending());
        let mut p = s.begin_poll();
        p.attempted_config = Some(config(0b1000)); // refused
        s.finish_poll(&p, Some(&caps));
        s.engine_gave_up(true);
        assert_eq!(
            s.following(),
            None,
            "not our own older push as another console's"
        );
    }

    /// N1: an origin-0 event that is no push of ours is still another
    /// console's: followed while driving, and the fallback while pending.
    #[test]
    fn wp_corenits_an_unattributed_push_that_is_not_ours_still_counts() {
        let caps = caps();
        let mut s = driving();
        assert!(s.note_config(&config(0b10).encode(), false, 0, Some(&caps)));
        assert_eq!(
            s.following().map(|f| (f.origin, f.config.channel_mask)),
            Some((0, 0b10))
        );

        let mut s = driving();
        s.judge_own(Some(config(0b100)));
        pushed(&mut s, config(0b100));
        s.judge_own(Some(config(0b1000)));
        assert!(s.note_config(&config(0b10).encode(), false, 0, Some(&caps)));
        let mut p = s.begin_poll();
        p.attempted_config = Some(config(0b1000));
        s.finish_poll(&p, Some(&caps));
        s.engine_gave_up(true);
        assert_eq!(s.following().map(|f| f.config.channel_mask), Some(0b10));
    }

    /// N1: a hub that names another session is believed, even for a
    /// configuration we pushed too.
    #[test]
    fn wp_corenits_an_attributed_push_of_the_same_config_is_theirs() {
        let mut s = driving();
        s.judge_own(Some(config(0b100)));
        pushed(&mut s, config(0b100));
        s.judge_own(Some(config(0b1000)));
        assert!(s.note_config(&config(0b100).encode(), false, 3, Some(&caps())));
        let mut p = s.begin_poll();
        p.attempted_config = Some(config(0b1000));
        s.finish_poll(&p, Some(&caps()));
        s.engine_gave_up(true);
        assert_eq!(s.following().map(|f| f.origin), Some(3));
    }

    /// N2 / R6: our Z pushed, read back (agreed), then A's later Y read back
    /// (following Y); the queue then drains our stale Z echo and Y's event.
    /// The stale echo changes nothing: following throughout.
    #[test]
    fn wp_corenits_r6_stale_own_echo_after_a_readback_moved_us_to_following() {
        let caps = caps();
        let mut s = driving();
        s.judge_own(Some(config(0b100)));
        pushed(&mut s, config(0b100));
        readback(&mut s, config(0b100));
        assert!(s.agreed() && s.pending());
        readback(&mut s, config(0b10));
        assert_eq!(s.following().map(|f| f.config.channel_mask), Some(0b10));
        assert!(!s.pending());
        assert!(s.note_config(&config(0b100).encode(), true, 7, Some(&caps)));
        let between = s.following();
        assert!(s.note_config(&config(0b10).encode(), false, 3, Some(&caps)));
        assert_eq!(
            s.following().map(|f| (f.origin, f.config.channel_mask)),
            Some((3, 0b10))
        );
        assert_eq!(
            between.map(|f| f.config.channel_mask),
            Some(0b10),
            "after our stale echo the device still holds A's 0b10"
        );
    }

    /// N2: the stale echo is used up; taking control afterwards pushes ours
    /// again, and that push's echo makes us drive. The last choice wins.
    #[test]
    fn wp_corenits_after_a_stale_echo_a_new_push_of_ours_drives() {
        let caps = caps();
        let mut s = driving();
        s.judge_own(Some(config(0b100)));
        pushed(&mut s, config(0b100));
        readback(&mut s, config(0b100));
        readback(&mut s, config(0b10));
        assert!(s.note_config(&config(0b100).encode(), true, 7, Some(&caps)));
        assert!(s.note_config(&config(0b10).encode(), false, 3, Some(&caps)));
        s.take_control();
        pushed(&mut s, config(0b100));
        assert!(s.note_config(&config(0b100).encode(), true, 7, Some(&caps)));
        assert!(!s.pending());
        assert_eq!(s.following(), None);
        // And a later push of A's is followed.
        assert!(s.note_config(&config(0b1000).encode(), false, 3, Some(&caps)));
        assert_eq!(s.following().map(|f| f.origin), Some(3));
    }

    /// The unechoed list stays bounded on firmware without the event.
    #[test]
    fn wp_corenits_unechoed_pushes_are_bounded() {
        let mut s = Sharing::default();
        for m in 1..=20u16 {
            s.judge_own(Some(config(m)));
            pushed(&mut s, config(m));
        }
        assert_eq!(s.own_pushes.len(), OWN_PUSHES_MAX);
        assert_eq!(s.own_pushes.last().map(|p| p.config.channel_mask), Some(20));
        s.reset();
        assert!(s.own_pushes.is_empty());
    }
}

// -- WP PGCORE --
#[cfg(test)]
mod tests_wp_pgcore {
    use super::*;
    use dspi_transport::FakeDevice;
    use dspi_transport::fake::FakeGeneration;

    /// One poll's view of a paged device says what it is and passes the
    /// paged calls.
    #[test]
    fn wp_pgcore_held_passes_the_generation_and_the_paged_calls() {
        for generation in FakeGeneration::ALL {
            let mut t =
                dspi_transport::paged::open(Box::new(FakeDevice::demo_generation(generation)))
                    .unwrap();
            let mut share = PollShare::default();
            let mut held = Held::new(&mut *t, &mut share);
            assert_eq!(held.carries_pages(), generation.is_paged());
            assert_eq!(held.generation(), t_generation(generation));
            assert_eq!(held.paged_discovery().is_some(), generation.is_paged());
            let r = held.control_in_paged(dspi_proto::paged::cmds::ROUTE_MASTER_VOLUME, 0, 4);
            assert_eq!(r.is_ok(), generation.is_paged(), "{generation:?}: {r:?}");
        }
    }

    fn t_generation(g: FakeGeneration) -> Option<dspi_transport::Generation> {
        let t = dspi_transport::paged::open(Box::new(FakeDevice::demo_generation(g))).unwrap();
        t.generation()
    }
}
