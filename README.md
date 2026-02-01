# FlintCLI

A command-line tool for running [Flint](https://github.com/FlintTestMC/flint-core) tests against Minecraft servers. Connects via [Azalea](https://github.com/azalea-rs/azalea) and executes tests deterministically using Minecraft's `/tick` command.

## About Flint

**Flint** is a Minecraft testing framework with two main components:

- **[flint-core](https://github.com/FlintTestMC/flint-core)**: Core library for test specifications, parsing, loading, and spatial utilities.
- **FlintCLI** (this project): CLI tool that runs tests against live servers via Azalea.

## Requirements

- Rust nightly (`rustup override set nightly`)
- Minecraft server 1.21.5+
- Bot needs operator permissions on the server

## Installation

```bash
cargo build --release
```

## Quick start

Run all tests in a directory:
```bash
flintmc example_tests/ -s localhost:25565 -r
```

Run a single test:
```bash
flintmc example_tests/basic_placement.json -s localhost:25565
```

Filter by tags:
```bash
flintmc -s localhost:25565 -t redstone
```

Enter interactive mode (in-game chat commands, test recording):
```bash
flintmc -s localhost:25565 -i
```

See **[USAGE.md](USAGE.md)** for the full reference: all flags, output modes, interactive commands, test recording guide, and test format specification.

## License

MIT
