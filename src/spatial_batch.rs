//! Batch layout helpers: simulation-distance limits and test focus positions.

use flint_core::spatial::calculate_test_offsets_for_batch_default;
use flint_core::test_spec::TestSpec;

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
    (simulation_distance.saturating_sub(2) as i32) * 16
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
        let offsets = calculate_test_offsets_for_batch_default(&current);
        let paired: Vec<(TestSpec, [i32; 3])> = current.iter().cloned().zip(offsets).collect();

        if max_extent_from_origin(&paired) > max_radius && current.len() > 1 {
            let overflow = current.pop().expect("current has at least two tests");
            batches.push(current);
            current = vec![overflow];
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
}
