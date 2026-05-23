//! JSONL event emission for visualization tools (e.g. FlintViz).
//!
//! All coordinates in emitted events are in **test-local** space — the world
//! offset applied during parallel execution is subtracted before write, so
//! consumers can stay oblivious to FlintCLI's grid layout.

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event<'a> {
    RunStarted {
        test: &'a str,
        region: [[i32; 3]; 2],
    },
    Tick {
        tick: u32,
        set: Vec<BlockSet<'a>>,
        removed: Vec<[i32; 3]>,
    },
    Assert {
        tick: u32,
        pos: [i32; 3],
        passed: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        expected: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        actual: Option<&'a str>,
    },
    RunCompleted {
        asserts_passed: u32,
        asserts_failed: u32,
    },
}

#[derive(Serialize)]
struct BlockSet<'a> {
    pos: [i32; 3],
    id: &'a str,
}

pub struct JsonlWriter {
    writer: BufWriter<File>,
    prev: HashMap<[i32; 3], String>,
    offset: [i32; 3],
}

impl JsonlWriter {
    pub fn create<P: AsRef<Path>>(path: P, offset: [i32; 3]) -> Result<Self> {
        let path_ref = path.as_ref();
        let file = File::create(path_ref)
            .with_context(|| format!("opening {} for event emission", path_ref.display()))?;
        Ok(Self {
            writer: BufWriter::new(file),
            prev: HashMap::new(),
            offset,
        })
    }

    fn write_event(&mut self, event: &Event<'_>) -> Result<()> {
        serde_json::to_writer(&mut self.writer, event)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn run_started(&mut self, test: &str, region_world: [[i32; 3]; 2]) -> Result<()> {
        let region = [
            sub_offset(region_world[0], self.offset),
            sub_offset(region_world[1], self.offset),
        ];
        self.write_event(&Event::RunStarted { test, region })
    }

    /// Diff `world_blocks` against the previous scan and emit a `tick` event
    /// with the resulting deltas in test-local coords.
    pub fn emit_tick(&mut self, tick: u32, world_blocks: HashMap<[i32; 3], String>) -> Result<()> {
        let mut set = Vec::new();
        let mut removed = Vec::new();

        for (pos, id) in &world_blocks {
            if self.prev.get(pos).map(|s| s.as_str()) != Some(id.as_str()) {
                set.push(BlockSet {
                    pos: sub_offset(*pos, self.offset),
                    id: id.as_str(),
                });
            }
        }
        for pos in self.prev.keys() {
            if !world_blocks.contains_key(pos) {
                removed.push(sub_offset(*pos, self.offset));
            }
        }

        self.write_event(&Event::Tick { tick, set, removed })?;
        self.prev = world_blocks;
        Ok(())
    }

    pub fn emit_assert(
        &mut self,
        tick: u32,
        pos: [i32; 3],
        passed: bool,
        expected: Option<&str>,
        actual: Option<&str>,
    ) -> Result<()> {
        self.write_event(&Event::Assert {
            tick,
            pos,
            passed,
            expected,
            actual,
        })
    }

    pub fn run_completed(&mut self, asserts_passed: u32, asserts_failed: u32) -> Result<()> {
        self.write_event(&Event::RunCompleted {
            asserts_passed,
            asserts_failed,
        })
    }
}

fn sub_offset(p: [i32; 3], offset: [i32; 3]) -> [i32; 3] {
    [p[0] - offset[0], p[1] - offset[1], p[2] - offset[2]]
}
