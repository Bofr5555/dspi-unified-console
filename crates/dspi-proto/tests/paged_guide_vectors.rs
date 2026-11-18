//! WP PGPROTO: vectors transcribed by hand from upstream's migration guide
//! (`Documentation/Features/vendor_protocol_migration_guide.md` at firmware
//! `103ba37`), the section named on each test. They are the independent oracle:
//! the planner is generated from `legacy_map.json`, these bytes are copied from
//! the guide's text.

use dspi_proto::paged::legacy::{VC_ERR_ADDRESS, VC_ERR_STATE, VC_ERR_UNKNOWN, VC_ERR_VALUE};
use dspi_proto::paged::registry::{self, AddrKind, Flags};
use dspi_proto::paged::{
    Answer, CmdId, CommandInfo, GetLen, LastResult, LegacyBody, Next, Outcome, PageCommands,
    PageRev, PageSet, PagedBody, PagedDir, PagedRequest, Plan, PlanError, SysPlatform,
    decode_page_list, legacy_equivalent, plan,
};

fn rp2350() -> PageSet {
    PageSet::vendored(|c| c.build.is_none() || c.build == Some("PICO_RP2350"))
}

fn plan_in(op: u8, value: u16, len: u16) -> Plan {
    plan(op, value, LegacyBody::In { len }, &rp2350()).expect("planned")
}

fn plan_out(op: u8, value: u16, data: &[u8]) -> Plan {
    plan(op, value, LegacyBody::Out(data), &rp2350()).expect("planned")
}

fn req(p: &Plan) -> &PagedRequest {
    p.single().expect("one request")
}

fn setup(p: &Plan) -> [u8; 8] {
    req(p).setup_packet()
}

fn outcome(mut p: Plan, a: Answer<'_>) -> Outcome {
    assert!(matches!(p.start(), Next::Send(_)));
    match p.feed(a) {
        Next::Done(o) => o,
        Next::Send(r) => panic!("a second request {r:?}"),
    }
}

/// Guide 3.2, "A SET with an address" and "A GET".
#[test]
fn wp_pgproto_guide_3_2_output_gain() {
    let p = plan_out(0x74, 1, &[0x00, 0x00, 0xC0, 0xC0]);
    assert_eq!(setup(&p), [0x41, 0x21, 0x01, 0x00, 0x02, 0x05, 0x04, 0x00]);
    assert_eq!(req(&p).body, PagedBody::Set(vec![0x00, 0x00, 0xC0, 0xC0]));
    assert_eq!(outcome(p, Answer::Ack), Outcome::Ack);

    let p = plan_in(0x75, 1, 4);
    assert_eq!(setup(&p), [0xC1, 0x21, 0x01, 0x00, 0x02, 0x05, 0x04, 0x00]);
    assert_eq!(
        outcome(p, Answer::Data(&[0x00, 0x00, 0xC0, 0xC0])),
        Outcome::Reply(vec![0x00, 0x00, 0xC0, 0xC0])
    );
}

/// Guide 3.2, "A former write-as-read" and "A zero-length action".
#[test]
fn wp_pgproto_guide_3_2_actions() {
    // RESET_BUFFER_STATS 0xB1, mask 1: C1 B1 01 00 02 00 01 00 -> IN 01.
    let p = plan_in(0xB1, 0x0001, 1);
    assert_eq!(setup(&p), [0x41, 0x4F, 0x00, 0x00, 0x02, 0x01, 0x01, 0x00]);
    assert_eq!(req(&p).body, PagedBody::Set(vec![0x01]));
    assert_eq!(outcome(p, Answer::Ack), Outcome::Reply(vec![0x01]));

    // RTA_CONTROL 0x0E action 2 -> MEAS.RTA_RESET, no data stage.
    let p = plan_in(0x0E, 0x0002, 1);
    assert_eq!(setup(&p), [0x41, 0x2A, 0x00, 0x00, 0x02, 0x09, 0x00, 0x00]);
    assert_eq!(outcome(p, Answer::Ack), Outcome::Reply(vec![0x01]));
}

