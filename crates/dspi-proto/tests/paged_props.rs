//! WP PGPROTO: properties of the planner, the reverse view and the decoders
//! (PLAN-PG shared rules: no panic, no page-0 step, no zero-length GET, SET
//! lengths within bounds, planner-then-reverse identity on every invertible row,
//! the extension row tunnels only with page 0xE0 listed).

use dspi_proto::paged::legacy::LEGACY_OPS;
use dspi_proto::paged::registry;
use dspi_proto::paged::{
    Answer, CmdId, CommandInfo, EXT_PAGE, EXT_TUNNEL, LastResult, LegacyBody, Next, PageCommands,
    PageSet, PagedBody, PagedDir, PlanError, SysPlatform, UsbDir, decode_page_list,
    legacy_equivalent, plan, reaches,
};
use proptest::prelude::*;

fn full() -> PageSet {
    PageSet::vendored(|_| true)
}

/// Every `(opcode, direction)` the map has.
fn sides() -> Vec<(u8, UsbDir)> {
    let mut v = Vec::new();
    for o in &LEGACY_OPS {
        if o.out.is_some() {
            v.push((o.code, UsbDir::Out));
        }
        if o.r#in.is_some() {
            v.push((o.code, UsbDir::In));
        }
    }
    v
}

/// A payload whose length the target the request reaches accepts.
fn payload_for(opcode: u8, value: u16, seed: u8, bytes: &[u8]) -> Vec<u8> {
    let len = match reaches(opcode, value, UsbDir::Out).first() {
        Some((id, PagedDir::Set)) => match registry::command(*id) {
            Some(c) if c.flags.has(registry::Flags::BULK) => 1 + usize::from(seed) % 64,
            Some(c) => {
                let span = usize::from(c.set_max - c.set_min) + 1;
                usize::from(c.set_min) + usize::from(seed) % span
            }
            None => 0,
        },
        _ => usize::from(seed) % 8,
    };
    bytes.iter().copied().cycle().take(len).collect()
}

