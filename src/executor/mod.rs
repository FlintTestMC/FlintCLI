//! Test executor module - core test orchestration

mod actions;
mod block;
mod handlers;
mod recorder;
mod tick;

use crate::bot::TestBot;
use anyhow::Result;
use colored::Colorize;
use flint_core::loader::TestLoader;
use flint_core::results::{ActionOutcome, AssertFailure, TestResult};
use flint_core::test_spec::{TestSpec, TimelineEntry};
use flint_core::timeline::TimelineAggregate;
use std::io::Write;

pub use tick::{COMMAND_DELAY_MS, MIN_RETRY_DELAY_MS};

// Timing constants
const CLEANUP_DELAY_MS: u64 = 200;
const TEST_RESULT_DELAY_MS: u64 = 50;
const DEFAULT_TESTS_DIR: &str = "FlintBenchmark/tests";

// Progress bar constants
const PROGRESS_BAR_WIDTH: usize = 40;

/// Output from a test run, including results and failure details
pub struct TestRunOutput {
    pub results: Vec<TestResult>,
    /// First failure detail per failed test: (test_name, failure_detail)
    pub failures: Vec<(String, AssertFailure)>,
}

pub struct TestExecutor {
    bot: TestBot,
    action_delay_ms: u64,
    recorder: Option<recorder::RecorderState>,
    verbose: bool,
    quiet: bool,
    fail_fast: bool,
    pos1: Option<[i32; 3]>,
}

impl Default for TestExecutor {
    fn default() -> Self {
        Self {
            bot: TestBot::new(),
            action_delay_ms: COMMAND_DELAY_MS,
            recorder: None,
            verbose: false,
            quiet: false,
            fail_fast: false,
            pos1: None,
        }
    }
}

impl TestExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_action_delay(&mut self, delay_ms: u64) {
        self.action_delay_ms = delay_ms;
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

    pub async fn connect(&mut self, server: &str) -> Result<()> {
        self.bot.connect(server).await
    }

    /// Helper to get a mutable reference to the recorder, or return an error
    fn require_recorder(&mut self) -> Option<&mut recorder::RecorderState> {
        self.recorder.as_mut()
    }

    /// Helper to apply the standard command delay
    async fn delay(&self) {
        tokio::time::sleep(tokio::time::Duration::from_millis(self.action_delay_ms)).await;
    }

