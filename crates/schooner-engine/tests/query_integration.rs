//! End-to-end integration coverage for the Query stack.
//!
//! Crate-internal unit tests cover each layer in isolation. These
//! tests exercise the *combination* — Schedule + Query + change
//! detection + filters — in shapes that mirror what Phase F's
//! renderer and Phase G's controller will need:
//!
//! - `(&Transform, &MeshHandle)` read-only join with a marker filter
//!   (`Without<Hidden>`) — the renderer's draw-list shape.
//! - `(WriteOnly<Transform>, &FpsController)` mutable + read tuple,
//!   driven from a system that consumes `Res<Input>` and `Res<Time>`
//!   — the controller's shape.
//! - Change detection across schedule ticks: a system that mutates
//!   via `Query<WriteOnly<T>>` shows up in `world.changed_since` for
//!   downstream systems to react to.
//!
//! Stand-in component types live here so we don't depend on
//! components that haven't been built yet (the real `Transform`,
//! `MeshHandle`, etc. land in Phase F/G).

use schooner_engine::ecs::{Query, Res, ResMut, Schedule, Stage, Without, World, WriteOnly};

// --- stand-in components that mirror the upcoming engine surface ----

#[derive(Debug, Clone, Copy, PartialEq)]
struct Transform {
    x: f32,
    y: f32,
    z: f32,
}

impl Transform {
    fn at(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MeshHandle(u32);

#[derive(Debug, PartialEq)]
struct Hidden;

#[derive(Debug, PartialEq)]
struct FpsController {
    speed: f32,
}

// --- stand-in resources ----------------------------------------------

#[derive(Debug, Default)]
struct Input {
    forward: f32,
}

#[derive(Debug, Default)]
struct Time {
    delta_secs: f32,
}

#[derive(Debug, Default)]
struct DrawList {
    items: Vec<(Transform, MeshHandle)>,
}

// --- tests -----------------------------------------------------------

#[test]
fn renderer_shaped_query_collects_transform_meshhandle_pairs() {
    let mut world = World::new();
    let a = world.spawn();
    let b = world.spawn();
    let c = world.spawn(); // No mesh — must be excluded.
    world.insert(a, Transform::at(1.0, 0.0, 0.0));
    world.insert(a, MeshHandle(10));
    world.insert(b, Transform::at(0.0, 2.0, 0.0));
    world.insert(b, MeshHandle(20));
    world.insert(c, Transform::at(0.0, 0.0, 3.0));

    world.insert_resource(DrawList::default());

    let mut sched = Schedule::new();
    sched.add_system(
        &mut world,
        Stage::Update,
        |mut list: ResMut<DrawList>, q: Query<(&Transform, &MeshHandle)>| {
            for (t, m) in q {
                list.items.push((*t, *m));
            }
        },
    );
    sched.run(&mut world);

    let list = world.resource::<DrawList>().unwrap();
    let mut got = list.items.clone();
    got.sort_by_key(|(_, m)| m.0);
    assert_eq!(
        got,
        vec![
            (Transform::at(1.0, 0.0, 0.0), MeshHandle(10)),
            (Transform::at(0.0, 2.0, 0.0), MeshHandle(20)),
        ]
    );
}

#[test]
fn renderer_shaped_query_with_hidden_filter_excludes_marked_entities() {
    let mut world = World::new();
    let visible = world.spawn();
    let hidden = world.spawn();
    world.insert(visible, Transform::at(1.0, 0.0, 0.0));
    world.insert(visible, MeshHandle(7));
    world.insert(hidden, Transform::at(2.0, 0.0, 0.0));
    world.insert(hidden, MeshHandle(8));
    world.insert(hidden, Hidden);

    world.insert_resource(DrawList::default());

    let mut sched = Schedule::new();
    sched.add_system(
        &mut world,
        Stage::Update,
        |mut list: ResMut<DrawList>, q: Query<(&Transform, &MeshHandle), Without<Hidden>>| {
            for (t, m) in q {
                list.items.push((*t, *m));
            }
        },
    );
    sched.run(&mut world);

    let list = world.resource::<DrawList>().unwrap();
    assert_eq!(
        list.items,
        vec![(Transform::at(1.0, 0.0, 0.0), MeshHandle(7))]
    );
}

#[test]
fn controller_shaped_query_writes_transform_from_input_and_time() {
    let mut world = World::new();
    let player = world.spawn();
    world.insert(player, Transform::at(0.0, 0.0, 0.0));
    world.insert(player, FpsController { speed: 5.0 });

    world.insert_resource(Input { forward: 1.0 });
    world.insert_resource(Time { delta_secs: 0.1 });

    let mut sched = Schedule::new();
    sched.add_system(
        &mut world,
        Stage::Update,
        |input: Res<Input>, time: Res<Time>, q: Query<(WriteOnly<Transform>, &FpsController)>| {
            for (mut t, ctrl) in q {
                // forward * speed * delta — mirrors the eventual
                // `fps_move` system shape.
                t.z += input.forward * ctrl.speed * time.delta_secs;
            }
        },
    );
    sched.run(&mut world);
    sched.run(&mut world); // Two ticks → +1.0 z.

    let t = world.get::<Transform>(player).unwrap();
    assert!((t.z - 1.0).abs() < 1e-5, "expected z≈1.0, got {}", t.z);
}

#[test]
fn write_through_query_is_visible_in_changed_since() {
    let mut world = World::new();
    let a = world.spawn();
    let b = world.spawn();
    world.insert(a, Transform::at(0.0, 0.0, 0.0));
    world.insert(b, Transform::at(0.0, 0.0, 0.0));

    let baseline = world.current_tick();

    let mut sched = Schedule::new();
    sched.add_system(
        &mut world,
        Stage::Update,
        |q: Query<WriteOnly<Transform>>| {
            for mut t in q {
                t.x += 1.0;
            }
        },
    );
    sched.run(&mut world);

    let mut changed: Vec<u32> = world
        .changed_since::<Transform>(baseline)
        .map(|(e, _)| e.index)
        .collect();
    changed.sort();
    let mut expected = vec![a.index, b.index];
    expected.sort();
    assert_eq!(changed, expected);
}

#[test]
fn multi_system_pipeline_composes_through_world_state() {
    // Tick 1 sets up positions via WriteOnly<Transform>; the
    // renderer system reads them into a draw list. Both systems run
    // in the same Update stage, exercising the in-stage state-flow.
    let mut world = World::new();
    let e = world.spawn();
    world.insert(e, Transform::at(0.0, 0.0, 0.0));
    world.insert(e, MeshHandle(42));

    world.insert_resource(Input { forward: 2.0 });
    world.insert_resource(DrawList::default());

    let mut sched = Schedule::new();
    sched
        .add_system(
            &mut world,
            Stage::Update,
            |input: Res<Input>, q: Query<WriteOnly<Transform>>| {
                for mut t in q {
                    t.z = input.forward;
                }
            },
        )
        .add_system(
            &mut world,
            Stage::Update,
            |mut list: ResMut<DrawList>, q: Query<(&Transform, &MeshHandle)>| {
                list.items.clear();
                for (t, m) in q {
                    list.items.push((*t, *m));
                }
            },
        );
    sched.run(&mut world);

    let list = world.resource::<DrawList>().unwrap();
    assert_eq!(
        list.items,
        vec![(Transform::at(0.0, 0.0, 2.0), MeshHandle(42))]
    );
}
