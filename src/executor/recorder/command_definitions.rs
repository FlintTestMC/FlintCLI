// Recorder commands. Parsed by build.rs; not compiled directly.

#[main_command(
    id = "Start",
    aliases = ["!record"],
    help = "!record <name> [player] - Start recording"
)]
fn start(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    if context.args.is_empty() {
        executor
            .bot
            .send_command("say Usage: !record <test_name> [player_name]")?;
        return Ok(());
    }
    let player_name = context
        .args
        .get(1)
        .cloned()
        .or_else(|| context.sender.clone());
    executor.handle_record_start(&context.args[0], context.test_loader, player_name)
}

#[recorder_command(
    id = "Open",
    aliases = ["!recorder"],
    help = "!recorder - Reopen recorder controls"
)]
fn open(executor: &mut TestExecutor, _context: &mut FlintCommandContext<'_>) -> Result<()> {
    if executor.recorder.is_none() {
        executor
            .bot
            .send_command("say No recording in progress. Use !record <name> to start.")?;
    }
    Ok(())
}

#[recorder_command(
    id = "Tick",
    aliases = ["!tick", "!next"],
    help = "!tick/!next - Detect changes and advance one tick",
    label = "Next tick"
)]
fn tick(executor: &mut TestExecutor, _context: &mut FlintCommandContext<'_>) -> Result<()> {
    executor.handle_record_tick()
}

#[recorder_command(
    id = "Use",
    aliases = ["!use"],
    help = "!use [item] - Record interaction at player pose",
    label = "Record use"
)]
fn use_item(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    executor.handle_record_use(context.args)
}

#[recorder_command(
    id = "AssertChanges",
    aliases = ["!assert_changes"],
    help = "!assert_changes - Convert detected changes to assertions",
    label = "Assert changes"
)]
fn assert_changes(
    executor: &mut TestExecutor,
    _context: &mut FlintCommandContext<'_>,
) -> Result<()> {
    executor.handle_record_assert_changes()
}

#[recorder_command(
    id = "Assert",
    aliases = ["!assert"],
    help = "!assert <x> <y> <z> - Assert block or selected region"
)]
fn assert(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    if context.args.len() < 3 {
        executor
            .bot
            .send_command("say Usage: !assert <x> <y> <z>")?;
        return Ok(());
    }
    executor.last_assert_pos = context.args.to_vec();
    executor.handle_record_assert(context.args)
}

#[recorder_command(
    id = "AssertTarget",
    aliases = ["!assert_target"],
    label = "Assert looked-at block"
)]
fn assert_target(
    executor: &mut TestExecutor,
    _context: &mut FlintCommandContext<'_>,
) -> Result<()> {
    executor.handle_record_assert_target()
}

#[recorder_command(
    id = "Position",
    aliases = ["!pos1", "!pos"],
    help = "!pos1 <x> <y> <z> / !pos - Set or clear region corner",
    label = "Clear first corner"
)]
fn position(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    if (!context.args.is_empty() && context.args.len() < 3) || context.args.len() > 3 {
        executor
            .bot
            .send_command("say Usage: !pos1 <x> <y> <z>")?;
        return Ok(());
    }
    executor.handle_pos1(context.args);
    Ok(())
}

#[recorder_command(
    id = "PositionTarget",
    aliases = ["!pos1_target"],
    label = "Set corner from target"
)]
fn position_target(
    executor: &mut TestExecutor,
    _context: &mut FlintCommandContext<'_>,
) -> Result<()> {
    executor.handle_record_pos1_target()
}

#[recorder_command(
    id = "Sprint",
    aliases = ["!sprint"],
    help = "!sprint <ticks> - Advance and assert after each tick"
)]
fn sprint(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    if context.args.len() != 1 {
        executor
            .bot
            .send_command("say Usage: !sprint <ticks>")?;
        return Ok(());
    }
    let ticks = context.args[0].parse::<u32>().unwrap_or(1);
    if ticks == 0 {
        executor
            .bot
            .send_command("say Sprint ticks must be greater than 0")?;
        return Ok(());
    }
    if executor.last_assert_pos.is_empty() {
        executor
            .bot
            .send_command("say Please assert a position first, which should be used for each sprint")?;
        return Ok(());
    }
    executor.handle_record_sprint(ticks)
}

#[recorder_command(
    id = "SprintTarget",
    aliases = ["!sprint_target"],
    label = "Tick + assert target"
)]
fn sprint_target(
    executor: &mut TestExecutor,
    _context: &mut FlintCommandContext<'_>,
) -> Result<()> {
    executor.handle_record_sprint_target()
}

#[recorder_command(
    id = "Save",
    aliases = ["!save"],
    help = "!save - Save recording",
    label = "Save recording"
)]
fn save(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    if executor.handle_record_save()? {
        context.test_loader.verify_and_rebuild_index()?;
        *context.all_test_files = context.test_loader.collect_all_test_files()?;
    }
    Ok(())
}

#[recorder_command(
    id = "Cancel",
    aliases = ["!cancel"],
    help = "!cancel - Discard recording",
    label = "Cancel recording"
)]
fn cancel(executor: &mut TestExecutor, _context: &mut FlintCommandContext<'_>) -> Result<()> {
    executor.handle_record_cancel()
}
