mod bot;
mod executor;
mod test_spec;

use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "flintmc")]
#[command(about = "Minecraft server testing framework", long_about = None)]
struct Args {
    /// Path to test file or directory
    #[arg(value_name = "PATH")]
    path: PathBuf,

    /// Server address (e.g., localhost:25565)
    #[arg(short, long)]
    server: String,

    /// Recursively search directories for test files
    #[arg(short, long)]
    recursive: bool,
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

    // Collect test files
    let test_files = collect_test_files(&args.path, args.recursive)?;

    if test_files.is_empty() {
        eprintln!("{} No test files found at: {}", "Error:".red().bold(), args.path.display());
        std::process::exit(1);
    }

    println!("Found {} test file(s)\n", test_files.len());

    // Connect to server
    let mut executor = executor::TestExecutor::new();
    println!("{} Connecting to {}...", "→".blue(), args.server);
    executor.connect(&args.server).await?;
    println!("{} Connected successfully\n", "✓".green());

    // Run all tests
    let mut results = Vec::new();
    for test_file in &test_files {
        match test_spec::TestSpec::from_file(test_file) {
            Ok(test) => {
                match executor.run_test(&test).await {
                    Ok(result) => results.push(result),
                    Err(e) => {
                        eprintln!("{} Test execution failed: {}", "Error:".red().bold(), e);
                    }
                }
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

    // Print summary
    println!("\n{}", "═".repeat(60).dimmed());
    println!("{}", "Test Summary".cyan().bold());
    println!("{}", "═".repeat(60).dimmed());

    let total_passed = results.iter().filter(|r| r.success).count();
    let total_failed = results.len() - total_passed;

    for result in &results {
        let status = if result.success {
            "PASS".green().bold()
        } else {
            "FAIL".red().bold()
        };
        println!("  [{}] {}", status, result.test_name);
    }

    println!("\n{} tests run: {} passed, {} failed\n",
        results.len(),
        total_passed.to_string().green(),
        total_failed.to_string().red()
    );

    if total_failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}

fn collect_test_files(path: &PathBuf, recursive: bool) -> Result<Vec<PathBuf>> {
    let mut test_files = Vec::new();

    if path.is_file() {
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            test_files.push(path.clone());
        }
    } else if path.is_dir() {
        if recursive {
            collect_json_files_recursive(path, &mut test_files)?;
        } else {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
                    test_files.push(path);
                }
            }
        }
    }

    test_files.sort();
    Ok(test_files)
}

fn collect_json_files_recursive(dir: &PathBuf, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files_recursive(&path, files)?;
        } else if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            files.push(path);
        }
    }
    Ok(())
}