    /// Interactive mode: listen for chat commands and execute them
    pub async fn interactive_mode(&mut self, test_loader: &mut TestLoader) -> Result<()> {
        // Interactive mode always uses verbose output
        self.verbose = true;

        // Send help message to chat (without ! to avoid self-triggering)
        self.bot
            .send_command("say FlintMC Interactive Mode active")
            .await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(COMMAND_DELAY_MS)).await;
        self.bot.send_command("say Type: help, search, run, run-all, run-tags, list, reload, stop (prefix with !)").await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(COMMAND_DELAY_MS)).await;

        // Drain any messages (including our own welcome messages)
        tick::drain_chat_messages(&mut self.bot).await;

        // Collect all tests upfront (mutable to allow reload)
        let mut all_test_files = test_loader.collect_all_test_files()?;

        loop {
            // Poll for chat messages
            if let Some((sender, message)) = self
                .bot
                .recv_chat_timeout(std::time::Duration::from_millis(tick::CHAT_POLL_TIMEOUT_MS))
                .await
            {
                let Some((command, args)) = handlers::parse_command(&message) else {
                    continue;
                };

                match command.as_str() {
                    "!help" => {
                        self.handle_help().await?;
                    }

                    "!list" => {
                        self.handle_list(&all_test_files).await?;
                    }

                    "!search" => {
                        if args.is_empty() {
                            self.bot
                                .send_command("say Usage: !search <pattern>")
                                .await?;
                            continue;
                        }
                        let pattern = args.join(" ");
                        self.handle_search(&all_test_files, &pattern).await?;
                    }

                    "!run" => {
                        if args.is_empty() {
                            self.bot
                                .send_command("say Usage: !run <test_name> [step]")
                                .await?;
                            continue;
                        }

                        // Check for step flag
                        let (test_name, step_mode) =
                            if args.last().map(|s| s.as_str()) == Some("step") && args.len() > 1 {
                                (args[..args.len() - 1].join(" "), true)
                            } else {
                                (args.join(" "), false)
                            };

                        self.handle_run(&all_test_files, &test_name, step_mode)
                            .await?;
                    }

                    "!run-all" => {
                        self.handle_run_all(&all_test_files).await?;
                    }

                    "!run-tags" => {
                        if args.is_empty() {
                            self.bot
                                .send_command("say Usage: !run-tags <tag1,tag2,...>")
                                .await?;
                            continue;
                        }
                        let tags: Vec<String> =
                            args[0].split(',').map(|s| s.trim().to_string()).collect();
                        self.handle_run_tags(test_loader, &tags).await?;
                    }

                    "!stop" => {
                        self.bot
                            .send_command("say Exiting interactive mode. Goodbye!")
                            .await?;
                        return Ok(());
                    }

                    "!reload" => {
                        test_loader.verify_and_rebuild_index()?;
                        all_test_files = test_loader.collect_all_test_files()?;
                        self.bot
                            .send_command(&format!("say Reloaded {} tests", all_test_files.len()))
                            .await?;
                    }

                    // Recorder commands
                    "!record" => {
                        if args.is_empty() {
                            self.bot
                                .send_command("say Usage: !record <test_name> [player_name]")
                                .await?;
                            self.bot
                                .send_command(
                                    "say Example: !record my_test or !record fence/fence_connect",
                                )
                                .await?;
                            continue;
                        }
                        let test_name = args[0].clone();
                        // If player name not provided, use sender if available
                        let player_name = args.get(1).cloned().or_else(|| sender.clone());
                        self.handle_record_start(&test_name, test_loader, player_name)
                            .await?;
                    }
                    "!assert_changes" => {
                        self.handle_record_assert_changes().await?;
                    }

                    "!tick" | "!next" => {
                        self.handle_record_tick().await?;
                    }

                    "!pos1" | "!pos" => {
                        if args.len() < 3 {
                            self.bot
                                .send_command("say Usage: !assert <x> <y> <z>")
                                .await?;
                            continue;
                        }
                        self.handle_pos1(&args);
                    }

                    "!assert" => {
                        if args.len() < 3 {
                            self.bot
                                .send_command("say Usage: !assert <x> <y> <z>")
                                .await?;
                            continue;
                        }
                        self.handle_record_assert(&args).await?;
                    }

                    "!save" => {
                        if self.handle_record_save().await? {
                            // Reload tests after successful save
                            test_loader.verify_and_rebuild_index()?;
                            all_test_files = test_loader.collect_all_test_files()?;
                        }
                    }

                    "!cancel" => {
                        self.handle_record_cancel().await?;
                    }

                    _ => {
                        if command.starts_with('!') {
                            self.bot
                                .send_command(&format!(
                                    "say Unknown command: {}. Type !help for commands.",
                                    command
                                ))
                                .await?;
                        }
                    }
                }
            }
        }
    }

    /// Scan blocks in a cube around a center point (ignores air)
    async fn scan_blocks_around(
        &self,
        center: [i32; 3],
        radius: i32,
    ) -> Result<std::collections::HashMap<[i32; 3], String>> {
        let mut blocks = std::collections::HashMap::new();

        for x in (center[0] - radius)..=(center[0] + radius) {
            for y in (center[1] - radius).max(-64)..=(center[1] + radius).min(319) {
                for z in (center[2] - radius)..=(center[2] + radius) {
                    let pos = [x, y, z];
                    if let Ok(Some(block)) = self.bot.get_block(pos).await {
                        let block_id = block::extract_block_id(&block);
                        // Ignore air blocks
                        if !block_id.to_lowercase().contains("air") {
                            blocks.insert(pos, block_id);
                        }
                    }
                }
            }
        }

        Ok(blocks)
    }

    /// Run tests in parallel with merged timeline
    pub async fn run_tests_parallel(
        &mut self,
        tests_with_offsets: &[(TestSpec, [i32; 3])],
        break_after_setup: bool,
    ) -> Result<TestRunOutput> {
        let verbose = self.verbose;

        if verbose {
            println!(
                "{} Running {} tests in parallel\n",
                "→".blue().bold(),
                tests_with_offsets.len()
            );
        }

        // Build global merged timeline using flint-core
        let aggregate = TimelineAggregate::from_tests(tests_with_offsets);

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

        // Clean all test areas before starting
        if verbose {
            println!("{} Cleaning all test areas...", "→".blue());
        }
        for (test, offset) in tests_with_offsets.iter() {
            let region = test.cleanup_region();
            let world_min = actions::apply_offset(region[0], *offset);
            let world_max = actions::apply_offset(region[1], *offset);
            let cmd = format!(
                "fill {} {} {} {} {} {} air",
                world_min[0], world_min[1], world_min[2], world_max[0], world_max[1], world_max[2]
            );
            self.bot.send_command(&cmd).await?;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(CLEANUP_DELAY_MS)).await;

        // Freeze time globally
        self.bot.send_command("tick freeze").await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(COMMAND_DELAY_MS)).await;

        // Break after setup if requested
        let mut stepping_mode = false;
        if break_after_setup {
            let should_continue = tick::wait_for_step(
                &mut self.bot,
                "After test setup (cleanup complete, time frozen)",
            )
            .await?;
            stepping_mode = !should_continue;
        }

        // Track results per test: (passed_assertions, failed_assertions)
        let mut test_results: Vec<(usize, usize)> = vec![(0, 0); tests_with_offsets.len()];

        // Track first failure detail per test
        let mut test_failures: Vec<Option<AssertFailure>> =
            (0..tests_with_offsets.len()).map(|_| None).collect();

        // Track which tests have been cleaned up
        let mut tests_cleaned: Vec<bool> = vec![false; tests_with_offsets.len()];

        // Calculate max tick for each test
        let mut test_max_ticks: Vec<u32> = vec![0; tests_with_offsets.len()];
        for (tick_num, entries) in &aggregate.timeline {
            for (test_idx, _, _) in entries {
                test_max_ticks[*test_idx] = test_max_ticks[*test_idx].max(*tick_num);
            }
        }

        let show_progress = !verbose && !self.quiet;
        let fail_fast = self.fail_fast;

        // Execute merged timeline
        let mut current_tick = 0;
        while current_tick <= aggregate.max_tick {
            if let Some(entries) = aggregate.timeline.get(&current_tick) {
                for (test_idx, entry, value_idx) in entries {
                    let (test, offset) = &tests_with_offsets[*test_idx];

                    match self
                        .execute_action(current_tick, entry, *value_idx, *offset)
                        .await
                    {
                        Ok(ActionOutcome::AssertPassed) => {
                            test_results[*test_idx].0 += 1;
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
                                    String::from(&detail.expected).green(),
                                    String::from(&detail.actual).red()
                                );
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
                    let region = test.cleanup_region();
                    let world_min = actions::apply_offset(region[0], *offset);
                    let world_max = actions::apply_offset(region[1], *offset);
                    let cmd = format!(
                        "fill {} {} {} {} {} {} air",
                        world_min[0],
                        world_min[1],
                        world_min[2],
                        world_max[0],
                        world_max[1],
                        world_max[2]
                    );
                    self.bot.send_command(&cmd).await?;
                    tests_cleaned[test_idx] = true;
                    tokio::time::sleep(tokio::time::Duration::from_millis(COMMAND_DELAY_MS)).await;
                }
            }

            // Check for breakpoint
            if aggregate.breakpoints.contains(&current_tick) || stepping_mode {
                let should_continue = tick::wait_for_step(
                    &mut self.bot,
                    &format!("End of tick {} (before step to next tick)", current_tick),
                )
                .await?;
                stepping_mode = !should_continue;
            }

            // Advance to next tick
            if current_tick < aggregate.max_tick {
                if stepping_mode {
                    tick::step_tick(&mut self.bot, verbose).await?;
                    tokio::time::sleep(tokio::time::Duration::from_millis(CLEANUP_DELAY_MS)).await;
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

                    let sprint_time_ms = if ticks_to_sprint == 1 {
                        tick::step_tick(&mut self.bot, verbose).await?
                    } else if ticks_to_sprint > 1 {
                        tick::sprint_ticks(&mut self.bot, ticks_to_sprint, verbose).await?
                    } else {
                        0
                    };

                    let retry_delay = sprint_time_ms.max(MIN_RETRY_DELAY_MS);
                    tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay)).await;

                    current_tick += ticks_to_sprint;
                }
            } else {
                current_tick += 1;
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

        // Unfreeze time
        self.bot.send_command("tick unfreeze").await?;

        // Clean up remaining tests
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
                let region = test.cleanup_region();
                let world_min = actions::apply_offset(region[0], *offset);
                let world_max = actions::apply_offset(region[1], *offset);
                let cmd = format!(
                    "fill {} {} {} {} {} {} air",
                    world_min[0],
                    world_min[1],
                    world_min[2],
                    world_max[0],
                    world_max[1],
                    world_max[2]
                );
                self.bot.send_command(&cmd).await?;
                tests_cleaned[test_idx] = true;
                tokio::time::sleep(tokio::time::Duration::from_millis(COMMAND_DELAY_MS)).await;
            }
        }

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
        self.bot.send_command(&format!("say {}", summary)).await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(COMMAND_DELAY_MS)).await;

        // Send individual test results to chat
        for result in &results {
            let status = if result.success { "PASS" } else { "FAIL" };
            let msg = format!("say [{}] {}", status, result.test_name);
            self.bot.send_command(&msg).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(TEST_RESULT_DELAY_MS)).await;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(CLEANUP_DELAY_MS)).await;

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

        Ok(TestRunOutput { results, failures })
    }

    async fn execute_action(
        &mut self,
        tick: u32,
        entry: &TimelineEntry,
        value_idx: usize,
        offset: [i32; 3],
    ) -> Result<ActionOutcome> {
        actions::execute_action(
            &mut self.bot,
            tick,
            entry,
            value_idx,
            offset,
            self.action_delay_ms,
            self.verbose,
        )
        .await
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
