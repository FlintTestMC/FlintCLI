//! State management for recording sessions

use anyhow::Result;
use flint_core::test_spec::{
    ActionType, BlockCheck, BlockPlacement, CleanupSpec, SetupSpec, TestSpec, TickSpec,
    TimelineEntry,
};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::executor::block::make_block;

use super::actions::{RecordedAction, TimelineStep};
use super::bounding_box::BoundingBox;

// Constants
const DEFAULT_SCAN_RADIUS: i32 = 16;
const DEFAULT_CLEANUP_REGION: [[i32; 3]; 2] = [[0, 0, 0], [10, 10, 10]];

/// State for an active recording session
pub struct RecorderState {
    /// Test name (e.g., "fence_connect" or "fence/fence_connect")
    pub test_name: String,
    /// Full path where the test file will be saved
    pub test_path: PathBuf,
    /// Current recording tick
    pub current_tick: u32,
    /// Recorded timeline steps
    pub timeline: Vec<TimelineStep>,
    /// Bounding box of all affected blocks
    pub bounds: BoundingBox,
    /// Block states snapshot for change detection (world_pos -> block_id)
    pub snapshot: HashMap<[i32; 3], String>,
    /// Origin point (first block changed becomes 0,0,0)
    pub origin: Option<[i32; 3]>,
    /// Player name/entity to track
    pub player_name: Option<String>,
    /// Center position for scanning (typically player position)
    pub scan_center: Option<[i32; 3]>,
    /// Scan radius around player to detect block changes
    pub scan_radius: i32,
}

impl RecorderState {
    /// Create a new recorder state
    pub fn new(test_name: &str, tests_dir: &std::path::Path) -> Self {
        // Parse test_name which may include subdirectories like "fence/fence_connect"
        let test_path = if test_name.contains('/') {
            let parts: Vec<&str> = test_name.split('/').collect();
            let mut path = tests_dir.to_path_buf();
            for part in &parts[..parts.len() - 1] {
                path.push(part);
            }
            path.push(format!("{}.json", parts.last().unwrap()));
            path
        } else {
            tests_dir.join(format!("{}.json", test_name))
        };

        Self {
            test_name: test_name.to_string(),
            test_path,
            current_tick: 0,
            timeline: Vec::new(),
            bounds: BoundingBox::new(),
            snapshot: HashMap::new(),
            origin: None,
            player_name: None,
            scan_center: None,
            scan_radius: DEFAULT_SCAN_RADIUS,
        }
    }

    /// Set the scan center for block change detection
    pub fn set_scan_center(&mut self, pos: [i32; 3]) {
        self.scan_center = Some(pos);
    }

    /// Set the origin point (normalizes all positions relative to this)
    pub fn set_origin(&mut self, pos: [i32; 3]) {
        if self.origin.is_none() {
            self.origin = Some(pos);
        }
    }

    /// Convert world position to local position (relative to origin)
    #[must_use]
    pub fn to_local(&self, world_pos: [i32; 3]) -> [i32; 3] {
        if let Some(origin) = self.origin {
            [
                world_pos[0] - origin[0],
                world_pos[1] - origin[1],
                world_pos[2] - origin[2],
            ]
        } else {
            world_pos
        }
    }

    /// Get or create the timeline step for the current tick
    fn get_or_create_current_step(&mut self) -> &mut TimelineStep {
        if self.timeline.is_empty() || self.timeline.last().unwrap().tick != self.current_tick {
            self.timeline.push(TimelineStep {
                tick: self.current_tick,
                actions: Vec::new(),
            });
        }
        self.timeline.last_mut().unwrap()
    }

    /// Remove any existing Place/Remove actions for this position in the current tick
    fn deduplicate_actions(&mut self, pos: [i32; 3]) {
        let step = self.get_or_create_current_step();
        step.actions.retain(|a| match a {
            RecordedAction::Place { pos: p, .. } => *p != pos,
            RecordedAction::Remove { pos: p } => *p != pos,
            // Keep asserts/others
            _ => true,
        });
    }

    /// Record a block placement
    pub fn record_place(&mut self, world_pos: [i32; 3], block: &str) {
        // Set origin on first placement
        self.set_origin(world_pos);

        let local_pos = self.to_local(world_pos);
        self.bounds.expand(local_pos);

        // Deduplicate before adding
        self.deduplicate_actions(local_pos);

        let step = self.get_or_create_current_step();
        step.actions.push(RecordedAction::Place {
            pos: local_pos,
            block: block.to_string(),
        });

        // Update snapshot
        self.snapshot.insert(world_pos, block.to_string());
    }

