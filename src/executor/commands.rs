//! Generated main and recorder command registry and dispatch.
//!
//! General commands live in `executor/command_definitions.rs`; recorder commands
//! live in `executor/recorder/command_definitions.rs`. `build.rs` combines them.

use super::TestExecutor;
use anyhow::Result;
use flint_core::loader::TestLoader;
use std::path::PathBuf;

pub struct FlintCommandContext<'a> {
    pub args: &'a [String],
    pub sender: Option<String>,
    pub test_loader: &'a mut TestLoader,
    pub all_test_files: &'a mut Vec<PathBuf>,
    pub exit_interactive: bool,
}

include!(concat!(env!("OUT_DIR"), "/flint_commands.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_dialog_triggers_and_callbacks_are_unique() {
        let mut triggers = std::collections::HashSet::new();
        let mut callbacks = std::collections::HashSet::new();
        for spec in COMMANDS.iter().filter(|spec| spec.dialog.is_some()) {
            assert_eq!(spec.scope, CommandScope::Recorder);
            assert!(triggers.insert(spec.dialog.unwrap().trigger));
            assert!(callbacks.insert(spec.callback));
            assert_eq!(from_callback(spec.callback), Some(spec.command));
        }
    }

    #[test]
    fn generated_commands_keep_main_and_recorder_scopes_separate() {
        assert_eq!(
            COMMANDS
                .iter()
                .find(|spec| spec.aliases.contains(&"!record"))
                .unwrap()
                .scope,
            CommandScope::Main
        );
        assert_eq!(
            COMMANDS
                .iter()
                .find(|spec| spec.aliases.contains(&"!recorder"))
                .unwrap()
                .scope,
            CommandScope::Recorder
        );
    }

    #[test]
    fn generated_chat_aliases_are_unique() {
        let mut aliases = std::collections::HashSet::new();
        for spec in COMMANDS {
            for alias in spec.aliases {
                assert!(aliases.insert(*alias));
                assert_eq!(from_chat(alias), Some(spec.command));
            }
        }
    }
}
