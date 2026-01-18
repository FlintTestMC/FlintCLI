mod bot;
mod executor;

use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;
use flint_core::loader::TestLoader;
use flint_core::results::TestResult;
use flint_core::spatial::calculate_test_offset_default;
use flint_core::test_spec::TestSpec;
use std::path::Path;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

// Constants
const CHUNK_SIZE: usize = 100;
const GRID_SIZE: usize = 10; // Tests are arranged in a 10x10 grid
const SEPARATOR_WIDTH: usize = 60;

/// Print a separator line
fn print_separator() {
    println!("{}", "═".repeat(SEPARATOR_WIDTH).dimmed());
}

/// Print chunk header
fn print_chunk_header(chunk_idx: usize, total_chunks: usize, chunk_len: usize) {
    println!(
        "{} {} Chunk {}/{} ({} tests in {}x{} grid)",
        "═".repeat(SEPARATOR_WIDTH).dimmed(),
        "→".blue().bold(),
        chunk_idx + 1,
        total_chunks,
        chunk_len,
        GRID_SIZE,
        GRID_SIZE
    );
    print_separator();
    println!();
}

/// Print test summary
fn print_test_summary(results: &[TestResult]) {
    println!("\n{}", "═".repeat(SEPARATOR_WIDTH).dimmed());
    println!("{}", "Test Summary".cyan().bold());
    print_separator();

    let total_passed = results.iter().filter(|r| r.success).count();
    let total_failed = results.len() - total_passed;

    for result in results {
        let status = if result.success {
            "PASS".green().bold()
        } else {
            "FAIL".red().bold()
        };
        println!("  [{}] {}", status, result.test_name);
    }

    println!(
        "\n{} tests run: {} passed, {} failed\n",
        results.len(),
        total_passed.to_string().green(),
        total_failed.to_string().red()
    );
}

#[derive(Parser, Debug)]
#[command(name = "flintmc")]
#[command(about = "Minecraft server testing framework", long_about = None)]
struct Args {
    /// Path to test file or directory
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,

    /// Server address (e.g., localhost:25565)
    #[arg(short, long)]
    server: String,

    /// Recursively search directories for test files
    #[arg(short, long)]
    recursive: bool,

    /// Break after test setup (cleanup phase) to allow manual inspection
    #[arg(long)]
    break_after_setup: bool,

    /// Use in-game chat for breakpoint control (type 's' or 'c' in chat)
    #[arg(long)]
    chat_control: bool,

    /// Filter tests by tags (can be specified multiple times)
    #[arg(short = 't', long = "tag")]
    tags: Vec<String>,

    /// Delay in milliseconds between each action (default: 100)
    #[arg(short = 'd', long = "action-delay", default_value = "100")]
    action_delay: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    println!("{}", "FlintMC - Minecraft Testing Framework".green().bold());
    println!();

    let test_loader = if let Some(ref path) = args.path {
        println!("{} Loading tests from {}...", "→".blue(), path.display());
        TestLoader::new(path, args.recursive)
            .with_context(|| format!("Failed to initialize test loader for path: {}", path.display()))?
    } else {
        let default_path = Path::new("FlintBenchmark/tests");
        TestLoader::new(default_path, true)
            .with_context(|| format!("Failed to initialize test loader for default path: {}", default_path.display()))?
    };

    // Collect test files - use tags if provided, otherwise collect all
    let test_files = if !args.tags.is_empty() {
        println!("{} Filtering by tags: {:?}", "→".blue(), args.tags);
        test_loader.collect_by_tags(&args.tags)
            .with_context(|| format!("Failed to collect tests by tags: {:?}", args.tags))?
    } else {
        test_loader.collect_all_test_files()
            .context("Failed to collect test files")?
    };

    if test_files.is_empty() {
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

    println!("Found {} test file(s)\n", test_files.len());

    // Connect to server
    let mut executor = executor::TestExecutor::new();

    // Set action delay
    executor.set_action_delay(args.action_delay);
    if args.action_delay != 100 {
        println!(
            "{} Action delay set to {} ms",
            "→".yellow(),
            args.action_delay
        );
    }

    // Enable chat control if requested
    if args.chat_control {
        executor.set_chat_control(true);
        println!(
            "{} Chat control enabled - you can type 's' or 'c' in game chat",
            "→".yellow()
        );
    }

    println!("{} Connecting to {}...", "→".blue(), args.server);
    executor.connect(&args.server).await?;
    println!("{} Connected successfully\n", "✓".green());

    // Load all tests and run in chunks
    let total_tests = test_files.len();
    let chunks: Vec<_> = test_files.chunks(CHUNK_SIZE).collect();
    let total_chunks = chunks.len();

    println!(
        "{} Running {} tests in {} chunk(s) of up to {}",
        "→".blue().bold(),
        total_tests,
        total_chunks,
        CHUNK_SIZE
    );
    println!(
        "  Each chunk uses a {}x{} grid around spawn\n",
        GRID_SIZE, GRID_SIZE
    );

    let mut all_results = Vec::new();

    for (chunk_idx, chunk) in chunks.iter().enumerate() {
        print_chunk_header(chunk_idx, total_chunks, chunk.len());

        let mut tests_with_offsets = Vec::new();
        for (test_index, test_file) in chunk.iter().enumerate() {
            match TestSpec::from_file(test_file) {
                Ok(test) => {
                    // Calculate offset within this chunk (10x10 grid)
                    let offset = calculate_test_offset_default(test_index, chunk.len());
                    println!(
                        "  {} Grid position: {} (offset: [{}, {}, {}])",
                        "→".blue(),
                        format!("[{}/{}]", test_index + 1, chunk.len()).dimmed(),
                        offset[0],
                        offset[1],
                        offset[2]
                    );
                    tests_with_offsets.push((test, offset));
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

        println!();

        // Run this chunk of tests in parallel using merged timeline
        let chunk_results = executor
            .run_tests_parallel(&tests_with_offsets, args.break_after_setup)
            .await?;

        all_results.extend(chunk_results);

        if chunk_idx + 1 < total_chunks {
            println!(
                "\n{} Chunk {}/{} complete ({} tests). Moving to next chunk...\n",
                "✓".green().bold(),
                chunk_idx + 1,
                total_chunks,
                chunk.len()
            );
        }
    }

    // Print summary using aggregated results from all chunks
    print_test_summary(&all_results);

    if all_results.iter().any(|r| !r.success) {
        std::process::exit(1);
    }

    Ok(())
}