/// Guide 3.2, "A refused request and its reason".
#[test]
fn wp_pgproto_guide_3_2_refusal() {
    let p = plan_in(0x75, 9, 4);
    assert_eq!(setup(&p), [0xC1, 0x21, 0x09, 0x00, 0x02, 0x05, 0x04, 0x00]);
    let last = LastResult::decode(&[0x21, 0x05, 0x01, 0x13, 0x09, 0x00, 0x00, 0x00]).unwrap();
    assert_eq!(last.id, CmdId(0x0521));
    assert_eq!(last.dir, Some(PagedDir::Get));
    assert_eq!(last.result, VC_ERR_ADDRESS);
    assert!(last.matches(CmdId(0x0521), PagedDir::Get, 9));
    assert!(!last.matches(CmdId(0x0521), PagedDir::Get, 8));
    // A read that 1.1.6 STALLed too stays a STALL.
    assert_eq!(
        outcome(
            p,
            Answer::Refused {
                result: Some(last.result)
            }
        ),
        Outcome::Stall {
            result: Some(VC_ERR_ADDRESS)
        }
    );
}

/// Guide 4.6, `SYS.PLATFORM`, and its 1.1.6 form `GET_PLATFORM` 0x7F.
#[test]
fn wp_pgproto_guide_4_6_platform() {
    let p = plan_in(0x7F, 0, 7);
    assert_eq!(setup(&p), [0xC1, 0x20, 0x00, 0x00, 0x02, 0x01, 0x07, 0x00]);
    let sp = SysPlatform::decode(&[0x01, 0x01, 0x17, 0x09, 0x01, 0x07, 0x01]).unwrap();
    assert_eq!(
        (sp.platform, sp.major, sp.packed, sp.outputs),
        (1, 1, 0x17, 9)
    );
    assert_eq!((sp.minor, sp.patch, sp.beta), (Some(1), Some(7), Some(1)));
    let rp2040 = SysPlatform::decode(&[0x00, 0x01, 0x17, 0x05, 0x01, 0x07, 0x01]).unwrap();
    assert_eq!((rp2040.platform, rp2040.outputs), (0, 5));
    // A 1.1.6 board answers 4 bytes: no minor, patch or beta.
    assert_eq!(SysPlatform::decode(&[1, 1, 0x16, 9]).unwrap().beta, None);
}

/// Guide 4.7, the three discovery commands.
#[test]
fn wp_pgproto_guide_4_7_discovery() {
    // SYS.PAGE_LIST of a release build: 12 pages, revision 1.
    let list = decode_page_list(&[
        0x01, 0x01, 0x02, 0x01, 0x03, 0x01, 0x04, 0x01, 0x05, 0x01, 0x06, 0x01, 0x07, 0x01, 0x08,
        0x01, 0x09, 0x01, 0x0A, 0x01, 0x0B, 0x01, 0x0C, 0x01,
    ]);
    assert_eq!(list.len(), 12);
    assert_eq!(
        list[11],
        PageRev {
            page: 0x0C,
            revision: 1
        }
    );
    // It is what our vendored registry describes for a release build.
    let release: Vec<PageRev> = PageSet::vendored(|c| c.build.is_none()).pages().collect();
    assert_eq!(list, release);

    // SYS.PAGE_COMMANDS for FILTER (wValue 6): 0x10-0x12, GET and SET.
    let mut b = [0u8; 64];
    b[2] = 0x07;
    b[34] = 0x07;
    let m = PageCommands::decode(&b).unwrap();
    for cmd in 0x10..=0x12 {
        assert!(m.has(cmd, PagedDir::Get) && m.has(cmd, PagedDir::Set));
    }
    assert!(!m.has(0x13, PagedDir::Get));
    let vendored = PageSet::vendored(|c| c.build.is_none());
    for cmd in 0x10..=0x12u8 {
        assert!(vendored.contains(CmdId::new(6, cmd), PagedDir::Get));
    }
    assert!(!vendored.contains(CmdId::new(6, 0x13), PagedDir::Get));

    // SYS.COMMAND_INFO of STORE.PRESET_LOAD: SETUP C1 13 11 02 02 01 08 00.
    let info = CommandInfo::decode(&[0x38, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x02]).unwrap();
    assert!(
        info.flags
            .has(Flags::DEFERRED | Flags::FLASH | Flags::RESTART)
    );
    assert_eq!(info.addr, Ok(AddrKind::Preset));
    assert!(info.set && !info.get);
    let load = registry::by_name("STORE.PRESET_LOAD").unwrap();
    assert_eq!(load.id, CmdId(0x0211));
    assert_eq!(CommandInfo::of(load), info);
    let get_info = PagedRequest {
        id: registry::cmds::SYS_COMMAND_INFO,
        value: load.id.0,
        body: PagedBody::Get { len: 8 },
    };
    assert_eq!(
        get_info.setup_packet(),
        [0xC1, 0x13, 0x11, 0x02, 0x02, 0x01, 0x08, 0x00]
    );
}

