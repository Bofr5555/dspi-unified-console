//! WP FEAT2: a room's or zone's volume, the maths (the hub's side is tested
//! with dspi-mcp's in-process hub).

use serde_json::json;

use dspi_session::system_model::SystemModel;

use super::system_volume::{
    SystemVolumeAsk, SystemVolumeKind, SystemVolumePlan, SystemVolumeState, units_of, weigh,
};
use crate::{DspiCore, DspiError};

fn model() -> SystemModel {
    serde_json::from_value(json!({
        "rooms": [{"id": "bar", "name": "Bar"}, {"id": "hall"}, {"id": "empty"}],
        "zones": [{"id": "inside", "name": "Inside", "rooms": ["bar", "hall"]},
                  {"id": "none", "rooms": ["empty"]}],
        "senders": [{"id": "tx", "serial": "S-TX"}],
        "units": [
            {"id": "a", "name": "Left", "serial": "S-A", "room": "bar", "upstream": "tx"},
            {"id": "b", "serial": "S-B", "room": "bar", "upstream": "tx"},
            {"id": "c", "serial": "S-C", "room": "hall", "upstream": "tx"}
        ]
    }))
    .unwrap()
}

fn ask(room: &str, zone: &str, level: Option<f32>, step: Option<f32>) -> SystemVolumeAsk {
    SystemVolumeAsk {
        room: room.into(),
        zone: zone.into(),
        level_db: level,
        step_db: step,
        ..SystemVolumeAsk::default()
    }
}

#[test]
fn wp_feat2_the_units_of_a_room_and_a_zone() {
    let m = model();
    let p = units_of(&m, &ask("bar", "", Some(-20.0), None)).unwrap();
    let serials: Vec<&str> = p.units.iter().map(|u| &*u.serial).collect();
    assert_eq!(serials, ["S-A", "S-B"]);
    assert_eq!((&*p.name, p.number, p.of), ("Bar", 1, 3));
    assert_eq!((&*p.units[0].name, &*p.units[1].name), ("Left", "b"));
    let p = units_of(&m, &ask("", "inside", Some(-20.0), None)).unwrap();
    assert_eq!(p.units.len(), 3);
    assert_eq!((&*p.name, p.number, p.of), ("Inside", 1, 2));
    for bad in [
        ask("attic", "", None, None),
        ask("", "attic", None, None),
        // A room with no units, a zone of such rooms, a zone's id as a room.
        ask("empty", "", None, None),
        ask("", "none", None, None),
        ask("inside", "", None, None),
    ] {
        assert!(matches!(
            units_of(&m, &bad),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
}

fn weighed(a: &SystemVolumeAsk, before: f32) -> (Option<f32>, bool, SystemVolumeState) {
    let mut u = units_of(&model(), &ask("hall", "", None, None))
        .unwrap()
        .units
        .remove(0);
    weigh(a, &mut u, before);
    assert_eq!(u.before_db, Some(before));
    (u.target_db, u.louder, u.state)
}

#[test]
fn wp_feat2_levels_and_steps_stay_in_range_and_know_louder() {
    use SystemVolumeState::{Ready, Refused, Unchanged};
    let level = |l| ask("hall", "", Some(l), None);
    let step = |s| ask("hall", "", None, Some(s));
    assert_eq!(weighed(&level(-20.0), -30.0), (Some(-20.0), true, Ready));
    assert_eq!(weighed(&level(-40.0), -30.0), (Some(-40.0), false, Ready));
    assert_eq!(
        weighed(&level(-30.0), -30.0),
        (Some(-30.0), false, Unchanged)
    );
    assert_eq!(weighed(&step(3.0), -30.0), (Some(-27.0), true, Ready));
    assert_eq!(weighed(&step(-3.0), -30.0), (Some(-33.0), false, Ready));
    // At the range's ends.
    assert_eq!(weighed(&step(10.0), -4.0), (Some(0.0), true, Ready));
    assert_eq!(weighed(&step(10.0), 0.0), (Some(0.0), false, Unchanged));
    assert_eq!(weighed(&step(-10.0), -55.0), (Some(-60.0), false, Ready));
    // The master volume: 0.5 dB steps, never to its mute, no step from it.
    let master = |level, step| SystemVolumeAsk {
        volume: SystemVolumeKind::Master,
        ..ask("hall", "", level, step)
    };
    assert_eq!(
        weighed(&master(Some(-20.2), None), -30.0),
        (Some(-20.0), true, Ready)
    );
    assert_eq!(
        weighed(&master(None, Some(-60.0)), -100.0),
        (Some(-127.0), false, Ready)
    );
    assert_eq!(
        weighed(&master(None, Some(3.0)), -128.0),
        (None, false, Refused)
    );
    assert_eq!(
        weighed(&master(Some(-40.0), None), -128.0),
        (Some(-40.0), true, Ready)
    );
}

#[test]
fn wp_feat2_an_ask_or_a_plan_that_is_not_one_is_refused_before_anything_is_sent() {
    let core = DspiCore::new();
    for bad in [
        ask("", "", Some(-20.0), None),
        ask("bar", "inside", Some(-20.0), None),
        ask("bar", "", None, None),
        ask("bar", "", Some(-20.0), Some(1.0)),
        ask("bar", "", Some(0.1), None),
        ask("bar", "", Some(-60.1), None),
        ask("bar", "", Some(f32::NAN), None),
        ask("bar", "", None, Some(60.1)),
        ask("bar", "", None, Some(f32::INFINITY)),
        SystemVolumeAsk {
            volume: SystemVolumeKind::Master,
            ..ask("bar", "", Some(-128.0), None)
        },
    ] {
        assert!(
            matches!(
                core.system_volume_plan(bad.clone()),
                Err(DspiError::InvalidArgument { .. })
            ),
            "{bad:?}"
        );
    }
    assert_eq!(
        core.system_volume_plan(ask("bar", "", Some(-20.0), None)),
        Err(DspiError::Disconnected)
    );
    let mut plan = units_of(&model(), &ask("bar", "", Some(-20.0), None)).unwrap();
    let a = plan.ask.clone();
    for u in &mut plan.units {
        weigh(&a, u, -30.0);
    }
    assert!(plan.louder());
    let mut loud = plan.clone();
    loud.units[1].target_db = Some(3.0);
    let mut twice = plan.clone();
    twice.units[1].serial = "S-A".into();
    for bad in [loud, twice] {
        assert!(matches!(
            core.system_volume_set(bad),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
    // Nothing ready: nothing sent, the plan back as it was.
    for u in &mut plan.units {
        u.refuse("offline", "not attached");
    }
    assert_eq!(core.system_volume_set(plan.clone()), Ok(plan.clone()));
    // JSON both ways, as the C ABI and the web take it.
    let text = serde_json::to_string(&plan).unwrap();
    assert_eq!(
        serde_json::from_str::<SystemVolumePlan>(&text).unwrap(),
        plan
    );
}
