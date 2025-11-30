# FlintCLI

A command-line interface for running [Flint](https://github.com/FlintTestMC/flint-core) tests against Minecraft servers. FlintCLI uses [Azalea](https://github.com/azalea-rs/azalea) to connect to servers and execute tests deterministically using Minecraft's `/tick` command.

## About Flint

**Flint** is a Minecraft testing framework that consists of two main components (for now):

- **[flint-core](https://github.com/FlintTestMC/flint-core)**: The core library containing test specifications, parsers, test loading, and spatial utilities. This is the foundation that can be integrated into various tools and environments.
- **FlintCLI** (this project): A CLI tool that uses `flint-core` to run tests against live Minecraft servers via the Azalea bot framework.

## Features

- **Timeline-based testing**: Actions are executed at specific game ticks for deterministic behavior
- **JSON test specification**: Write tests in simple JSON format
- **Block mechanics testing**: Test block states, properties, and interactions
- **Directory support**: Run single test files or entire directories of tests
- **Fast execution**: Uses `/tick freeze` and `/tick step` to skip empty ticks

## Requirements

- Rust 1.85+ (2024 edition)
- Minecraft server 1.21.5+
- Bot needs operator permissions on the server

## Installation

```bash
cargo build --release
```

## Usage

### Run a single test file:
```bash
cargo run -- example_tests/basic_placement.json --server localhost:25565
```

### Run all tests in a directory:
```bash
cargo run -- example_tests/ --server localhost:25565
```

### Run all tests recursively:
```bash
cargo run -- example_tests/ --server localhost:25565 --recursive
```

### Debugging with breakpoints and stepping:
```bash
# Break after test setup (cleanup) to inspect the initial state
cargo run -- example_tests/breakpoint_demo.json --server localhost:25565 --break-after-setup

# Tests can also define breakpoints in their JSON to pause at specific ticks
# See example_tests/breakpoint_demo.json

# Use in-game chat for control (type 's' or 'c' in Minecraft chat)
cargo run -- example_tests/breakpoint_demo.json --server localhost:25565 --break-after-setup --chat-control
```

When a breakpoint is hit, you have two options:
- **`s` (step)**: Execute only the next tick, then break again. Useful for step-by-step debugging.
- **`c` (continue)** or **Enter**: Continue execution until the next breakpoint or test completion.

**Chat Control Mode** (`--chat-control`):
- Instead of typing commands in the terminal, type them in Minecraft chat
- Join the server and type `s` or `c` in chat when at a breakpoint
- Perfect for inspecting blocks in-game while stepping through the test
- Commands must be separate chat messages (e.g., just type `s` and press Enter)

Example stepping workflow:
1. Test hits breakpoint after setup
2. Type `s` (in terminal or chat) to step through tick 0
3. Type `s` again to step through tick 1
4. Type `c` to continue to the next breakpoint
5. Type `c` to finish the test

## Test Format

Each test is a JSON file with the following structure:

```json
{
  "flintVersion": "0.1",
  "name": "test_name",
  "description": "Optional description",
  "tags": ["tag1", "tag2"],
  "dependencies": ["optional_dependency1", "optional_dependency2"],
  "setup": {
    "cleanup": {
      "region": [[x1, y1, z1], [x2, y2, z2]]
    }
  },
  "breakpoints": [1, 3],
  "timeline": [
    {
      "at": 0,
      "do": "place",
      "pos": [x, y, z],
      "block": "minecraft:block_id"
    },
    {
      "at": 1,
      "do": "assert",
      "checks": [
        {"pos": [x, y, z], "is": "minecraft:block_id"}
      ]
    }
  ]
}
```

The `setup.cleanup` field is optional. If specified, the framework will:
1. Fill the area with air **before** the test runs
2. Fill the area with air **after** the test completes

This ensures tests don't interfere with each other.

The `breakpoints` field is optional. If specified, execution will pause at the end of each listed tick, before stepping to the next tick. This allows you to manually inspect the world state in-game during test execution.

## Available Actions

### Block Operations

**place** - Place a single block
```json
{
  "at": 0,
  "do": "place",
  "pos": [x, y, z],
  "block": "minecraft:block_id"
}
```

**place_each** - Place multiple blocks
```json
{
  "at": 0,
  "do": "place_each",
  "blocks": [
    {"pos": [x1, y1, z1], "block": "minecraft:block_id"},
    {"pos": [x2, y2, z2], "block": "minecraft:block_id"}
  ]
}
```

**fill** - Fill a region with blocks
```json
{
  "at": 0,
  "do": "fill",
  "region": [[x1, y1, z1], [x2, y2, z2]],
  "with": "minecraft:block_id"
}
```

**remove** - Remove a block (replace with air)
```json
{
  "at": 0,
  "do": "remove",
  "pos": [x, y, z]
}
```

### Assertions

**assert** - Check block type(s) at position(s)
```json
{
  "at": 1,
  "do": "assert",
  "checks": [
    {"pos": [x, y, z], "is": "minecraft:block_id"}
  ]
}
```

**assert_state** - Check block property value(s)
```json
{
  "at": 1,
  "do": "assert_state",
  "pos": [x, y, z],
  "state": "property_name",
  "values": ["expected_value"]
}
```

For multiple ticks, use an array:
```json
{
  "at": [1, 2, 3],
  "do": "assert_state",
  "pos": [x, y, z],
  "state": "powered",
  "values": ["false", "true", "false"]
}
```

## Example Tests

See the `example_tests/` directory for examples:

- `basic_placement.json` - Simple block placement
- `fences/fence_connects_to_block.json` - Fence connection mechanics
- `fences/fence_to_fence.json` - Fence-to-fence connections
- `redstone/lever_basic.json` - Lever placement and state
- `water/water_source.json` - Water source block

## How It Works

1. `flint-core` loads and parses test JSON files
2. Bot connects to server in spectator mode via Azalea
3. Tests are spatially offset to run in parallel without interference
4. Server time is frozen with `/tick freeze`
5. Actions are grouped by tick and executed
6. Between tick groups, `/tick step 1` advances time
7. Azalea tracks world state from server updates
8. Assertions verify expected block states
9. Results are collected and reported

## Architecture

FlintCLI is built on top of `flint-core` and focuses on Minecraft server integration:

```
FlintCLI (this repo):
src/
├── main.rs      - CLI argument parsing and test orchestration
├── bot.rs       - Azalea bot controller and server connection
└── executor.rs  - Test execution and timeline management via Azalea

flint-core (dependency):
- Test specification and JSON parsing
- Test file discovery and loading
- Spatial offset calculation for parallel tests
- Core test primitives and actions
```

## License

MIT
