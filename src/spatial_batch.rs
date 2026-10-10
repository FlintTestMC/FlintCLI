//! Batch layout helpers: simulation-distance limits and test focus positions.

use flint_core::spatial::calculate_test_offsets_for_batch_default;
use flint_core::test_spec::TestSpec;

const CHUNK_WIDTH: i32 = 16;
const SIMULATION_MARGIN_CHUNKS: u32 = 2;
const X_OFFSET_MOD_TAG_PREFIX: &str = "x-offset-mod-";
const Z_OFFSET_TAG_PREFIX: &str = "z-offset=";

/// Calculate the normal compact layout, then move position-sensitive tests to the
/// nearest coordinates in their declared safe location zone.
pub fn calculate_test_offsets(tests: &[TestSpec]) -> Vec<[i32; 3]> {
    if let [test] = tests
        && exact_z_offset(test).is_none()
        && x_offset_constraint(test).is_none()
    {
        return vec![[0, 0, 0]];
    }

    tests
        .iter()
        .zip(calculate_test_offsets_for_batch_default(tests))
        .map(|(test, mut offset)| {
            if let Some(z) = exact_z_offset(test) {
                offset[2] = z;
            }
            if let Some((modulus, phase)) = x_offset_constraint(test) {
                offset[0] = nearest_congruent(offset[0], phase, modulus);
            }
            offset
        })
        .collect()
}

pub fn pair_tests_with_offsets(tests: Vec<TestSpec>) -> Vec<(TestSpec, [i32; 3])> {
    let offsets = calculate_test_offsets(&tests);
    tests.into_iter().zip(offsets).collect()
}

fn x_offset_constraint(test: &TestSpec) -> Option<(i32, i32)> {
    test.tags.iter().find_map(|tag| {
        let (modulus, phase) = tag.strip_prefix(X_OFFSET_MOD_TAG_PREFIX)?.split_once('=')?;
        let modulus = modulus.parse::<i32>().ok()?;
        let phase = phase.parse::<i32>().ok()?;
        (modulus > 0).then_some((modulus, phase.rem_euclid(modulus)))
    })
}

fn exact_z_offset(test: &TestSpec) -> Option<i32> {
    test.tags
        .iter()
        .find_map(|tag| tag.strip_prefix(Z_OFFSET_TAG_PREFIX)?.parse().ok())
}

fn nearest_congruent(value: i32, phase: i32, modulus: i32) -> i32 {
    let lower = value - (value - phase).rem_euclid(modulus);
    let upper = lower + modulus;
    if value - lower <= upper - value {
        lower
    } else {
        upper
    }
}

/// Partition tests by their resolved world configuration while preserving the order in
/// which each configuration and test first appeared.
pub fn group_tests_by_world_config(tests: Vec<TestSpec>) -> Vec<Vec<TestSpec>> {
    let mut groups: Vec<Vec<TestSpec>> = Vec::new();
    for test in tests {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group[0].world_config() == test.world_config())
        {
            group.push(test);
        } else {
            groups.push(vec![test]);
        }
    }
    groups
}

/// Blocks from world origin (0, 0) that can still be simulated when the bot stands at
/// the layout center. Reserves two chunks of margin for the player and chunk edges.
pub fn simulation_radius_blocks(simulation_distance: u32) -> i32 {
    (simulation_distance.saturating_sub(SIMULATION_MARGIN_CHUNKS) as i32) * CHUNK_WIDTH
}

/// Maximum Chebyshev distance from origin for any corner of any test region in a batch.
pub fn max_extent_from_origin(tests: &[(TestSpec, [i32; 3])]) -> i32 {
    let mut max = 0;
    for (test, offset) in tests {
        let region = test.cleanup_region();
        for corner in [region[0], region[1]] {
            let wx = offset[0] + corner[0];
            let wz = offset[2] + corner[2];
            max = max.max(wx.abs().max(wz.abs()));
        }
    }
    max
}

/// Maximum horizontal span occupied by a batch. When a timeline teleports the shared
/// player, every test region must remain within simulation distance of that new focus.
pub fn max_horizontal_span(tests: &[(TestSpec, [i32; 3])]) -> i32 {
    let mut bounds: Option<[i32; 4]> = None;
    for (test, offset) in tests {
        let region = test.cleanup_region();
        let test_bounds = [
            offset[0] + region[0][0],
            offset[0] + region[1][0],
            offset[2] + region[0][2],
            offset[2] + region[1][2],
        ];
        bounds = Some(match bounds {
            Some([min_x, max_x, min_z, max_z]) => [
                min_x.min(test_bounds[0]),
                max_x.max(test_bounds[1]),
                min_z.min(test_bounds[2]),
                max_z.max(test_bounds[3]),
            ],
            None => test_bounds,
        });
    }

    bounds.map_or(0, |[min_x, max_x, min_z, max_z]| {
        (max_x - min_x).max(max_z - min_z)
    })
}

