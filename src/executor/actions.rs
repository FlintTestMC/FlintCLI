//! Test action execution - block placement, assertions, etc.

use crate::executor::adapter::{MinecraftPlayer, MinecraftWorld};
use crate::executor::block;
use anyhow::Result;
use colored::Colorize;
use flint_core::results::{
    ActionOutcome, AssertEntityFail, AssertFailure, AssertPosition, InfoType,
};
use flint_core::test_spec::AssertType;
use flint_core::test_spec::{ActionType, Item, PlayerSlot, TimelineEntry};
use flint_core::traits::{FlintPlayer, FlintWorld};

/// Execute a single test action
/// Returns the outcome: Action (non-assertion), AssertPassed, or AssertFailed with details
pub fn execute_action(
    world: &mut MinecraftWorld,
    player: &mut Option<Box<dyn FlintPlayer>>,
    tick: u32,
    entry: &TimelineEntry,
    _value_idx: usize,
    verbose: bool,
) -> Result<ActionOutcome> {
    match &entry.action_type {
        ActionType::Place { pos, block } => {
            world.set_block_checked(*pos, block)?;
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
            Ok(ActionOutcome::Action)
        }

        ActionType::PlaceEach { blocks } => {
            for placement in blocks {
                world.set_block_checked(placement.pos, &placement.block)?;
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
            world.bot.send_command_synced(&cmd)?;

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
            Ok(ActionOutcome::Action)
        }

        ActionType::Remove { pos } => {
            let air = flint_core::test_spec::Block {
                id: "minecraft:air".to_string(),
                properties: Default::default(),
            };
            world.set_block_checked(*pos, &air)?;
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
            Ok(ActionOutcome::Action)
        }

        ActionType::Summon {
            entity_alias,
            entity_type,
            pos,
            nbt,
        } => {
            if verbose {
                println!(
                    "    {} Tick {}: summon {} as {} at [{}, {}, {}]",
                    "→".blue(),
                    tick,
                    entity_type,
                    entity_alias,
                    pos[0],
                    pos[1],
                    pos[2]
                );
            }
            world.summon_entity(entity_alias, entity_type, *pos, nbt.as_ref());
            Ok(ActionOutcome::Action)
        }

        ActionType::Assert { checks } => {
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
                            let matches_props =
                                matches_id && block::properties_match(&actual, expected_block);

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
                            if let Some(p) = p.as_any_mut().downcast_mut::<MinecraftPlayer>() {
                                p.restore_inventory()?;
                            }
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
                    AssertType::Entity(check) => {
                        let requested_nbt = check
                            .nbt
                            .as_ref()
                            .map(|nbt| nbt.requested_paths())
                            .unwrap_or_default();
                        let actual = world.get_entity(&check.entity_alias, &requested_nbt);
                        if !flint_core::runner::entity_matches(&actual, check) {
                            return Ok(ActionOutcome::AssertFailed(
                                AssertEntityFail::new(tick, check, &actual).into(),
                            ));
                        }

                        if verbose {
                            println!(
                                "    {} Tick {}: assert entity {} matches expected",
                                "✓".green(),
                                tick,
                                check.entity_alias
                            );
                        }
                    }
                }
            }
            Ok(ActionOutcome::AssertPassed)
        }

        ActionType::Tp {
            entity_alias,
            pos,
            rot,
        } => {
            if verbose {
                println!(
                    "    {} Tick {}: tp entity {} to [{}, {}, {}] with {:?}",
                    "→".blue(),
                    tick,
                    entity_alias,
                    pos[0],
                    pos[1],
                    pos[2],
                    rot
                );
            }
            if entity_alias == "player" {
                let p = player.get_or_insert_with(|| world.create_player());
                let p = p
                    .as_any_mut()
                    .downcast_mut::<MinecraftPlayer>()
                    .ok_or_else(|| anyhow::anyhow!("unsupported FlintPlayer implementation"))?;
                p.teleport_checked(*pos, *rot)?;
            } else {
                world.teleport_entity(entity_alias, *pos, *rot);
            }
            Ok(ActionOutcome::Action)
        }

        ActionType::Interact { item } => {
            if verbose {
                println!("    {} Tick {}: interact with {:?}", "→".blue(), tick, item);
            }
            let p = player
                .as_mut()
                .expect("interact requires an existing player");
            let p = p
                .as_any_mut()
                .downcast_mut::<MinecraftPlayer>()
                .ok_or_else(|| anyhow::anyhow!("unsupported FlintPlayer implementation"))?;
            if let Some(item_id) = item {
                let it = Item::new(item_id);
                p.set_slot_checked(PlayerSlot::Hotbar1, Some(&it))?;
                p.select_hotbar_checked(1)?;
            }
            p.interact_checked()?;
            Ok(ActionOutcome::Action)
        }

        ActionType::SetSlot { slot, item, count } => {
            let p = player.get_or_insert_with(|| world.create_player());
            let p = p
                .as_any_mut()
                .downcast_mut::<MinecraftPlayer>()
                .ok_or_else(|| anyhow::anyhow!("unsupported FlintPlayer implementation"))?;
            if let Some(item_id) = item {
                let it = Item::with_count(item_id, *count);
                p.set_slot_checked(*slot, Some(&it))?;
            } else {
                p.set_slot_checked(*slot, None)?;
            }
            Ok(ActionOutcome::Action)
        }

        ActionType::SelectHotbar { slot } => {
            let p = player.get_or_insert_with(|| world.create_player());
            let p = p
                .as_any_mut()
                .downcast_mut::<MinecraftPlayer>()
                .ok_or_else(|| anyhow::anyhow!("unsupported FlintPlayer implementation"))?;
            p.select_hotbar_checked(*slot)?;
            Ok(ActionOutcome::Action)
        }
    }
}
