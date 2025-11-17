use crate::bot::TestBot;
use anyhow::Result;
use colored::Colorize;
use flint_core::results::TestResult;
use flint_core::test_spec::{ActionType, TestSpec, TimelineEntry};
use flint_core::timeline::TimelineAggregate;
use std::io::{self, Write};

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
            self.bot.send_command("say Waiting for step/continue (s = step, c = continue)").await?;

            // First, drain any old messages from the chat queue
            while self
                .bot
                .recv_chat_timeout(std::time::Duration::from_millis(10))
                .await
                .is_some()
            {
                // Discard old messages
            }

            // Now wait for a fresh chat command
            loop {
                if let Some(message) = self
                    .bot
                    .recv_chat_timeout(std::time::Duration::from_millis(100))
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
                    if trimmed.ends_with(" s") || trimmed == "s" || trimmed.ends_with(" step") || trimmed == "step" {
                        println!("  {} Received 's' from chat", "→".blue());
                        return Ok(false); // Step mode
                    } else if trimmed.ends_with(" c") || trimmed == "c" || trimmed.ends_with(" continue") || trimmed == "continue" {
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

    fn apply_offset(&self, pos: [i32; 3], offset: [i32; 3]) -> [i32; 3] {
        [pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]]
    }

    /// Poll for a block at the given position with retries
    /// This handles timing issues in CI environments where block updates may take longer
    async fn poll_block_with_retry(
        &self,
        world_pos: [i32; 3],
        expected_block: &str,
        max_attempts: u32,
        delay_ms: u64,
    ) -> Result<Option<String>> {
        let expected_name = expected_block
            .trim_start_matches("minecraft:")
            .to_lowercase()
            .replace("_", "");

        for attempt in 0..max_attempts {
            let block = self.bot.get_block(world_pos).await?;

            // Check if the block matches what we expect
            if let Some(ref actual) = block {
                let actual_lower = actual.to_lowercase();
                if actual_lower.contains(&expected_name)
                    || actual_lower.replace("_", "").contains(&expected_name)
                {
                    return Ok(block);
                }
            }

            // If not the last attempt, wait before retrying
            if attempt < max_attempts - 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
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
        max_attempts: u32,
        delay_ms: u64,
    ) -> Result<Option<String>> {
        for attempt in 0..max_attempts {
            let state_value = self.bot.get_block_state_property(world_pos, state).await?;
            if state_value.is_some() {
                return Ok(state_value);
            }
            if attempt < max_attempts - 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            }
        }
        Ok(None)
    }

    pub async fn connect(&mut self, server: &str) -> Result<()> {
        self.bot.connect(server).await
    }

    /// Sprint ticks and capture the time taken from server output
    /// Returns the ms per tick from the server's sprint completion message
    async fn sprint_ticks(&mut self, ticks: u32) -> Result<u64> {
        // Clear any pending chat messages
        while self
            .bot
            .recv_chat_timeout(std::time::Duration::from_millis(10))
            .await
            .is_some()
        {
            // Discard old messages
        }

        // Send the sprint command
        self.bot
            .send_command(&format!("tick sprint {}", ticks))
            .await?;

        // Wait for the "Sprint completed" message
        // Server message format: "Sprint completed with X ticks per second, or Y ms per tick"
        let timeout = std::time::Duration::from_secs(30);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            if let Some(message) = self
                .bot
                .recv_chat_timeout(std::time::Duration::from_millis(100))
                .await
            {
                // Look for "Sprint completed" message
                if message.contains("Sprint completed") {
                    // Try to extract ms per tick
                    // Format: "... or X ms per tick"
                    if let Some(ms_part) = message.split("or ").nth(1) {
                        if let Some(ms_str) = ms_part.split(" ms per tick").next() {
                            if let Ok(ms) = ms_str.trim().parse::<f64>() {
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
                        }
                    }
                    // If we found the message but couldn't parse, use default
                    println!(
                        "    {} Sprint {} ticks completed (timing not parsed)",
                        "⚡".dimmed(),
                        ticks
                    );
                    return Ok(200);
                }
            }
        }

        // Timeout - return default
        println!(
            "    {} Sprint {} ticks (no completion message received)",
            "⚡".dimmed(),
            ticks
        );
        Ok(200)
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
        println!("  {} unique tick steps with actions", aggregate.unique_tick_count());
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
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Freeze time globally
        self.bot.send_command("tick freeze").await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

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
                    self.sprint_ticks(1).await?;
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    current_tick += 1;
                } else {
                    // In continue mode, sprint to next event or breakpoint
                    // Use the aggregate's helper method to find the next event
                    let next_event_tick = aggregate.next_event_tick(current_tick)
                        .unwrap_or(aggregate.max_tick + 1);

                    // Calculate how many ticks to sprint
                    let ticks_to_sprint = if next_event_tick <= aggregate.max_tick {
                        next_event_tick - current_tick
                    } else {
                        aggregate.max_tick - current_tick
                    };

                    // Sprint the ticks
                    let sprint_time_ms = if ticks_to_sprint > 0 {
                        self.sprint_ticks(ticks_to_sprint).await?
                    } else {
                        0
                    };

                    // Use sprint timing for retry delay (ensure at least 200ms)
                    let retry_delay = sprint_time_ms.max(200);
                    tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay)).await;

                    current_tick += ticks_to_sprint;
                }
            } else {
                current_tick += 1;
            }
        }

        // Unfreeze time
        self.bot.send_command("tick unfreeze").await?;

        // Clean all test areas after completion
        println!("\n{} Cleaning up all test areas...", "→".blue());
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
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

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
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Send individual test results to chat
        for result in &results {
            let status = if result.success { "PASS" } else { "FAIL" };
            let msg = format!("say [{}] {}", status, result.test_name);
            self.bot.send_command(&msg).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }

        // Give messages time to be sent before potential disconnect
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

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
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
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

                    // Poll with retries: 10 attempts, 50ms apart = up to 500ms total
                    // This handles timing issues in CI environments
                    let actual_block = self
                        .poll_block_with_retry(world_pos, &check.is, 10, 50)
                        .await?;

                    let expected_name = check.is.trim_start_matches("minecraft:");
                    let success = if let Some(ref actual) = actual_block {
                        let actual_lower = actual.to_lowercase();
                        let expected_lower = expected_name.to_lowercase().replace("_", "");
                        actual_lower.contains(&expected_lower)
                            || actual_lower.replace("_", "").contains(&expected_lower)
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

                // Poll with retries: 10 attempts, 50ms apart = up to 500ms total
                // This handles timing issues in CI environments
                let actual_value = self
                    .poll_block_state_with_retry(world_pos, state, 10, 50)
                    .await?;

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

