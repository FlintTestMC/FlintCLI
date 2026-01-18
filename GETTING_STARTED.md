# Getting Started with FlintMC

FlintMC is now ready to use! Here's how to get started.

## Prerequisites

1. **Rust Nightly Toolchain** (required by Azalea):
   ```bash
   rustup install nightly
   # Already set for this directory via rustup override
   ```

2. **Minecraft Server** (1.21.8+):
   - You need a running Minecraft server
   - The bot needs **operator permissions** to run commands and avoid spam kicks
   - Give the bot op: `/op flintmc_testbot`
   - **Important**: Without op permissions, the bot will be kicked for spamming when running multiple tests

## Building

```bash
cargo build --release
```

## Running Tests

### Single Test File
```bash
cargo run -- example_tests/basic_placement.json --server localhost:25565
```

### All Tests in a Directory
```bash
cargo run -- example_tests/ --server localhost:25565
```

### Recursively Run All Tests
```bash
cargo run -- example_tests/ --server localhost:25565 --recursive
```

## Writing Your First Test

Create a file `my_test.json`:

```json
{
  "name": "my_first_test",
  "description": "Test that I can place and verify a block",
  "actions": [
    {
      "tick": 0,
      "action": "setblock",
      "pos": [0, 64, 0],
      "block": "minecraft:diamond_block"
    },
    {
      "tick": 2,
      "action": "assert_block",
      "pos": [0, 64, 0],
      "block": "minecraft:diamond_block"
    }
  ]
}
```

Run it:
```bash
cargo run -- my_test.json --server localhost:25565
```

## Example Tests Included

- `example_tests/basic_placement.json` - Simple block placement
- `example_tests/fences/fence_connects_to_block.json` - Fence connection mechanics
- `example_tests/fences/fence_to_fence.json` - Fence-to-fence connections
- `example_tests/redstone/lever_basic.json` - Lever state testing
- `example_tests/water/water_source.json` - Water block testing

## How It Works

1. Bot connects to server as `flintmc_testbot`
2. Time is frozen with `/tick freeze`
3. Actions are executed at their specified tick
4. Between ticks, `/tick step 1` advances time
5. Assertions verify block states
6. Time is unfrozen with `/tick unfreeze`
7. Results are reported

## Troubleshooting

### "Bot not initialized"
- Make sure the server is running and accessible
- Check that the server address is correct
- Ensure the bot can connect (check server whitelist/firewall)

### "Bot needs op permissions"
- Run `/op flintmc_testbot` on your server
- The bot needs op to execute `/setblock`, `/fill`, and `/tick` commands

### Assertion failures
- Increase the tick wait between setting blocks and asserting
- Server may need more ticks to process block updates
- Check that block names are correct (e.g., "minecraft:stone")

## Next Steps

- Write tests for your custom block mechanics
- Test redstone contraptions
- Verify water/lava flow behavior
- Test piston mechanics
- Validate door and fence gate behavior

When you provide a vanilla server, you can test the framework immediately!
