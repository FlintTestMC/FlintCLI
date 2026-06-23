//! Test action execution - block placement, assertions, etc.

use anyhow::Result;
use colored::Colorize;
use flint_core::results::{ActionOutcome, AssertFailure, AssertPosition, InfoType};
use flint_core::test_spec::AssertType::Block;
use flint_core::test_spec::{ActionType, BlockSpec, TimelineEntry, Item, PlayerSlot};
use flint_core::traits::{FlintPlayer, FlintWorld};

// Constants for action timing
pub const PLACE_EACH_DELAY_MS: u64 = 10;

/// Execute a single test action
/// Returns the outcome: Action (non-assertion), AssertPassed, or AssertFailed with details
pub fn execute_action(
    world: &mut dyn FlintWorld,
    player: &mut Option<Box<dyn FlintPlayer>>,
    tick: u32,
    entry: &TimelineEntry,
    _value_idx: usize,
    action_delay_ms: u64,
    verbose: bool,
) -> Result<ActionOutcome> {
    match &entry.action_type {
        ActionType::Place { pos, block } => {
            world.set_block(*pos, block);
            if verbose {
                println!(
                    "    {} Tick {}: place at [{}, {}, {}] = {}",
                    "→".blue(),
                    tick,
                    pos[0],
                    pos[1],
                    pos[2],
                    block.to_command().dimmed()
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(action_delay_ms));
            Ok(ActionOutcome::Action)
        }

        ActionType::PlaceEach { blocks } => {
            for placement in blocks {
                world.set_block(placement.pos, &placement.block);
                if verbose {
                    println!(
                        "    {} Tick {}: place at [{}, {}, {}] = {}",
                        "→".blue(),
                        tick,
                        placement.pos[0],
                        placement.pos[1],
                        placement.pos[2],
                        placement.block.to_command().dimmed()
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(PLACE_EACH_DELAY_MS));
            }
            Ok(ActionOutcome::Action)
        }

        ActionType::Fill { region, with } => {
            // Fill coordinates can be potentially inverted
            let min_x = region[0][0].min(region[1][0]);
            let max_x = region[0][0].max(region[1][0]);
            let min_y = region[0][1].min(region[1][1]);
            let max_y = region[0][1].max(region[1][1]);
            let min_z = region[0][2].min(region[1][2]);
            let max_z = region[0][2].max(region[1][2]);

            for x in min_x..=max_x {
                for y in min_y..=max_y {
                    for z in min_z..=max_z {
                        world.set_block([x, y, z], with);
                    }
                }
            }

            if verbose {
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
                    with.to_command().dimmed()
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(action_delay_ms));
            Ok(ActionOutcome::Action)
        }

        ActionType::Remove { pos } => {
            let air = flint_core::test_spec::Block {
                id: "minecraft:air".to_string(),
                properties: Default::default(),
            };
            world.set_block(*pos, &air);
            if verbose {
                println!(
                    "    {} Tick {}: remove at [{}, {}, {}]",
                    "→".blue(),
                    tick,
                    pos[0],
                    pos[1],
                    pos[2]
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(action_delay_ms));
            Ok(ActionOutcome::Action)
        }

        ActionType::Assert { checks } => {
            for check in checks {
                let Block(check) = check else {
                    anyhow::bail!("TODO: AssertType::Inventory not yet implemented");
                };
                let BlockSpec::Single(expected_block) = &check.is else {
                    anyhow::bail!("TODO: BlockSpec::Multiple not yet implemented");
                };

                let actual = world.get_block(check.pos);

                // Helper function to check ID match (allowing for optional minecraft: prefix difference)
                let check_id = |actual: &str, expected: &str| -> bool {
                    let actual_clean = actual.strip_prefix("minecraft:").unwrap_or(actual);
                    let expected_clean = expected.strip_prefix("minecraft:").unwrap_or(expected);
                    actual_clean.to_lowercase() == expected_clean.to_lowercase()
                };

                // Check block type
                let matches_id = check_id(&actual.id, &expected_block.id);
                let mut matches_props = true;

                if matches_id {
                    for (prop_name, expected_value) in &expected_block.properties {
                        if let Some(actual_value) = actual.properties.get(prop_name) {
                            if actual_value.to_lowercase() != expected_value.to_lowercase() {
                                matches_props = false;
                                break;
                            }
                        } else {
                            matches_props = false;
                            break;
                        }
                    }
                }

                if !matches_id || !matches_props {
                    if verbose {
                        println!(
                            "    {} Tick {}: assert block at [{}, {}, {}] expected {}, got {}",
                            "✗".red().bold(),
                            tick,
                            check.pos[0],
                            check.pos[1],
                            check.pos[2],
                            expected_block.id.green(),
                            actual.id.red()
                        );
                    }

                    return Ok(ActionOutcome::AssertFailed(AssertFailure {
                        tick,
                        expected: InfoType::Block(expected_block.clone()),
                        actual: InfoType::Block(actual),
                        position: AssertPosition::from_array(check.pos),
                        error_message: "Block was different".to_string(),
                        execution_time_ms: None,
                    }));
                }

                if verbose {
                    println!(
                        "    {} Tick {}: assert block at [{}, {}, {}] is {}",
                        "✓".green(),
                        tick,
                        check.pos[0],
                        check.pos[1],
                        check.pos[2],
                        expected_block.id.dimmed()
                    );
                }
            }
            Ok(ActionOutcome::AssertPassed)
        }

        ActionType::UseItemOn { pos, face, item } => {
            let p = player.get_or_insert_with(|| world.create_player());
            if let Some(item_id) = item {
                let it = Item::new(item_id);
                p.set_slot(PlayerSlot::Hotbar1, Some(&it));
                p.select_hotbar(1);
            }
            p.use_item_on(*pos, face);
            std::thread::sleep(std::time::Duration::from_millis(action_delay_ms));
            Ok(ActionOutcome::Action)
        }

        ActionType::SetSlot { slot, item, count } => {
            let p = player.get_or_insert_with(|| world.create_player());
            if let Some(item_id) = item {
                let it = Item::with_count(item_id, *count);
                p.set_slot(*slot, Some(&it));
            } else {
                p.set_slot(*slot, None);
            }
            std::thread::sleep(std::time::Duration::from_millis(action_delay_ms));
            Ok(ActionOutcome::Action)
        }

        ActionType::SelectHotbar { slot } => {
            let p = player.get_or_insert_with(|| world.create_player());
            p.select_hotbar(*slot);
            std::thread::sleep(std::time::Duration::from_millis(action_delay_ms));
            Ok(ActionOutcome::Action)
        }
    }
}
