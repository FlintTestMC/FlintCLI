//! Command handlers for interactive mode

use anyhow::Result;
use flint_core::loader::TestLoader;
use flint_core::spatial::pair_tests_with_offsets;
use flint_core::test_spec::TestSpec;
use std::path::PathBuf;

use super::{DEFAULT_TESTS_DIR, TestExecutor, block, commands, recorder, tick};
use crate::spatial_batch::group_tests_by_world_config;

fn command_action(label: &str, command: &str) -> serde_json::Value {
    serde_json::json!({
        "label": label,
        "action": { "type": "run_command", "command": command }
    })
}

fn ui_action(spec: &commands::FlintCommandSpec) -> serde_json::Value {
    let dialog = spec.dialog.expect("dialog action spec");
    command_action(
        dialog.label,
        &format!("trigger flintmc_recorder set {}", dialog.trigger),
    )
}

fn recorder_dialog(test_name: &str, tick: u32, action_count: usize) -> serde_json::Value {
    let actions: Vec<_> = commands::COMMANDS
        .iter()
        .filter(|spec| spec.scope == commands::CommandScope::Recorder && spec.dialog.is_some())
        .map(ui_action)
        .collect();
    serde_json::json!({
        "type": "minecraft:multi_action",
        "title": format!("FlintMC Recorder: {test_name} ({tick}/{action_count})"),
        "columns": 2,
        "actions": actions,
        "pause": false,
        "after_action": "wait_for_response"
    })
}

/// Parse command parts from a chat message
/// Returns (command, args) if a valid command was found
pub fn parse_command(message: &str) -> Option<(String, Vec<String>)> {
    // Skip bot's own messages
    if message.contains("flintmc_testbot") || message.contains("[Server]") {
        return None;
    }

    let msg_lower = message.to_lowercase();

    if let Some(action) = msg_lower
        .split_whitespace()
        .find_map(|part| part.strip_prefix("__flintmc_recorder_"))
    {
        let command = commands::from_callback(action)?;
        return Some((commands::primary_alias(command).to_string(), Vec::new()));
    }

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

pub fn is_bot_sender(sender: Option<&str>) -> bool {
    sender == Some("flintmc_testbot")
}

fn load_test_specs(test_files: &[PathBuf]) -> impl Iterator<Item = TestSpec> + '_ {
    test_files
        .iter()
        .filter_map(|test_file| TestSpec::from_file(test_file, false).ok())
}

fn test_label(test: &TestSpec) -> String {
    if test.tags.is_empty() {
        test.name.clone()
    } else {
        format!("{} [{}]", test.name, test.tags.join(", "))
    }
}

fn find_test(test_files: &[PathBuf], name: &str) -> Option<TestSpec> {
    let name = name.to_lowercase();
    let mut partial_match = None;

    for test in load_test_specs(test_files) {
        let test_name = test.name.to_lowercase();
        if test_name == name {
            return Some(test);
        }
        if partial_match.is_none() && test_name.contains(&name) {
            partial_match = Some(test);
        }
    }

    partial_match
}

impl TestExecutor {
    // Command handlers

    pub(super) fn show_recorder_dialog(&self) -> Result<()> {
        let Some(recorder) = self.recorder.as_ref() else {
            return Ok(());
        };
        let target = recorder.player_name.as_deref().unwrap_or("@p");
        let action_count = recorder
            .timeline
            .iter()
            .map(|step| step.actions.len())
            .sum();
        self.bot.send_command(&format!(
            "scoreboard players enable {target} flintmc_recorder"
        ))?;
        self.show_dialog(
            target,
            &recorder_dialog(&recorder.test_name, recorder.current_tick, action_count),
        )
    }

    fn show_dialog(&self, target: &str, dialog: &serde_json::Value) -> Result<()> {
        validate_entity_target(target)?;
        self.bot
            .send_command(&format!("dialog show {target} {dialog}"))
    }

    pub(super) fn poll_recorder_dialog_action(&self) -> Result<()> {
        if self.recorder.is_none() {
            return Ok(());
        }
        for spec in commands::COMMANDS
            .iter()
            .filter(|spec| spec.dialog.is_some())
        {
            let dialog = spec.dialog.expect("filtered dialog action");
            let action = commands::callback_id(spec.command);
            self.bot.send_command(&format!(
                "execute as @a[scores={{flintmc_recorder={}}}] run tell flintmc_testbot __flintmc_recorder_{action}",
                dialog.trigger
            ))?;
        }
        self.bot.send_command(
            "scoreboard players reset @a[scores={flintmc_recorder=1..}] flintmc_recorder",
        )?;
        Ok(())
    }

