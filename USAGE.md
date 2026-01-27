# Usage

## Command line

```
flintmc [OPTIONS] --server <SERVER> [PATH]
```

### Arguments

| Argument | Description |
|----------|-------------|
| `[PATH]` | Path to a test file (`.json`) or directory. Defaults to `FlintBenchmark/tests` |

### Options

| Flag | Short | Description |
|------|-------|-------------|
| `--server <SERVER>` | `-s` | Server address (e.g., `localhost:25565`). Required |
| `--recursive` | `-r` | Recursively search directories for test files |
| `--break-after-setup` | | Pause after test setup (cleanup phase) for manual inspection |
| `--tag <TAG>` | `-t` | Filter tests by tag. Can be specified multiple times |
| `--interactive` | `-i` | Enter interactive mode (listen for in-game chat commands) |
| `--action-delay <MS>` | `-d` | Delay between actions in milliseconds (default: 100) |
| `--verbose` | `-v` | Show detailed per-action output during execution |
| `--quiet` | `-q` | Suppress the progress bar |
| `--format <FORMAT>` | | Output format: `pretty` (default), `json`, `tap`, `junit` |

## Running tests

### Single test file
```bash
flintmc example_tests/basic_placement.json -s localhost:25565
```

### Directory of tests
```bash
flintmc example_tests/ -s localhost:25565
```

### Recursive directory search
```bash
flintmc example_tests/ -s localhost:25565 -r
```

### Filter by tags
```bash
flintmc -s localhost:25565 -t redstone -t pistons
```

## Output modes

### Default (concise)

Shows a progress bar and a summary. On failure, a tree view groups failures by test path:

```
Running 1,247 tests...
[████████████████████████████████████████] 8,293/8,293 ticks

12 of 1,247 tests failed (12.921s)

├── redstone
│   ├── repeater_chain
│   │   └─ t8: expected powered=true, got powered=false @ (4,100,0)
│   └── comparator_measure
│       └─ t6: expected signal=13, got signal=12 @ (0,64,3)
└── blocks
    └── falling
        └── sand_fall_distance
            └─ t14: expected sand, got air @ (0,90,0)

1,235 passed, 12 failed
```

On success:
```
Running 1,247 tests...
[████████████████████████████████████████] 8,293/8,293 ticks

✓ All 1,247 tests passed (12.847s)
```

### Verbose (`-v`)

Prints every action and assertion as it happens, chunk headers, grid positions, cleanup messages, and per-test pass/fail status. Useful for debugging individual tests.

### Quiet (`-q`)

Same as default but without the progress bar. Useful for CI where carriage returns aren't rendered well.

### JSON (`--format json`)

Machine-readable JSON output. Structured output goes to stdout; logs and progress go to stderr.

```bash
flintmc -s localhost:25565 -r tests/ --format json 2>/dev/null
```

```json
{
  "summary": {
    "total": 6,
    "passed": 5,
    "failed": 1,
    "duration_secs": 4.812
  },
  "tests": [
    { "name": "basic_block_placement", "success": true, "total_ticks": 3, "execution_time_ms": 450 },
    { "name": "lever_basic", "success": false, "total_ticks": 5, "execution_time_ms": 620 }
  ],
  "failures": [
    {
      "test": "lever_basic",
      "tick": 5,
      "expected": "powered=true",
      "actual": "powered=false",
      "position": [10, 101, 10]
    }
  ]
}
```

### TAP (`--format tap`)

