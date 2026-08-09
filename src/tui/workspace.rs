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

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};
use ratatui::Frame;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line as GridLine, Point};
use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::vte::ansi::Color as AnsiColor;
use alacritty_terminal::vte::ansi::NamedColor;

use crate::models::IndexEntry;

use super::ACCENT;

/// Renders the workspace screen: a sidebar list of journeys on the left and
/// the active terminal grid (with tab bar, if multiple tabs) on the right.
pub(super) fn render_workspace(
    frame: &mut Frame,
    entries: &[IndexEntry],
    filtered: &[usize],
    selected: usize,
    workspace: &WorkspaceState,
) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(1)])
        .split(area);

    render_sidebar(frame, chunks[0], entries, filtered, selected, workspace);
    render_terminal_area(frame, chunks[1], workspace);
}

fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    entries: &[IndexEntry],
    filtered: &[usize],
    selected: usize,
    workspace: &WorkspaceState,
) {
    let is_focused = workspace.focus == Focus::Sidebar;
    let border_style = if is_focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Journeys ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|&idx| {
            let entry = &entries[idx];
            let has_tabs = workspace.has_tabs(&entry.id);
            let marker = if has_tabs { "\u{25cf} " } else { "  " };
            let style = if has_tabs {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, style),
                Span::raw(entry.title.clone()),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(selected));

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_terminal_area(frame: &mut Frame, area: Rect, workspace: &WorkspaceState) {
    let tabs = workspace.tabs_for_selected();
    let is_focused = workspace.focus == Focus::Terminal;

    if tabs.is_empty() {
        let border_style = Style::default().fg(Color::DarkGray);
        let block = Block::default()
            .title(" Terminal ")
            .borders(Borders::ALL)
            .border_style(border_style);
        let msg = Paragraph::new("\n  Select a journey and press Enter")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let active_idx = workspace.active_tab_index();

    let has_tab_bar = tabs.len() > 1;
    let chunks = if has_tab_bar {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(0), Constraint::Min(1)])
            .split(area)
    };

    if has_tab_bar {
        let tab_titles: Vec<Line> = tabs
            .iter()
            .enumerate()
            .map(|(i, _tab)| {
                let label = format!(" {} ", i + 1);
                if i == active_idx {
                    Line::from(Span::styled(label, Style::default().fg(ACCENT)))
                } else {
                    Line::from(Span::raw(label))
                }
            })
            .collect();
        let tab_bar = Tabs::new(tab_titles).select(active_idx);
        frame.render_widget(tab_bar, chunks[0]);
    }

    let border_style = if is_focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .title(format!(" {} ", tabs[active_idx].title))
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    if let Some(terminal) = workspace.active_terminal() {
        render_term_grid(frame, inner, terminal);
    }
}

fn render_term_grid(frame: &mut Frame, area: Rect, terminal: &EmbeddedTerminal) {
    terminal.with_term(|term| {
        let grid = term.grid();
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
                    buf_cell.set_style(cell_style(cell));
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
