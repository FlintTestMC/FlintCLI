//! Test executor module - core test orchestration

mod actions;
pub mod adapter;
mod block;
mod commands;
mod events;
mod handlers;
mod recorder;
mod tick;

use crate::bot::TestBot;
use adapter::MinecraftWorld;
use anyhow::Result;
use colored::Colorize;
use flint_core::loader::TestLoader;
use flint_core::results::{ActionOutcome, AssertFailure, AssertPosition, TestResult};
use flint_core::test_spec::{ActionType, AssertType, TestSpec, TimelineEntry};
use flint_core::timeline::TimelineAggregate;
use flint_core::traits::{FlintPlayer, FlintWorld};
use std::collections::BTreeSet;
use std::io::Write;

// Timing constants
const DEFAULT_TESTS_DIR: &str = "FlintBenchmark/tests";

fn asserted_entity_types(test: &TestSpec) -> BTreeSet<String> {
    test.timeline
        .iter()
        .filter_map(|entry| match &entry.action_type {
            ActionType::Assert { checks } => Some(checks),
            _ => None,
        })
        .flatten()
        .filter_map(|check| match check {
            AssertType::Entity(check) => check.entity_type.clone(),
            _ => None,
        })
        .filter(|entity_type| {
            !entity_type.is_empty()
                && entity_type.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | ':' | '.')
                })
        })
        .collect()
}

// Progress bar constants
const PROGRESS_BAR_WIDTH: usize = 40;

/// Output from a test run, including results and failure details
pub struct TestRunOutput {
    pub results: Vec<TestResult>,
    /// First failure detail per failed test: (test_name, failure_detail)
    pub failures: Vec<(String, AssertFailure)>,
}

pub struct TestExecutor {
    pub bot: TestBot,
    recorder: Option<recorder::RecorderState>,
    verbose: bool,
    quiet: bool,
    fail_fast: bool,
    pos1: Option<[i32; 3]>,
    last_assert_pos: Vec<String>,
    events_path: Option<std::path::PathBuf>,
    events: Option<events::JsonlWriter>,
    enable_breakpoints: bool,
}

impl Default for TestExecutor {
    fn default() -> Self {
        Self {
            bot: TestBot::new(),
            recorder: None,
            verbose: false,
            quiet: false,
            fail_fast: false,
            pos1: None,
            last_assert_pos: vec![],
            events_path: None,
            events: None,
            enable_breakpoints: true,
        }
    }
}

