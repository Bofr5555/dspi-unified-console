//! WP PGFAKE1: which firmware generation a [`super::FakeDevice`] is, and the
//! hard rules of PLAN-PG section 1.1 that fail a test by themselves (R9a).
//!
//! | Generation | Firmware | Protocol |
//! |---|---|---|
//! | [`FakeGeneration::Stock116`] | G1s, stock upstream 1.1.6 | page 0 only, no extensions |
//! | [`FakeGeneration::Bofr116`] | G1b, our `bofr-main` 1.1.6, with PAGEGUARD (`bofr-main` 8385455) or an older image without it | page 0 + `REQ_EXTENSION` 0x00 |
//! | [`FakeGeneration::Stock117`] | G2s, stock upstream 1.1.7 (`release/v1.1.7` 103ba37) | paged; page 0 refused (`VC_ERR_UNKNOWN`) |
//! | [`FakeGeneration::Bofr117`] | G2b, our dual-stack port (OD-PGb) | paged + the tunnel `0xE010` + legacy page 0 |
//!
//! [`super::FakeOptions::generation`] `None` keeps the fake as it always was: a
//! 1.1.6-generation device whose extension channel and page guard follow
//! [`super::FakeOptions::extensions`] and [`super::FakeOptions::page_guard`].
//! `Some` sets both from the generation (they are overwritten), and a paged
//! generation answers through the paged front (`fake/paged.rs`).
//!
//! **Violations** (PLAN-PG section 1.1, "the two hard rules"): a paged request
//! other than the probe on a 1.1.6 generation; a page-0 request on
//! `Stock117`; a page-0 request on `Bofr117` after that device answered the
//! probe as paged (our client then speaks only paged to it, OD-PGd: the
//! legacy stack is for hosts that never sent the probe). Each one is counted
//! ([`super::FakeRemote::violations`]) and panics on the spot unless
//! [`super::FakeOptions::allow_violation`] is on; and a device that saw one
//! fails in `Drop` as well, for a violation whose panic a worker thread
//! swallowed.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::{Capture, DEMO_STATUS, FakeDevice, FakeOptions, FakeRemote, State};

/// A firmware generation (PLAN-PG "the owner's requirement" table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeGeneration {
    /// G1s: stock upstream 1.1.6 and older. No extension channel; a page is
    /// ignored and the same-number opcode runs.
    Stock116,
    /// G1b: our `bofr-main`, 1.1.6 generation, with the extension channel.
    /// `page_guard`: PAGEGUARD, which STALLs any request that names a page,
    /// the probe included; without it the page is ignored as on stock.
    Bofr116 { page_guard: bool },
    /// G2s: stock upstream 1.1.7, strict rules, no extensions. `revision`:
    /// the revision every upstream page reports in `SYS.PAGE_LIST`; a page's
    /// vendored revision wins when it is higher, so 0 is "as vendored"
    /// ([`FakeGeneration::STOCK117`]). Above the vendored one, a page accepts
    /// SETs longer than the vendored maximum (a later revision may grow a
    /// payload at the end, migration guide 4.7, PLAN-PG R8b).
    Stock117 { revision: u8 },
    /// G2b: our port onto 1.1.7, dual-stack: the paged stack with page
    /// `0xE0` and the tunnel `0xE010`, plus the 1.1.6 stack on page 0
    /// (extension channel included) for hosts that never send the probe.
    Bofr117,
}

impl FakeGeneration {
    /// Stock 1.1.7 with every page at its vendored revision.
    pub const STOCK117: Self = Self::Stock117 { revision: 0 };

    /// Every generation once (`Bofr116` with and without the guard), for
    /// tests that run over the whole matrix.
    pub const ALL: [Self; 5] = [
        Self::Stock116,
        Self::Bofr116 { page_guard: false },
        Self::Bofr116 { page_guard: true },
        Self::STOCK117,
        Self::Bofr117,
    ];

    /// Whether it answers the probe as paged firmware (1.1.7 generation).
    pub const fn is_paged(self) -> bool {
        matches!(self, Self::Stock117 { .. } | Self::Bofr117)
    }