    pub(super) fn consume_recorder_dialog_action(&self, sender: Option<&str>) -> Result<()> {
        let target = sender
            .or_else(|| {
                self.recorder
                    .as_ref()
                    .and_then(|recorder| recorder.player_name.as_deref())
            })
            .unwrap_or("@a[name=!flintmc_testbot,limit=1,sort=nearest]");
        validate_entity_target(target)?;
        // Reset before the handler refreshes and re-enables the trigger. This
        // makes a button click one-shot even if multiple poll responses arrive.
        self.bot.send_command(&format!(
            "scoreboard players reset {target} flintmc_recorder"
        ))
    }

    pub(super) fn handle_help(&mut self) -> Result<()> {
        self.bot.send_command("say Commands:")?;
        let active_scope = if self.recorder.is_some() {
            commands::CommandScope::Recorder
        } else {
            commands::CommandScope::Main
        };
        for help in commands::COMMANDS
            .iter()
            .filter(|spec| spec.scope == active_scope)
            .filter_map(|spec| spec.help)
        {
            self.bot.send_command(&format!("say {help}"))?;
        }
        Ok(())
    }

    pub(super) fn handle_list(&mut self, all_test_files: &[std::path::PathBuf]) -> Result<()> {
        self.bot
            .send_command(&format!("say Found {} tests:", all_test_files.len()))?;
        for test in load_test_specs(all_test_files) {
            self.bot
                .send_command_synced(&format!("say - {}", test_label(&test)))?;
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
        for test in load_test_specs(all_test_files) {
            if test.name.to_lowercase().contains(&pattern_lower) {
                self.bot
                    .send_command_synced(&format!("say - {}", test_label(&test)))?;
                found += 1;
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
        if let Some(test) = find_test(all_test_files, test_name) {
            if step_mode {
                self.bot.send_command(&format!(
                    "say Running test: {} (step mode - type 's' or 'c')",
                    test.name
                ))?;
            } else {
                self.bot
                    .send_command(&format!("say Running test: {}", test.name))?;
            }

            let tests_with_offsets = pair_tests_with_offsets(vec![test]);
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

    pub(super) fn handle_run_all(&mut self, all_test_files: &[std::path::PathBuf]) -> Result<()> {
        self.bot.send_command(&format!(
            "say Running all {} tests...",
            all_test_files.len()
        ))?;

        let (passed, failed) = self.run_test_groups(load_test_specs(all_test_files).collect())?;
        self.bot.send_command(&format!(
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

        self.bot.send_command(&format!(
            "say Running {} tests with tags {:?}...",
            test_files.len(),
            tags
        ))?;

        let (passed, failed) = self.run_test_groups(load_test_specs(&test_files).collect())?;
        self.bot.send_command(&format!(
            "say Results: {} passed, {} failed",
            passed, failed
        ))?;
        Ok(())
    }

    fn run_test_groups(&mut self, specs: Vec<TestSpec>) -> Result<(usize, usize)> {
        let mut passed = 0;
        let mut failed = 0;
        for group in group_tests_by_world_config(specs) {
            let tests_with_offsets = pair_tests_with_offsets(group);
            let output = self.run_tests_parallel(&tests_with_offsets, false)?;
            passed += output.results.iter().filter(|r| r.success).count();
            failed += output.results.iter().filter(|r| !r.success).count();
        }
        Ok((passed, failed))
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
        // Never let the fallback select FlintMC itself: @p is relative to the
        // command source and therefore commonly resolves to the bot.
        recorder_state.player_name = player_name
            .or_else(|| Some("@a[name=!flintmc_testbot,limit=1,sort=nearest]".to_string()));

        // Get tracked player position to set scan center.
        let scan_center = match self.query_record_player_pose(&recorder_state) {
            Ok((pos, _)) => [
                pos[0].floor() as i32,
                pos[1].floor() as i32,
                pos[2].floor() as i32,
            ],
            Err(_) => {
                self.bot.send_command(
                    "say Warning: Could not get player position, using bot position",
                )?;
                self.bot.get_position().unwrap_or([0, 64, 0])
            }
        };

        recorder_state.set_scan_center(scan_center);
        recorder_state.scan_radius = 10; // 10 block radius for scanning

        // Take initial snapshot of blocks
        let initial_blocks = self.scan_blocks_around(scan_center, recorder_state.scan_radius)?;
        recorder_state.snapshot = initial_blocks;

        self.recorder = Some(recorder_state);

        // Dialog buttons communicate through a trigger objective, avoiding the
        // confirmation screen Minecraft shows for chat-sending click actions.
        self.bot
            .send_command("scoreboard objectives add flintmc_recorder trigger")?;

        // Freeze time for controlled recording
        self.bot.send_command_synced("tick freeze")?;

        self.bot.send_command(
            "tellraw @a[name=!flintmc_testbot] {\"text\":\"Recorder: press Esc to close the controls and move/build. Recording stays active; type !recorder to reopen the controls without advancing.\",\"color\":\"yellow\"}",
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

        // Snapshot before advancing tick to capture all changes
        self.handle_record_snapshot()?;

        // Step the game tick
        tick::step_tick(&mut self.bot, false)?;

        // Now advance our recording tick counter
        let recorder = self.require_recorder().unwrap();
        recorder.next_tick();

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
            } else {
                self.bot.send_command(&format!(
                    "say No block found at [{}, {}, {}]",
                    pos[0], pos[1], pos[2]
                ))?;
            }
        }
        Ok(())
    }

    pub(super) fn handle_record_assert_target(&mut self) -> Result<()> {
        let Some(pos) = self.looked_at_record_block()? else {
            self.bot
                .send_command("say Recorder: no block in sight within 6 blocks.")?;
            return Ok(());
        };
        let args = pos.map(|coordinate| coordinate.to_string()).to_vec();
        self.last_assert_pos = args.clone();
        self.handle_record_assert(&args)
    }

    pub(super) fn handle_record_pos1_target(&mut self) -> Result<()> {
        let Some(pos) = self.looked_at_record_block()? else {
            self.bot
                .send_command("say Recorder: no block in sight within 6 blocks.")?;
            return Ok(());
        };
        self.pos1 = Some(pos);
        Ok(())
    }

    pub(super) fn handle_record_sprint_target(&mut self) -> Result<()> {
        let Some(pos) = self.looked_at_record_block()? else {
            self.bot
                .send_command("say Recorder: no block in sight within 6 blocks.")?;
            return Ok(());
        };
        self.last_assert_pos = pos.map(|coordinate| coordinate.to_string()).to_vec();
        self.handle_record_sprint(1)
    }

    pub(super) fn handle_record_assert_changes(&mut self) -> Result<()> {
        let Some(recorder) = self.require_recorder() else {
            self.bot.send_command("say No recording in progress.")?;
            return Ok(());
        };

        let count = recorder.convert_actions_to_asserts();
        let _ = count;
        Ok(())
    }

    pub(super) fn handle_record_use(&mut self, args: &[String]) -> Result<()> {
        if self.recorder.is_none() {
            self.bot
                .send_command("say No recording in progress. Use !record <name> to start.")?;
            return Ok(());
        }

        let item = args.first().cloned();
        if args.len() > 1 {
            self.bot.send_command("say Usage: !use [item]")?;
            return Ok(());
        }

        self.handle_record_snapshot()?;

        let (pos, rot) = {
            let recorder = self.recorder.as_ref().unwrap();
            self.query_record_player_pose(recorder)?
        };

        let recorder = self.recorder.as_mut().unwrap();
        recorder.record_use(pos, Some(rot), item.clone());

        Ok(())
    }

    pub(super) fn handle_record_save(&mut self) -> Result<bool> {
        let Some(recorder) = self.recorder.take() else {
            self.bot.send_command("say No recording in progress.")?;
            return Ok(false);
        };

        // Check if there's anything to save
        if recorder.timeline.is_empty() {
            self.bot
                .send_command("say Warning: No actions recorded! Test will be empty.")?;
        }

        match recorder.save() {
            Ok(path) => {
                self.bot.send_command(&format!(
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
        let recorder = match self.recorder.as_ref() {
            Some(r) => r,
            None => {
                self.bot.send_command("say No recording in progress.")?;
                return Ok(());
            }
        };

        let scan_radius = recorder.scan_radius;
        let scan_center = recorder.scan_center.unwrap_or([0, 64, 0]);

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
                recorder.record_remove(*pos);
                changes += 1;
            }
        }

        let _ = changes;
        Ok(())
    }

    pub(super) fn handle_record_cancel(&mut self) -> Result<()> {
        if self.recorder.take().is_some() {
            // Unfreeze time after cancelling
            self.bot.send_command("tick unfreeze")?;
            self.bot.send_command("say Recording cancelled.")?;
        } else {
            self.bot.send_command("say No recording in progress.")?;
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

    fn query_record_player_pose(
        &self,
        recorder: &recorder::RecorderState,
    ) -> Result<([f64; 3], [f32; 2])> {
        let target = recorder
            .player_name
            .as_deref()
            .unwrap_or("@a[name=!flintmc_testbot,limit=1,sort=nearest]");
        let pos = self.query_entity_vec3(target, "Pos")?;
        let rot = self.query_entity_vec2_f32(target, "Rotation")?;
        Ok((pos, rot))
    }

    fn looked_at_record_block(&self) -> Result<Option<[i32; 3]>> {
        let Some(recorder) = self.recorder.as_ref() else {
            return Ok(None);
        };
        let (feet, rot) = self.query_record_player_pose(recorder)?;
        let yaw = (rot[0] as f64).to_radians();
        let pitch = (rot[1] as f64).to_radians();
        let direction = [
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let eye = [feet[0], feet[1] + 1.62, feet[2]];
        let mut previous = None;
        for step in 1..=120 {
            let distance = step as f64 * 0.05;
            let pos = [
                (eye[0] + direction[0] * distance).floor() as i32,
                (eye[1] + direction[1] * distance).floor() as i32,
                (eye[2] + direction[2] * distance).floor() as i32,
            ];
            if previous == Some(pos) {
                continue;
            }
            previous = Some(pos);
            if let Some(block) = self.bot.get_block(pos)?
                && !block::extract_block_id(&block)
                    .to_lowercase()
                    .contains("air")
            {
                return Ok(Some(pos));
            }
        }
        Ok(None)
    }

    fn query_entity_vec3(&self, target: &str, path: &str) -> Result<[f64; 3]> {
        let values = self.query_entity_numbers(target, path)?;
        if values.len() < 3 {
            anyhow::bail!("entity {path} query returned fewer than 3 values");
        }
        Ok([values[0], values[1], values[2]])
    }

    fn query_entity_vec2_f32(&self, target: &str, path: &str) -> Result<[f32; 2]> {
        let values = self.query_entity_numbers(target, path)?;
        if values.len() < 2 {
            anyhow::bail!("entity {path} query returned fewer than 2 values");
        }
        Ok([values[0] as f32, values[1] as f32])
    }

    fn query_entity_numbers(&self, target: &str, path: &str) -> Result<Vec<f64>> {
        validate_entity_target(target)?;
        while self
            .bot
            .recv_chat_timeout(std::time::Duration::from_millis(
                tick::CHAT_DRAIN_TIMEOUT_MS,
            ))
            .is_some()
        {}
        self.bot
            .send_command(&format!("data get entity {target} {path}"))?;

        let timeout = std::time::Duration::from_secs(3);
        let started = std::time::Instant::now();
        while started.elapsed() < timeout {
            if let Some((_, message)) = self
                .bot
                .recv_chat_timeout(std::time::Duration::from_millis(tick::CHAT_POLL_TIMEOUT_MS))
            {
                if message.contains(path) || message.contains("entity data") {
                    let values = parse_numbers_after_colon(&message);
                    if !values.is_empty() {
                        return Ok(values);
                    }
                }
                if message.contains("No entity was found") || message.contains("Found no elements")
                {
                    anyhow::bail!("failed to query entity {target} {path}: {message}");
                }
            }
        }

        anyhow::bail!("timed out querying entity {target} {path}")
    }
}

fn validate_entity_target(target: &str) -> Result<()> {
    if target.is_empty()
        || target
            .chars()
            .any(|c| c.is_whitespace() || c == '/' || c == ';')
    {
        anyhow::bail!("invalid player/entity target for recording: {target}");
    }
    Ok(())
}

fn parse_numbers_after_colon(message: &str) -> Vec<f64> {
    let value_part = message
        .split_once(':')
        .map(|(_, value)| value)
        .unwrap_or(message);
    value_part
        .split(|c: char| {
            !(c.is_ascii_digit() || c == '-' || c == '+' || c == '.' || c == 'e' || c == 'E')
        })
        .filter_map(|part| {
            if part.is_empty() || part == "-" || part == "+" || part == "." {
                None
            } else {
                part.parse::<f64>().ok()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{is_bot_sender, parse_command};

    #[test]
    fn recorder_dialog_callback_maps_to_existing_command() {
        assert_eq!(
            parse_command("__flintmc_recorder_assert_changes"),
            Some(("!assert_changes".to_string(), vec![]))
        );
    }

    #[test]
    fn identifies_bot_as_command_sender() {
        assert!(is_bot_sender(Some("flintmc_testbot")));
        assert!(!is_bot_sender(Some("Coco9486")));
        assert!(!is_bot_sender(None));
    }
}
