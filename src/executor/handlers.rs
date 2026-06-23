//! Command handlers for interactive mode

use anyhow::Result;
use flint_core::loader::TestLoader;
use flint_core::spatial::calculate_test_offset_default;
use flint_core::test_spec::TestSpec;

use super::{
    COMMAND_DELAY_MS, DEFAULT_TESTS_DIR, TEST_RESULT_DELAY_MS, TestExecutor, block, recorder,
};

/// Parse command parts from a chat message
/// Returns (command, args) if a valid command was found
pub fn parse_command(message: &str) -> Option<(String, Vec<String>)> {
    // Skip bot's own messages
    if message.contains("flintmc_testbot") || message.contains("[Server]") {
        return None;
    }

    let msg_lower = message.to_lowercase();

    // Extract command from message (look for !command pattern)
    let cmd_start = msg_lower.find('!')?;
    let command_str = &message[cmd_start..];

    let parts: Vec<&str> = command_str.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let command = parts[0].to_lowercase();
    let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();

    Some((command, args))
}

impl TestExecutor {
    // Command handlers

    pub(super) fn handle_help(&mut self) -> Result<()> {
        self.bot.send_command("say Commands:")?;
        self.bot
            .send_command("say !search <pattern> - Search tests by name")?;
        self.bot
            .send_command("say !run <test_name> [step] - Run a specific test")?;
        self.bot
            .send_command("say !run-all - Run all tests")?;
        self.bot
            .send_command("say !run-tags <tag1,tag2> - Run tests with tags")?;
        self.bot.send_command("say !list - List all tests")?;
        self.bot
            .send_command("say !reload - Reload test files")?;
        self.bot
            .send_command("say Recorder: !record <name>, !tick/!next, !save, !cancel")?;
        self.bot
            .send_command("say Recorder actions: !assert <x> <y> <z>, !assert_changes")?;
        self.bot
            .send_command(
                "say Recorder actions: !pos1 <x> <y> <z>, !pos - Allow to use assert for a 3d area",
            )?;
        self.bot
            .send_command("say Recorder actions: !sprint <tick> - ticks this ticks and asserts after each tick")?;
        self.bot
            .send_command("say !stop - Exit interactive mode")?;
        Ok(())
    }

