//! ECS scenario benchmarks.
//!
//! Each scenario is generic over [`BenchEcs`]. To add a second
//! implementation (e.g. an archetype variant), implement `BenchEcs`
//! for it and add a `register::<NewImpl>(c)` line in `main`.
//!
//! ## Scenario set
//!
//! | scenario                  | what it stresses                                      |
//! |---------------------------|-------------------------------------------------------|
//! | `bulk_spawn`              | Spawn N entities + insert 2 components each           |
//! | `iterate_pos_vel`         | Hot iteration: `(WriteOnly<Pos>, &Vel)` join          |
//! | `iterate_pos_mut`         | Mutable single-component iter (Mut<T> tick-bump cost) |
//! | `random_lookup`           | Random `world.get::<Pos>(entity)`                     |
//! | `insert_remove_churn`     | Repeated insert+remove on Pos (sparse swap-remove)    |
//! | `fragmented_iteration`    | `(WriteOnly<Pos>, &Vel)` when only 1/8 carries Vel    |
//! | `query_with_filter`       | `&Pos` filtered by `Without<Tag>` over half-tagged set|
//!
//! Each parameterised over 1k / 10k / 100k entities. Run via
//! `cargo bench -p bench-ecs`.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

use bench_ecs::{BenchEcs, Bulk, Pos, Vel, schooner_v1::SchoonerV1};

const SIZES: &[usize] = &[1_000, 10_000, 100_000];

// --- helpers ----------------------------------------------------------

/// Build a world with N entities each carrying Pos + Vel.
fn world_pos_vel<E: BenchEcs>(n: usize) -> (E::World, Vec<E::Entity>) {
    let mut world = E::new_world();
    let mut entities = Vec::with_capacity(n);
    for i in 0..n {
        let e = E::spawn(&mut world);
        E::insert_pos(
            &mut world,
            e,
            Pos {
                x: i as f32,
                y: 0.0,
                z: 0.0,
            },
        );
        E::insert_vel(
            &mut world,
            e,
            Vel {
                x: 0.5,
                y: 0.0,
                z: 0.0,
            },
        );
        entities.push(e);
    }
    (world, entities)
}

// --- scenarios --------------------------------------------------------

fn bench_bulk_spawn<E: BenchEcs>(c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("{}/bulk_spawn", E::name()));
    for &n in SIZES {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut world = E::new_world();
                for i in 0..n {
                    let e = E::spawn(&mut world);
                    E::insert_pos(
                        &mut world,
                        e,
                        Pos {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                    );
                    E::insert_vel(
                        &mut world,
                        e,
                        Vel {
                            x: 0.5,
                            y: 0.0,
                            z: 0.0,
                        },
                    );
                }
                black_box(world)
            });
        });
    }
    group.finish();
}

fn bench_iterate_pos_vel<E: BenchEcs>(c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("{}/iterate_pos_vel", E::name()));
    for &n in SIZES {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let (mut world, _) = world_pos_vel::<E>(n);
            b.iter(|| {
                E::iterate_pos_vel(&mut world, &mut |p, v| {
                    p.x += v.x;
                    p.y += v.y;
                    p.z += v.z;
                });
                black_box(&world);
            });
        });
    }
    group.finish();
}

fn bench_iterate_pos_mut<E: BenchEcs>(c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("{}/iterate_pos_mut", E::name()));
    for &n in SIZES {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let (mut world, _) = world_pos_vel::<E>(n);
            b.iter(|| {
                E::iterate_pos_mut(&mut world, &mut |p| {
                    p.x += 1.0;
                });
                black_box(&world);
            });
        });
    }
    group.finish();
}

fn bench_random_lookup<E: BenchEcs>(c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("{}/random_lookup", E::name()));
    for &n in SIZES {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let (world, entities) = world_pos_vel::<E>(n);
            // Deterministic pseudo-random index pattern (no RNG dep).
            // LCG step keeps the access pattern non-trivial enough to
            // defeat sequential cache prefetch.
            b.iter(|| {
                let mut idx: u64 = 0;
                let mut acc = 0.0f32;
                for _ in 0..n {
                    idx = idx.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let pick = (idx as usize) % entities.len();
                    if let Some(p) = E::get_pos(&world, entities[pick]) {
                        acc += p.x;
                    }
                }
                black_box(acc)
            });
        });
    }
    group.finish();
}

