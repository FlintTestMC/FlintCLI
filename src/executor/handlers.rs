//! Command handlers for interactive mode

use anyhow::{Context, Result};
use flint_core::loader::TestLoader;
use flint_core::test_spec::TestSpec;
use std::path::PathBuf;

use super::{
    DEFAULT_TESTS_DIR, TestExecutor, block, commands, parsing::numbers_after_colon, recorder, tick,
};
use crate::spatial_batch::{group_tests_by_world_config, pair_tests_with_offsets};

const RECORDER_GUIDANCE: &str = "Recorder commands work in chat; type !help to list them or !recorder to open the optional controls. Press Esc to close the controls; recording stays active.";

fn block_snapshot_changes(
    previous: &std::collections::HashMap<[i32; 3], String>,
    current: &std::collections::HashMap<[i32; 3], String>,
    min: [i32; 3],
    max: [i32; 3],
) -> Vec<([i32; 3], Option<String>)> {
    let in_region =
        |pos: &[i32; 3]| (0..3).all(|axis| pos[axis] >= min[axis] && pos[axis] <= max[axis]);
    let mut positions: Vec<_> = previous
        .keys()
        .filter(|pos| in_region(pos))
        .chain(current.keys())
        .copied()
        .collect();
    positions.sort_unstable();
    positions.dedup();

    positions
        .into_iter()
        .filter_map(|pos| {
            let before = previous.get(&pos);
            let after = current.get(&pos);
            (before != after).then(|| (pos, after.cloned()))
        })
        .collect()
}

fn position_within_scan(pos: [i32; 3], center: [i32; 3], radius: i32) -> bool {
    (0..3).all(|axis| (pos[axis] - center[axis]).abs() <= radius)
}

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
        "body": [{
            "type": "minecraft:plain_message",
            "contents": { "text": RECORDER_GUIDANCE, "color": "yellow" }
        }],
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

fn load_test_specs(test_files: &[PathBuf]) -> impl Iterator<Item = Result<TestSpec>> + '_ {
    test_files.iter().map(|test_file| {
        TestSpec::from_file(test_file, false)
            .with_context(|| format!("Failed to load test {}", test_file.display()))
    })
}

fn test_label(test: &TestSpec) -> String {
    if test.tags.is_empty() {
        test.name.clone()
    } else {
        format!("{} [{}]", test.name, test.tags.join(", "))
    }
}

