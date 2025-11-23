use crate::bot::TestBot;
use anyhow::Result;
use colored::Colorize;
use flint_core::results::TestResult;
use flint_core::test_spec::{ActionType, TestSpec, TimelineEntry};
use flint_core::timeline::TimelineAggregate;
use std::io::{self, Write};

// Constants for timing and retries
const CHAT_DRAIN_TIMEOUT_MS: u64 = 10;
const CHAT_POLL_TIMEOUT_MS: u64 = 100;
const COMMAND_DELAY_MS: u64 = 100;
const CLEANUP_DELAY_MS: u64 = 200;
const BLOCK_POLL_ATTEMPTS: u32 = 10;
const BLOCK_POLL_DELAY_MS: u64 = 50;
const PLACE_EACH_DELAY_MS: u64 = 10;
const GAMETIME_QUERY_TIMEOUT_SECS: u64 = 5;
const TICK_STEP_TIMEOUT_SECS: u64 = 5;
const TICK_STEP_POLL_MS: u64 = 50;
const TEST_RESULT_DELAY_MS: u64 = 50;
const SPRINT_TIMEOUT_SECS: u64 = 30;
const MIN_RETRY_DELAY_MS: u64 = 200;

pub struct TestExecutor {
    bot: TestBot,
    use_chat_control: bool,
}

impl Default for TestExecutor {
    fn default() -> Self {
        Self {
            bot: TestBot::new(),
            use_chat_control: false,
        }
    }
}

