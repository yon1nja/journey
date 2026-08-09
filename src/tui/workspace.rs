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
    /// 0 = Details tab, 1+ = terminal tab (1-based index into `tabs`)
    right_pane_tab: usize,
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
                right_pane_tab: 0,
            });
        entry.tabs.push(tab);
        entry.right_pane_tab = entry.tabs.len(); // 1-based: point to the new tab
        self.selected_journey = Some(journey_id.to_string());
        self.focus = Focus::Terminal;
        Ok(())
    }

    pub fn close_active_tab(&mut self) {
        let id = self.selected_journey.clone();
        if let Some(id) = &id {
            if let Some(entry) = self.journals.get_mut(id) {
                if entry.right_pane_tab == 0 || entry.tabs.is_empty() {
                    return; // Can't close the Details tab
                }
                let terminal_idx = entry.right_pane_tab - 1;
                entry.tabs.remove(terminal_idx);
                if entry.tabs.is_empty() {
                    self.journals.remove(id);
                    self.focus = Focus::Sidebar;
                } else if terminal_idx >= entry.tabs.len() {
                    entry.right_pane_tab = entry.tabs.len(); // clamp to last
                }
                // else: right_pane_tab stays, next terminal slides into this index
            }
        }
    }

    pub fn active_terminal(&self) -> Option<&EmbeddedTerminal> {
        let id = self.selected_journey.as_ref()?;
        let entry = self.journals.get(id)?;
        if entry.right_pane_tab == 0 {
            return None;
        }
        entry
            .tabs
            .get(entry.right_pane_tab - 1)
            .map(|t| &t.terminal)
    }

    /// Returns true when the right pane is showing a terminal tab (not the Details tab).
    pub fn active_tab_is_terminal(&self) -> bool {
        self.active_terminal().is_some()
    }

    pub fn tabs_for_selected(&self) -> &[TerminalTab] {
        self.selected_journey
            .as_ref()
            .and_then(|id| self.journals.get(id))
            .map(|e| e.tabs.as_slice())
            .unwrap_or(&[])
    }

    /// The current right-pane tab index: 0 = Details, 1+ = terminal tab.
    pub fn right_pane_tab(&self) -> usize {
        self.selected_journey
            .as_ref()
            .and_then(|id| self.journals.get(id))
            .map(|e| e.right_pane_tab)
            .unwrap_or(0)
    }

    pub fn has_tabs(&self, journey_id: &str) -> bool {
        self.journals
            .get(journey_id)
            .map(|e| !e.tabs.is_empty())
            .unwrap_or(false)
    }

    /// Cycle to the next right-pane tab (Details -> Term1 -> Term2 -> ... -> Details).
    pub fn next_tab(&mut self) {
        if let Some(id) = &self.selected_journey {
            if let Some(entry) = self.journals.get_mut(id) {
                if !entry.tabs.is_empty() {
                    let total = entry.tabs.len() + 1; // +1 for Details tab
                    entry.right_pane_tab = (entry.right_pane_tab + 1) % total;
                }
            }
        }
    }

    /// Cycle to the previous right-pane tab.
    pub fn prev_tab(&mut self) {
        if let Some(id) = &self.selected_journey {
            if let Some(entry) = self.journals.get_mut(id) {
                if !entry.tabs.is_empty() {
                    let total = entry.tabs.len() + 1;
                    entry.right_pane_tab = entry
                        .right_pane_tab
                        .checked_sub(1)
                        .unwrap_or(total - 1);
                }
            }
        }
    }

    pub fn toggle_focus(&mut self) {
        if self.active_tab_is_terminal() {
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

// ── Rendering helpers ────────────────────────────────────────────────

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Tabs;
use ratatui::Frame;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line as GridLine, Point};
use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::vte::ansi::Color as AnsiColor;
use alacritty_terminal::vte::ansi::NamedColor;

use super::ACCENT;

/// Renders the tab bar for the right pane: [Details | Term 1 | Term 2 | ...]
pub(super) fn render_right_pane_tab_bar(
    frame: &mut Frame,
    area: Rect,
    workspace: &WorkspaceState,
) {
    let terminal_tabs = workspace.tabs_for_selected();
    let active = workspace.right_pane_tab();

    let mut titles: Vec<Line> = vec![Line::from(Span::raw(" Details "))];
    for (i, _tab) in terminal_tabs.iter().enumerate() {
        titles.push(Line::from(Span::raw(format!(" Term {} ", i + 1))));
    }

    let tab_bar = Tabs::new(titles)
        .select(active)
        .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(tab_bar, area);
}

/// Renders the terminal grid content from an embedded terminal into the given area.
/// Also renders the cursor position with reversed colors.
pub(super) fn render_term_grid(frame: &mut Frame, area: Rect, terminal: &EmbeddedTerminal) {
    terminal.with_term(|term| {
        let grid = term.grid();
        let cursor_point = grid.cursor.point;
        let buf = frame.buffer_mut();
        let buf_area = *buf.area();

        for row in 0..area.height {
            for col in 0..area.width {
                if (col as usize) >= grid.columns() {
                    continue;
                }

                let point = Point::new(GridLine(row as i32), Column(col as usize));
                let cell = &grid[point];
                let x = area.x + col;
                let y = area.y + row;

                if x < buf_area.right() && y < buf_area.bottom() {
                    let buf_cell = &mut buf[(x, y)];
                    buf_cell.set_char(cell.c);
                    let mut style = cell_style(cell);
                    // Render cursor with reversed colors
                    if point == cursor_point {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                    buf_cell.set_style(style);
                }
            }
        }
    });
}

fn cell_style(cell: &alacritty_terminal::term::cell::Cell) -> Style {
    let mut style = Style::default();

    style = style.fg(ansi_to_ratatui_color(&cell.fg));
    style = style.bg(ansi_to_ratatui_color(&cell.bg));

    if cell.flags.contains(CellFlags::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.flags.contains(CellFlags::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.flags.contains(CellFlags::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.flags.contains(CellFlags::INVERSE) {
        style = style.add_modifier(Modifier::REVERSED);
    }
    if cell.flags.contains(CellFlags::DIM) {
        style = style.add_modifier(Modifier::DIM);
    }
    if cell.flags.contains(CellFlags::HIDDEN) {
        style = style.add_modifier(Modifier::HIDDEN);
    }
    if cell.flags.contains(CellFlags::STRIKEOUT) {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }

    style
}

fn ansi_to_ratatui_color(color: &AnsiColor) -> Color {
    match color {
        AnsiColor::Named(named) => match named {
            NamedColor::Black | NamedColor::DimBlack => Color::Black,
            NamedColor::Red | NamedColor::DimRed => Color::Red,
            NamedColor::Green | NamedColor::DimGreen => Color::Green,
            NamedColor::Yellow | NamedColor::DimYellow => Color::Yellow,
            NamedColor::Blue | NamedColor::DimBlue => Color::Blue,
            NamedColor::Magenta | NamedColor::DimMagenta => Color::Magenta,
            NamedColor::Cyan | NamedColor::DimCyan => Color::Cyan,
            NamedColor::White | NamedColor::DimWhite => Color::White,
            NamedColor::BrightBlack => Color::DarkGray,
            NamedColor::BrightRed => Color::LightRed,
            NamedColor::BrightGreen => Color::LightGreen,
            NamedColor::BrightYellow => Color::LightYellow,
            NamedColor::BrightBlue => Color::LightBlue,
            NamedColor::BrightMagenta => Color::LightMagenta,
            NamedColor::BrightCyan => Color::LightCyan,
            NamedColor::BrightWhite => Color::White,
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground => {
                Color::Reset
            }
            NamedColor::Background => Color::Reset,
            NamedColor::Cursor => Color::Reset,
        },
        AnsiColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(idx) => Color::Indexed(*idx),
    }
}