fn find_test(test_files: &[PathBuf], name: &str) -> (Option<TestSpec>, Vec<anyhow::Error>) {
    let name = name.to_lowercase();
    let mut partial_match = None;
    let mut errors = Vec::new();

    for test in load_test_specs(test_files) {
        let test = match test {
            Ok(test) => test,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        let test_name = test.name.to_lowercase();
        if test_name == name {
            return (Some(test), errors);
        }
        if partial_match.is_none() && test_name.contains(&name) {
            partial_match = Some(test);
        }
    }

    (partial_match, errors)
}

impl TestExecutor {
    // Command handlers

    fn report_test_load_errors(&self, errors: &[anyhow::Error]) -> Result<()> {
        for error in errors {
            for line in format!("{error:#}").lines() {
                self.bot.send_command(&format!("say Error: {line}"))?;
            }
        }
        Ok(())
    }

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
            match test {
                Ok(test) => self
                    .bot
                    .send_command_synced(&format!("say - {}", test_label(&test)))?,
                Err(error) => self.report_test_load_errors(&[error])?,
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
        for test in load_test_specs(all_test_files) {
            let test = match test {
                Ok(test) => test,
                Err(error) => {
                    self.report_test_load_errors(&[error])?;
                    continue;
                }
            };
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
        self.last_failure_links.clear();
        let (test, load_errors) = find_test(all_test_files, test_name);
        self.report_test_load_errors(&load_errors)?;
        if let Some(test) = test {
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
        } else if load_errors.is_empty() {
            self.bot
                .send_command(&format!("say Test '{}' not found", test_name))?;
        }
        Ok(())
    }

    pub(super) fn handle_run_all(&mut self, all_test_files: &[std::path::PathBuf]) -> Result<()> {
        self.last_failure_links.clear();
        self.bot.send_command(&format!(
            "say Running all {} tests...",
            all_test_files.len()
        ))?;

        let (tests, load_errors): (Vec<_>, Vec<_>) =
            load_test_specs(all_test_files).partition(Result::is_ok);
        let tests = tests.into_iter().filter_map(Result::ok).collect();
        let load_errors: Vec<_> = load_errors.into_iter().filter_map(Result::err).collect();
        self.report_test_load_errors(&load_errors)?;
        let (passed, failed) = self.run_test_groups(tests)?;
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
        self.last_failure_links.clear();
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

        let (tests, load_errors): (Vec<_>, Vec<_>) =
            load_test_specs(&test_files).partition(Result::is_ok);
        let tests = tests.into_iter().filter_map(Result::ok).collect();
        let load_errors: Vec<_> = load_errors.into_iter().filter_map(Result::err).collect();
        self.report_test_load_errors(&load_errors)?;
        let (passed, failed) = self.run_test_groups(tests)?;
        self.bot.send_command(&format!(
            "say Results: {} passed, {} failed",
            passed, failed
        ))?;
        Ok(())
    }

    pub(super) fn handle_failure_link(&self, args: &[String], sender: Option<&str>) -> Result<()> {
        let filter = args.join(" ").to_lowercase();
        let links: Vec<_> = self
            .last_failure_links
            .iter()
            .filter(|(test_name, _)| {
                filter.is_empty() || test_name.to_lowercase().contains(&filter)
            })
            .collect();

        if links.is_empty() {
            self.bot
                .send_command("say No matching failures from the latest test run.")?;
            return Ok(());
        }

        let target = sender.unwrap_or("@a[name=!flintmc_testbot]");
        validate_entity_target(target)?;
        for (test_name, url) in links {
            let message = serde_json::json!({
                "text": format!("[Open failure: {test_name}]"),
                "color": "red",
                "underlined": true,
                "click_event": {
                    "action": "open_url",
                    "url": url
                },
                "hover_event": {
                    "action": "show_text",
                    "value": { "text": "Open this failure in FlintViz" }
                }
            });
            self.bot
                .send_command(&format!("tellraw {target} {message}"))?;
        }
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

    fn record_player_is_within_scan(&self) -> Result<bool> {
        let Some(recorder) = self.recorder.as_ref() else {
            return Ok(false);
        };
        let center = recorder.scan_center.unwrap_or([0, 64, 0]);
        let radius = recorder.scan_radius;
        let (player_pos, _) = self.query_record_player_pose(recorder)?;
        let player_pos = [
            player_pos[0].floor() as i32,
            player_pos[1].floor() as i32,
            player_pos[2].floor() as i32,
        ];

        if position_within_scan(player_pos, center, radius) {
            return Ok(true);
        }

        self.bot.send_command(&format!(
            "say Recorder warning: player [{}, {}, {}] is outside the scan centered at [{}, {}, {}] with radius {}. Move back before recording changes.",
            player_pos[0],
            player_pos[1],
            player_pos[2],
            center[0],
            center[1],
            center[2],
            radius
        ))?;
        Ok(false)
    }

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
            Err(error) => {
                self.bot.send_command(
                    "say Recorder could not start: failed to locate the tracked player",
                )?;
                eprintln!("Recorder could not query tracked player position: {error:#}");
                return Ok(());
            }
        };

        recorder_state.set_scan_center(scan_center);

        // Take initial snapshot of blocks
        let initial_blocks = self.scan_blocks_around(scan_center, recorder_state.scan_radius)?;
        recorder_state.snapshot = initial_blocks;

        self.recorder = Some(recorder_state);
        self.recorder_dialog_enabled = false;

        // Dialog buttons communicate through a trigger objective, avoiding the
        // confirmation screen Minecraft shows for chat-sending click actions.
        self.bot
            .send_command("scoreboard objectives add flintmc_recorder trigger")?;

        // Freeze time for controlled recording
        self.bot.send_command_synced("tick freeze")?;

        let guidance = serde_json::json!({ "text": RECORDER_GUIDANCE, "color": "yellow" });
        self.bot
            .send_command(&format!("tellraw @a[name=!flintmc_testbot] {guidance}"))?;

        Ok(())
    }

    pub(super) fn handle_record_tick(&mut self) -> Result<()> {
        // Check if recorder exists first
        if self.recorder.is_none() {
            self.bot
                .send_command("say No recording in progress. Use !record <name> to start.")?;
            return Ok(());
        }

        if !self.record_player_is_within_scan()? {
            return Ok(());
        }

        // Refresh the buffer without adding actions, then step exactly one tick.
        let (scan_center, scan_radius) = {
            let recorder = self.recorder.as_ref().unwrap();
            (
                recorder.scan_center.unwrap_or([0, 64, 0]),
                recorder.scan_radius,
            )
        };
        let current_blocks = self.scan_blocks_around(scan_center, scan_radius)?;
        self.recorder.as_mut().unwrap().snapshot = current_blocks;

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
                self.bot.send_command(&format!(
                    "say Recorder: asserted [{}, {}, {}] is {}",
                    pos[0], pos[1], pos[2], block_id
                ))?;
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
        if self.recorder.is_none() {
            self.bot.send_command("say No recording in progress.")?;
            return Ok(());
        }

        if !self.record_player_is_within_scan()? {
            return Ok(());
        }

        let recorder = self.recorder.as_ref().unwrap();

        let scan_radius = recorder.scan_radius;
        let scan_center = recorder.scan_center.unwrap_or([0, 64, 0]);
        let min = [
            scan_center[0] - scan_radius,
            (scan_center[1] - scan_radius).max(-64),
            scan_center[2] - scan_radius,
        ];
        let max = [
            scan_center[0] + scan_radius,
            (scan_center[1] + scan_radius).min(319),
            scan_center[2] + scan_radius,
        ];
        let current_blocks = self.scan_region(min, max)?;
        let changes = block_snapshot_changes(&recorder.snapshot, &current_blocks, min, max);

        let recorder = self.recorder.as_mut().unwrap();
        for (pos, current_block) in &changes {
            recorder.add_assertion(*pos, current_block.as_deref().unwrap_or("minecraft:air"));
        }
        recorder.snapshot = current_blocks;
        let count = changes.len();
        self.bot.send_command(&format!(
            "say Recorder: converted {count} block changes to assertions"
        ))?;
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
        if self.recorder.is_none() {
            self.bot.send_command("say No recording in progress.")?;
            return Ok(());
        }

        if !self.record_player_is_within_scan()? {
            return Ok(());
        }

        let recorder = self.recorder.as_ref().unwrap();

        let scan_radius = recorder.scan_radius;
        let scan_center = recorder.scan_center.unwrap_or([0, 64, 0]);
        let min = [
            scan_center[0] - scan_radius,
            (scan_center[1] - scan_radius).max(-64),
            scan_center[2] - scan_radius,
        ];
        let max = [
            scan_center[0] + scan_radius,
            (scan_center[1] + scan_radius).min(319),
            scan_center[2] + scan_radius,
        ];

        // Scan current blocks
        let current_blocks = self.scan_region(min, max)?;

        // Compare with the previous saved world state. Both scans contain only
        // non-air blocks, so a missing current entry is a removal.
        let changes = block_snapshot_changes(&recorder.snapshot, &current_blocks, min, max);
        let recorder = self.recorder.as_mut().unwrap();
        for (pos, current_block) in &changes {
            match current_block {
                Some(block) => recorder.record_place(*pos, block),
                None => recorder.record_remove(*pos),
            }
        }
        recorder.snapshot = current_blocks;
        self.bot.send_command(&format!(
            "say Recorder: snapshot centered at [{}, {}, {}], recorded {} block changes",
            scan_center[0],
            scan_center[1],
            scan_center[2],
            changes.len()
        ))?;
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
                    let values = numbers_after_colon(&message);
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

#[cfg(test)]
mod tests {
    use super::{
        RECORDER_GUIDANCE, block_snapshot_changes, find_test, is_bot_sender, parse_command,
        position_within_scan, recorder_dialog,
    };

    #[test]
    fn scan_boundary_is_inclusive() {
        assert!(position_within_scan([42, 32, -32], [10, 0, 0], 32));
        assert!(!position_within_scan([43, 32, -32], [10, 0, 0], 32));
        assert!(!position_within_scan([42, 33, -32], [10, 0, 0], 32));
    }

    #[test]
    fn snapshot_diff_records_additions_changes_and_removals_in_position_order() {
        let previous = std::collections::HashMap::from([
            ([2, 0, 0], "minecraft:stone".to_string()),
            ([0, 0, 0], "minecraft:dirt".to_string()),
        ]);
        let current = std::collections::HashMap::from([
            ([0, 0, 0], "minecraft:grass_block".to_string()),
            ([1, 0, 0], "minecraft:oak_planks".to_string()),
        ]);

        assert_eq!(
            block_snapshot_changes(&previous, &current, [-10, -10, -10], [10, 10, 10]),
            vec![
                ([0, 0, 0], Some("minecraft:grass_block".to_string())),
                ([1, 0, 0], Some("minecraft:oak_planks".to_string())),
                ([2, 0, 0], None),
            ]
        );
        assert!(
            block_snapshot_changes(&current, &current, [-10, -10, -10], [10, 10, 10]).is_empty()
        );
    }

    #[test]
    fn snapshot_diff_does_not_remove_blocks_outside_moving_scan_window() {
        let previous = std::collections::HashMap::from([
            ([0, 64, 0], "minecraft:stone".to_string()),
            ([100, 64, 100], "minecraft:dirt".to_string()),
        ]);
        let current =
            std::collections::HashMap::from([([101, 64, 100], "minecraft:oak_planks".to_string())]);

        assert_eq!(
            block_snapshot_changes(&previous, &current, [90, 54, 90], [110, 74, 110]),
            vec![
                ([100, 64, 100], None),
                ([101, 64, 100], Some("minecraft:oak_planks".to_string())),
            ]
        );
    }

    #[test]
    fn recorder_dialog_callback_maps_to_existing_command() {
        assert_eq!(
            parse_command("__flintmc_recorder_assert_changes"),
            Some(("!assert_changes".to_string(), vec![]))
        );
        assert_eq!(
            parse_command("__flintmc_recorder_snapshot"),
            Some(("!snapshot".to_string(), vec![]))
        );
    }

    #[test]
    fn identifies_bot_as_command_sender() {
        assert!(is_bot_sender(Some("flintmc_testbot")));
        assert!(!is_bot_sender(Some("Coco9486")));
        assert!(!is_bot_sender(None));
    }

    #[test]
    fn recorder_dialog_explains_chat_commands_and_escape() {
        let dialog = recorder_dialog("example", 2, 3);
        let guidance = dialog["body"][0]["contents"]["text"]
            .as_str()
            .expect("recorder dialog guidance");

        assert_eq!(guidance, RECORDER_GUIDANCE);
        assert!(guidance.contains("!help"));
        assert!(guidance.contains("Esc"));
        assert!(guidance.contains("!recorder"));
        assert!(
            dialog["actions"]
                .as_array()
                .is_some_and(|actions| !actions.is_empty())
        );
    }

    #[test]
    fn test_lookup_preserves_load_errors() {
        let missing = std::path::PathBuf::from("definitely-missing-flint-test.json");
        let (test, errors) = find_test(&[missing], "example");

        assert!(test.is_none());
        assert_eq!(errors.len(), 1);
        assert!(format!("{:#}", errors[0]).contains("Failed to load test"));
    }
}