fn index() -> impl Strategy<Value = usize> {
    0..sides().len()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2048, ..ProptestConfig::default() })]

    /// Whatever a caller asks, a plan never puts a request on page 0, never asks a
    /// GET of 0 bytes, never sends a SET the vendored revision refuses, and only
    /// names commands the device lists.
    #[test]
    fn proptest_wp_pgproto_plans_are_well_formed(
        i in index(), value in any::<u16>(), seed in any::<u8>(),
        bytes in proptest::collection::vec(any::<u8>(), 1..80), len in 0u16..300,
        exact in any::<bool>(),
    ) {
        let (opcode, dir) = sides()[i];
        let data = if exact { payload_for(opcode, value, seed, &bytes) } else { bytes.clone() };
        let body = match dir {
            UsbDir::In => LegacyBody::In { len },
            UsbDir::Out => LegacyBody::Out(&data),
        };
        let pages = full();
        match plan(opcode, value, body, &pages) {
            Ok(mut p) => {
                if let Next::Send(r) = p.start() {
                    prop_assert!(!r.id.is_legacy());
                    prop_assert!(pages.contains(r.id, r.dir()));
                    let c = registry::command(r.id).expect("a registry row");
                    match &r.body {
                        PagedBody::Get { len } => prop_assert!(*len > 0),
                        PagedBody::Set(d) => {
                            prop_assert!(c.set_len_ok(d.len()), "{:?} {} bytes", r.id, d.len())
                        }
                    }
                    // Any answer finishes a single-step plan without a panic.
                    // -- WP PGCUST -- multi-step custom recipes are checked in
                    // `paged::custom`'s own properties.
                    if p.single().is_some() {
                    for a in [Answer::Ack, Answer::Data(&bytes), Answer::Refused { result: None },
                              Answer::Refused { result: Some(seed) }] {
                        let mut q = plan(opcode, value, body, &pages).expect("same plan");
                        let _ = q.start();
                        prop_assert!(matches!(q.feed(a), Next::Done(_)));
                    }
                    }
                }
            }
            Err(PlanError::Table { .. }) => prop_assert!(false, "table error"),
            Err(_) => {}
        }
    }

    /// Planner then reverse is the identity on every invertible row: the 1.1.6
    /// request the reverse view gives plans to the very same paged request.
    #[test]
    fn proptest_wp_pgproto_planner_then_reverse(
        i in index(), value in any::<u16>(), seed in any::<u8>(),
        bytes in proptest::collection::vec(any::<u8>(), 1..80),
    ) {
        let (opcode, dir) = sides()[i];
        let data = payload_for(opcode, value, seed, &bytes);
        let body = match dir {
            UsbDir::In => LegacyBody::In { len: 64 },
            UsbDir::Out => LegacyBody::Out(&data),
        };
        let pages = full();
        let Ok(p) = plan(opcode, value, body, &pages) else { return Ok(()); };
        let Some(r) = p.single().cloned() else { return Ok(()); };
        let payload = match &r.body { PagedBody::Set(d) => d.clone(), PagedBody::Get { .. } => Vec::new() };
        let eq = legacy_equivalent(r.id, r.dir(), r.value, &payload)
            .unwrap_or_else(|| panic!("no reverse for 0x{opcode:02X} {dir:?} 0x{value:04X} -> {r:?}"));
        let body2 = match eq.dir {
            UsbDir::In => LegacyBody::In { len: 64 },
            UsbDir::Out => LegacyBody::Out(&eq.data),
        };
        let p2 = plan(eq.opcode, eq.value, body2, &pages).expect("the reverse plans");
        let r2 = p2.single().expect("one request");
        prop_assert_eq!(r2.id, r.id);
        prop_assert_eq!(r2.value, r.value);
        match (&r2.body, &r.body) {
            (PagedBody::Set(a), PagedBody::Set(b)) => prop_assert_eq!(a, b),
            (PagedBody::Get { .. }, PagedBody::Get { .. }) => {}
            other => prop_assert!(false, "direction changed: {other:?}"),
        }
    }

    /// The reverse view and `reaches` never panic, whatever the request.
    #[test]
    fn proptest_wp_pgproto_reverse_never_panics(
        id in any::<u16>(), set in any::<bool>(), value in any::<u16>(),
        payload in proptest::collection::vec(any::<u8>(), 0..70), op in any::<u8>(),
    ) {
        let dir = if set { PagedDir::Set } else { PagedDir::Get };
        if let Some(eq) = legacy_equivalent(CmdId(id), dir, value, &payload) {
            prop_assert!(!reaches(eq.opcode, eq.value, eq.dir).is_empty());
        }
        let _ = reaches(op, value, if set { UsbDir::Out } else { UsbDir::In });
    }

    /// No decoder panics on device bytes.
    #[test]
    fn proptest_wp_pgproto_decoders_never_panic(b in proptest::collection::vec(any::<u8>(), 0..80)) {
        let list = decode_page_list(&b);
        prop_assert_eq!(list.len(), b.len() / 2);
        let _ = PageCommands::decode(&b);
        let _ = CommandInfo::decode(&b);
        let _ = LastResult::decode(&b);
        let _ = SysPlatform::decode(&b);
        let set = PageSet::from_discovery(&list, |_| Some(&b[..]));
        prop_assert_eq!(set.pages().count(), list.iter().map(|p| p.page).collect::<std::collections::BTreeSet<_>>().len());
    }

    /// The extension row tunnels exactly when the device lists the tunnel, keeps
    /// `wValue` and payload, and is otherwise refused with nothing sent.
    #[test]
    fn proptest_wp_pgproto_ext_row_needs_page_e0(
        value in any::<u16>(), payload in proptest::collection::vec(any::<u8>(), 0..64),
        len in 1u16..256, listed in any::<bool>(), out in any::<bool>(),
    ) {
        let mut pages = full();
        if listed {
            let mut m = PageCommands::EMPTY;
            m.insert(EXT_TUNNEL.cmd(), PagedDir::Get);
            m.insert(EXT_TUNNEL.cmd(), PagedDir::Set);
            pages.insert(EXT_PAGE, 1, m);
        }
        let body = if out { LegacyBody::Out(&payload) } else { LegacyBody::In { len } };
        match plan(0x00, value, body, &pages) {
            Ok(p) => {
                prop_assert!(listed);
                let r = p.single().expect("one request");
                prop_assert_eq!(r.id, EXT_TUNNEL);
                prop_assert_eq!(r.value, value);
                prop_assert_eq!(r.id.w_index(), 0xE002);
                match &r.body {
                    PagedBody::Set(d) => prop_assert_eq!(d, &payload),
                    PagedBody::Get { len: l } => prop_assert_eq!(*l, len),
                }
            }
            Err(e) => {
                prop_assert!(!listed);
                prop_assert_eq!(e, PlanError::NoExtChannel);
            }
        }
    }
}

