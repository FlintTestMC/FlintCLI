# FlintMC Features

## ✅ Implemented Features

### Core Framework
- **Timeline-based execution** - Actions execute at specific game ticks
- **Tick control** - Uses `/tick freeze` and `/tick step` for deterministic testing
- **JSON test specification** - No Rust knowledge required to write tests
- **One test = one JSON file** - Simple organization
- **Directory support** - Run single files or entire directories (with `--recursive`)

### Test Actions
- **setblock** - Place individual blocks
- **fill** - Fill regions with blocks
- **assert_block** - Verify block type at position
- **assert_block_state** - Verify block properties (e.g., fence connections, lever state)

### Test Management
- **Automatic cleanup** - Optional before/after test area clearing
- **Test isolation** - Each test has its own cleanup zone
- **Progress tracking** - Visual feedback with colored output
- **Test summary** - Clear pass/fail reporting

### Bot Capabilities
- **Offline mode** - No Microsoft account required
- **Auto-reconnect** - Handles connection issues gracefully
- **World state tracking** - Reads block states from server
- **Command execution** - Runs `/setblock`, `/fill`, `/tick` commands
- **Operator support** - Works with server OP permissions

### Output
- **Colored terminal output** - Green checkmarks, red X's, blue arrows
- **Timeline visualization** - Shows tick-by-tick execution
- **Detailed errors** - Shows expected vs actual values
- **Test statistics** - Counts passed/failed assertions

## 📋 Test Examples Included

1. **basic_placement.json** - Simple block placement
2. **fence_connects_to_block.json** - Fence connection mechanics
3. **fence_to_fence.json** - Fence-to-fence connections
4. **lever_basic.json** - Redstone lever state changes
5. **water_source.json** - Water source blocks

All tests include cleanup zones and pass successfully!

## 🔧 Technical Details

### Architecture
- **Language:** Rust 2024 edition (nightly)
- **Bot Framework:** Azalea (latest from GitHub)
- **Async Runtime:** Tokio
- **CLI:** Clap v4
- **Serialization:** Serde + serde_json

### Performance
- **Fast execution** - Only steps through required ticks
- **Minimal waits** - 100-200ms delays for server updates
- **Parallel capable** - Can run multiple bots (future feature)

### Compatibility
- **Minecraft:** 1.21.10+
- **Server:** Vanilla, Paper, Spigot (with online-mode=false)
- **Platform:** Linux, macOS, Windows

## 🚀 Usage

```bash
# Single test
cargo run -- example_tests/basic_placement.json --server localhost:25565

# All tests in directory
cargo run -- example_tests/ --server localhost:25565

# Recursive test discovery
cargo run -- example_tests/ --server localhost:25565 --recursive
```

## ✨ What Makes FlintMC Special

1. **Deterministic** - Same test always produces same results
2. **Fast** - No waiting for real-time game ticks
3. **Precise** - Tests exact game tick when changes occur
4. **Clean** - Automatic cleanup prevents test interference
5. **Simple** - JSON tests are easy to read and write
6. **Extensible** - Easy to add new test actions

## 📊 Current Test Results

**5/5 tests passing** ✅
- ✅ Basic block placement
- ✅ Fence connects to solid block
- ✅ Fence connects to another fence
- ✅ Lever state changes
- ✅ Water source block detection

**100% pass rate!**