/// Guide 6.4, the split commands.
#[test]
fn wp_pgproto_guide_6_4_splits() {
    // LIMITER 0x81, wValue (output << 8) | index.
    let thr = plan_out(0x81, 0x0301, &[0, 0, 0xA0, 0xC1]);
    assert_eq!((req(&thr).id, req(&thr).value), (CmdId(0x0711), 3));
    let all = plan_out(0x81, 0xFF00, &[0, 0, 0x80, 0x3F]);
    assert_eq!((req(&all).id, req(&all).value), (CmdId(0x0710), 0xFF));
    let meters = plan_in(0x81, 0x0280, 18);
    assert_eq!((req(&meters).id, req(&meters).value), (CmdId(0x0718), 0));
    let status = plan_in(0x81, 0x0081, 4);
    assert_eq!(req(&status).id, CmdId(0x0719));
    // An index 1.1.6 ignored on a write and STALLed on a read.
    let mut ignored = plan_out(0x81, 0x0004, &[0; 4]);
    assert!(ignored.single().is_none(), "nothing is sent");
    assert_eq!(
        ignored.start(),
        Next::Done(Outcome::Swallowed { result: None })
    );
    assert!(matches!(
        plan(0x81, 0x0004, LegacyBody::In { len: 4 }, &rp2350()),
        Err(PlanError::RefusedBy116 {
            opcode: 0x81,
            value: 4
        })
    ));

    // SET_TUBE_PARAM / GET_TUBE_PARAM: index n -> 0x0860 + n, wValue 0.
    for (i, id) in [(0u16, 0x0860u16), (1, 0x0861), (8, 0x0868), (13, 0x086D)] {
        let s = plan_out(0x3E, i, &[0; 4]);
        assert_eq!((req(&s).id, req(&s).value), (CmdId(id), 0));
        let g = plan_in(0x3F, i, 4);
        assert_eq!((req(&g).id, req(&g).value), (CmdId(id), 0));
    }
    // UPMIX_SET_PARAM / UPMIX_GET_PARAM (RP2350 only): index n -> 0x0890 + n.
    for (i, id) in [(0u16, 0x0890u16), (2, 0x0892), (13, 0x089D)] {
        assert_eq!(req(&plan_out(0x4C, i, &[0; 4])).id, CmdId(id));
        assert_eq!(req(&plan_in(0x4D, i, 4)).id, CmdId(id));
    }
    // ... absent on an RP2040, refused locally (nothing sent).
    let rp2040 = PageSet::vendored(|c| c.build.is_none());
    assert!(matches!(
        plan(0x4D, 0, LegacyBody::In { len: 4 }, &rp2040),
        Err(PlanError::Absent {
            id: CmdId(0x0890),
            dir: PagedDir::Get
        })
    ));
    // UPMIX_SET_CONFIG / UPMIX_GET_CONFIG fold into FX.UPMIX_CONFIG (44 bytes).
    assert_eq!(req(&plan_out(0x4A, 0, &[0; 44])).id, CmdId(0x0880));
    assert_eq!(req(&plan_in(0x4E, 0, 16)).id, CmdId(0x0881));

    // RTA_GET_CAPS: 0 -> MEAS.RTA_CAPS, n -> MEAS.RTA_BAND_CENTRES with n - 1.
    let caps = plan_in(0x0A, 0, 16);
    assert_eq!((req(&caps).id, req(&caps).value), (CmdId(0x0921), 0));
    let chunk = plan_in(0x0A, 2, 64);
    assert_eq!((req(&chunk).id, req(&chunk).value), (CmdId(0x0922), 1));

    // GET_CS_CAPS, SIGGEN_GET_CAPS, GET_CS_AUX_STATE: 0xFFFF picks the table read.
    for (op, all, one) in [
        (0x86u8, 0x0A10u16, 0x0A11u16),
        (0xA8, 0x0942, 0x0943),
        (0x05, 0x0A62, 0x0A60),
    ] {
        let a = plan_in(op, 0xFFFF, 8);
        assert_eq!((req(&a).id, req(&a).value), (CmdId(all), 0), "0x{op:02X}");
        let o = plan_in(op, 3, 8);
        assert_eq!((req(&o).id, req(&o).value), (CmdId(one), 3), "0x{op:02X}");
    }

    // OUTPUT_LATENCY 0x03: OUT -> OUTPUT.LATENCY; IN 0 / 1 -> status / options.
    let set = plan_out(0x03, 0, &[2]);
    assert_eq!(
        setup(&set),
        [0x41, 0x20, 0x00, 0x00, 0x02, 0x04, 0x01, 0x00]
    );
    assert_eq!(req(&plan_in(0x03, 0, 12)).id, CmdId(0x0421));
    assert_eq!(req(&plan_in(0x03, 1, 56)).id, CmdId(0x0422));

    // RTA_CONTROL 0x0E and SIGGEN_CONTROL 0xA6.
    for (action, rta, sig) in [
        (0u16, 0x0929u16, 0x0949u16),
        (1, 0x0928, 0x0948),
        (2, 0x092A, 0x094A),
    ] {
        let r = plan_in(0x0E, action, 1);
        assert_eq!(
            (req(&r).id, req(&r).body.clone()),
            (CmdId(rta), PagedBody::Set(vec![]))
        );
        let s = plan_in(0xA6, action, 1);
        assert_eq!((req(&s).id, req(&s).value), (CmdId(sig), 0));
    }
    assert!(plan(0xA6, 3, LegacyBody::In { len: 1 }, &rp2350()).is_err());

    // CS_IR_LEARN 0x8F: 0 cancel, 1 arm (zero-length SETs), 2 the status GET.
    assert_eq!(req(&plan_in(0x8F, 0, 1)).id, CmdId(0x0A33));
    assert_eq!(req(&plan_in(0x8F, 1, 1)).id, CmdId(0x0A32));
    let st = plan_in(0x8F, 2, 8);
    assert_eq!(
        (req(&st).id, req(&st).body.clone()),
        (CmdId(0x0A31), PagedBody::Get { len: 8 })
    );

    // CS_MACRO_FIRE 0x25: a macro, or 0xFFFF to cancel.
    let fire = plan_in(0x25, 5, 1);
    assert_eq!((req(&fire).id, req(&fire).value), (CmdId(0x0A58), 5));
}

