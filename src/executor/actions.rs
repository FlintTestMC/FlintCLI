//! Test action execution - block placement, assertions, etc.

use crate::executor::adapter::MinecraftWorld;
use anyhow::Result;
use colored::Colorize;
use flint_core::results::{ActionOutcome, AssertFailure, AssertPosition, InfoType};
use flint_core::test_spec::AssertType;
use flint_core::test_spec::{ActionType, Item, PlayerSlot, TimelineEntry};
use flint_core::traits::{FlintPlayer, FlintWorld};

// Constants for action timing
pub const PLACE_EACH_DELAY_MS: u64 = 10;

/// Execute a single test action
/// Returns the outcome: Action (non-assertion), AssertPassed, or AssertFailed with details
pub fn execute_action(
    world: &mut MinecraftWorld,
    player: &mut Option<Box<dyn FlintPlayer>>,
    tick: u32,
    entry: &TimelineEntry,
    _value_idx: usize,
    action_delay_ms: u64,
    verbose: bool,
) -> Result<ActionOutcome> {
    let _ = world.ensure_focus();

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
            let world_min = [
                region[0][0] + world.offset[0],
                region[0][1] + world.offset[1],
                region[0][2] + world.offset[2],
            ];
            let world_max = [
                region[1][0] + world.offset[0],
                region[1][1] + world.offset[1],
                region[1][2] + world.offset[2],
            ];
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
            world.bot.send_command(&cmd)?;

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
                    block_spec.dimmed()
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
            // Wait a small delay to allow block updates from the same tick to propagate to the client
            std::thread::sleep(std::time::Duration::from_millis(50));

            for check in checks {
                match check {
                    AssertType::Block(check) => {
                        let expected_blocks = check.is.to_vec();
                        let actual = world.get_block(check.pos);

                        // Helper function to check ID match (allowing for optional minecraft: prefix difference)
                        let check_id = |actual: &str, expected: &str| -> bool {
                            let actual_clean = actual.strip_prefix("minecraft:").unwrap_or(actual);
                            let expected_clean =
                                expected.strip_prefix("minecraft:").unwrap_or(expected);
                            actual_clean.to_lowercase() == expected_clean.to_lowercase()
                        };

                        let mut matched_any = false;
                        for expected_block in &expected_blocks {
                            // Check block type
                            let matches_id = check_id(&actual.id, &expected_block.id);
                            let mut matches_props = true;

                            if matches_id {
                                for (prop_name, expected_value) in &expected_block.properties {
                                    if let Some(actual_value) = actual.properties.get(prop_name) {
                                        if actual_value.to_lowercase()
                                            != expected_value.to_lowercase()
                                        {
                                            matches_props = false;
                                            break;
                                        }
                                    } else {
                                        matches_props = false;
                                        break;
                                    }
                                }
                            }

                            if matches_id && matches_props {
                                matched_any = true;
                                break;
                            }
                        }

                        if !matched_any {
                            let first_expected =
                                expected_blocks.first().cloned().unwrap_or_else(|| {
                                    flint_core::test_spec::Block {
                                        id: "minecraft:air".to_string(),
                                        properties: Default::default(),
                                    }
                                });

                            if verbose {
                                println!(
                                    "    {} Tick {}: assert block at [{}, {}, {}] expected {}, got {}",
                                    "✗".red().bold(),
                                    tick,
                                    check.pos[0],
                                    check.pos[1],
                                    check.pos[2],
                                    first_expected.id.green(),
                                    actual.id.red()
                                );
                            }

                            return Ok(ActionOutcome::AssertFailed(AssertFailure {
                                tick,
                                expected: InfoType::Blocks(expected_blocks),
                                actual: InfoType::Block(actual),
                                position: AssertPosition::from_array(check.pos),
                                error_message: "Block was different".to_string(),
                                execution_time_ms: None,
                            }));
                        }

                        if verbose {
                            let first_expected =
                                expected_blocks.first().cloned().unwrap_or_else(|| {
                                    flint_core::test_spec::Block {
                                        id: "minecraft:air".to_string(),
                                        properties: Default::default(),
                                    }
                                });
                            println!(
                                "    {} Tick {}: assert block at [{}, {}, {}] is {}",
                                "✓".green(),
                                tick,
                                check.pos[0],
                                check.pos[1],
                                check.pos[2],
                                first_expected.id.dimmed()
                            );
                        }
                    }
                    AssertType::Inventory(check) => {
                        let actual = if let Some(p) = player {
                            p.get_slot(check.slot, Vec::new())
                        } else {
                            None
                        };

                        let match_ok = match (&check.is, &actual) {
                            (None, None) => true,
                            (Some(expected), Some(act)) => {
                                let actual_clean =
                                    act.id.strip_prefix("minecraft:").unwrap_or(&act.id);
                                let expected_clean = expected
                                    .id
                                    .strip_prefix("minecraft:")
                                    .unwrap_or(&expected.id);
                                let id_matches =
                                    actual_clean.to_lowercase() == expected_clean.to_lowercase();
                                let count_matches = act.count == expected.count;
                                id_matches && count_matches
                            }
                            _ => false,
                        };

                        if !match_ok {
                            if verbose {
                                println!(
                                    "    {} Tick {}: assert inventory slot {:?} expected {:?}, got {:?}",
                                    "✗".red().bold(),
                                    tick,
                                    check.slot,
                                    check.is,
                                    actual
                                );
                            }
                            return Ok(ActionOutcome::AssertFailed(AssertFailure {
                                tick,
                                expected: check
                                    .is
                                    .clone()
                                    .map(InfoType::Item)
                                    .unwrap_or_else(|| InfoType::String("empty".to_string())),
                                actual: actual
                                    .clone()
                                    .map(InfoType::Item)
                                    .unwrap_or_else(|| InfoType::String("empty".to_string())),
                                position: AssertPosition::from_array([0, 0, 0]),
                                error_message: "Inventory slot content was different".to_string(),
                                execution_time_ms: None,
                            }));
                        }

                        if verbose {
                            println!(
                                "    {} Tick {}: assert inventory slot {:?} matches expected",
                                "✓".green(),
                                tick,
                                check.slot
                            );
                        }
                    }
                }
            }
            Ok(ActionOutcome::AssertPassed)
        }

        ActionType::UseItemOn { pos, face, item } => {
            if verbose {
                println!(
                    "    {} Tick {}: use_item_on at [{}, {}, {}] with {:?}",
                    "→".blue(),
                    tick,
                    pos[0],
                    pos[1],
                    pos[2],
                    item
                );
            }
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
