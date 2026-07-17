//! Test action execution - block placement, assertions, etc.

use crate::executor::adapter::{MinecraftPlayer, MinecraftWorld};
use anyhow::Result;
use colored::Colorize;
use flint_core::results::{ActionOutcome, AssertEntityFail, AssertFailure, AssertTimeFail};
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
                nbt: None,
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
                        let requested_nbt = expected_blocks
                            .iter()
                            .filter_map(|block| block.nbt.as_ref())
                            .flat_map(|nbt| nbt.requested_paths())
                            .collect::<Vec<_>>();
                        let actual = world.get_block(check.pos, &requested_nbt);

                        let mut matched_any = false;
                        for expected_block in &expected_blocks {
                            if flint_core::runner::block_matches(&actual, expected_block) {
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
                                        nbt: None,
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

                            return Ok(ActionOutcome::AssertFailed(AssertFailure::new_block(
                                tick,
                                expected_blocks,
                                actual,
                                check.pos,
                            )));
                        }

                        if verbose {
                            let first_expected =
                                expected_blocks.first().cloned().unwrap_or_else(|| {
                                    flint_core::test_spec::Block {
                                        id: "minecraft:air".to_string(),
                                        properties: Default::default(),
                                        nbt: None,
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
                            return Ok(ActionOutcome::AssertFailed(AssertFailure::new_inventory(
                                tick,
                                check.is.clone(),
                                actual,
                                check.slot,
                            )));
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
                    AssertType::Time(check) => {
                        let actual = crate::executor::adapter::query_daytime(&world.bot)?;
                        if actual != check.time {
                            return Ok(ActionOutcome::AssertFailed(
                                AssertTimeFail::new(tick, check.time, actual).into(),
                            ));
                        }
                        if verbose {
                            println!(
                                "    {} Tick {}: assert world daytime is {}",
                                "✓".green(),
                                tick,
                                actual
                            );
                        }
                    }
                    AssertType::Entity(check) => {
                        let requested_nbt = check.nbt.requested_paths();
                        let actual = if let Some(alias) = check.entity_alias.as_deref() {
                            world.get_entity(alias, &requested_nbt)
                        } else {
                            world.find_entity(
                                check
                                    .entity_type
                                    .as_deref()
                                    .expect("entity check requires an alias or entity type"),
                                &requested_nbt,
                            )
                        };
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
                                check
                                    .entity_alias
                                    .as_deref()
                                    .or(check.entity_type.as_deref())
                                    .unwrap_or("unknown entity")
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