/// Guide 6.6, the former write-as-reads, and its worked examples.
#[test]
fn wp_pgproto_guide_6_6_write_as_reads() {
    // Worked example 1: SET_OUTPUT_TYPE slot 0 type 5, C1 C0 00 05 02 00 01 00.
    let p = plan_in(0xC0, 0x0500, 1);
    assert_eq!(setup(&p), [0x41, 0x10, 0x00, 0x00, 0x02, 0x04, 0x01, 0x00]);
    assert_eq!(req(&p).body, PagedBody::Set(vec![0x05]));
    let last = LastResult::decode(&[0x10, 0x04, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00]).unwrap();
    assert_eq!(
        (last.id, last.dir, last.result),
        (CmdId(0x0410), Some(PagedDir::Set), VC_ERR_VALUE)
    );
    assert!(p.needs_result());
    // "0x01 for VALUE or PIN_INVALID".
    assert_eq!(
        outcome(
            p,
            Answer::Refused {
                result: Some(VC_ERR_VALUE)
            }
        ),
        Outcome::Reply(vec![0x01])
    );

    // Worked example 2: CS_MACRO_FIRE 0xFFFF -> CS.MACRO_CANCEL, 41 59 00 00 02 0A 00 00.
    let p = plan_in(0x25, 0xFFFF, 1);
    assert_eq!(setup(&p), [0x41, 0x59, 0x00, 0x00, 0x02, 0x0A, 0x00, 0x00]);
    assert_eq!(outcome(p, Answer::Ack), Outcome::Reply(vec![0x01]));

    // PRESET_LOAD slot 3: 41 11 03 00 02 02 00 00; slot 10 refused with ADDRESS -> 0x01.
    let p = plan_in(0x91, 3, 1);
    assert_eq!(setup(&p), [0x41, 0x11, 0x03, 0x00, 0x02, 0x02, 0x00, 0x00]);
    assert_eq!(outcome(p, Answer::Ack), Outcome::Reply(vec![0x00]));
    let p = plan_in(0x91, 10, 1);
    assert_eq!(setup(&p), [0x41, 0x11, 0x0A, 0x00, 0x02, 0x02, 0x00, 0x00]);
    assert_eq!(
        outcome(
            p,
            Answer::Refused {
                result: Some(VC_ERR_ADDRESS)
            }
        ),
        Outcome::Reply(vec![0x01])
    );

    // Table rows: (opcode, old wValue, id, new wValue, payload).
    let rows: &[(u8, u16, u16, u16, &[u8])] = &[
        (0x52, 0, 0x0221, 0, &[]),
        (0x53, 0, 0x0230, 0, &[]),
        (0x68, 0x0001, 0x0340, 0, &[0x01]),
        (0x7C, 0x0702, 0x0411, 2, &[0x07]),
        (0x90, 0x0004, 0x0210, 4, &[]),
        (0x92, 0x0009, 0x0212, 9, &[]),
        (0x9D, 0, 0x0A18, 0, &[]),
        (0x9E, 0, 0x0A19, 0, &[]),
        (0xC2, 0x0115, 0x0C10, 1, &[0x15]),
        (0xC4, 0x0200, 0x0C20, 0, &[0x01]),
        (0xC4, 0x0000, 0x0C20, 0, &[0x00]),
        (0xC8, 0x0100, 0x0C22, 0, &[0xFF]),
        (0xC8, 0x0080, 0x0C22, 0, &[0x80]),
        (0xD6, 0, 0x0552, 0, &[]),
        (0xE4, 0x0203, 0x0322, 2, &[0x03]),
        (0xE9, 0x0105, 0x0323, 1, &[0x01]),
        (0xEC, 0, 0x0441, 0, &[]),
        (0xF0, 0, 0x0130, 0, &[]),
        (0xF1, 0x0310, 0x0330, 3, &[0x10]),
        (0xF3, 0x0004, 0x0331, 0, &[0x04]),
        (0xFE, 0x0001, 0x0C11, 0, &[0x01]),
    ];
    for (op, old, id, value, data) in rows {
        let p = plan_in(*op, *old, 1);
        let r = req(&p);
        assert_eq!(
            (r.id, r.value, r.body.clone()),
            (CmdId(*id), *value, PagedBody::Set(data.to_vec())),
            "0x{op:02X} wValue 0x{old:04X}"
        );
    }

    // Status bytes: TEST_DAC_HW_MUTE STATE -> 0x03 (its override); SET_ADAT_INPUT_ENABLE
    // STATE -> 0x02 (its override); PRESET_* UNKNOWN is unmapped: the STALL stays.
    let dac = plan_in(0xEC, 0, 1);
    assert_eq!(
        outcome(
            dac,
            Answer::Refused {
                result: Some(VC_ERR_STATE)
            }
        ),
        Outcome::Reply(vec![0x03])
    );
    let adat = plan_in(0x68, 1, 1);
    assert_eq!(
        outcome(
            adat,
            Answer::Refused {
                result: Some(VC_ERR_STATE)
            }
        ),
        Outcome::Reply(vec![0x02])
    );
    let save = plan_in(0x90, 1, 1);
    assert_eq!(
        outcome(
            save,
            Answer::Refused {
                result: Some(VC_ERR_UNKNOWN)
            }
        ),
        Outcome::Stall {
            result: Some(VC_ERR_UNKNOWN)
        }
    );
    // ack1: every refusal stays a STALL, even with a known result.
    let boot = plan_in(0xF0, 0, 1);
    assert!(!boot.needs_result());
    assert_eq!(
        outcome(
            boot,
            Answer::Refused {
                result: Some(VC_ERR_STATE)
            }
        ),
        Outcome::Stall {
            result: Some(VC_ERR_STATE)
        }
    );
}