fn bench_insert_remove_churn<E: BenchEcs>(c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("{}/insert_remove_churn", E::name()));
    for &n in SIZES {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            // Pre-spawn N empty entities so each iteration churns
            // Pos insert+remove against the same pool — measures the
            // sparse swap-remove + dense compaction cost without
            // also paying the entity-allocation cost each loop.
            let mut world = E::new_world();
            let entities: Vec<_> = (0..n).map(|_| E::spawn(&mut world)).collect();
            b.iter(|| {
                for &e in &entities {
                    E::insert_pos(&mut world, e, Pos::default());
                }
                for &e in &entities {
                    E::remove_pos(&mut world, e);
                }
                black_box(&world);
            });
        });
    }
    group.finish();
}

fn bench_fragmented_iteration<E: BenchEcs>(c: &mut Criterion) {
    // Pos covers all N entities; Vel covers only 1/8 of them.
    // Sparse-set picks Vel as the driver (smallest set), so the
    // per-driver-entity cost is the Pos probe.
    let mut group = c.benchmark_group(format!("{}/fragmented_iteration", E::name()));
    for &n in SIZES {
        group.throughput(Throughput::Elements((n / 8).max(1) as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let mut world = E::new_world();
            for i in 0..n {
                let e = E::spawn(&mut world);
                E::insert_pos(&mut world, e, Pos::default());
                if i % 8 == 0 {
                    E::insert_vel(&mut world, e, Vel::default());
                }
            }
            b.iter(|| {
                E::iterate_pos_vel(&mut world, &mut |p, v| {
                    p.x += v.x;
                });
                black_box(&world);
            });
        });
    }
    group.finish();
}

fn bench_query_with_filter<E: BenchEcs>(c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("{}/query_with_filter", E::name()));
    for &n in SIZES {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let mut world = E::new_world();
            for i in 0..n {
                let e = E::spawn(&mut world);
                E::insert_pos(&mut world, e, Pos::default());
                if i % 2 == 0 {
                    E::insert_tag(&mut world, e);
                }
            }
            b.iter(|| {
                let mut acc = 0.0f32;
                E::iterate_pos_without_tag(&mut world, &mut |p| {
                    acc += p.x;
                });
                black_box(acc);
            });
        });
    }
    group.finish();
}

fn bench_bulk_component_iteration<E: BenchEcs>(c: &mut Criterion) {
    // Same shape as `iterate_pos_vel` but each entity also carries a
    // 256-byte `Bulk` component to inflate the sparse-set's cache
    // footprint. The bulk is inserted but NOT iterated — it lives in
    // a separate sparse-set, so it shouldn't affect Pos/Vel iteration
    // cost in this storage model. Useful as a control to verify that
    // (the sparse-set's per-component column layout pays off).
    let mut group = c.benchmark_group(format!("{}/iterate_with_bulk_present", E::name()));
    for &n in SIZES {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let mut world = E::new_world();
            for i in 0..n {
                let e = E::spawn(&mut world);
                E::insert_pos(
                    &mut world,
                    e,
                    Pos {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                );
                E::insert_vel(&mut world, e, Vel::default());
                E::insert_bulk(&mut world, e, Bulk::default());
            }
            b.iter(|| {
                E::iterate_pos_vel(&mut world, &mut |p, v| {
                    p.x += v.x;
                });
                black_box(&world);
            });
        });
    }
    group.finish();
}

// --- registration -----------------------------------------------------

/// Register every scenario for one ECS impl. To compare a new impl,
/// add a `register::<NewImpl>(c)` call in `all_benches`.
fn register<E: BenchEcs>(c: &mut Criterion) {
    bench_bulk_spawn::<E>(c);
    bench_iterate_pos_vel::<E>(c);
    bench_iterate_pos_mut::<E>(c);
    bench_random_lookup::<E>(c);
    bench_insert_remove_churn::<E>(c);
    bench_fragmented_iteration::<E>(c);
    bench_query_with_filter::<E>(c);
    bench_bulk_component_iteration::<E>(c);
}

fn all_benches(c: &mut Criterion) {
    register::<SchoonerV1>(c);
    // When an archetype impl exists:
    // register::<ArchetypeV1>(c);
}

criterion_group!(benches, all_benches);
criterion_main!(benches);