[Test Anything Protocol](https://testanything.org/) version 13. Supported by most CI systems.

```bash
flintmc -s localhost:25565 -r tests/ --format tap 2>/dev/null
```

```
TAP version 13
1..6
ok 1 - basic_block_placement
ok 2 - fence_connects_to_block
not ok 3 - lever_basic
  ---
  message: "expected powered=true, got powered=false"
  at: [10, 101, 10]
  tick: 5
  ...
ok 4 - fence_connects_to_fence
ok 5 - repeater_feedback_clock
ok 6 - water_source_block
```

### JUnit XML (`--format junit`)

JUnit XML format for CI systems like Jenkins, GitLab CI, and GitHub Actions.

```bash
flintmc -s localhost:25565 -r tests/ --format junit > results.xml 2>build.log
```

```xml
<?xml version="1.0" encoding="UTF-8"?>
<testsuites tests="6" failures="1" time="4.812">
  <testsuite name="flintmc" tests="6" failures="1" time="4.812">
    <testcase classname="" name="basic_block_placement" time="0.450" />
    <testcase classname="" name="lever_basic" time="0.620">
      <failure message="expected powered=true, got powered=false at (10,101,10) tick 5"/>
    </testcase>
  </testsuite>
</testsuites>
```

All non-pretty formats suppress the progress bar and send log messages to stderr, so stdout can be piped cleanly to a file.

## Debugging with breakpoints

Tests can define breakpoints at specific ticks in their JSON:
```json
{
  "breakpoints": [1, 3]
}
```

You can also break after the setup phase from the CLI:
```bash
flintmc example_tests/test.json -s localhost:25565 --break-after-setup
```

When a breakpoint is hit, type in the terminal:
- **`s`** -- step one tick, then pause again
- **`c`** -- continue to the next breakpoint or end of test

## Interactive mode

Start with `-i`:
```bash
flintmc -s localhost:25565 -i
```

The bot joins the server and listens for chat commands (prefixed with `!`):

| Command | Description |
|---------|-------------|
| `!help` | List commands |
| `!list` | List all loaded tests |
| `!search <pattern>` | Search tests by name |
| `!run <name> [step]` | Run a test. Append `step` for step-through mode |
| `!run-all` | Run every loaded test |
| `!run-tags <tag1,tag2>` | Run tests matching comma-separated tags |
| `!reload` | Reload test files from disk |
| `!stop` | Exit interactive mode |

Interactive mode always uses verbose output.

## Recording tests

The recorder lets you create tests by performing actions in-game. The bot watches for block changes around its position and records them into a timeline.

### Workflow

1. Start interactive mode and join the same server:
   ```bash
   flintmc -s localhost:25565 -i
   ```

2. Begin recording:
   ```
   !record redstone/my_test
   ```
   The name can include `/` for subdirectories. Time is automatically frozen.

3. Place and break blocks in-game. The bot scans for changes in a 10-block radius around its position.

4. When ready to advance a tick:
   ```
   !tick
   ```
   This snapshots the current block changes, steps the game one tick, and advances the recording tick counter. You can also use `!next` as an alias.

5. Add assertions for blocks you want to verify:
   ```
   !assert <x> <y> <z>
   ```
   Records the block at that position as an expected value.

6. To convert all detected changes in the current tick into assertions instead of placements:
   ```
   !assert_changes
   ```

7. Save the test:
   ```
   !save
   ```
   The JSON file is written to the tests directory. The test index is automatically reloaded so you can immediately run it with `!run`.

8. Or discard:
   ```
   !cancel
   ```

### Recorder commands

| Command | Description |
|---------|-------------|
| `!record <name> [player]` | Start recording. Optional player name for position tracking |
| `!tick` / `!next` | Snapshot changes and advance one game tick |
| `!assert <x> <y> <z>` | Assert the block at the given coordinates |
| `!assert_changes` | Convert all detected block changes to assertions |
| `!save` | Save the recording as a JSON test file |
| `!cancel` | Discard the recording and unfreeze time |

### Tips

- The recorder auto-detects block placements and removals within scan range.
- Positions are stored relative to the first block changed (origin), so tests are portable.
- The cleanup region is computed automatically from the bounding box of all affected blocks.
- Saved tests are tagged with `recorded` so you can filter them: `flintmc -s ... -t recorded`.

## Test format

Tests are JSON files:

```json
{
  "flintVersion": "0.1",
  "name": "test_name",
  "description": "Optional description",
  "tags": ["tag1", "tag2"],
  "setup": {
    "cleanup": {
      "region": [[0, 60, 0], [10, 70, 10]]
    }
  },
  "breakpoints": [1, 3],
  "timeline": [
    {
      "at": 0,
      "do": "place",
      "pos": [0, 64, 0],
      "block": "minecraft:stone"
    },
    {
      "at": 1,
      "do": "assert",
      "checks": [
        { "pos": [0, 64, 0], "is": "minecraft:stone" }
      ]
    }
  ]
}
```

`setup.cleanup.region` defines the area cleared before and after the test. Optional but recommended to avoid test interference.

`breakpoints` lists ticks where execution pauses for inspection. Optional.

### Actions

**place** -- place a single block:
```json
{ "at": 0, "do": "place", "pos": [0, 64, 0], "block": "minecraft:stone" }
```

**place_each** -- place multiple blocks:
```json
{
  "at": 0,
  "do": "place_each",
  "blocks": [
    { "pos": [0, 64, 0], "block": "minecraft:stone" },
    { "pos": [1, 64, 0], "block": "minecraft:dirt" }
  ]
}
```

**fill** -- fill a region:
```json
{ "at": 0, "do": "fill", "region": [[0, 64, 0], [5, 64, 5]], "with": "minecraft:stone" }
```

**remove** -- replace with air:
```json
{ "at": 0, "do": "remove", "pos": [0, 64, 0] }
```

**assert** -- check block type(s):
```json
{
  "at": 1,
  "do": "assert",
  "checks": [
    { "pos": [0, 64, 0], "is": "minecraft:stone" }
  ]
}
```

Blocks can include state properties:
```json
{ "pos": [0, 64, 0], "is": { "id": "minecraft:oak_fence", "properties": { "east": "true" } } }
```

**assert_state** -- check a property across multiple ticks:
```json
{
  "at": [1, 2, 3],
  "do": "assert_state",
  "pos": [0, 64, 0],
  "state": "powered",
  "values": ["false", "true", "false"]
}
```

## How it works

1. Tests are loaded and arranged in a spatial grid (up to 100 per chunk, 10x10)
2. The bot connects via [Azalea](https://github.com/azalea-rs/azalea) and freezes time with `/tick freeze`
3. Timelines from all tests in a chunk are merged into a single tick-ordered sequence
4. At each tick with scheduled actions, commands are sent (`/setblock`, `/fill`)
5. Empty tick ranges are skipped with `/tick sprint` for speed
6. Assertions read block state from Azalea's world tracking and compare against expected values
7. After all ticks complete, time is unfrozen and areas are cleaned up
