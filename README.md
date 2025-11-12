# FlintMC

A Minecraft server testing framework written in Rust using Azalea. Tests are specified in JSON and executed deterministically using Minecraft's `/tick` command.

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

## Test Format

Each test is a JSON file with the following structure:

```json
{
  "name": "test_name",
  "description": "Optional description",
  "cleanup": {
    "from": [x1, y1, z1],
    "to": [x2, y2, z2]
  },
  "actions": [
    {
      "tick": 0,
      "action": "setblock",
      "pos": [x, y, z],
      "block": "minecraft:block_id"
    },
    {
      "tick": 1,
      "action": "assert_block",
      "pos": [x, y, z],
      "block": "minecraft:block_id"
    }
  ]
}
```

The `cleanup` field is optional. If specified, the framework will:
1. Fill the area with air **before** the test runs
2. Fill the area with air **after** the test completes

This ensures tests don't interfere with each other.

## Available Actions

### Block Operations

**setblock** - Place a single block
```json
{
  "tick": 0,
  "action": "setblock",
  "pos": [x, y, z],
  "block": "minecraft:block_id"
}
```

**fill** - Fill a region with blocks
```json
{
  "tick": 0,
  "action": "fill",
  "from": [x1, y1, z1],
  "to": [x2, y2, z2],
  "block": "minecraft:block_id"
}
```

### Assertions

**assert_block** - Check block type at position
```json
{
  "tick": 1,
  "action": "assert_block",
  "pos": [x, y, z],
  "block": "minecraft:block_id"
}
```

**assert_block_state** - Check block property value
```json
{
  "tick": 1,
  "action": "assert_block_state",
  "pos": [x, y, z],
  "property": "property_name",
  "value": "expected_value"
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

1. Bot connects to server in spectator mode
2. Test timeline is constructed from JSON
3. Server time is frozen with `/tick freeze`
4. Actions are grouped by tick and executed
5. Between tick groups, `/tick step 1` advances time
6. Azalea tracks world state from server updates
7. Assertions verify expected block states
8. Results are collected and reported

## Architecture

```
src/
├── main.rs       - CLI and test runner
├── test_spec.rs  - JSON parsing and test specification
├── bot.rs        - Azalea bot controller
└── executor.rs   - Test execution and timeline management
```

## License

MIT