    pub(super) fn handle_list(
        &mut self,
        all_test_files: &[std::path::PathBuf],
    ) -> Result<()> {
        self.bot
            .send_command(&format!("say Found {} tests:", all_test_files.len()))?;
        for test_file in all_test_files {
            if let Ok(test) = TestSpec::from_file(test_file, false) {
                let tags = if test.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", test.tags.join(", "))
                };
                self.bot
                    .send_command(&format!("say - {}{}", test.name, tags))?;
                std::thread::sleep(std::time::Duration::from_millis(TEST_RESULT_DELAY_MS));
            }
        }
        Ok(())
    }

    pub(super) fn handle_search(
        &mut self,
        all_test_files: &[std::path::PathBuf],
        pattern: &str,
    ) -> Result<()> {
        let pattern_lower = pattern.to_lowercase();
        let mut found = 0;
        for test_file in all_test_files {
            if let Ok(test) = TestSpec::from_file(test_file, false) {
                if test.name.to_lowercase().contains(&pattern_lower) {
                    let tags = if test.tags.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", test.tags.join(", "))
                    };
                    self.bot
                        .send_command(&format!("say - {}{}", test.name, tags))?;
                    found += 1;
                    std::thread::sleep(std::time::Duration::from_millis(TEST_RESULT_DELAY_MS));
                }
            }
        }
        if found == 0 {
            self.bot
                .send_command(&format!("say No tests matching '{}'", pattern))?;
        } else {
            self.bot
                .send_command(&format!("say Found {} matching tests", found))?;
        }
        Ok(())
    }

    pub(super) fn handle_run(
        &mut self,
        all_test_files: &[std::path::PathBuf],
        test_name: &str,
        step_mode: bool,
    ) -> Result<()> {
        let name_lower = test_name.to_lowercase();

        // First pass: look for exact match
        let mut found_test = None;
        for test_file in all_test_files {
            if let Ok(test) = TestSpec::from_file(test_file, false) {
                if test.name.to_lowercase() == name_lower {
                    found_test = Some(test);
                    break;
                }
            }
        }

        // Second pass: fall back to partial match if no exact match
        if found_test.is_none() {
            for test_file in all_test_files {
                if let Ok(test) = TestSpec::from_file(test_file, false) {
                    if test.name.to_lowercase().contains(&name_lower) {
                        found_test = Some(test);
                        break;
                    }
                }
            }
        }

        if let Some(test) = found_test {
            if step_mode {
                self.bot
                    .send_command(&format!(
                        "say Running test: {} (step mode - type 's' or 'c')",
                        test.name
                    ))?;
            } else {
                self.bot
                    .send_command(&format!("say Running test: {}", test.name))?;
            }

            let offset = calculate_test_offset_default(0, 1);
            let tests_with_offsets = vec![(test, offset)];
            let output = self.run_tests_parallel(&tests_with_offsets, step_mode)?;

            for result in &output.results {
                let status = if result.success { "PASS" } else { "FAIL" };
                self.bot
                    .send_command(&format!("say [{}] {}", status, result.test_name))?;
            }
        } else {
            self.bot
                .send_command(&format!("say Test '{}' not found", test_name))?;
        }
        Ok(())
    }

    pub(super) fn handle_run_all(
        &mut self,
        all_test_files: &[std::path::PathBuf],
    ) -> Result<()> {
        self.bot
            .send_command(&format!(
                "say Running all {} tests...",
                all_test_files.len()
            ))?;

        let mut tests_with_offsets = Vec::new();
        for (idx, test_file) in all_test_files.iter().enumerate() {
            if let Ok(test) = TestSpec::from_file(test_file, false) {
                let offset = calculate_test_offset_default(idx, all_test_files.len());
                tests_with_offsets.push((test, offset));
            }
        }

        let output = self.run_tests_parallel(&tests_with_offsets, false)?;

        let passed = output.results.iter().filter(|r| r.success).count();
        let failed = output.results.len() - passed;
        self.bot
            .send_command(&format!(
                "say Results: {} passed, {} failed",
                passed, failed
            ))?;
        Ok(())
    }

    pub(super) fn handle_run_tags(
        &mut self,
        test_loader: &TestLoader,
        tags: &[String],
    ) -> Result<()> {
        let test_files = test_loader.collect_by_tags(tags);

        if test_files.is_empty() {
            self.bot
                .send_command(&format!("say No tests found with tags: {:?}", tags))?;
            return Ok(());
        }

        self.bot
            .send_command(&format!(
                "say Running {} tests with tags {:?}...",
                test_files.len(),
                tags
            ))?;

        let mut tests_with_offsets = Vec::new();
        for (idx, test_file) in test_files.iter().enumerate() {
            if let Ok(test) = TestSpec::from_file(test_file, false) {
                let offset = calculate_test_offset_default(idx, test_files.len());
                tests_with_offsets.push((test, offset));
            }
        }

        let output = self.run_tests_parallel(&tests_with_offsets, false)?;

        let passed = output.results.iter().filter(|r| r.success).count();
        let failed = output.results.len() - passed;
        self.bot
            .send_command(&format!(
                "say Results: {} passed, {} failed",
                passed, failed
            ))?;
        Ok(())
    }

    // Recorder command handlers

    pub(super) fn handle_record_start(
        &mut self,
        test_name: &str,
        _test_loader: &TestLoader,
        player_name: Option<String>,
    ) -> Result<()> {
        if self.recorder.is_some() {
            self.bot
                .send_command("say Recording already in progress. Use !save or !cancel first.")?;
            return Ok(());
        }

        let tests_root = std::path::Path::new(DEFAULT_TESTS_DIR);
        let mut recorder_state = recorder::RecorderState::new(test_name, tests_root);
        // Default to @p if nothing works
        recorder_state.player_name = player_name.or_else(|| Some("@p".to_string()));

        // Get bot position to set scan center
        let scan_center = match self.bot.get_position() {
            Ok(pos) => pos,
            Err(_) => {
                self.bot
                    .send_command("say Warning: Could not get bot position, using spawn")?;
                [0, 64, 0]
            }
        };

        recorder_state.set_scan_center(scan_center);
        recorder_state.scan_radius = 10; // 10 block radius for scanning

        // Take initial snapshot of blocks
        let initial_blocks = self.scan_blocks_around(scan_center, recorder_state.scan_radius)?;
        recorder_state.snapshot = initial_blocks;

        self.recorder = Some(recorder_state);

        // Freeze time for controlled recording
        self.bot.send_command("tick freeze")?;
        std::thread::sleep(std::time::Duration::from_millis(COMMAND_DELAY_MS));

        self.bot
            .send_command(&format!("say Recording started: {}", test_name))?;
        self.bot
            .send_command("say Time frozen. Block changes will be detected automatically!")?;
        self.bot
            .send_command(
                "say Commands: !assert (add check), !tick (step game tick), !save, !cancel",
            )?;

        Ok(())
    }

    pub(super) fn handle_record_tick(&mut self) -> Result<()> {
        // Check if recorder exists first
        if self.recorder.is_none() {
            self.bot
                .send_command("say No recording in progress. Use !record <name> to start.")?;
            return Ok(());
        }

        let current_tick = self.recorder.as_ref().unwrap().current_tick;

        // Snapshot before advancing tick to capture all changes
        self.handle_record_snapshot()?;

        // Step the game tick
        self.bot.send_command("tick step")?;
        self.delay();

        // Now advance our recording tick counter
        let recorder = self.require_recorder().unwrap();
        recorder.next_tick();
        let new_tick = recorder.current_tick;

        self.bot
            .send_command(&format!(
                "say Stepped game tick, now recording tick {} (was {})",
                new_tick, current_tick
            ))?;

        Ok(())
    }

    pub(super) fn handle_pos1(&mut self, args: &[String]) {
        if args.is_empty() {
            self.pos1 = None;
            return;
        }
        let x = args[0].parse::<i32>().unwrap_or(0);
        let y = args[1].parse::<i32>().unwrap_or(0);
        let z = args[2].parse::<i32>().unwrap_or(0);
        self.pos1 = Some([x, y, z]);
    }

    pub(super) fn handle_record_assert(&mut self, args: &[String]) -> Result<()> {
        let _recorder = match self.recorder.as_mut() {
            Some(r) => r,
            None => {
                self.bot
                    .send_command("say No recording in progress. Use !record <name> to start.")?;
                return Ok(());
            }
        };

        // Parse coordinates from args
        let x = args[0].parse::<i32>().unwrap_or(0);
        let y = args[1].parse::<i32>().unwrap_or(0);
        let z = args[2].parse::<i32>().unwrap_or(0);
        let block_pos = [x, y, z];
        let mut blocks = Vec::new();
        if let Some(pos1) = self.pos1 {
            let min_x = block_pos[0].min(pos1[0]);
            let max_x = block_pos[0].max(pos1[0]);
            let min_y = block_pos[1].min(pos1[1]);
            let max_y = block_pos[1].max(pos1[1]);
            let min_z = block_pos[2].min(pos1[2]);
            let max_z = block_pos[2].max(pos1[2]);

            for x in min_x..=max_x {
                for y in min_y..=max_y {
                    for z in min_z..=max_z {
                        blocks.push([x, y, z]);
                    }
                }
            }
        } else {
            blocks.push(block_pos)
        }
        // Get block at position
        for pos in blocks {
            if let Some(block_str) = self.bot.get_block(pos)? {
                let block_id = block::extract_block_id(&block_str);
                let recorder = self.recorder.as_mut().unwrap();
                recorder.add_assertion(pos, &block_id);

                self.bot
                    .send_command(&format!(
                        "say Added assert at [{}, {}, {}] = {}",
                        pos[0], pos[1], pos[2], block_id
                    ))?;
            } else {
                self.bot
                    .send_command(&format!(
                        "say No block found at [{}, {}, {}]",
                        pos[0], pos[1], pos[2]
                    ))?;
            }
        }
        Ok(())
    }

    pub(super) fn handle_record_assert_changes(&mut self) -> Result<()> {
        let Some(recorder) = self.require_recorder() else {
            self.bot
                .send_command("say No recording in progress.")?;
            return Ok(());
        };

        let count = recorder.convert_actions_to_asserts();
        self.bot
            .send_command(&format!(
                "say Converted {} actions to assertions for this tick.",
                count
            ))?;
        Ok(())
    }

    pub(super) fn handle_record_save(&mut self) -> Result<bool> {
        let Some(recorder) = self.recorder.take() else {
            self.bot
                .send_command("say No recording in progress.")?;
            return Ok(false);
        };

        // Check if there's anything to save
        if recorder.timeline.is_empty() {
            self.bot
                .send_command("say Warning: No actions recorded! Test will be empty.")?;
        }

        match recorder.save() {
            Ok(path) => {
                self.bot
                    .send_command(&format!(
                        "say Test saved to: {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ))?;
                println!("Test saved to: {}", path.display());

                // Print execution commands
                self.bot
                    .send_command(&format!("say To execute: !run {}", recorder.test_name))?;
                println!(
                    "To execute this test locally:\ncargo run -- --server localhost:25565 {}",
                    recorder.test_name
                );
            }
            Err(e) => {
                self.bot
                    .send_command(&format!("say Failed to save test: {}", e))?;
                eprintln!("Failed to save: {}", e);
                return Err(e);
            }
        }

        // Unfreeze time after recording
        self.bot.send_command("tick unfreeze")?;

        Ok(true)
    }

    pub(super) fn handle_record_snapshot(&mut self) -> Result<()> {
        let recorder = match self.recorder.as_mut() {
            Some(r) => r,
            None => {
                self.bot
                    .send_command("say No recording in progress.")?;
                return Ok(());
            }
        };

        let scan_center = recorder.scan_center.unwrap_or([0, 64, 0]);
        let scan_radius = recorder.scan_radius;

        self.bot
            .send_command("say Scanning for block changes...")?;

        // Scan current blocks
        let current_blocks = self.scan_blocks_around(scan_center, scan_radius)?;

        // Compare with initial snapshot and record differences
        let mut changes = 0;
        let recorder = self.recorder.as_mut().unwrap();
        let initial_snapshot = recorder.snapshot.clone();

        for (pos, current_block) in &current_blocks {
            let prev_block = initial_snapshot.get(pos);
            let is_air = current_block.to_lowercase().contains("air");

            // Check if changed
            let changed = match prev_block {
                Some(prev) => prev != current_block,
                None => !is_air, // New non-air block
            };

            if changed {
                if is_air {
                    recorder.record_remove(*pos);
                } else {
                    recorder.record_place(*pos, current_block);
                }
                changes += 1;
            }
        }

        // Also check for blocks that were removed (in initial but now air/gone)
        for pos in initial_snapshot.keys() {
            if !current_blocks.contains_key(pos) {
                // Block is gone (probably outside scan range now, skip)
                continue;
            }
            let current = current_blocks.get(pos);
            if current
                .map(|b| b.to_lowercase().contains("air"))
                .unwrap_or(true)
            {
                // Was a block, now is air
                let recorder = self.recorder.as_mut().unwrap();
                recorder.record_remove(*pos);
                changes += 1;
            }
        }

        self.bot
            .send_command(&format!("say Found {} block changes", changes))?;
        Ok(())
    }

    pub(super) fn handle_record_cancel(&mut self) -> Result<()> {
        if self.recorder.take().is_some() {
            // Unfreeze time after cancelling
            self.bot.send_command("tick unfreeze")?;
            self.bot.send_command("say Recording cancelled.")?;
        } else {
            self.bot
                .send_command("say No recording in progress.")?;
        }
        Ok(())
    }

    pub(super) fn handle_record_sprint(&mut self, ticks: u32) -> Result<()> {
        for _ in 0..ticks {
            self.handle_record_tick()?;
            self.handle_record_assert(&self.last_assert_pos.clone())?;
        }
        Ok(())
    }
}
