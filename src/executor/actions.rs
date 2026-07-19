//! Minecraft-facing wrapper around flint-core's shared action dispatcher.

use crate::executor::adapter::MinecraftWorld;
use anyhow::Result;
use colored::Colorize;
use flint_core::results::ActionOutcome;
use flint_core::runner::execute_action as execute_core_action;
use flint_core::test_spec::TimelineEntry;
use flint_core::traits::FlintPlayer;

pub fn execute_action(
    world: &mut MinecraftWorld,
    player: &mut Option<Box<dyn FlintPlayer>>,
    tick: u32,
    entry: &TimelineEntry,
    _value_idx: usize,
    verbose: bool,
) -> Result<ActionOutcome> {
    if verbose {
        println!("    {} Tick {}: {:?}", "→".blue(), tick, entry.action_type);
    }

    execute_core_action(world, player, &entry.action_type, tick)
}
