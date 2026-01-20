//! Recorded action types for the test recorder

/// A recorded action in the timeline
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedAction {
    Place { pos: [i32; 3], block: String },
    Remove { pos: [i32; 3] },
    Assert { pos: [i32; 3], block: String },
}

/// A step in the recorded timeline
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimelineStep {
    pub tick: u32,
    pub actions: Vec<RecordedAction>,
}
