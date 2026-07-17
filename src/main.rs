mod bot;
mod executor;
mod spatial_batch;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, ValueEnum};
use clap_complete::Shell;
use colored::Colorize;
use flint_core::format;
use flint_core::format::{format_number, print_concise_summary, print_test_summary};
use flint_core::loader::TestLoader;
use flint_core::results::AssertFailure;
use flint_core::spatial::calculate_test_offsets_for_batch_default;
use flint_core::test_spec::{ActionType, TestSpec};
use spatial_batch::{group_tests_by_world_config, split_tests_by_simulation_distance};
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;
use tracing_subscriber::EnvFilter;

/// Output format for test results
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum OutputFormat {
    /// Human-readable colored output (default)
    #[default]
    Pretty,
    /// Machine-readable JSON
    Json,
    /// Test Anything Protocol v13
    Tap,
    /// JUnit XML
    Junit,
}

// Constants
const CHUNK_SIZE: usize = 100;
const SEPARATOR_WIDTH: usize = 60;

/// Print a separator line
fn print_separator() {
    println!("{}", "═".repeat(SEPARATOR_WIDTH).dimmed());
}

/// Print chunk header
fn print_chunk_header(chunk_idx: usize, total_chunks: usize, chunk_len: usize) {
    println!(
        "{} {} Chunk {}/{} ({} tests)",
        "═".repeat(SEPARATOR_WIDTH).dimmed(),
        "→".blue().bold(),
        chunk_idx + 1,
        total_chunks,
        chunk_len,
    );
    print_separator();
    println!();
}

// ─────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "flintmc")]
#[command(about = "Minecraft server testing framework", long_about = None)]
struct Args {
    /// Path to test file or directory
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,

    /// Server address (e.g., localhost:25565)
    #[arg(short, long)]
    server: Option<String>,

    /// Recursively search directories for test files
    #[arg(short, long)]
    recursive: bool,

    /// Break after test setup (cleanup phase) to allow manual inspection
    #[arg(long)]
    break_after_setup: bool,

    /// Filter tests by tags (can be specified multiple times)
    #[arg(short = 't', long = "tag")]
    tags: Vec<String>,

    /// Interactive mode: listen for chat commands (!search, !run, !run-all, !run-tags)
    #[arg(short = 'i', long)]
    interactive: bool,

    /// Start interactive mode and immediately begin recording a test with this name
    #[arg(long, value_name = "NAME")]
    record: Option<String>,

    /// Verbose output: show all per-action details during test execution
    #[arg(short, long)]
    verbose: bool,

    /// Quiet mode: suppress progress bar
    #[arg(short, long)]
    quiet: bool,

    /// Stop after the first test failure
    #[arg(long)]
    fail_fast: bool,

    /// List discovered tests and exit
    #[arg(long)]
    list: bool,

    /// Show what would be run without connecting to the server
    #[arg(long)]
    dry_run: bool,

    /// Output format for test results
    #[arg(long, value_enum, default_value_t = OutputFormat::Pretty)]
    format: OutputFormat,

    /// Generate shell completions and exit
    #[arg(long, value_enum)]
    completions: Option<Shell>,

    /// Emit per-tick state diffs as JSONL to PATH (single-test only).
    /// Each line is one event: run_started, tick, assert, or run_completed.
    /// Coordinates are emitted in test-local space.
    #[arg(long, value_name = "PATH")]
    emit_events: Option<PathBuf>,
}

fn initialize_logging() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

fn generate_completions(shell: Shell) {
    clap_complete::generate(
        shell,
        &mut Args::command(),
        "flintmc",
        &mut std::io::stdout(),
    );
}

