//! Tick management - gametime queries, stepping, and sprinting

use crate::bot::TestBot;
use anyhow::Result;
use colored::Colorize;

// Constants for tick timing
pub const CHAT_DRAIN_TIMEOUT_MS: u64 = 10;
pub const CHAT_POLL_TIMEOUT_MS: u64 = 100;
pub const GAMETIME_QUERY_TIMEOUT_SECS: u64 = 5;
pub const TICK_STEP_TIMEOUT_SECS: u64 = 5;
pub const SPRINT_TIMEOUT_SECS: u64 = 30;
pub const MIN_RETRY_DELAY_MS: u64 = 200;

/// Drain old chat messages from the bot's queue
pub fn drain_chat_messages(bot: &mut TestBot) {
    while bot
        .recv_chat_timeout(std::time::Duration::from_millis(CHAT_DRAIN_TIMEOUT_MS))
        .is_some()
    {
        // Discard old messages
    }
}

/// Returns true to continue, false to step to next tick only
pub fn wait_for_step(bot: &mut TestBot, reason: &str) -> Result<bool> {
    println!(
        "\n{} {} {}",
        "⏸".yellow().bold(),
        "BREAKPOINT:".yellow().bold(),
        reason
    );

    println!(
        "  Waiting for in-game chat command: {} = step, {} = continue",
        "s".cyan().bold(),
        "c".cyan().bold()
    );

    // Send chat message to inform player
    bot.send_command("say Waiting for step/continue (s = step, c = continue)")?;

    // First, drain any old messages from the chat queue
    drain_chat_messages(bot);

    // Now wait for a fresh chat command
    loop {
        if let Some((_, message)) =
            bot.recv_chat_timeout(std::time::Duration::from_millis(CHAT_POLL_TIMEOUT_MS))
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
}

/// Query the current game time from the server
/// Returns the game time in ticks
pub fn query_gametime(bot: &mut TestBot) -> Result<u32> {
    // Clear any pending chat messages
    drain_chat_messages(bot);

    // Send the time query command
    bot.send_command("time query gametime")?;

    // Wait for response: "The time is <number>"
    let timeout = std::time::Duration::from_secs(GAMETIME_QUERY_TIMEOUT_SECS);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if let Some((_, message)) =
            bot.recv_chat_timeout(std::time::Duration::from_millis(CHAT_POLL_TIMEOUT_MS))
        {
            // Look for "The time is" message
            if message.contains("The game time is") {
                // Extract the time value
                if let Some(time_str) = message.split("The game time is ").nth(1) {
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
/// Returns the time taken in ms
pub fn step_tick(bot: &mut TestBot, verbose: bool) -> Result<u64> {
    let before = query_gametime(bot)?;

    let start = std::time::Instant::now();
    // First acknowledge that the server processed the command, then verify the
    // scheduled tick itself completed by observing game time advance.
    bot.send_command_synced("tick step")?;

    // Each game-time query blocks until its server response arrives, so no polling
    // delay is needed between verification attempts.
    let timeout = std::time::Duration::from_secs(TICK_STEP_TIMEOUT_SECS);
    let poll_start = std::time::Instant::now();

    loop {
        let after = query_gametime(bot)?;

        if after > before {
            bot.sync_client_world()?;
            let elapsed = start.elapsed().as_millis() as u64;
            if verbose {
                println!(
                    "    {} Stepped 1 tick (verified: {} -> {}) in {} ms",
                    "→".dimmed(),
                    before,
                    after,
                    elapsed
                );
            }
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
pub fn sprint_ticks(bot: &mut TestBot, ticks: u32, verbose: bool) -> Result<u64> {
    // Clear any pending chat messages
    drain_chat_messages(bot);

    // Account for Minecraft's off-by-one bug: "tick sprint N" executes N+1 ticks
    // So to execute `ticks` ticks, we request ticks-1
    let ticks_to_request = ticks - 1;

    // Send the sprint command
    bot.send_command(&format!("tick sprint {}", ticks_to_request))?;

    // Wait for the "Sprint completed" message
    // Server message format: "Sprint completed with X ticks per second, or Y ms per tick"
    let timeout = std::time::Duration::from_secs(SPRINT_TIMEOUT_SECS);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if let Some((_, message)) =
            bot.recv_chat_timeout(std::time::Duration::from_millis(CHAT_POLL_TIMEOUT_MS))
        {
            // Look for "Sprint completed" message
            if message.contains("Sprint completed") {
                bot.sync_client_world()?;
                // Try to extract ms per tick
                // Format: "... or X ms per tick"
                if let Some(ms_part) = message.split("or ").nth(1)
                    && let Some(ms_str) = ms_part.split(" ms per tick").next()
                    && let Ok(ms) = ms_str.trim().parse::<f64>()
                {
                    let ms_rounded = ms.ceil() as u64;
                    if verbose {
                        println!(
                            "    {} Sprint {} ticks completed in {} ms per tick",
                            "⚡".dimmed(),
                            ticks,
                            ms_rounded
                        );
                    }
                    // Return total time: ms per tick * number of ticks
                    return Ok(ms_rounded * ticks as u64);
                }
                // If we found the message but couldn't parse, use default
                if verbose {
                    println!(
                        "    {} Sprint {} ticks completed (timing not parsed)",
                        "⚡".dimmed(),
                        ticks
                    );
                }
                return Ok(MIN_RETRY_DELAY_MS);
            }
        }
    }

    anyhow::bail!("Sprint verification timeout: no completion message for {ticks} ticks")
}
