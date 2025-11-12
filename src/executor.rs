use crate::bot::TestBot;
use crate::test_spec::{Action, ActionType, TestSpec};
use anyhow::Result;
use colored::Colorize;
use std::collections::HashMap;

pub struct TestExecutor {
    bot: TestBot,
}

impl TestExecutor {
    pub fn new() -> Self {
        Self {
            bot: TestBot::new(),
        }
    }

    pub async fn connect(&mut self, server: &str) -> Result<()> {
        self.bot.connect(server).await
    }

    pub async fn run_test(&mut self, test: &TestSpec) -> Result<TestResult> {
        println!("\n{} {}", "Running test:".cyan().bold(), test.name.bold());
        if let Some(desc) = &test.description {
            println!("  {}", desc.dimmed());
        }

        let max_tick = test.max_tick();
        println!("  Timeline: {} ticks\n", max_tick);

        // Clean up test area if specified
        if let Some(cleanup) = &test.cleanup {
            println!("  {} Cleaning test area...", "→".blue());
            let cmd = format!(
                "fill {} {} {} {} {} {} air",
                cleanup.from[0], cleanup.from[1], cleanup.from[2],
                cleanup.to[0], cleanup.to[1], cleanup.to[2]
            );
            self.bot.send_command(&cmd).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        // Group actions by tick
        let mut actions_by_tick: HashMap<u32, Vec<&Action>> = HashMap::new();
        for action in &test.actions {
            actions_by_tick
                .entry(action.tick)
                .or_insert_with(Vec::new)
                .push(action);
        }

        // Freeze time
        self.bot.send_command("tick freeze").await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let mut current_tick = 0;
        let mut passed = 0;
        let mut failed = 0;

        // Execute actions tick by tick
        while current_tick <= max_tick {
            if let Some(actions) = actions_by_tick.get(&current_tick) {
                for action in actions {
                    match self.execute_action(current_tick, action).await {
                        Ok(true) => {
                            passed += 1;
                        }
                        Ok(false) => {
                            // Non-assertion action
                        }
                        Err(e) => {
                            failed += 1;
                            println!(
                                "    {} Tick {}: {}",
                                "✗".red().bold(),
                                current_tick,
                                e.to_string().red()
                            );
                        }
                    }
                }
            }

            // Step to next tick
            if current_tick < max_tick {
                self.bot.send_command("tick step 1").await?;
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
            current_tick += 1;
        }

        // Unfreeze time
        self.bot.send_command("tick unfreeze").await?;

        // Clean up test area after test
        if let Some(cleanup) = &test.cleanup {
            println!("\n  {} Cleaning up test area...", "→".blue());
            let cmd = format!(
                "fill {} {} {} {} {} {} air",
                cleanup.from[0], cleanup.from[1], cleanup.from[2],
                cleanup.to[0], cleanup.to[1], cleanup.to[2]
            );
            self.bot.send_command(&cmd).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        let success = failed == 0;
        println!();
        if success {
            println!("  {} Test passed: {} assertions", "✓".green().bold(), passed);
        } else {
            println!(
                "  {} Test failed: {} passed, {} failed",
                "✗".red().bold(),
                passed,
                failed
            );
        }

        Ok(TestResult {
            test_name: test.name.clone(),
            passed,
            failed,
            success,
        })
    }

    async fn execute_action(&mut self, tick: u32, action: &Action) -> Result<bool> {
        match &action.action_type {
            ActionType::Setblock { pos, block } => {
                let cmd = format!("setblock {} {} {} {}", pos[0], pos[1], pos[2], block);
                self.bot.send_command(&cmd).await?;
                println!(
                    "    {} Tick {}: setblock at [{}, {}, {}] = {}",
                    "→".blue(),
                    tick,
                    pos[0],
                    pos[1],
                    pos[2],
                    block.dimmed()
                );
                Ok(false)
            }

            ActionType::Fill { from, to, block } => {
                let cmd = format!(
                    "fill {} {} {} {} {} {} {}",
                    from[0], from[1], from[2], to[0], to[1], to[2], block
                );
                self.bot.send_command(&cmd).await?;
                println!(
                    "    {} Tick {}: fill [{},{},{}] to [{},{},{}] = {}",
                    "→".blue(),
                    tick,
                    from[0],
                    from[1],
                    from[2],
                    to[0],
                    to[1],
                    to[2],
                    block.dimmed()
                );
                Ok(false)
            }

            ActionType::AssertBlock { pos, block } => {
                // Wait a moment for server to send block update
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                let actual_block = self.bot.get_block(*pos).await?;

                let expected_name = block.trim_start_matches("minecraft:");
                let success = if let Some(ref actual) = actual_block {
                    // Convert to lowercase and check if it contains the expected block name
                    // Handle both "Stone" and "stone", "OakPlanks" and "oak_planks"
                    let actual_lower = actual.to_lowercase();
                    let expected_lower = expected_name.to_lowercase().replace("_", "");
                    actual_lower.contains(&expected_lower) ||
                    actual_lower.replace("_", "").contains(&expected_lower)
                } else {
                    false
                };

                if success {
                    println!(
                        "    {} Tick {}: assert block at [{}, {}, {}] is {}",
                        "✓".green(),
                        tick,
                        pos[0],
                        pos[1],
                        pos[2],
                        block.dimmed()
                    );
                    Ok(true)
                } else {
                    anyhow::bail!(
                        "Block at [{}, {}, {}] is not {} (got {:?})",
                        pos[0],
                        pos[1],
                        pos[2],
                        block,
                        actual_block
                    );
                }
            }

            ActionType::AssertBlockState { pos, property, value } => {
                // Wait a moment for server to send block update
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                let actual_value = self.bot.get_block_state_property(*pos, property).await?;

                let success = if let Some(ref actual) = actual_value {
                    actual.contains(value)
                } else {
                    false
                };

                if success {
                    println!(
                        "    {} Tick {}: assert block at [{}, {}, {}] property {} = {}",
                        "✓".green(),
                        tick,
                        pos[0],
                        pos[1],
                        pos[2],
                        property.dimmed(),
                        value.dimmed()
                    );
                    Ok(true)
                } else {
                    anyhow::bail!(
                        "Block at [{}, {}, {}] property {} is not {} (got {:?})",
                        pos[0],
                        pos[1],
                        pos[2],
                        property,
                        value,
                        actual_value
                    );
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct TestResult {
    pub test_name: String,
    pub passed: usize,
    pub failed: usize,
    pub success: bool,
}
