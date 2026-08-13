use std::process::{Command, Output};

const SERVER_ENV: &str = "FLINTMC_TEST_SERVER";

fn run_flintmc(args: &[&str]) -> Output {
    let server = std::env::var(SERVER_ENV)
        .unwrap_or_else(|_| panic!("{SERVER_ENV} must contain a Minecraft server address"));

    Command::new(env!("CARGO_BIN_EXE_flintmc"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .args(["--server", &server])
        .output()
        .expect("failed to run FlintCLI")
}

fn command_output(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} failed with {}\n{}",
        output.status,
        command_output(output),
    );
}

#[test]
#[ignore = "requires a live Minecraft server"]
fn valid_fill_passes() {
    let output = run_flintmc(&["tests/command_receipts/valid_fill.json"]);
    assert_success("valid fill", &output);
}

#[test]
#[ignore = "requires a live Minecraft server"]
fn rejected_fill_fails_for_the_right_reason() {
    let output = run_flintmc(&["tests/command_receipts/rejected_fill.json", "--verbose"]);
    let output_text = command_output(&output);

    assert!(
        !output.status.success(),
        "rejected fill unexpectedly passed\n{output_text}"
    );
    assert!(
        output_text.contains("server rejected command"),
        "rejected fill failed for an unexpected reason\n{output_text}"
    );
}

#[test]
#[ignore = "requires a live Minecraft server"]
fn benchmark_suite_passes() {
    let output = run_flintmc(&["FlintBenchmark/tests/", "--recursive", "--quiet"]);
    assert_success("benchmark suite", &output);
}
