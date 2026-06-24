//! Batch layout helpers: simulation-distance limits and test focus positions.

use flint_core::spatial::calculate_test_offsets_for_batch_default;
use flint_core::test_spec::TestSpec;

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

/// World position to teleport the bot so a test's chunks load and simulate reliably.
pub fn test_focus_pos(test: &TestSpec, offset: [i32; 3]) -> [i32; 3] {
    let region = test.cleanup_region();
    let cx = offset[0] + (region[0][0] + region[1][0]) / 2;
    let cz = offset[2] + (region[0][2] + region[1][2]) / 2;
    let y_top = region[0][1].max(region[1][1]);
    let cy = (offset[1] + y_top + 2).clamp(-60, 320);
    [cx, cy, cz]
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
        let paired: Vec<(TestSpec, [i32; 3])> = current
            .iter()
            .cloned()
            .zip(offsets)
            .collect();

        if max_extent_from_origin(&paired) > max_radius && current.len() > 1 {
            current.pop();
            batches.push(current);
            current = vec![paired.last().unwrap().0.clone()];
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
}