/// Guide 7.1: reply lengths of the registry (a sample transcribed from the table).
#[test]
fn wp_pgproto_guide_7_1_reply_lengths() {
    let rows: &[(&str, u16, Option<u16>)] = &[
        ("SYS.PROTOCOL_INFO", 0x0110, Some(16)),
        ("SYS.PAGE_LIST", 0x0111, None),
        ("SYS.PAGE_COMMANDS", 0x0112, Some(64)),
        ("SYS.COMMAND_INFO", 0x0113, Some(8)),
        ("SYS.LAST_RESULT", 0x0114, Some(8)),
        ("SYS.PLATFORM", 0x0120, Some(7)),
        ("SYS.SERIAL", 0x0121, Some(16)),
        ("SYS.BUILD_INFO", 0x0122, Some(64)),
        ("SYS.DEVICE_NAME", 0x0124, Some(32)),
        ("SYS.SYSTEM_STATUS", 0x0140, Some(16)),
        ("SYS.BUFFER_STATS", 0x0142, Some(44)),
        ("SYS.PIN_STATE", 0x0150, Some(24)),
        ("SYS.PIN_CLAIMS", 0x0151, None),
        ("STORE.PRESET_NAME", 0x0213, Some(32)),
        ("STORE.DIRECTORY", 0x0215, Some(7)),
        ("STORE.ALL_PARAMS", 0x0240, Some(6136)),
        ("STORE.ALL_PARAMS_CHUNK", 0x0241, None),
        ("INPUT.STATUS", 0x0311, Some(12)),
        ("INPUT.SPDIF_RX_CH_STATUS", 0x0321, Some(24)),
        ("FX.UPMIX_STATUS", 0x0881, Some(16)),
        ("FX.UPMIX_ENABLE", 0x0890, Some(4)),
        ("CS.CAPS", 0x0A10, Some(52)),
        ("CS.STATUS", 0x0A12, Some(41)),
        ("CS.BINDING_PIN_AVAIL", 0x0A22, Some(80)),
        ("CLOCK.MCK_PIN_AVAIL", 0x0C23, Some(24)),
        ("DIAG.CLOCK_DIAG", 0xF010, None),
        ("DIAG.LOOPBACK_STATS", 0xF020, Some(8)),
        ("DIAG.LOOP_PROF_BG_PACE", 0xF033, Some(2)),
    ];
    for (name, id, len) in rows {
        let c = registry::by_name(name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(c.id, CmdId(*id), "{name}");
        let want = match len {
            Some(n) => GetLen::Fixed(*n),
            None => GetLen::Var,
        };
        assert_eq!(c.get_len, want, "{name}");
    }
    // Build conditions (guide 4.8).
    assert_eq!(
        registry::by_name("FX.UPMIX_CONFIG").unwrap().build,
        Some("PICO_RP2350")
    );
    assert_eq!(
        registry::by_name("DIAG.LOOPBACK_STATS").unwrap().build,
        Some("DSPI_LOOPBACK")
    );
}

/// The reverse view on the guide's examples.
#[test]
fn wp_pgproto_guide_reverse_examples() {
    let gain = legacy_equivalent(CmdId(0x0521), PagedDir::Set, 1, &[0, 0, 0xC0, 0xC0]).unwrap();
    assert_eq!(
        (gain.opcode, gain.value, gain.data),
        (0x74, 1, vec![0, 0, 0xC0, 0xC0])
    );
    let ty = legacy_equivalent(CmdId(0x0410), PagedDir::Set, 0, &[5]).unwrap();
    assert_eq!((ty.opcode, ty.value, ty.action), (0xC0, 0x0500, true));
    let cancel = legacy_equivalent(CmdId(0x0A59), PagedDir::Set, 0, &[]).unwrap();
    assert_eq!((cancel.opcode, cancel.value), (0x25, 0xFFFF));
    let band = legacy_equivalent(CmdId(0x0922), PagedDir::Get, 1, &[]).unwrap();
    assert_eq!((band.opcode, band.value), (0x0A, 2));
    // PLAN-PG PGHPOL: ROUTE.MASTER_VOLUME_SAVED GET 0x0553 is a read, never FACTORY_RESET.
    let saved = legacy_equivalent(CmdId(0x0553), PagedDir::Get, 0, &[]);
    assert_ne!(saved.as_ref().map(|s| s.opcode), Some(0x53));
    assert!(saved.is_none_or(|s| !s.action));
    // New in 1.1.7, no 1.1.6 form.
    assert_eq!(
        legacy_equivalent(CmdId(0x0150), PagedDir::Get, 0, &[]),
        None
    );
    assert_eq!(
        legacy_equivalent(CmdId(0x0110), PagedDir::Get, 0, &[]),
        None
    );
}