impl TestExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    pub fn set_quiet(&mut self, quiet: bool) {
        self.quiet = quiet;
    }

    pub fn set_fail_fast(&mut self, fail_fast: bool) {
        self.fail_fast = fail_fast;
    }

    pub fn set_enable_breakpoints(&mut self, enable: bool) {
        self.enable_breakpoints = enable;
    }

    /// Enable JSONL event emission.
    pub fn set_events_path(&mut self, path: std::path::PathBuf) {
        self.events_path = Some(path);
    }

    pub fn connect(&mut self, server: &str) -> Result<()> {
        self.bot.connect(server)
    }

    /// Start a recording session before or during interactive mode.
    pub fn start_recording(
        &mut self,
        test_name: &str,
        test_loader: &TestLoader,
        player_name: Option<String>,
    ) -> Result<()> {
        self.handle_record_start(test_name, test_loader, player_name)
    }

    /// Helper to get a mutable reference to the recorder, or return an error
    fn require_recorder(&mut self) -> Option<&mut recorder::RecorderState> {
        self.recorder.as_mut()
    }

    /// Helper to apply offset to a position
    fn apply_offset(&self, pos: [i32; 3], offset: [i32; 3]) -> [i32; 3] {
        [pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]]
    }

    fn forceload_regions(
        &self,
        tests_with_offsets: &[(TestSpec, [i32; 3])],
        add: bool,
    ) -> Result<()> {
        for (test, offset) in tests_with_offsets {
            let region = test.cleanup_region();
            let min = self.apply_offset(region[0], *offset);
            let max = self.apply_offset(region[1], *offset);
            let cx0 = min[0].div_euclid(16);
            let cz0 = min[2].div_euclid(16);
            let cx1 = max[0].div_euclid(16);
            let cz1 = max[2].div_euclid(16);
            let verb = if add { "add" } else { "remove" };
            let cmd = format!("forceload {verb} {cx0} {cz0} {cx1} {cz1}");
            self.bot.send_command_synced(&cmd)?;
        }
        Ok(())
    }

    /// Interactive mode: listen for chat commands and execute them
    pub fn interactive_mode(&mut self, test_loader: &mut TestLoader) -> Result<()> {
        // Interactive mode always uses verbose output
        self.verbose = true;

        // Send help message to chat (without ! to avoid self-triggering)
        self.bot
            .send_command_synced("say FlintMC Interactive Mode active")?;
        self.bot.send_command_synced(
            "say Type: help, search, run, run-all, run-tags, list, reload, stop (prefix with !)",
        )?;

        // A recording started from the CLI opens its controls immediately.
        // Normal interactive mode keeps its existing chat interface unchanged.
        if self.recorder.is_some() {
            self.show_recorder_dialog()?;
        }

        // Drain any messages
        tick::drain_chat_messages(&mut self.bot);

        // Collect all tests upfront
        let mut all_test_files = test_loader.collect_all_test_files()?;
        let mut next_recorder_ui_poll = std::time::Instant::now();
        let mut last_recorder_ui_action = None::<std::time::Instant>;

        loop {
            if self.recorder.is_some() && std::time::Instant::now() >= next_recorder_ui_poll {
                self.poll_recorder_dialog_action()?;
                next_recorder_ui_poll =
                    std::time::Instant::now() + std::time::Duration::from_millis(500);
            }
            // Poll for chat messages
            if let Some((sender, message)) = self
                .bot
                .recv_chat_timeout(std::time::Duration::from_millis(tick::CHAT_POLL_TIMEOUT_MS))
            {
                if handlers::is_bot_sender(sender.as_deref()) {
                    continue;
                }
                let Some((command, args)) = handlers::parse_command(&message) else {
                    continue;
                };
                let is_recorder_ui_action = message.contains("__flintmc_recorder_");
                if is_recorder_ui_action {
                    let now = std::time::Instant::now();
                    if last_recorder_ui_action
                        .is_some_and(|previous| now.duration_since(previous).as_millis() < 750)
                    {
                        continue;
                    }
                    last_recorder_ui_action = Some(now);
                    self.consume_recorder_dialog_action(sender.as_deref())?;
                }

                match command.as_str() {
                    _ if commands::from_chat(&command).is_some() => {
                        let flint_command = commands::from_chat(&command).expect("guarded command");
                        let mut context = commands::FlintCommandContext {
                            args: &args,
                            sender: sender.clone(),
                            test_loader,
                            all_test_files: &mut all_test_files,
                            exit_interactive: false,
                        };
                        commands::dispatch(self, flint_command, &mut context)?;
                        if context.exit_interactive {
                            return Ok(());
                        }
                    }

                    _ => {
                        if command.starts_with('!') {
                            self.bot.send_command(&format!(
                                "say Unknown command: {}. Type !help for commands.",
                                command
                            ))?;
                        }
                    }
                }

                // Dialog actions enter through chat like their command equivalents.
                // Replace the waiting screen with current recorder state after each action.
                if self.recorder.is_some() {
                    self.show_recorder_dialog()?;
                }
            }
        }
    }

    /// Scan blocks in a cube around a center point (ignores air)
    fn scan_blocks_around(
        &self,
        center: [i32; 3],
        radius: i32,
    ) -> Result<std::collections::HashMap<[i32; 3], String>> {
        self.scan_region(
            [center[0] - radius, center[1] - radius, center[2] - radius],
            [center[0] + radius, center[1] + radius, center[2] + radius],
        )
    }

    /// Scan blocks within an AABB (inclusive bounds, in world coordinates).
    fn scan_region(
        &self,
        min: [i32; 3],
        max: [i32; 3],
    ) -> Result<std::collections::HashMap<[i32; 3], String>> {
        let mut blocks = std::collections::HashMap::new();
        for x in min[0]..=max[0] {
            for y in min[1].max(-64)..=max[1].min(319) {
                for z in min[2]..=max[2] {
                    let pos = [x, y, z];
                    if let Ok(Some(block)) = self.bot.get_block(pos) {
                        let block_id = block::extract_block_id(&block);
                        if !block_id.to_lowercase().contains("air") {
                            blocks.insert(pos, block_id);
                        }
                    }
                }
            }
        }
        Ok(blocks)
    }

    fn validate_test_batch(tests_with_offsets: &[(TestSpec, [i32; 3])]) -> Result<&TestSpec> {
        let Some((first, _)) = tests_with_offsets.first() else {
            anyhow::bail!("cannot run an empty test batch");
        };
        if tests_with_offsets
            .iter()
            .any(|(test, _)| test.world_config() != first.world_config())
        {
            anyhow::bail!("parallel tests must use the same world configuration");
        }
        Ok(first)
    }

    fn layout_center(tests_with_offsets: &[(TestSpec, [i32; 3])]) -> [f64; 3] {
        let (min, max) = tests_with_offsets.iter().fold(
            ([i32::MAX; 3], [i32::MIN; 3]),
            |(mut min, mut max), (test, offset)| {
                let region = test.cleanup_region();
                for axis in 0..3 {
                    min[axis] = min[axis].min(region[0][axis] + offset[axis]);
                    max[axis] = max[axis].max(region[1][axis] + offset[axis]);
                }
                (min, max)
            },
        );
        [
            f64::from((min[0] + max[0]).div_euclid(2)) + 0.5,
            64.0,
            f64::from((min[2] + max[2]).div_euclid(2)) + 0.5,
        ]
    }

    fn configure_batch_world(&mut self, first: &TestSpec) -> Result<()> {
        self.bot.send_command_synced("tick freeze")?;

        let world_config = first.world_config();
        let mut gamerules: Vec<_> = world_config.gamerules.iter().collect();
        gamerules.sort_by_key(|(name, _)| *name);
        for (name, value) in gamerules {
            self.bot
                .send_command_synced(&format!("gamerule {name} {value}"))?;
        }
        self.bot
            .send_command_synced(&format!("time set {}", world_config.time))?;
        self.bot
            .send_command_synced(&format!("weather {}", world_config.weather))?;
        Ok(())
    }

    fn cleanup_test_area(&self, test: &TestSpec, offset: [i32; 3]) -> Result<()> {
        let region = test.cleanup_region();
        let min = self.apply_offset(region[0], offset);
        let max = self.apply_offset(region[1], offset);
        self.bot.send_command_synced(&format!(
            "fill {} {} {} {} {} {} air",
            min[0], min[1], min[2], max[0], max[1], max[2]
        ))?;
        for entity_type in asserted_entity_types(test) {
            self.bot.send_command_synced(&format!(
                "kill @e[type={entity_type},x={},y={},z={},dx={},dy={},dz={}]",
                min[0],
                min[1],
                min[2],
                max[0] - min[0],
                max[1] - min[1],
                max[2] - min[2]
            ))?;
        }
        Ok(())
    }

    fn create_batch_worlds(
        &self,
        tests_with_offsets: &[(TestSpec, [i32; 3])],
    ) -> Vec<MinecraftWorld> {
        tests_with_offsets
            .iter()
            .map(|(test, offset)| {
                let region = test.cleanup_region();
                MinecraftWorld {
                    bot: self.bot.clone(),
                    offset: *offset,
                    current_tick: 0,
                    entities: std::collections::HashMap::new(),
                    entity_bounds: Some([
                        self.apply_offset(region[0], *offset),
                        self.apply_offset(region[1], *offset),
                    ]),
                }
            })
            .collect()
    }

    fn create_batch_players(
        worlds: &mut [MinecraftWorld],
        tests_with_offsets: &[(TestSpec, [i32; 3])],
    ) -> Result<Vec<Option<Box<dyn FlintPlayer>>>> {
        tests_with_offsets
            .iter()
            .enumerate()
            .map(|(index, (spec, _))| {
                let Some(config) = spec.setup.as_ref().and_then(|setup| setup.player.as_ref())
                else {
                    return Ok(None);
                };
                let mut player = worlds[index].create_player();
                let minecraft_player = player
                    .as_any_mut()
                    .downcast_mut::<adapter::MinecraftPlayer>()
                    .ok_or_else(|| anyhow::anyhow!("unsupported FlintPlayer implementation"))?;
                for (slot, item) in &config.inventory {
                    minecraft_player.set_slot_checked(*slot, Some(item))?;
                }
                minecraft_player.select_hotbar_checked(config.selected_hotbar)?;
                minecraft_player.set_game_mode_checked(config.game_mode)?;
                Ok(Some(player))
            })
            .collect()
    }

    fn test_max_ticks(aggregate: &TimelineAggregate<'_>, test_count: usize) -> Vec<u32> {
        let mut max_ticks = vec![0; test_count];
        for (tick_num, entries) in &aggregate.timeline {
            for (test_idx, _, _) in entries {
                max_ticks[*test_idx] = max_ticks[*test_idx].max(*tick_num);
            }
        }
        max_ticks
    }

    /// Run tests in parallel with merged timeline
    pub fn run_tests_parallel(
        &mut self,
        tests_with_offsets: &[(TestSpec, [i32; 3])],
        break_after_setup: bool,
    ) -> Result<TestRunOutput> {
        let verbose = self.verbose;

        let first = Self::validate_test_batch(tests_with_offsets)?;

        if verbose {
            println!(
                "{} Running {} tests in parallel\n",
                "→".blue().bold(),
                tests_with_offsets.len()
            );
        }

        // Open the events writer if --emit-events was passed.
        if let Some(events_path) = self.events_path.clone() {
            if tests_with_offsets.len() != 1 {
                anyhow::bail!(
                    "--emit-events requires exactly one test (got {}); pass a single test file",
                    tests_with_offsets.len()
                );
            }
            let offset = tests_with_offsets[0].1;
            self.events = Some(events::JsonlWriter::create(&events_path, offset)?);
        }

        // Build global merged timeline using flint-core
        let aggregate = TimelineAggregate::from_tests(tests_with_offsets);

        // The stable focus point is the center of the complete parallel layout. Parking
        // here keeps every test as close as possible to the client's loaded chunks.
        let layout_center = Self::layout_center(tests_with_offsets);

        if verbose {
            println!("  Global timeline: {} ticks", aggregate.max_tick);
            println!(
                "  {} unique tick steps with actions",
                aggregate.unique_tick_count()
            );
            if !aggregate.breakpoints.is_empty() {
                let mut sorted_breakpoints: Vec<_> = aggregate.breakpoints.iter().collect();
                sorted_breakpoints.sort();
                println!(
                    "  {} breakpoints at ticks: {:?}",
                    aggregate.breakpoints.len(),
                    sorted_breakpoints
                );
            }
            if break_after_setup {
                println!("  {} Break after setup enabled", "→".yellow());
            }
            println!();
        }

        self.configure_batch_world(first)?;

        // Clean all test areas before starting
        if verbose {
            println!("{} Cleaning all test areas...", "→".blue());
        }
        for (test, offset) in tests_with_offsets.iter() {
            self.cleanup_test_area(test, *offset)?;
        }

        self.forceload_regions(tests_with_offsets, true)?;

        // Break after setup if requested
        let mut stepping_mode = false;
        if break_after_setup {
            let should_continue = tick::wait_for_step(
                &mut self.bot,
                "After test setup (cleanup complete, time frozen)",
            )?;
            stepping_mode = !should_continue;
        }

        // Emit `run_started` and pre-compute the scan AABB in world coords.
        let scan_bounds: Option<([i32; 3], [i32; 3])> = if self.events.is_some() {
            let (test, offset) = &tests_with_offsets[0];
            let region = test.cleanup_region();
            let world_min = self.apply_offset(region[0], *offset);
            let world_max = self.apply_offset(region[1], *offset);
            if let Some(events) = self.events.as_mut() {
                events.run_started(&test.name, [world_min, world_max])?;
            }
            Some((world_min, world_max))
        } else {
            None
        };

        // Track results per test: (passed_assertions, failed_assertions)
        let mut test_results: Vec<(usize, usize)> = vec![(0, 0); tests_with_offsets.len()];

        // Track first failure detail per test
        let mut test_failures: Vec<Option<AssertFailure>> =
            (0..tests_with_offsets.len()).map(|_| None).collect();

        // Track which tests have been cleaned up
        let mut tests_cleaned: Vec<bool> = vec![false; tests_with_offsets.len()];

        // Calculate max tick for each test
        let test_max_ticks = Self::test_max_ticks(&aggregate, tests_with_offsets.len());

        let show_progress = !verbose && !self.quiet;
        let fail_fast = self.fail_fast;

        // Initialize per-test worlds and players using the trait model
        let mut worlds = self.create_batch_worlds(tests_with_offsets);
        let mut players = Self::create_batch_players(&mut worlds, tests_with_offsets)?;

        // Execute merged timeline
        let mut current_tick = 0;
        while current_tick <= aggregate.max_tick {
            if let Some(entries) = aggregate.timeline.get(&current_tick) {
                for (test_idx, entry, value_idx) in entries {
                    let (test, _) = &tests_with_offsets[*test_idx];
                    let world = &mut worlds[*test_idx];
                    let player = &mut players[*test_idx];

                    match self.execute_action(world, player, current_tick, entry, *value_idx) {
                        Ok(ActionOutcome::AssertPassed) => {
                            test_results[*test_idx].0 += 1;
                            if let Some(events) = self.events.as_mut()
                                && let ActionType::Assert { checks } = &entry.action_type
                            {
                                for check in checks {
                                    let AssertType::Block(block_check) = check else {
                                        anyhow::bail!(
                                            "TODO: emit events for AssertType::Inventory not yet implemented"
                                        );
                                    };
                                    events.emit_assert(
                                        current_tick,
                                        block_check.pos,
                                        true,
                                        None,
                                        None,
                                    )?;
                                }
                            }
                        }
                        Ok(ActionOutcome::Action) => {}
                        Ok(ActionOutcome::AssertFailed(detail)) => {
                            test_results[*test_idx].1 += 1;
                            if verbose {
                                println!(
                                    "    {} [{}] Tick {}: expected {}, got {}",
                                    "✗".red().bold(),
                                    test.name,
                                    current_tick,
                                    String::from(detail.expected()).green(),
                                    String::from(detail.actual()).red()
                                );
                            }
                            if let Some(events) = self.events.as_mut() {
                                let expected: String = detail.expected().into();
                                let actual: String = detail.actual().into();
                                let AssertPosition::Coordinate { x, y, z } = detail.position()
                                else {
                                    anyhow::bail!(
                                        "TODO: emit events for AssertPosition::Slot not yet implemented"
                                    );
                                };
                                events.emit_assert(
                                    current_tick,
                                    [x, y, z],
                                    false,
                                    Some(&expected),
                                    Some(&actual),
                                )?;
                            }
                            // Store first failure per test
                            if test_failures[*test_idx].is_none() {
                                test_failures[*test_idx] = Some(detail);
                            }
                            if fail_fast {
                                break;
                            }
                        }
                        Err(e) => {
                            test_results[*test_idx].1 += 1;
                            if verbose {
                                println!(
                                    "    {} [{}] Tick {}: {}",
                                    "✗".red().bold(),
                                    test.name,
                                    current_tick,
                                    e.to_string().red()
                                );
                            }
                            if fail_fast {
                                break;
                            }
                        }
                    }
                }
            }

            // Break out of the timeline loop on first failure
            if fail_fast && test_results.iter().any(|(_, failed)| *failed > 0) {
                break;
            }

            // Clean up tests that have completed
            for test_idx in 0..tests_with_offsets.len() {
                if !tests_cleaned[test_idx] && current_tick > test_max_ticks[test_idx] {
                    let (test, offset) = &tests_with_offsets[test_idx];
                    if verbose {
                        println!(
                            "\n{} Cleaning up test [{}] (completed at tick {})...",
                            "→".blue(),
                            test.name,
                            test_max_ticks[test_idx]
                        );
                    }
                    self.cleanup_test_area(test, *offset)?;
                    tests_cleaned[test_idx] = true;
                    players[test_idx] = None;
                    self.bot.park_at(layout_center)?;
                }
            }

            // Check for breakpoint
            if (self.enable_breakpoints && aggregate.breakpoints.contains(&current_tick))
                || stepping_mode
            {
                let should_continue = tick::wait_for_step(
                    &mut self.bot,
                    &format!("End of tick {} (before step to next tick)", current_tick),
                )?;
                stepping_mode = !should_continue;
            }

            // Advance to next tick.
            if let Some((scan_min, scan_max)) = scan_bounds {
                tick::step_tick(&mut self.bot, verbose)?;
                let world_blocks = self.scan_region(scan_min, scan_max)?;
                if let Some(events) = self.events.as_mut() {
                    events.emit_tick(current_tick, world_blocks)?;
                }
                current_tick += 1;
            } else if current_tick < aggregate.max_tick {
                if stepping_mode {
                    tick::step_tick(&mut self.bot, verbose)?;
                    current_tick += 1;
                } else {
                    let next_event_tick = aggregate
                        .next_event_tick(current_tick)
                        .unwrap_or(aggregate.max_tick + 1);

                    let ticks_to_sprint = if next_event_tick <= aggregate.max_tick {
                        next_event_tick - current_tick
                    } else {
                        aggregate.max_tick - current_tick
                    };

                    if ticks_to_sprint == 1 {
                        tick::step_tick(&mut self.bot, verbose)?
                    } else if ticks_to_sprint > 1 {
                        tick::sprint_ticks(&mut self.bot, ticks_to_sprint, verbose)?
                    } else {
                        0
                    };

                    current_tick += ticks_to_sprint;
                }
            } else {
                current_tick += 1;
            }

            // Update tick counts in the FlintWorld adapter instances
            for world in &mut worlds {
                world.current_tick = current_tick as u64;
            }

            // Update progress bar in non-verbose mode
            if show_progress {
                print_progress_bar(current_tick.min(aggregate.max_tick), aggregate.max_tick);
            }
        }

        // Clear progress bar line
        if show_progress {
            println!();
        }

        // Emit run_completed
        if let Some(events) = self.events.as_mut() {
            let asserts_passed: u32 = test_results.iter().map(|(p, _)| *p as u32).sum();
            let asserts_failed: u32 = test_results.iter().map(|(_, f)| *f as u32).sum();
            events.run_completed(asserts_passed, asserts_failed)?;
        }

        // Clean up remaining tests while their chunks are still force-loaded.
        for test_idx in 0..tests_with_offsets.len() {
            if !tests_cleaned[test_idx] {
                let (test, offset) = &tests_with_offsets[test_idx];
                if verbose {
                    println!(
                        "\n{} Cleaning up remaining test [{}]...",
                        "→".blue(),
                        test.name
                    );
                }
                self.cleanup_test_area(test, *offset)?;
                tests_cleaned[test_idx] = true;
                players[test_idx] = None;
                self.bot.park_at(layout_center)?;
            }
        }

        // Release the chunks only after cleanup has completed, then resume time.
        self.forceload_regions(tests_with_offsets, false)?;
        self.bot.send_command("tick unfreeze")?;

        // Build results
        let results: Vec<TestResult> = tests_with_offsets
            .iter()
            .enumerate()
            .map(|(idx, (test, _))| {
                let (passed, failed) = test_results[idx];
                let success = failed == 0;

                if verbose {
                    println!();
                    if success {
                        println!(
                            "  {} [{}] Test passed: {} assertions",
                            "✓".green().bold(),
                            test.name,
                            passed
                        );
                    } else {
                        println!(
                            "  {} [{}] Test failed: {} passed, {} failed",
                            "✗".red().bold(),
                            test.name,
                            passed,
                            failed
                        );
                    }
                }

                if success {
                    TestResult::new(test.name.clone())
                } else {
                    TestResult::new(test.name.clone())
                        .with_failure_reason(format!("{} assertions failed", failed))
                }
            })
            .collect();

        // Send test results summary to chat
        let total_passed = results.iter().filter(|r| r.success).count();
        let total_failed = results.len() - total_passed;
        let summary = format!(
            "Tests complete: {}/{} passed, {} failed",
            total_passed,
            results.len(),
            total_failed
        );
        self.bot.send_command_synced(&format!("say {}", summary))?;

        // Send individual test results to chat
        for result in &results {
            let status = if result.success { "PASS" } else { "FAIL" };
            let msg = format!("say [{}] {}", status, result.test_name);
            self.bot.send_command_synced(&msg)?;
        }

        // Collect failure details
        let failures: Vec<(String, AssertFailure)> = tests_with_offsets
            .iter()
            .enumerate()
            .filter_map(|(idx, (test, _))| {
                test_failures[idx]
                    .take()
                    .map(|detail| (test.name.clone(), detail))
            })
            .collect();

        // A virtual player may have left the shared bot inside a test region. Park the
        // physical bot at the layout center after every run, including playerless runs.
        self.bot.park_at(layout_center)?;

        Ok(TestRunOutput { results, failures })
    }

    fn execute_action(
        &mut self,
        world: &mut MinecraftWorld,
        player: &mut Option<Box<dyn FlintPlayer>>,
        tick: u32,
        entry: &TimelineEntry,
        value_idx: usize,
    ) -> Result<ActionOutcome> {
        actions::execute_action(world, player, tick, entry, value_idx, self.verbose)
    }
}

/// Print a progress bar to stdout
fn print_progress_bar(current: u32, total: u32) {
    if total == 0 {
        return;
    }
    let ratio = current as f64 / total as f64;
    let filled = (ratio * PROGRESS_BAR_WIDTH as f64) as usize;
    let empty = PROGRESS_BAR_WIDTH - filled;

    let bar = format!(
        "\r[{}{}] {}/{}",
        "█".repeat(filled),
        " ".repeat(empty),
        format_number(current),
        format_number(total),
    );
    print!("{} ticks", bar);
    let _ = std::io::stdout().flush();
}

/// Format a number with comma separators (e.g., 1247 -> "1,247")
pub fn format_number(n: u32) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}
