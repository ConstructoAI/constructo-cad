use acadrust::Handle;
use glam::DVec3;

use crate::command::{CadCommand, CmdOption, CmdResult};

pub struct ConstraintBarOptionCommand {
    handles: Vec<Handle>,
}

impl ConstraintBarOptionCommand {
    pub fn new(handles: Vec<Handle>) -> Self {
        Self { handles }
    }

    fn finish(&mut self, command: &str) -> CmdResult {
        CmdResult::Relaunch(command.to_string(), std::mem::take(&mut self.handles))
    }
}

impl CadCommand for ConstraintBarOptionCommand {
    fn name(&self) -> &'static str {
        "CONSTRAINTBAR"
    }

    fn prompt(&self) -> String {
        "CONSTRAINTBAR  Enter an option [Show/Hide/Reset] <Show>:".to_string()
    }

    fn options(&self) -> Vec<CmdOption> {
        vec![
            CmdOption::new("Show", "S"),
            CmdOption::new("Hide", "H"),
            CmdOption::new("Reset", "R"),
        ]
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let keyword = text.trim().trim_start_matches('_').to_ascii_uppercase();
        match keyword.as_str() {
            "S" | "SHOW" => Some(self.finish("GCSHOW")),
            "H" | "HIDE" => Some(self.finish("GCHIDE")),
            "R" | "RESET" => Some(self.finish("CONSTRAINTBAR_RESET")),
            _ => None,
        }
    }

    fn on_point(&mut self, _point: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        self.finish("GCSHOW")
    }

    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}

inventory::submit!(crate::command::CommandRegistration {
    names: &["CONSTRAINTBAR"]
});