fn create_test_loader(args: &Args) -> Result<TestLoader> {
    match args.path.as_deref() {
        Some(path) => {
            if args.verbose {
                println!("{} Loading tests from {}...", "→".blue(), path.display());
            }
            TestLoader::new(path, args.recursive).with_context(|| {
                format!(
                    "Failed to initialize test loader for path: {}",
                    path.display()
                )
            })
        }
        None => {
            let path = Path::new("FlintBenchmark/tests");
            TestLoader::new(path, true).with_context(|| {
                format!(
                    "Failed to initialize test loader for default path: {}",
                    path.display()
                )
            })
        }
    }
}

fn collect_test_files(args: &Args, loader: &TestLoader) -> Result<Vec<PathBuf>> {
    if args.tags.is_empty() {
        loader
            .collect_all_test_files()
            .context("Failed to collect test files")
    } else {
        if args.verbose {
            println!("{} Filtering by tags: {:?}", "→".blue(), args.tags);
        }
        Ok(loader.collect_by_tags(&args.tags))
    }
}

fn require_discovered_tests(args: &Args, test_files: &[PathBuf], interactive: bool) -> Result<()> {
    if !test_files.is_empty() || interactive {
        return Ok(());
    }
    let location = if !args.tags.is_empty() {
        format!("with tags: {:?}", args.tags)
    } else if let Some(path) = args.path.as_ref() {
        format!("at: {}", path.display())
    } else {
        "at default path: FlintBenchmark/tests".to_string()
    };
    anyhow::bail!("No test files found {location}")
}

fn print_test_list(test_files: &[PathBuf]) {
    for test_file in test_files {
        match TestSpec::from_file(test_file, false) {
            Ok(test) => println!("{}", test.name),
            Err(error) => eprintln!(
                "{} Failed to load test {}: {}",
                "Error:".red().bold(),
                test_file.display(),
                error
            ),
        }
    }
}

fn print_dry_run(test_files: &[PathBuf]) {
    let chunks: Vec<_> = test_files.chunks(CHUNK_SIZE).collect();
    println!(
        "{} tests, {} {} (up to {} tests per batch)\n",
        format_number(test_files.len()),
        chunks.len(),
        if chunks.len() == 1 {
            "batch"
        } else {
            "batches"
        },
        CHUNK_SIZE
    );
    for (chunk_idx, chunk) in chunks.iter().enumerate() {
        if chunks.len() > 1 {
            println!(
                "Batch {}/{} ({} tests)",
                chunk_idx + 1,
                chunks.len(),
                chunk.len()
            );
        }
        for test_file in *chunk {
            match TestSpec::from_file(test_file, false) {
                Ok(test) => {
                    let offset =
                        calculate_test_offsets_for_batch_default(std::slice::from_ref(&test))[0];
                    let assertions = test
                        .timeline
                        .iter()
                        .filter(|entry| matches!(entry.action_type, ActionType::Assert { .. }))
                        .count();
                    let tags = if test.tags.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", test.tags.join(", "))
                    };
                    println!(
                        "  {} ({}t, {}a, offset [{},{},{}]){}",
                        test.name,
                        test.max_tick(),
                        assertions,
                        offset[0],
                        offset[1],
                        offset[2],
                        tags.dimmed()
                    );
                }
                Err(error) => eprintln!(
                    "{} Failed to load test {}: {}",
                    "Error:".red().bold(),
                    test_file.display(),
                    error
                ),
            }
        }
    }
}

fn configured_executor(
    args: &Args,
    test_count: usize,
    interactive: bool,
) -> Result<executor::TestExecutor> {
    let mut executor = executor::TestExecutor::new();
    executor.set_verbose(args.verbose);
    executor.set_quiet(args.quiet || !matches!(args.format, OutputFormat::Pretty));
    executor.set_fail_fast(args.fail_fast);
    executor.set_enable_breakpoints(interactive);
    if let Some(path) = args.emit_events.clone() {
        if test_count != 1 {
            anyhow::bail!("--emit-events requires exactly one test file (got {test_count})");
        }
        executor.set_events_path(path);
    }
    Ok(executor)
}

