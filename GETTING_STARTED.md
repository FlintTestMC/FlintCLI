# Getting Started with FlintMC

FlintMC is a Minecraft testing framework that connects a bot to your world and executes automated tests. Here's how to get started.

## Prerequisites

- **Rust** — Install from [rust-lang.org](https://rust-lang.org/)
- **Minecraft 1.21.11** — Java Edition

## Setting Up Minecraft

FlintMC works with both dedicated servers and LAN worlds. This guide uses a LAN world for simplicity.

### Creating the World

For reliable, reproducible tests, configure your world with these settings:

1. **Game Mode**: Creative
2. **World Type**: Superflat
3. **Superflat Preset**: Use a stable block layer (stone or bedrock) — avoid grass/dirt which can change over time

In **More World Options → Game Rules**, disable these to prevent random events from interfering with tests:

| Game Rule | Setting |
|-----------|---------|
| Spawn mobs | Off |
| Spawn monsters | Off |
| Spawn pillager patrols | Off |
| Spawn phantoms | Off |
| Spawn Wandering Trader | Off |
| Spawn Wardens | Off |
| Daylight cycle | Off |
| Weather cycle | Off |

> **Why?** Random spawns and environmental changes can interfere with block-based tests and cause unexpected failures.

### Opening to LAN

Once your world is created:

1. Press `Esc` to open the pause menu
2. Click **Open to LAN**
3. Set **Allow Cheats** to **On** (required for `/tick` and `/setblock` commands)
4. Click **Start LAN World**

> **Note**: The port number shown in chat (e.g., "Local game hosted on port 25565") will be needed when starting the bot.

> **For dedicated servers**: The bot needs operator permissions. Run `/op flintmc_testbot` on your server before connecting.

## Starting the Bot

Build and run FlintMC in interactive mode:

```bash
cargo run -- -i --server localhost:25565
```

Replace `25565` with your chosen port if different.

The bot (`flintmc_testbot`) will join your world and announce itself in chat. You'll see:
```
FlintMC Interactive Mode active
Type: help, search, run, run-all, run-tags, list, reload, stop, record (prefix with !)
```

## Using the Bot

All commands use the `!` prefix. Type commands in Minecraft chat.

### Quick Command Reference

| Command | Description |
|---------|-------------|
| `!help` | Show all available commands |
| `!list` | List all loaded tests |
| `!search <pattern>` | Search tests by name |
| `!run <test_name>` | Run a specific test |
| `!run-all` | Run all tests |
| `!run-tags <tag1,tag2>` | Run tests with specific tags |
| `!reload` | Reload test files from disk |
| `!stop` | Exit interactive mode |

## Creating a Test

The easiest way to create tests is by recording your actions in-game.

### Recording Workflow

1. **Start recording**
   ```
   !record my_test_name
   ```
   This freezes game time and starts tracking block changes.

2. **Make changes** — Place or break blocks as needed

3. **Advance the timeline**
   ```
   !tick
   ```
   This captures all block changes since recording started (or since the last `!tick`), then steps the game forward one tick.

4. **Add assertions** — Verify specific blocks at coordinates:
   ```
   !assert <x> <y> <z>
   ```
   > **Tip**: Press `F3` in Minecraft to see coordinates. Use `F3+F6` and enable looking_at_block in always to permanently display the targeted block info on screen.
   
   Or automatically assert all changes from this tick:
   ```
   !assert_changes
   ```

> **💡 Tip: Separate block changes from assertions**
> 
> Always put assertions on a **different tick** than block placements. This gives the game time to process block updates (like redstone signals, water flow, or fence connections).
> 
> **Good pattern:**
> ```
> !record my_test
> (place blocks)
> !tick             ← captures block changes, records them on tick 0, advances to tick 1
> !assert_changes   ← adds assertions on tick 1 (after blocks have updated)
> !save
> ```
> 
> **Why?** Some block behaviors (redstone, fluids, connections) need at least one game tick to propagate. Asserting on the same tick as placement may fail.

5. **Save the test**
   ```
   !save
   ```
   The test is saved to `FlintBenchmark/tests/<test_name>.json`

### Other Recording Commands

| Command | Description |
|---------|-------------|
| `!next` | Alias for `!tick` |
| `!cancel` | Discard the current recording |

### Editing Tests

After saving, you can manually edit the JSON file to:

- Add a description
- Add tags for organization (e.g., `"tags": ["redstone", "basic"]`)
- Adjust timing between actions
- Add or remove assertions

See [README.md](README.md) for the full test format specification.

## Running Tests

### From Interactive Mode

```
!run lever_basic
!run-all
!run-tags redstone
```

### From Command Line

```bash
# Run a single test
cargo run -- FlintBenchmark/tests/my_test.json --server localhost:25565

# Run all tests in a directory
cargo run -- FlintBenchmark/tests/ --server localhost:25565 --recursive
```

## Troubleshooting

### Bot won't connect
- Ensure the LAN world is open and cheats are enabled
- Check the port matches between Minecraft and the command
- Verify no firewall is blocking local connections

### Commands not working
- Make sure you're typing in chat (press `T`), not the command console
- Commands must start with `!` (e.g., `!help`, not `help`)

### Tests failing unexpectedly
- Increase tick delays between placing blocks and asserting
- Check block IDs use the `minecraft:` prefix
- Ensure no mobs or environmental changes are interfering

### Debugging a test

Use **step mode** to pause execution at each tick and inspect the world:

```
!run my_test step
```

**Where to look**: Tests are executed around position `(0, 100, 0)`. Teleport there to watch the test:
```
/tp @s 0 100 0
```

In step mode, the test pauses after each tick. Type in chat:
- `s` to step to the next tick
- `c` to continue running normally

This is invaluable for finding timing issues or incorrect block positions.

## Next Steps

- Explore the example tests in `example_tests/`
- Read the full test format documentation in [README.md](README.md)