    /// Record a block removal
    pub fn record_remove(&mut self, world_pos: [i32; 3]) {
        if self.origin.is_none() {
            // Can't remove before any placement
            return;
        }

        let local_pos = self.to_local(world_pos);
        self.bounds.expand(local_pos);

        // Deduplicate before adding
        self.deduplicate_actions(local_pos);

        let step = self.get_or_create_current_step();
        step.actions.push(RecordedAction::Remove { pos: local_pos });

        // Update snapshot - store air to track the removal
        self.snapshot.insert(world_pos, "minecraft:air".to_string());
    }

    /// Add an assertion for a block
    pub fn add_assertion(&mut self, world_pos: [i32; 3], block: &str) {
        if self.origin.is_none() {
            self.set_origin(world_pos);
        }

        let local_pos = self.to_local(world_pos);
        self.bounds.expand(local_pos);

        let step = self.get_or_create_current_step();
        step.actions.push(RecordedAction::Assert {
            pos: local_pos,
            block: block.to_string(),
        });
    }

    /// Convert all Place/Remove actions in the current tick to Assertions
    pub fn convert_actions_to_asserts(&mut self) -> usize {
        let mut converted_count = 0;

        if let Some(step) = self.timeline.last_mut()
            && step.tick == self.current_tick
        {
            let mut new_actions = Vec::new();

            // Drain existing actions and convert them
            for action in step.actions.drain(..) {
                match action {
                    RecordedAction::Place { pos, block } => {
                        new_actions.push(RecordedAction::Assert { pos, block });
                        converted_count += 1;
                    }
                    RecordedAction::Remove { pos } => {
                        // Removing a block means asserting it is air
                        new_actions.push(RecordedAction::Assert {
                            pos,
                            block: "minecraft:air".to_string(),
                        });
                        converted_count += 1;
                    }
                    // Keep existing asserts unchanged
                    assert_action @ RecordedAction::Assert { .. } => {
                        new_actions.push(assert_action);
                    }
                }
            }

            step.actions = new_actions;
        }

        converted_count
    }

    /// Advance to the next tick
    pub fn next_tick(&mut self) {
        self.current_tick += 1;
    }

    /// Generate a TestSpec from the recorded data
    #[must_use]
    pub fn generate_test_spec(&self) -> TestSpec {
        let cleanup_region = if self.bounds.is_valid() {
            self.bounds.to_cleanup_region(1)
        } else {
            DEFAULT_CLEANUP_REGION
        };

        // Build timeline entries using flint-core types
        let mut timeline_entries: Vec<TimelineEntry> = Vec::new();

        for step in &self.timeline {
            // Group actions by type for this tick
            let mut placements: Vec<BlockPlacement> = Vec::new();
            let mut checks: Vec<BlockCheck> = Vec::new();

            for action in &step.actions {
                match action {
                    RecordedAction::Place { pos, block } => {
                        placements.push(BlockPlacement {
                            pos: *pos,
                            block: make_block(block),
                        });
                    }
                    RecordedAction::Remove { pos } => {
                        placements.push(BlockPlacement {
                            pos: *pos,
                            block: make_block("minecraft:air"),
                        });
                    }
                    RecordedAction::Assert { pos, block } => {
                        checks.push(BlockCheck {
                            pos: *pos,
                            is: make_block(block),
                        });
                    }
                }
            }

            // Emit place_each if there are placements
            if !placements.is_empty() {
                timeline_entries.push(TimelineEntry {
                    at: TickSpec::Single(step.tick),
                    action_type: ActionType::PlaceEach { blocks: placements },
                });
            }

            // Emit assert if there are checks
            if !checks.is_empty() {
                timeline_entries.push(TimelineEntry {
                    at: TickSpec::Single(step.tick),
                    action_type: ActionType::Assert { checks },
                });
            }
        }

        TestSpec {
            flint_version: None,
            name: self.test_name.replace('/', "_"),
            description: Some(format!("Recorded test: {}", self.test_name)),
            tags: vec!["recorded".to_string()],
            dependencies: Vec::new(),
            setup: Some(SetupSpec {
                cleanup: CleanupSpec {
                    region: cleanup_region,
                },
            }),
            timeline: timeline_entries,
            breakpoints: Vec::new(),
        }
    }

    /// Save the test to a file
    pub fn save(&self) -> Result<PathBuf> {
        let test_spec = self.generate_test_spec();

        // Create parent directories if needed
        if let Some(parent) = self.test_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write the JSON file with pretty formatting using serde
        let json_str = serde_json::to_string_pretty(&test_spec)?;
        std::fs::write(&self.test_path, json_str)?;

        Ok(self.test_path.clone())
    }
}
