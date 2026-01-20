//! Test action execution - block placement, assertions, etc.

use crate::bot::TestBot;
use anyhow::Result;
use colored::Colorize;
use flint_core::test_spec::{ActionType, TimelineEntry};

use super::block::block_matches;

// Constants for action timing
pub const BLOCK_POLL_ATTEMPTS: u32 = 10;
pub const BLOCK_POLL_DELAY_MS: u64 = 50;
pub const PLACE_EACH_DELAY_MS: u64 = 10;

/// Apply offset to a position
pub fn apply_offset(pos: [i32; 3], offset: [i32; 3]) -> [i32; 3] {
    [pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]]
}

/// Poll for a block at the given position with retries
/// This handles timing issues in CI environments where block updates may take longer
pub async fn poll_block_with_retry(
    bot: &TestBot,
    world_pos: [i32; 3],
    expected_block: &str,
) -> Result<Option<String>> {
    for attempt in 0..BLOCK_POLL_ATTEMPTS {
        let block = bot.get_block(world_pos).await?;

        // Check if the block matches what we expect
        if let Some(ref actual) = block
            && block_matches(actual, expected_block)
        {
            return Ok(block);
        }

        // If not the last attempt, wait before retrying
        if attempt < BLOCK_POLL_ATTEMPTS - 1 {
            tokio::time::sleep(tokio::time::Duration::from_millis(BLOCK_POLL_DELAY_MS)).await;
        }
    }

    // Return whatever we have after all retries
    bot.get_block(world_pos).await
}

/// Execute a single test action
/// Returns true if this was an assertion that passed, false otherwise
pub async fn execute_action(
    bot: &mut TestBot,
    tick: u32,
    entry: &TimelineEntry,
    _value_idx: usize,
    offset: [i32; 3],
    action_delay_ms: u64,
) -> Result<bool> {
    match &entry.action_type {
        ActionType::Place { pos, block } => {
            let world_pos = apply_offset(*pos, offset);
            let block_spec = block.to_command();
            let cmd = format!(
                "setblock {} {} {} {}",
                world_pos[0], world_pos[1], world_pos[2], block_spec
            );
            bot.send_command(&cmd).await?;
            println!(
                "    {} Tick {}: place at [{}, {}, {}] = {}",
                "→".blue(),
                tick,
                pos[0],
                pos[1],
                pos[2],
                block_spec.dimmed()
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(action_delay_ms)).await;
            Ok(false)
        }

        ActionType::PlaceEach { blocks } => {
            for placement in blocks {
                let world_pos = apply_offset(placement.pos, offset);
                let block_spec = placement.block.to_command();
                let cmd = format!(
                    "setblock {} {} {} {}",
                    world_pos[0], world_pos[1], world_pos[2], block_spec
                );
                bot.send_command(&cmd).await?;
                println!(
                    "    {} Tick {}: place at [{}, {}, {}] = {}",
                    "→".blue(),
                    tick,
                    placement.pos[0],
                    placement.pos[1],
                    placement.pos[2],
                    block_spec.dimmed()
                );
                tokio::time::sleep(tokio::time::Duration::from_millis(PLACE_EACH_DELAY_MS)).await;
            }
            Ok(false)
        }

        ActionType::Fill { region, with } => {
            let world_min = apply_offset(region[0], offset);
            let world_max = apply_offset(region[1], offset);
            let block_spec = with.to_command();
            let cmd = format!(
                "fill {} {} {} {} {} {} {}",
                world_min[0],
                world_min[1],
                world_min[2],
                world_max[0],
                world_max[1],
                world_max[2],
                block_spec
            );
            bot.send_command(&cmd).await?;
            println!(
                "    {} Tick {}: fill [{},{},{}] to [{},{},{}] = {}",
                "→".blue(),
                tick,
                region[0][0],
                region[0][1],
                region[0][2],
                region[1][0],
                region[1][1],
                region[1][2],
                block_spec.dimmed()
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(action_delay_ms)).await;
            Ok(false)
        }

        ActionType::Remove { pos } => {
            let world_pos = apply_offset(*pos, offset);
            let cmd = format!(
                "setblock {} {} {} air",
                world_pos[0], world_pos[1], world_pos[2]
            );
            bot.send_command(&cmd).await?;
            println!(
                "    {} Tick {}: remove at [{}, {}, {}]",
                "→".blue(),
                tick,
                pos[0],
                pos[1],
                pos[2]
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(action_delay_ms)).await;
            Ok(false)
        }

        ActionType::Assert { checks } => {
            for check in checks {
                let world_pos = apply_offset(check.pos, offset);

                // Poll with retries to handle timing issues in CI environments
                let actual_block = poll_block_with_retry(bot, world_pos, &check.is.id).await?;

                // Check block type
                let matches = actual_block
                    .as_ref()
                    .is_some_and(|actual| block_matches(actual, &check.is.id));

                if !matches {
                    anyhow::bail!(
                        "Block at [{}, {}, {}] is not {} (got {:?})",
                        check.pos[0],
                        check.pos[1],
                        check.pos[2],
                        check.is.id,
                        actual_block
                    );
                }

                // Check state properties if any are specified
                if !check.is.properties.is_empty() {
                    let actual_str = actual_block.as_ref().unwrap();

                    for (prop_name, prop_value) in &check.is.properties {
                        // Convert the expected value to string for comparison
                        let expected_value = match prop_value {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string().trim_matches('"').to_string(),
                        };

                        // Check if the property value is in the block state string
                        let actual_lower = actual_str.to_lowercase();
                        let prop_pattern = format!("{}: {}", prop_name, expected_value).to_lowercase();
                        let prop_pattern_quoted =
                            format!("{}: \"{}\"", prop_name, expected_value).to_lowercase();
                        // Handle numeric values with underscore prefix (e.g., level: _0)
                        let prop_pattern_underscore =
                            format!("{}: _{}", prop_name, expected_value).to_lowercase();

                        let prop_matches = actual_lower.contains(&prop_pattern)
                            || actual_lower.contains(&prop_pattern_quoted)
                            || actual_lower.contains(&prop_pattern_underscore);

                        if !prop_matches {
                            anyhow::bail!(
                                "Block at [{}, {}, {}] property '{}' is not '{}' (got {:?})",
                                check.pos[0],
                                check.pos[1],
                                check.pos[2],
                                prop_name,
                                expected_value,
                                actual_str
                            );
                        }

                        println!(
                            "    {} Tick {}: assert block at [{}, {}, {}] state {} = {}",
                            "✓".green(),
                            tick,
                            check.pos[0],
                            check.pos[1],
                            check.pos[2],
                            prop_name.dimmed(),
                            expected_value.dimmed()
                        );
                    }
                } else {
                    println!(
                        "    {} Tick {}: assert block at [{}, {}, {}] is {}",
                        "✓".green(),
                        tick,
                        check.pos[0],
                        check.pos[1],
                        check.pos[2],
                        check.is.id.dimmed()
                    );
                }
            }
            Ok(true)
        }
    }
}