fn run_interactive_mode(
    args: &Args,
    server: &str,
    executor: &mut executor::TestExecutor,
    loader: &mut TestLoader,
) -> Result<()> {
    println!(
        "{} Interactive mode enabled - listening for chat commands",
        "→".yellow().bold()
    );
    println!("  Commands: !search, !run, !run-all, !run-tags, !list, !reload, !help, !stop");
    println!("  During tests: type 's' to step, 'c' to continue\n");
    println!("{} Connecting to {}...", "→".blue(), server);
    executor.connect(server)?;
    println!("{} Connected successfully\n", "✓".green());
    if let Some(record_name) = args.record.as_deref() {
        executor.start_recording(record_name, loader, None)?;
    }
    executor.interactive_mode(loader)
}

fn main() -> Result<()> {
    initialize_logging();
    let args = Args::parse();

    if let Some(shell) = args.completions {
        generate_completions(shell);
        return Ok(());
    }

    let verbose = args.verbose;

    if verbose {
        println!("{}", "FlintMC - Minecraft Testing Framework".green().bold());
        println!();
    }

    let mut test_loader = create_test_loader(&args)?;
    let test_files = collect_test_files(&args, &test_loader)?;
    let interactive_mode = args.interactive || args.record.is_some();
    require_discovered_tests(&args, &test_files, interactive_mode)?;

    if verbose && !interactive_mode {
        println!("Found {} test file(s)\n", test_files.len());
    }

    if args.list {
        print_test_list(&test_files);
        return Ok(());
    }

    if args.dry_run {
        print_dry_run(&test_files);
        return Ok(());
    }

    let server = args
        .server
        .as_deref()
        .context("--server is required when running tests")?;
    let mut executor = configured_executor(&args, test_files.len(), interactive_mode)?;

    // Interactive mode: enter command loop
    if interactive_mode {
        return run_interactive_mode(&args, server, &mut executor, &mut test_loader);
    }

    if verbose {
        println!("{} Connecting to {}...", "→".blue(), server);
    }
    executor.connect(server)?;
    let effective_chunk_distance = executor.bot.effective_chunk_distance()?;
    let (view_distance, simulation_distance) = executor.bot.detected_distances();
    if verbose {
        println!(
            "{} Connected successfully (view: {}, simulation: {}, effective: {})\n",
            "✓".green(),
            view_distance,
            simulation_distance,
            effective_chunk_distance
        );
    }

    // Load all tests and run in chunks
    let total_tests = test_files.len();
    let chunks: Vec<_> = test_files.chunks(CHUNK_SIZE).collect();
    let total_chunks = chunks.len();

    if verbose {
        println!(
            "{} Running {} tests in {} chunk(s) of up to {}",
            "→".blue().bold(),
            total_tests,
            total_chunks,
            CHUNK_SIZE
        );
        println!(
            "  Each chunk is laid out from cleanup regions with {} block padding\n",
            8
        );
    } else {
        eprintln!("Running {} tests...", format_number(total_tests));
    }

    let start_time = Instant::now();
    let mut all_results = Vec::new();
    let mut all_failures: Vec<(String, AssertFailure)> = Vec::new();
    let mut test_specs_map = std::collections::HashMap::new();

    for (chunk_idx, chunk) in chunks.iter().enumerate() {
        if verbose {
            print_chunk_header(chunk_idx, total_chunks, chunk.len());
        }

        let mut tests_with_offsets = Vec::new();
        let mut chunk_specs = Vec::new();
        for test_file in chunk.iter() {
            match TestSpec::from_file(test_file, false) {
                Ok(test) => {
                    test_specs_map.insert(test.name.clone(), (test.clone(), test_file.clone()));
                    chunk_specs.push(test);
                }
                Err(e) => {
                    eprintln!(
                        "{} Failed to load test {}: {}",
                        "Error:".red().bold(),
                        test_file.display(),
                        e
                    );
                    std::process::exit(1);
                }
            }
        }

        let config_batches = group_tests_by_world_config(chunk_specs);
        let sim_batches: Vec<_> = config_batches
            .into_iter()
            .flat_map(|tests| split_tests_by_simulation_distance(tests, effective_chunk_distance))
            .collect();

        if verbose && sim_batches.len() > 1 {
            println!(
                "  {} Split into {} parallel batch(es) for simulation-distance={}\n",
                "→".blue(),
                sim_batches.len(),
                effective_chunk_distance
            );
        }

        for (sim_batch_idx, sim_batch) in sim_batches.iter().enumerate() {
            if verbose && sim_batches.len() > 1 {
                println!(
                    "  {} Simulation batch {}/{} ({} tests)",
                    "→".blue(),
                    sim_batch_idx + 1,
                    sim_batches.len(),
                    sim_batch.len()
                );
            }

            executor.bot.reset_to_test_origin()?;
            let offsets = calculate_test_offsets_for_batch_default(sim_batch);
            let bot_position = executor.bot.get_position()?;
            tests_with_offsets.clear();
            for (test_index, (test, offset)) in sim_batch.iter().cloned().zip(offsets).enumerate() {
                let offset = [
                    offset[0] + bot_position[0],
                    offset[1],
                    offset[2] + bot_position[2],
                ];
                if verbose {
                    println!(
                        "  {} Test {} (offset: [{}, {}, {}])",
                        "→".blue(),
                        format!("[{}/{}]", test_index + 1, sim_batch.len()).dimmed(),
                        offset[0],
                        offset[1],
                        offset[2]
                    );
                }
                tests_with_offsets.push((test, offset));
            }

            if verbose {
                println!();
            }

            let output =
                executor.run_tests_parallel(&tests_with_offsets, args.break_after_setup)?;

            all_results.extend(output.results);
            all_failures.extend(output.failures);

            if args.fail_fast && !all_failures.is_empty() {
                break;
            }
        }

        if args.fail_fast && !all_failures.is_empty() {
            break;
        }

        if verbose && chunk_idx + 1 < total_chunks {
            println!(
                "\n{} Chunk {}/{} complete ({} tests). Moving to next chunk...\n",
                "✓".green().bold(),
                chunk_idx + 1,
                total_chunks,
                chunk.len()
            );
        }
    }

    let elapsed = start_time.elapsed();

    match args.format {
        OutputFormat::Pretty => {
            if verbose {
                print_test_summary(&all_results, SEPARATOR_WIDTH);
            } else {
                print_concise_summary(&all_results, elapsed);
            }
        }
        OutputFormat::Json => format::print_json(&all_results, elapsed),
        OutputFormat::Tap => format::print_tap(&all_results),
        OutputFormat::Junit => format::print_junit(&all_results, elapsed),
    }

    if all_results.iter().any(|r| !r.success) {
        if matches!(args.format, OutputFormat::Pretty) && !all_failures.is_empty() {
            println!("{}", "═".repeat(SEPARATOR_WIDTH).dimmed());
            println!("{}", "Flint Visualizer Links:".cyan().bold());
            for (test_name, failure) in &all_failures {
                if let Some((spec, path)) = test_specs_map.get(test_name) {
                    let payload = flint_core::viz_link::FailurePayload::new(
                        spec.clone(),
                        Some(path.clone()),
                        vec![failure.clone()],
                        failure.tick(),
                    );
                    let base_url = std::env::var("FLINT_VIZ_URL")
                        .unwrap_or_else(|_| "https://flinttestmc.github.io/FlintViz/#".to_string());
                    if let Ok(url) = flint_core::viz_link::failure_url(&payload, &base_url) {
                        println!("  [Visualizer Link for {}]:", test_name.bold());
                        println!("  {}", url.underline().blue());
                    }
                }
            }
            println!("{}", "═".repeat(SEPARATOR_WIDTH).dimmed());
            println!();
        }
        std::process::exit(1);
    }

    Ok(())
}