/// Every non-custom row plans on a device that has everything, and every row
/// that plans to one request has a reverse (the identity property above is then
/// checked on random values; this pins that no row is silently non-invertible).
#[test]
fn wp_pgproto_every_simple_row_plans_and_reverses() {
    let pages = full();
    let mut planned = 0;
    for (opcode, dir) in sides() {
        for value in [0u16, 1, 2, 3, 0x80, 0x81, 0x0100, 0x0301, 0xFFFF] {
            let data = payload_for(opcode, value, 0, &[1, 2, 3, 4]);
            let body = match dir {
                UsbDir::In => LegacyBody::In { len: 8 },
                UsbDir::Out => LegacyBody::Out(&data),
            };
            let Ok(p) = plan(opcode, value, body, &pages) else {
                continue;
            };
            let Some(r) = p.single() else { continue };
            planned += 1;
            let payload = match &r.body {
                PagedBody::Set(d) => d.clone(),
                PagedBody::Get { .. } => Vec::new(),
            };
            assert!(
                legacy_equivalent(r.id, r.dir(), r.value, &payload).is_some(),
                "0x{opcode:02X} {dir:?} 0x{value:04X} -> {r:?} has no reverse"
            );
        }
    }
    assert!(planned > 500, "{planned}");
}

// -- WP PGCUST -- (was `wp_pgproto_custom_rows_wait_for_pgcust`: the stub's refusal)
/// Custom rows are planned by `paged::custom` now, never refused as unsupported.
#[test]
fn wp_pgcust_custom_rows_are_planned() {
    let pages = full();
    let customs = [0x28u8, 0x43, 0x50, 0x51, 0x71, 0x83, 0x97, 0xB2, 0xB3, 0xEE];
    for op in customs {
        assert!(
            plan(op, 0, LegacyBody::In { len: 8 }, &pages).is_ok(),
            "0x{op:02X}"
        );
    }
    for op in [0x42u8, 0x44, 0x70, 0xA1] {
        assert!(
            plan(op, 0, LegacyBody::Out(&[0; 18]), &pages).is_ok(),
            "0x{op:02X}"
        );
    }
    // Its reach is still known, for the hub's judges.
    assert!(
        reaches(0x51, 0, UsbDir::In).contains(&(registry::cmds::STORE_PRESET_SAVE, PagedDir::Set))
    );
}

/// R8b: SET lengths are judged locally only at the vendored page revision.
#[test]
fn wp_pgproto_set_lengths_only_at_the_vendored_revision() {
    let mut pages = full();
    // SET_OUTPUT_GAIN takes exactly 4 bytes.
    assert!(matches!(
        plan(0x74, 1, LegacyBody::Out(&[0; 6]), &pages),
        Err(PlanError::Length {
            id: CmdId(0x0521),
            len: 6,
            min: 4,
            max: 4
        })
    ));
    let route = registry::page(5).unwrap();
    let mut m = PageCommands::EMPTY;
    for c in registry::COMMANDS.iter().filter(|c| c.id.page() == 5) {
        if c.get {
            m.insert(c.id.cmd(), PagedDir::Get);
        }
        if c.set {
            m.insert(c.id.cmd(), PagedDir::Set);
        }
    }
    pages.insert(5, route.revision + 1, m);
    let p = plan(0x74, 1, LegacyBody::Out(&[0; 6]), &pages).expect("the device judges");
    assert_eq!(p.single().unwrap().body, PagedBody::Set(vec![0; 6]));
}

/// A GET of 0 bytes is never sent; trunc cuts SET_CS_NAME to 32 bytes.
#[test]
fn wp_pgproto_zero_gets_and_trunc() {
    let pages = full();
    assert!(matches!(
        plan(0x75, 1, LegacyBody::In { len: 0 }, &pages),
        Err(PlanError::ZeroLengthGet { .. })
    ));
    let name = [b'a'; 40];
    let p = plan(0x8B, 0, LegacyBody::Out(&name), &pages).expect("SET_CS_NAME");
    assert_eq!(p.single().unwrap().body, PagedBody::Set(vec![b'a'; 32]));
}

/// `on_stall`: UPMIX_GET_CONFIG answers 44 zero bytes when the command is unknown
/// (guide 6.4: "1.1.6 answered the GETs with zeros").
#[test]
fn wp_pgproto_on_stall_fills_like_116() {
    use dspi_proto::paged::Outcome;
    use dspi_proto::paged::legacy::{VC_ERR_ADDRESS, VC_ERR_UNKNOWN};
    let pages = full();
    let run = |result| {
        let mut p = plan(0x4B, 0, LegacyBody::In { len: 64 }, &pages).unwrap();
        assert!(p.needs_result());
        let _ = p.start();
        p.feed(Answer::Refused { result })
    };
    assert_eq!(
        run(Some(VC_ERR_UNKNOWN)),
        Next::Done(Outcome::Reply(vec![0; 44]))
    );
    assert_eq!(
        run(Some(VC_ERR_ADDRESS)),
        Next::Done(Outcome::Stall {
            result: Some(VC_ERR_ADDRESS)
        })
    );
    assert_eq!(run(None), Next::Done(Outcome::Stall { result: None }));
}
