use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSpec {
    pub name: String,
    pub description: Option<String>,
    pub actions: Vec<Action>,
    #[serde(default)]
    pub cleanup: Option<CleanupSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupSpec {
    pub from: [i32; 3],
    pub to: [i32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub tick: u32,
    #[serde(flatten)]
    pub action_type: ActionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ActionType {
    Setblock {
        pos: [i32; 3],
        block: String,
    },
    Fill {
        from: [i32; 3],
        to: [i32; 3],
        block: String,
    },
    AssertBlock {
        pos: [i32; 3],
        block: String,
    },
    AssertBlockState {
        pos: [i32; 3],
        property: String,
        value: String,
    },
}

impl TestSpec {
    pub fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let spec: TestSpec = serde_json::from_str(&content)?;
        Ok(spec)
    }

    pub fn max_tick(&self) -> u32 {
        self.actions.iter().map(|a| a.tick).max().unwrap_or(0)
    }
}