    /// Whether it has our extensions (on page 0, and on `Bofr117` also
    /// through the tunnel).
    pub const fn has_extensions(self) -> bool {
        matches!(self, Self::Bofr116 { .. } | Self::Bofr117)
    }

    /// Options for this generation, everything else default.
    pub fn options(self) -> FakeOptions {
        FakeOptions {
            generation: Some(self),
            ..FakeOptions::default()
        }
    }
}

/// The `SYS.PLATFORM` / `GET_PLATFORM` version bytes of a paged fake:
/// firmware 1.1.7 beta 1, as the migration guide's examples answer (4.6:
/// `01 01 17 09 01 07 01` on an RP2350).
const V117: [(usize, u8); 5] = [(1, 1), (2, 0x17), (4, 1), (5, 7), (6, 1)];

/// Make the options and the state agree with the generation. Called once,
/// by [`FakeDevice::new`].
pub(super) fn apply(s: &mut State) {
    let Some(g) = s.options.generation else {
        return;
    };
    s.options.extensions = g.has_extensions();
    s.options.page_guard = matches!(g, FakeGeneration::Bofr116 { page_guard: true });
    if g.is_paged() {
        for (i, v) in V117 {
            if let Some(b) = s.platform.get_mut(i) {
                *b = v;
            }
        }
    }
}

impl State {
    /// The generation the fake models; `None` is the classic 1.1.6 fake.
    pub(super) fn generation(&self) -> Option<FakeGeneration> {
        self.options.generation
    }

    /// Count a violation; panic unless the test allowed it. `what` says what
    /// was sent and why it is one.
    pub(super) fn violation(&mut self, what: String) {
        self.pg.violations = self.pg.violations.saturating_add(1);
        let allowed = self.options.allow_violation;
        self.pg.last_violation = Some(what.clone());
        if !allowed {
            panic!("{what}; set FakeOptions::allow_violation to let it through");
        }
    }
}

impl FakeDevice {
    /// The demo device ([`FakeDevice::demo`]: control surfaces, the
    /// analyser) as firmware of `generation`.
    pub fn demo_generation(generation: FakeGeneration) -> Self {
        let options = FakeOptions {
            generation: Some(generation),
            surfaces: true,
            analysis: true,
            ..FakeOptions::default()
        };
        let dev = Self::new(Capture::demo(), options);
        {
            let mut s = dev.shared.lock();
            if DEMO_STATUS.len() == s.status_len() {
                s.status = DEMO_STATUS.to_vec();
            }
        }
        dev
    }

    /// `Drop`'s check: a device poisoned by a violation the test did not
    /// allow fails, unless the thread is already panicking.
    pub(super) fn fail_if_poisoned(&self) {
        let (count, what, allowed) = {
            let s = self.shared.lock();
            (
                s.pg.violations,
                s.pg.last_violation.clone(),
                s.options.allow_violation,
            )
        };
        if count > 0 && !allowed && !std::thread::panicking() {
            panic!(
                "this fake saw {count} protocol violation(s) (PLAN-PG section 1.1), the last: {}",
                what.unwrap_or_default()
            );
        }
    }
}

impl FakeRemote {
    /// How many requests broke a hard rule of PLAN-PG section 1.1 (module
    /// docs), allowed or not.
    pub fn violations(&self) -> u32 {
        self.shared.lock().pg.violations
    }

    /// The last violation's description.
    pub fn last_violation(&self) -> Option<String> {
        self.shared.lock().pg.last_violation.clone()
    }

    // -- WP PGQT --
    /// Let violations through, counted and answered as the firmware would,
    /// never a panic ([`FakeOptions::allow_violation`]): for a fake opened
    /// inside a client app (the Qt smoke driver's generations), which reads
    /// [`FakeRemote::violations`] instead.
    ///
    /// [`FakeOptions::allow_violation`]: super::FakeOptions::allow_violation
    pub fn allow_violations(&self) {
        self.shared.lock().options.allow_violation = true;
    }
}