fn teleports_player(test: &TestSpec) -> bool {
    test.timeline.iter().any(|entry| {
        matches!(
            &entry.action_type,
            flint_core::test_spec::ActionType::Tp { entity_alias, .. }
                if entity_alias == "player"
        )
    })
}

/// Split a batch so every sub-batch fits within the server's simulation distance from the
/// layout center (origin). Tests stay parallel within each sub-batch.
pub fn split_tests_by_simulation_distance(
    tests: Vec<TestSpec>,
    simulation_distance: u32,
) -> Vec<Vec<TestSpec>> {
    if tests.is_empty() {
        return Vec::new();
    }

    let max_radius = simulation_radius_blocks(simulation_distance);
    let mut batches: Vec<Vec<TestSpec>> = Vec::new();
    let mut current: Vec<TestSpec> = Vec::new();

    for test in tests {
        current.push(test);
        let offsets = calculate_test_offsets(&current);
        let paired: Vec<(TestSpec, [i32; 3])> = current.iter().cloned().zip(offsets).collect();

        let moves_shared_player = current.iter().any(teleports_player);
        let exceeds_origin_radius = max_extent_from_origin(&paired) > max_radius;
        let exceeds_player_radius =
            moves_shared_player && max_horizontal_span(&paired) > max_radius;
        if (exceeds_origin_radius || exceeds_player_radius) && current.len() > 1 {
            let overflow = current.split_off(current.len() - 1);
            batches.push(current);
            current = overflow;
        }
    }

    if !current.is_empty() {
        batches.push(current);
    }

    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use flint_core::test_spec::ActionType;
    use flint_core::test_spec::{CleanupSpec, SetupSpec};

    fn test_spec(name: &str, region: [[i32; 3]; 2]) -> TestSpec {
        TestSpec {
            flint_version: None,
            name: name.to_string(),
            description: None,
            tags: vec![],
            minecraft_ids: vec![],
            dependencies: vec![],
            setup: Some(SetupSpec {
                cleanup: Some(CleanupSpec { region }),
                player: None,
                world: Default::default(),
            }),
            timeline: vec![],
            breakpoints: vec![],
        }
    }

    #[test]
    fn simulation_radius_reserves_margin() {
        assert_eq!(simulation_radius_blocks(10), 128);
        assert_eq!(simulation_radius_blocks(12), 160);
    }

    #[test]
    fn split_keeps_small_batch_intact() {
        let tests = vec![
            test_spec("a", [[0, 0, 0], [5, 0, 5]]),
            test_spec("b", [[0, 0, 0], [5, 0, 5]]),
        ];
        let batches = split_tests_by_simulation_distance(tests, 32);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);
    }

    #[test]
    fn split_breaks_wide_batch_for_low_sim_distance() {
        let tests: Vec<_> = (0..20)
            .map(|i| test_spec(&format!("t{i}"), [[-20, 0, -20], [20, 0, 20]]))
            .collect();
        let batches = split_tests_by_simulation_distance(tests, 6);
        assert!(batches.len() > 1);
        assert_eq!(batches.iter().map(|b| b.len()).sum::<usize>(), 20);
    }

    #[test]
    fn groups_matching_world_configs_together() {
        let day_a = test_spec("day-a", [[0, 0, 0], [1, 1, 1]]);
        let mut night = test_spec("night", [[0, 0, 0], [1, 1, 1]]);
        night.setup.as_mut().unwrap().world.time = "minecraft:night".to_string();
        let day_b = test_spec("day-b", [[0, 0, 0], [1, 1, 1]]);

        let groups = group_tests_by_world_config(vec![day_a, night, day_b]);

        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0]
                .iter()
                .map(|test| test.name.as_str())
                .collect::<Vec<_>>(),
            vec!["day-a", "day-b"]
        );
        assert_eq!(groups[1][0].name, "night");
    }

    #[test]
    fn split_keeps_player_timelines_in_parallel_batch() {
        let mut first = test_spec("first", [[0, 0, 0], [1, 1, 1]]);
        first.timeline.push(flint_core::test_spec::TimelineEntry {
            at: flint_core::test_spec::TickSpec::Single(0),
            action_type: ActionType::Interact { item: None },
        });
        let mut second = test_spec("second", [[0, 0, 0], [1, 1, 1]]);
        second.timeline.push(flint_core::test_spec::TimelineEntry {
            at: flint_core::test_spec::TickSpec::Single(0),
            action_type: ActionType::Tp {
                entity_alias: "player".to_string(),
                pos: [0.0, 0.0, 0.0],
                rot: None,
            },
        });

        let batches = split_tests_by_simulation_distance(vec![first, second], 32);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);
    }

    #[test]
    fn split_starts_next_cycle_when_player_teleport_would_unload_tests() {
        let distant = test_spec("distant", [[0, 0, 0], [100, 1, 1]]);
        let mut player = test_spec("player", [[0, 0, 0], [100, 1, 1]]);
        player.timeline.push(flint_core::test_spec::TimelineEntry {
            at: flint_core::test_spec::TickSpec::Single(0),
            action_type: ActionType::Tp {
                entity_alias: "player".to_string(),
                pos: [0.5, 0.0, 0.5],
                rot: None,
            },
        });
        let other = test_spec("other", [[0, 0, 0], [100, 1, 1]]);

        let batches = split_tests_by_simulation_distance(vec![distant, player, other], 10);

        assert_eq!(batches.len(), 3);
        assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), [1, 1, 1]);
    }

    #[test]
    fn player_timeline_stays_with_block_timelines() {
        let block_before = test_spec("before", [[0, 0, 0], [1, 1, 1]]);
        let mut player = test_spec("player", [[0, 0, 0], [1, 1, 1]]);
        player.timeline.push(flint_core::test_spec::TimelineEntry {
            at: flint_core::test_spec::TickSpec::Single(0),
            action_type: ActionType::Interact { item: None },
        });
        let block_after = test_spec("after", [[0, 0, 0], [1, 1, 1]]);

        let batches =
            split_tests_by_simulation_distance(vec![block_before, player, block_after], 32);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 3);
    }

    #[test]
    fn locationality_fixture_preserves_hash_cycle_in_parallel_layout() {
        let fixture_path = std::path::PathBuf::from("FlintBenchmark/tests/locationality.json");
        let fixture =
            TestSpec::from_file(&fixture_path, false).expect("load locationality fixture");
        let solo_offset = calculate_test_offsets(std::slice::from_ref(&fixture));
        let parallel_offsets = calculate_test_offsets(&[fixture.clone(), fixture]);

        assert_eq!(solo_offset, vec![[-15, 0, 0]]);
        assert_eq!(parallel_offsets, vec![[-47, 0, 0], [17, 0, 0]]);
        assert_eq!(parallel_offsets[1][0] - parallel_offsets[0][0], 64);
        assert!(
            parallel_offsets
                .iter()
                .all(|offset| (offset[0] - solo_offset[0][0]).rem_euclid(32) == 0)
        );
    }

    #[test]
    fn unconstrained_layout_is_unchanged() {
        let tests = vec![
            test_spec("a", [[-2, 0, 0], [32, 0, 0]]),
            test_spec("b", [[-2, 0, 0], [32, 0, 0]]),
        ];

        assert_eq!(
            calculate_test_offsets(&tests),
            calculate_test_offsets_for_batch_default(&tests)
        );
    }

    #[test]
    fn unconstrained_test_run_alone_uses_world_origin() {
        let test = test_spec("solo", [[-2, -1, -7], [14, 5, 23]]);

        assert_eq!(calculate_test_offsets(&[test]), vec![[0, 0, 0]]);
    }

    #[test]
    fn locationality_constraint_clamps_to_safe_z_plane() {
        let fixture_path = std::path::PathBuf::from("FlintBenchmark/tests/locationality.json");
        let fixture =
            TestSpec::from_file(&fixture_path, false).expect("load locationality fixture");
        let mut offset = [127, 0, -6];
        offset[2] = exact_z_offset(&fixture).expect("Z constraint");
        let (modulus, phase) = x_offset_constraint(&fixture).expect("X constraint");
        offset[0] = nearest_congruent(offset[0], phase, modulus);

        assert_eq!(offset, [113, 0, 0]);
        assert_eq!(offset[0].rem_euclid(modulus), phase);
    }
}
