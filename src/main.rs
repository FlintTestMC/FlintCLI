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
use spatial_batch::split_tests_by_simulation_distance;
use flint_core::test_spec::{ActionType, TestSpec};
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

    /// Delay in milliseconds between each action (default: 100)
    #[arg(short = 'd', long = "action-delay", default_value = "100")]
    action_delay: u64,

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

    /// Server simulation distance in chunks. Used to split parallel batches so every
    /// test region stays within ticking range of the bot at the layout center.
    #[arg(long, default_value = "10")]
    simulation_distance: u32,

    /// Milliseconds to wait after teleporting so chunks load on the client (0 = no wait).
    #[arg(long, default_value = "0")]
    chunk_load_delay: u64,
}

fn main() -> Result<()> {
    // Setup logging
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    if let Some(shell) = args.completions {
        clap_complete::generate(
            shell,
            &mut Args::command(),
            "flintmc",
            &mut std::io::stdout(),
        );
        return Ok(());
    }

    let verbose = args.verbose;

    if verbose {
        println!("{}", "FlintMC - Minecraft Testing Framework".green().bold());
        println!();
    }

    let mut test_loader = if let Some(ref path) = args.path {
        if verbose {
            println!("{} Loading tests from {}...", "→".blue(), path.display());
        }
        TestLoader::new(path, args.recursive).with_context(|| {
            format!(
                "Failed to initialize test loader for path: {}",
                path.display()
            )
        })?
    } else {
        let default_path = Path::new("FlintBenchmark/tests");
        TestLoader::new(default_path, true).with_context(|| {
            format!(
                "Failed to initialize test loader for default path: {}",
                default_path.display()
            )
        })?
    };

    // Collect test files - use tags if provided, otherwise collect all
    let test_files = if !args.tags.is_empty() {
        if verbose {
            println!("{} Filtering by tags: {:?}", "→".blue(), args.tags);
        }
        test_loader.collect_by_tags(&args.tags)
    } else {
        test_loader
            .collect_all_test_files()
            .context("Failed to collect test files")?
    };

    // In interactive mode, we don't require tests to be found initially
    if test_files.is_empty() && !args.interactive {
        let location = if !args.tags.is_empty() {
            format!("with tags: {:?}", args.tags)
        } else if let Some(ref path) = args.path {
            format!("at: {}", path.display())
        } else {
            "at default path: FlintBenchmark/tests".to_string()
        };
        eprintln!("{} No test files found {}", "Error:".red().bold(), location);
        std::process::exit(1);
    }

    if verbose && !args.interactive {
        println!("Found {} test file(s)\n", test_files.len());
    }

    // --list: print test names and exit
    if args.list {
        for test_file in &test_files {
            match TestSpec::from_file(test_file, false) {
                Ok(test) => println!("{}", test.name),
                Err(e) => {
                    eprintln!(
                        "{} Failed to load test {}: {}",
                        "Error:".red().bold(),
                        test_file.display(),
                        e
                    );
                }
            }
        }
        return Ok(());
    }

    // --dry-run: show execution plan and exit
    if args.dry_run {
        let chunks: Vec<_> = test_files.chunks(CHUNK_SIZE).collect();
        let n = chunks.len();
        println!(
            "{} tests, {} {} (up to {} tests per batch)",
            format_number(test_files.len()),
            n,
            if n == 1 { "batch" } else { "batches" },
            CHUNK_SIZE
        );
        println!();

        for (chunk_idx, chunk) in chunks.iter().enumerate() {
            if chunks.len() > 1 {
                println!(
                    "Batch {}/{} ({} tests)",
                    chunk_idx + 1,
                    chunks.len(),
                    chunk.len()
                );
            }
            for test_file in chunk.iter() {
                match TestSpec::from_file(test_file, false) {
                    Ok(test) => {
                        let offset =
                            calculate_test_offsets_for_batch_default(std::slice::from_ref(&test))
                                [0];
                        let max_tick = test.max_tick();
                        let assertions = test
                            .timeline
                            .iter()
                            .filter(|e| matches!(e.action_type, ActionType::Assert { .. }))
                            .count();
                        let tags = if test.tags.is_empty() {
                            String::new()
                        } else {
                            format!(" [{}]", test.tags.join(", "))
                        };
                        println!(
                            "  {} ({}t, {}a, offset [{},{},{}]){}",
                            test.name,
                            max_tick,
                            assertions,
                            offset[0],
                            offset[1],
                            offset[2],
                            tags.dimmed()
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "{} Failed to load test {}: {}",
                            "Error:".red().bold(),
                            test_file.display(),
                            e
                        );
                    }
                }
            }
        }
        return Ok(());
    }

    // Require --server for execution modes
    let server = args.server.as_deref().unwrap_or_else(|| {
        eprintln!(
            "{} --server is required when running tests",
            "Error:".red().bold()
        );
        std::process::exit(1);
    });

    // Connect to server
    let mut executor = executor::TestExecutor::new();

    // Set action delay
    executor.set_action_delay(args.action_delay);
    executor.set_chunk_load_delay(args.chunk_load_delay);
    executor.set_verbose(args.verbose);
    executor.set_quiet(args.quiet || !matches!(args.format, OutputFormat::Pretty));
    executor.set_fail_fast(args.fail_fast);
    executor.set_enable_breakpoints(false);
    if let Some(events_path) = args.emit_events.clone() {
        if test_files.len() != 1 {
            eprintln!(
                "{} --emit-events requires exactly one test file (got {})",
                "Error:".red().bold(),
                test_files.len()
            );
            std::process::exit(1);
        }
        executor.set_events_path(events_path);
    }

    if verbose && args.action_delay != 100 {
        println!(
            "{} Action delay set to {} ms",
            "→".yellow(),
            args.action_delay
        );
    }

    // Interactive mode: enter command loop
    if args.interactive {
        println!(
            "{} Interactive mode enabled - listening for chat commands",
            "→".yellow().bold()
        );
        println!("  Commands: !search, !run, !run-all, !run-tags, !list, !reload, !help, !stop");
        println!("  During tests: type 's' to step, 'c' to continue\n");

        println!("{} Connecting to {}...", "→".blue(), server);
        executor.connect(server)?;
        println!("{} Connected successfully\n", "✓".green());

        executor.interactive_mode(&mut test_loader)?;
        return Ok(());
    }

    if verbose {
        println!("{} Connecting to {}...", "→".blue(), server);
    }
    executor.connect(server)?;
    if verbose {
        println!("{} Connected successfully\n", "✓".green());
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

        let sim_batches =
            split_tests_by_simulation_distance(chunk_specs, args.simulation_distance);

        if verbose && sim_batches.len() > 1 {
            println!(
                "  {} Split into {} parallel batch(es) for simulation-distance={}\n",
                "→".blue(),
                sim_batches.len(),
                args.simulation_distance
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

            let offsets = calculate_test_offsets_for_batch_default(sim_batch);
            tests_with_offsets.clear();
            for (test_index, (test, offset)) in sim_batch.iter().cloned().zip(offsets).enumerate()
            {
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
                        failure.tick,
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