impl TestExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_chat_control(&mut self, enabled: bool) {
        self.use_chat_control = enabled;
    }

    fn apply_offset(&self, pos: [i32; 3], offset: [i32; 3]) -> [i32; 3] {
        [pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]]
    }

    /// Drain old chat messages
    async fn drain_chat_messages(&mut self) {
        while self
            .bot
            .recv_chat_timeout(std::time::Duration::from_millis(CHAT_DRAIN_TIMEOUT_MS))
            .await
            .is_some()
        {
            // Discard old messages
        }
    }

    /// Normalize block name for comparison (remove minecraft: prefix and underscores)
    fn normalize_block_name(name: &str) -> String {
        name.trim_start_matches("minecraft:")
            .to_lowercase()
            .replace("_", "")
    }

    /// Check if actual block matches expected block name
    fn block_matches(actual: &str, expected: &str) -> bool {
        let actual_lower = actual.to_lowercase();
        let expected_normalized = Self::normalize_block_name(expected);
        actual_lower.contains(&expected_normalized)
            || actual_lower.replace("_", "").contains(&expected_normalized)
    }

    /// Returns true to continue, false to step to next tick only
    async fn wait_for_step(&mut self, reason: &str) -> Result<bool> {
        println!(
            "\n{} {} {}",
            "⏸".yellow().bold(),
            "BREAKPOINT:".yellow().bold(),
            reason
        );

        if self.use_chat_control {
            println!(
                "  Waiting for in-game chat command: {} = step, {} = continue",
                "s".cyan().bold(),
                "c".cyan().bold()
            );

            // Send chat message to inform player
            self.bot
                .send_command("say Waiting for step/continue (s = step, c = continue)")
                .await?;

            // First, drain any old messages from the chat queue
            self.drain_chat_messages().await;

            // Now wait for a fresh chat command
            loop {
                if let Some(message) = self
                    .bot
                    .recv_chat_timeout(std::time::Duration::from_millis(CHAT_POLL_TIMEOUT_MS))
                    .await
                {
                    // Skip messages from the bot itself (contains "Waiting for step/continue")
                    if message.contains("Waiting for step/continue") {
                        continue;
                    }

                    // Look for commands in the message - match exact commands only
                    let msg_lower = message.to_lowercase();
                    let trimmed = msg_lower.trim();

                    // Match the message ending with just "s" or "c" (player commands)
                    if trimmed.ends_with(" s")
                        || trimmed == "s"
                        || trimmed.ends_with(" step")
                        || trimmed == "step"
                    {
                        println!("  {} Received 's' from chat", "→".blue());
                        return Ok(false); // Step mode
                    } else if trimmed.ends_with(" c")
                        || trimmed == "c"
                        || trimmed.ends_with(" continue")
                        || trimmed == "continue"
                    {
                        println!("  {} Received 'c' from chat", "→".blue());
                        return Ok(true); // Continue mode
                    }
                }
            }
        } else {
            println!(
                "  Commands: {} = step one tick, {} = continue to next breakpoint",
                "s".cyan().bold(),
                "c".cyan().bold()
            );
            print!("  > ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let cmd = input.trim().to_lowercase();

            match cmd.as_str() {
                "s" | "step" => Ok(false), // Step mode: only advance one tick
                _ => Ok(true),             // Continue mode (default for Enter or "c")
            }
        }
    }

    /// Poll for a block at the given position with retries
    /// This handles timing issues in CI environments where block updates may take longer
    async fn poll_block_with_retry(
        &self,
        world_pos: [i32; 3],
        expected_block: &str,
    ) -> Result<Option<String>> {
        for attempt in 0..BLOCK_POLL_ATTEMPTS {
            let block = self.bot.get_block(world_pos).await?;

            // Check if the block matches what we expect
            if let Some(ref actual) = block
                && Self::block_matches(actual, expected_block)
            {
                return Ok(block);
            }

            // If not the last attempt, wait before retrying
            if attempt < BLOCK_POLL_ATTEMPTS - 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(BLOCK_POLL_DELAY_MS)).await;
            }
        }

        // Return whatever we have after all retries
        self.bot.get_block(world_pos).await
    }

    /// Poll for a block state property at the given position with retries
    async fn poll_block_state_with_retry(
        &self,
        world_pos: [i32; 3],
        state: &str,
    ) -> Result<Option<String>> {
        for attempt in 0..BLOCK_POLL_ATTEMPTS {
            let state_value = self.bot.get_block_state_property(world_pos, state).await?;
            if state_value.is_some() {
                return Ok(state_value);
            }
            if attempt < BLOCK_POLL_ATTEMPTS - 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(BLOCK_POLL_DELAY_MS)).await;
            }
        }
        Ok(None)
    }

    pub async fn connect(&mut self, server: &str) -> Result<()> {
        self.bot.connect(server).await
    }

    /// Query the current game time from the server
    /// Returns the game time in ticks
    async fn query_gametime(&mut self) -> Result<u32> {
        // Clear any pending chat messages
        self.drain_chat_messages().await;

        // Send the time query command
        self.bot.send_command("time query gametime").await?;

        // Wait for response: "The time is <number>"
        let timeout = std::time::Duration::from_secs(GAMETIME_QUERY_TIMEOUT_SECS);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            if let Some(message) = self
                .bot
                .recv_chat_timeout(std::time::Duration::from_millis(CHAT_POLL_TIMEOUT_MS))
                .await
            {
                // Look for "The time is" message
                if message.contains("The time is") {
                    // Extract the time value
                    if let Some(time_str) = message.split("The time is ").nth(1) {
                        // Parse the number (might have formatting)
                        let time_clean = time_str
                            .chars()
                            .filter(|c| c.is_ascii_digit())
                            .collect::<String>();
                        if let Ok(time) = time_clean.parse::<u32>() {
                            return Ok(time);
                        }
                    }
                }
            }
        }

        anyhow::bail!("Failed to query game time: timeout waiting for response")
    }

    /// Step a single tick using /tick step and verify completion
    /// Returns the time taken
    async fn step_tick(&mut self) -> Result<u64> {
        let before = self.query_gametime().await?;

        let start = std::time::Instant::now();
        self.bot.send_command("tick step").await?;

        // Wait for the tick to actually complete by polling gametime
        let timeout = std::time::Duration::from_secs(TICK_STEP_TIMEOUT_SECS);
        let poll_start = std::time::Instant::now();

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(TICK_STEP_POLL_MS)).await;
            let after = self.query_gametime().await?;

            if after > before {
                let elapsed = start.elapsed().as_millis() as u64;
                println!(
                    "    {} Stepped 1 tick (verified: {} -> {}) in {} ms",
                    "→".dimmed(),
                    before,
                    after,
                    elapsed
                );
                return Ok(elapsed);
            }

            if poll_start.elapsed() >= timeout {
                anyhow::bail!("Tick step verification timeout: game time did not advance");
            }
        }
    }

    /// Sprint ticks and capture the time taken from server output
    /// Returns the ms per tick from the server's sprint completion message
    /// NOTE: Accounts for Minecraft's off-by-one bug where "tick sprint N" executes N+1 ticks
    async fn sprint_ticks(&mut self, ticks: u32) -> Result<u64> {
        // Clear any pending chat messages
        self.drain_chat_messages().await;

        // Account for Minecraft's off-by-one bug: "tick sprint N" executes N+1 ticks
        // So to execute `ticks` ticks, we request ticks-1
        let ticks_to_request = ticks - 1;

        // Send the sprint command
        self.bot
            .send_command(&format!("tick sprint {}", ticks_to_request))
            .await?;

        // Wait for the "Sprint completed" message
        // Server message format: "Sprint completed with X ticks per second, or Y ms per tick"
        let timeout = std::time::Duration::from_secs(SPRINT_TIMEOUT_SECS);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            if let Some(message) = self
                .bot
                .recv_chat_timeout(std::time::Duration::from_millis(CHAT_POLL_TIMEOUT_MS))
                .await
            {
                // Look for "Sprint completed" message
                if message.contains("Sprint completed") {
                    // Try to extract ms per tick
                    // Format: "... or X ms per tick"
                    if let Some(ms_part) = message.split("or ").nth(1)
                        && let Some(ms_str) = ms_part.split(" ms per tick").next()
                        && let Ok(ms) = ms_str.trim().parse::<f64>()
                    {
                        let ms_rounded = ms.ceil() as u64;
                        println!(
                            "    {} Sprint {} ticks completed in {} ms per tick",
                            "⚡".dimmed(),
                            ticks,
                            ms_rounded
                        );
                        // Return total time: ms per tick * number of ticks
                        return Ok(ms_rounded * ticks as u64);
                    }
                    // If we found the message but couldn't parse, use default
                    println!(
                        "    {} Sprint {} ticks completed (timing not parsed)",
                        "⚡".dimmed(),
                        ticks
                    );
                    return Ok(MIN_RETRY_DELAY_MS);
                }
            }
        }

        // Timeout - return default
        println!(
            "    {} Sprint {} ticks (no completion message received)",
            "⚡".dimmed(),
            ticks
        );
        Ok(MIN_RETRY_DELAY_MS)
    }

    pub async fn run_tests_parallel(
        &mut self,
        tests_with_offsets: &[(TestSpec, [i32; 3])],
        break_after_setup: bool,
    ) -> Result<Vec<TestResult>> {
        println!(
            "{} Running {} tests in parallel\n",
            "→".blue().bold(),
            tests_with_offsets.len()
        );

        // Build global merged timeline using flint-core
        let aggregate = TimelineAggregate::from_tests(tests_with_offsets);

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

        // Clean all test areas before starting
        println!("{} Cleaning all test areas...", "→".blue());
        for (test, offset) in tests_with_offsets.iter() {
            let region = test.cleanup_region();
            let world_min = self.apply_offset(region[0], *offset);
            let world_max = self.apply_offset(region[1], *offset);
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
            let should_continue = self
                .wait_for_step("After test setup (cleanup complete, time frozen)")
                .await?;
            stepping_mode = !should_continue;
        }

        // Track results per test
        let mut test_results: Vec<(usize, usize)> = vec![(0, 0); tests_with_offsets.len()]; // (passed, failed)

        // Track which tests have been cleaned up
        let mut tests_cleaned: Vec<bool> = vec![false; tests_with_offsets.len()];

        // Calculate max tick for each test
        let mut test_max_ticks: Vec<u32> = vec![0; tests_with_offsets.len()];
        for (tick, entries) in &aggregate.timeline {
            for (test_idx, _, _) in entries {
                test_max_ticks[*test_idx] = test_max_ticks[*test_idx].max(*tick);
            }
        }

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
                        Ok(true) => {
                            test_results[*test_idx].0 += 1; // increment passed
                        }
                        Ok(false) => {
                            // Non-assertion action
                        }
                        Err(e) => {
                            test_results[*test_idx].1 += 1; // increment failed
                            println!(
                                "    {} [{}] Tick {}: {}",
                                "✗".red().bold(),
                                test.name,
                                current_tick,
                                e.to_string().red()
                            );
                        }
                    }
                }
            }

            // Clean up tests that have completed (current tick exceeds their max tick)
            for test_idx in 0..tests_with_offsets.len() {
                if !tests_cleaned[test_idx] && current_tick > test_max_ticks[test_idx] {
                    let (test, offset) = &tests_with_offsets[test_idx];
                    println!(
                        "\n{} Cleaning up test [{}] (completed at tick {})...",
                        "→".blue(),
                        test.name,
                        test_max_ticks[test_idx]
                    );
                    let region = test.cleanup_region();
                    let world_min = self.apply_offset(region[0], *offset);
                    let world_max = self.apply_offset(region[1], *offset);
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

            // Check for breakpoint at end of this tick (before stepping)
            // Or if we're in stepping mode, break at every tick
            if aggregate.breakpoints.contains(&current_tick) || stepping_mode {
                let should_continue = self
                    .wait_for_step(&format!(
                        "End of tick {} (before step to next tick)",
                        current_tick
                    ))
                    .await?;
                stepping_mode = !should_continue;
            }

            // Advance to next tick (step or sprint depending on mode)
            if current_tick < aggregate.max_tick {
                if stepping_mode {
                    // In stepping mode, only advance one tick at a time
                    self.step_tick().await?;
                    tokio::time::sleep(tokio::time::Duration::from_millis(CLEANUP_DELAY_MS)).await;
                    current_tick += 1;
                } else {
                    // In continue mode, sprint to next event or breakpoint
                    // Use the aggregate's helper method to find the next event
                    let next_event_tick = aggregate
                        .next_event_tick(current_tick)
                        .unwrap_or(aggregate.max_tick + 1);

                    // Calculate how many ticks to sprint
                    let ticks_to_sprint = if next_event_tick <= aggregate.max_tick {
                        next_event_tick - current_tick
                    } else {
                        aggregate.max_tick - current_tick
                    };

                    // Sprint the ticks (use step_tick for single tick, sprint_ticks for multiple)
                    let sprint_time_ms = if ticks_to_sprint == 1 {
                        self.step_tick().await?
                    } else if ticks_to_sprint > 1 {
                        self.sprint_ticks(ticks_to_sprint).await?
                    } else {
                        0
                    };

                    // Use sprint timing for retry delay (ensure minimum)
                    let retry_delay = sprint_time_ms.max(MIN_RETRY_DELAY_MS);
                    tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay)).await;

                    current_tick += ticks_to_sprint;
                }
            } else {
                current_tick += 1;
            }
        }

        // Unfreeze time
        self.bot.send_command("tick unfreeze").await?;

        // Clean up any remaining tests that haven't been cleaned yet (edge case)
        for test_idx in 0..tests_with_offsets.len() {
            if !tests_cleaned[test_idx] {
                let (test, offset) = &tests_with_offsets[test_idx];
                println!(
                    "\n{} Cleaning up remaining test [{}]...",
                    "→".blue(),
                    test.name
                );
                let region = test.cleanup_region();
                let world_min = self.apply_offset(region[0], *offset);
                let world_max = self.apply_offset(region[1], *offset);
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

        // Give messages time to be sent before potential disconnect
        tokio::time::sleep(tokio::time::Duration::from_millis(CLEANUP_DELAY_MS)).await;

        Ok(results)
    }

    async fn execute_action(
        &mut self,
        tick: u32,
        entry: &TimelineEntry,
        value_idx: usize,
        offset: [i32; 3],
    ) -> Result<bool> {
        match &entry.action_type {
            ActionType::Place { pos, block } => {
                let world_pos = self.apply_offset(*pos, offset);
                let cmd = format!(
                    "setblock {} {} {} {}",
                    world_pos[0], world_pos[1], world_pos[2], block
                );
                self.bot.send_command(&cmd).await?;
                println!(
                    "    {} Tick {}: place at [{}, {}, {}] = {}",
                    "→".blue(),
                    tick,
                    pos[0],
                    pos[1],
                    pos[2],
                    block.dimmed()
                );
                Ok(false)
            }

            ActionType::PlaceEach { blocks } => {
                for placement in blocks {
                    let world_pos = self.apply_offset(placement.pos, offset);
                    let cmd = format!(
                        "setblock {} {} {} {}",
                        world_pos[0], world_pos[1], world_pos[2], placement.block
                    );
                    self.bot.send_command(&cmd).await?;
                    println!(
                        "    {} Tick {}: place at [{}, {}, {}] = {}",
                        "→".blue(),
                        tick,
                        placement.pos[0],
                        placement.pos[1],
                        placement.pos[2],
                        placement.block.dimmed()
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(PLACE_EACH_DELAY_MS))
                        .await;
                }
                Ok(false)
            }

            ActionType::Fill { region, with } => {
                let world_min = self.apply_offset(region[0], offset);
                let world_max = self.apply_offset(region[1], offset);
                let cmd = format!(
                    "fill {} {} {} {} {} {} {}",
                    world_min[0],
                    world_min[1],
                    world_min[2],
                    world_max[0],
                    world_max[1],
                    world_max[2],
                    with
                );
                self.bot.send_command(&cmd).await?;
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
                    with.dimmed()
                );
                Ok(false)
            }

            ActionType::Remove { pos } => {
                let world_pos = self.apply_offset(*pos, offset);
                let cmd = format!(
                    "setblock {} {} {} air",
                    world_pos[0], world_pos[1], world_pos[2]
                );
                self.bot.send_command(&cmd).await?;
                println!(
                    "    {} Tick {}: remove at [{}, {}, {}]",
                    "→".blue(),
                    tick,
                    pos[0],
                    pos[1],
                    pos[2]
                );
                Ok(false)
            }

            ActionType::Assert { checks } => {
                for check in checks {
                    let world_pos = self.apply_offset(check.pos, offset);

                    // Poll with retries to handle timing issues in CI environments
                    let actual_block = self.poll_block_with_retry(world_pos, &check.is).await?;

                    let success = if let Some(ref actual) = actual_block {
                        Self::block_matches(actual, &check.is)
                    } else {
                        false
                    };

                    if success {
                        println!(
                            "    {} Tick {}: assert block at [{}, {}, {}] is {}",
                            "✓".green(),
                            tick,
                            check.pos[0],
                            check.pos[1],
                            check.pos[2],
                            check.is.dimmed()
                        );
                    } else {
                        anyhow::bail!(
                            "Block at [{}, {}, {}] is not {} (got {:?})",
                            check.pos[0],
                            check.pos[1],
                            check.pos[2],
                            check.is,
                            actual_block
                        );
                    }
                }
                Ok(true)
            }

            ActionType::AssertState { pos, state, values } => {
                let world_pos = self.apply_offset(*pos, offset);
                let expected_value = &values[value_idx];

                // Poll with retries to handle timing issues in CI environments
                let actual_value = self.poll_block_state_with_retry(world_pos, state).await?;

                let success = if let Some(ref actual) = actual_value {
                    actual.contains(expected_value)
                } else {
                    false
                };

                if success {
                    println!(
                        "    {} Tick {}: assert block at [{}, {}, {}] state {} = {}",
                        "✓".green(),
                        tick,
                        pos[0],
                        pos[1],
                        pos[2],
                        state.dimmed(),
                        expected_value.dimmed()
                    );
                    Ok(true)
                } else {
                    anyhow::bail!(
                        "Block at [{}, {}, {}] state {} is not {} (got {:?})",
                        pos[0],
                        pos[1],
                        pos[2],
                        state,
                        expected_value,
                        actual_value
                    );
                }
            }
        }
    }
}
