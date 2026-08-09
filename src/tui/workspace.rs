use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use super::terminal_emulator::EmbeddedTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Terminal,
}

pub struct TerminalTab {
    pub terminal: EmbeddedTerminal,
    pub title: String,
}

struct JourneyTabs {
    tabs: Vec<TerminalTab>,
    active: usize,
}

pub struct WorkspaceState {
    pub focus: Focus,
    selected_journey: Option<String>,
    journals: HashMap<String, JourneyTabs>,
}

impl WorkspaceState {
    pub fn new() -> Self {
        Self {
            focus: Focus::Sidebar,
            selected_journey: None,
            journals: HashMap::new(),
        }
    }

    pub fn select_journey(&mut self, journey_id: &str) {
        self.selected_journey = Some(journey_id.to_string());
    }

    pub fn selected_journey_id(&self) -> Option<&str> {
        self.selected_journey.as_deref()
    }

    pub fn open_tab(
        &mut self,
        journey_id: &str,
        journey_title: &str,
        cwd: &Path,
        rows: u16,
        cols: u16,
    ) -> Result<()> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let terminal = EmbeddedTerminal::spawn(&shell, cwd, rows, cols)?;
        let tab = TerminalTab {
            terminal,
            title: journey_title.to_string(),
        };

        let entry = self
            .journals
            .entry(journey_id.to_string())
            .or_insert_with(|| JourneyTabs {
                tabs: Vec::new(),
                active: 0,
            });
        entry.tabs.push(tab);
        entry.active = entry.tabs.len() - 1;
        self.selected_journey = Some(journey_id.to_string());
        self.focus = Focus::Terminal;
        Ok(())
    }

    pub fn close_active_tab(&mut self) {
        let id = self.selected_journey.clone();
        if let Some(id) = &id {
            if let Some(entry) = self.journals.get_mut(id) {
                if !entry.tabs.is_empty() {
                    entry.tabs.remove(entry.active);
                    if entry.active >= entry.tabs.len() && entry.active > 0 {
                        entry.active -= 1;
                    }
                }
                if entry.tabs.is_empty() {
                    self.journals.remove(id);
                    self.focus = Focus::Sidebar;
                }
            }
        }
    }

    pub fn active_terminal(&self) -> Option<&EmbeddedTerminal> {
        let id = self.selected_journey.as_ref()?;
        let entry = self.journals.get(id)?;
        entry.tabs.get(entry.active).map(|t| &t.terminal)
    }

    pub fn tabs_for_selected(&self) -> &[TerminalTab] {
        self.selected_journey
            .as_ref()
            .and_then(|id| self.journals.get(id))
            .map(|e| e.tabs.as_slice())
            .unwrap_or(&[])
    }

    pub fn active_tab_index(&self) -> usize {
        self.selected_journey
            .as_ref()
            .and_then(|id| self.journals.get(id))
            .map(|e| e.active)
            .unwrap_or(0)
    }

    pub fn has_tabs(&self, journey_id: &str) -> bool {
        self.journals
            .get(journey_id)
            .map(|e| !e.tabs.is_empty())
            .unwrap_or(false)
    }

    pub fn next_tab(&mut self) {
        if let Some(id) = &self.selected_journey {
            if let Some(entry) = self.journals.get_mut(id) {
                if !entry.tabs.is_empty() {
                    entry.active = (entry.active + 1) % entry.tabs.len();
                }
            }
        }
    }

    pub fn prev_tab(&mut self) {
        if let Some(id) = &self.selected_journey {
            if let Some(entry) = self.journals.get_mut(id) {
                if !entry.tabs.is_empty() {
                    entry.active = entry.active.checked_sub(1).unwrap_or(entry.tabs.len() - 1);
                }
            }
        }
    }

    pub fn toggle_focus(&mut self) {
        if self.active_terminal().is_some() {
            self.focus = match self.focus {
                Focus::Sidebar => Focus::Terminal,
                Focus::Terminal => Focus::Sidebar,
            };
        }
    }
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self::new()
    }
}
