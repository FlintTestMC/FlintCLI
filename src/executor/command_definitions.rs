// General interactive commands. Parsed by build.rs; not compiled directly.

#[main_command(id = "Help", aliases = ["!help"], help = "!help - Show commands")]
fn help(executor: &mut TestExecutor, _context: &mut FlintCommandContext<'_>) -> Result<()> {
    executor.handle_help()
}

#[main_command(id = "List", aliases = ["!list"], help = "!list - List all tests")]
fn list(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    executor.handle_list(context.all_test_files)
}

#[main_command(id = "Search", aliases = ["!search"], help = "!search <pattern> - Search tests by name")]
fn search(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    if context.args.is_empty() {
        executor.bot.send_command("say Usage: !search <pattern>")?;
        return Ok(());
    }
    executor.handle_search(context.all_test_files, &context.args.join(" "))
}

#[main_command(id = "Run", aliases = ["!run"], help = "!run <test_name> [step] - Run a specific test")]
fn run(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    if context.args.is_empty() {
        executor.bot.send_command("say Usage: !run <test_name> [step]")?;
        return Ok(());
    }
    let (test_name, step_mode) = if context.args.last().map(String::as_str) == Some("step") && context.args.len() > 1 {
        (context.args[..context.args.len() - 1].join(" "), true)
    } else {
        (context.args.join(" "), false)
    };
    executor.handle_run(context.all_test_files, &test_name, step_mode)
}

#[main_command(id = "RunAll", aliases = ["!run-all"], help = "!run-all - Run all tests")]
fn run_all(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    executor.handle_run_all(context.all_test_files)
}

#[main_command(id = "RunTags", aliases = ["!run-tags"], help = "!run-tags <tag1,tag2> - Run tests with tags")]
fn run_tags(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    if context.args.is_empty() {
        executor.bot.send_command("say Usage: !run-tags <tag1,tag2,...>")?;
        return Ok(());
    }
    let tags = context.args[0].split(',').map(|tag| tag.trim().to_string()).collect::<Vec<_>>();
    executor.handle_run_tags(context.test_loader, &tags)
}

#[main_command(id = "Reload", aliases = ["!reload"], help = "!reload - Reload test files")]
fn reload(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    context.test_loader.verify_and_rebuild_index()?;
    *context.all_test_files = context.test_loader.collect_all_test_files()?;
    executor.bot.send_command(&format!("say Reloaded {} tests", context.all_test_files.len()))
}

#[main_command(id = "Stop", aliases = ["!stop"], help = "!stop - Exit interactive mode")]
fn stop(executor: &mut TestExecutor, context: &mut FlintCommandContext<'_>) -> Result<()> {
    executor.bot.send_command("say Exiting interactive mode. Goodbye!")?;
    context.exit_interactive = true;
    Ok(())
}
