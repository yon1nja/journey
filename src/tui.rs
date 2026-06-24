use std::fs;
use std::io::{self, IsTerminal, Stderr, Stdout, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use pulldown_cmark::{
    CodeBlockKind, Event as MarkdownEvent, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, HighlightSpacing, List, ListItem, ListState, Paragraph, Row,
    Table, TableState, Wrap,
};
use ratatui::{Frame, Terminal};

use crate::app;
use crate::events;
use crate::git;
use crate::models::{IndexEntry, JourneyStatus, RepoRef};
use crate::storage;

type JourneyTerminal = Terminal<CrosstermBackend<TerminalWriter>>;

enum TerminalWriter {
    Stdout(Stdout),
    Stderr(Stderr),
}

impl Write for TerminalWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            TerminalWriter::Stdout(stdout) => stdout.write(buf),
            TerminalWriter::Stderr(stderr) => stderr.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            TerminalWriter::Stdout(stdout) => stdout.flush(),
            TerminalWriter::Stderr(stderr) => stderr.flush(),
        }
    }
}

fn terminal_writer() -> TerminalWriter {
    if io::stdout().is_terminal() {
        TerminalWriter::Stdout(io::stdout())
    } else {
        TerminalWriter::Stderr(io::stderr())
    }
}

const ACTIONS: [JourneyAction; 8] = [
    JourneyAction::Cd,
    JourneyAction::Resume,
    JourneyAction::Worktree,
    JourneyAction::Link,
    JourneyAction::Unlink,
    JourneyAction::Pause,
    JourneyAction::Archive,
    JourneyAction::Abandon,
];

const ACCENT: Color = Color::Indexed(75);
const ACCENT_TEXT: Color = Color::Black;

pub(crate) fn run_journey_app(
    home: &Path,
    cwd: &Path,
    default_filter: JourneyStatus,
) -> Result<Option<String>> {
    let mut app = JourneyApp::new(home, cwd, default_filter)?;

    enable_raw_mode().context("failed to enable raw terminal mode")?;

    let mut writer = terminal_writer();
    if let Err(err) = execute!(writer, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(err).context("failed to enter alternate screen");
    }

    let backend = CrosstermBackend::new(writer);
    let mut terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(err) => {
            let _ = disable_raw_mode();
            let mut writer = terminal_writer();
            let _ = execute!(writer, LeaveAlternateScreen);
            return Err(err).context("failed to initialize terminal");
        }
    };
    let result = app.run(&mut terminal);
    let cleanup = restore_terminal(&mut terminal);

    match (result, cleanup) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(err),
    }
}

fn restore_terminal(terminal: &mut JourneyTerminal) -> Result<()> {
    disable_raw_mode().context("failed to disable raw terminal mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;
    Ok(())
}

struct JourneyApp {
    home: PathBuf,
    cwd: PathBuf,
    rows: Vec<IndexEntry>,
    filtered: Vec<usize>,
    filter: StatusFilter,
    query: String,
    selected: usize,
    table_state: TableState,
    focus: Focus,
    action_selected: usize,
    action_state: ListState,
    dialog: Dialog,
    notice: Option<Notice>,
    output: Option<String>,
    should_quit: bool,
}

impl JourneyApp {
    fn new(home: &Path, cwd: &Path, default_filter: JourneyStatus) -> Result<Self> {
        let mut app = Self {
            home: home.to_path_buf(),
            cwd: cwd.to_path_buf(),
            rows: Vec::new(),
            filtered: Vec::new(),
            filter: StatusFilter::Status(default_filter),
            query: String::new(),
            selected: 0,
            table_state: TableState::default(),
            focus: Focus::Journeys,
            action_selected: 0,
            action_state: ListState::default(),
            dialog: Dialog::None,
            notice: None,
            output: None,
            should_quit: false,
        };
        app.reload()?;
        Ok(app)
    }

    fn run(&mut self, terminal: &mut JourneyTerminal) -> Result<Option<String>> {
        loop {
            terminal.draw(|frame| render(frame, self))?;

            if self.should_quit {
                return Ok(self.output.take());
            }

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key)?;
                    }
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return Ok(());
        }

        let dialog_action = match &mut self.dialog {
            Dialog::None => {
                self.handle_main_key(key);
                None
            }
            Dialog::NewTitle { input } => {
                Some(handle_text_input(input, key, DialogKeyTarget::NewTitle))
            }
            Dialog::NewDescription { input, .. } => Some(handle_text_input(
                input,
                key,
                DialogKeyTarget::NewDescription,
            )),
            Dialog::WorktreeBranch { input, .. } => Some(handle_text_input(
                input,
                key,
                DialogKeyTarget::WorktreeBranch,
            )),
            Dialog::WorktreePath { input, .. } => {
                Some(handle_text_input(input, key, DialogKeyTarget::WorktreePath))
            }
            Dialog::Unlink {
                selected,
                repos,
                state,
                ..
            } => match key.code {
                KeyCode::Esc => Some(DialogAction::Cancel),
                KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                    sync_list_state(state, *selected, repos.len());
                    None
                }
                KeyCode::Down => {
                    if !repos.is_empty() {
                        *selected = (*selected + 1).min(repos.len().saturating_sub(1));
                    }
                    sync_list_state(state, *selected, repos.len());
                    None
                }
                KeyCode::Enter => Some(DialogAction::SubmitUnlink),
                _ => None,
            },
        };

        if let Some(action) = dialog_action {
            self.apply_dialog_action(action)?;
        }

        Ok(())
    }

    fn handle_main_key(&mut self, key: KeyEvent) {
        match self.focus {
            Focus::Journeys => self.handle_journey_key(key),
            Focus::Actions => self.handle_action_key(key),
        }
    }

    fn handle_journey_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                if self.query.is_empty() {
                    self.should_quit = true;
                } else {
                    self.query.clear();
                    self.apply_filter(None);
                }
            }
            KeyCode::Char('q') if self.query.is_empty() && key.modifiers.is_empty() => {
                self.should_quit = true;
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.dialog = Dialog::NewTitle {
                    input: String::new(),
                };
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.reload_with_notice();
            }
            KeyCode::Tab => self.cycle_filter(false),
            KeyCode::BackTab => self.cycle_filter(true),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Enter => {
                if self.selected_entry().is_some() {
                    self.focus = Focus::Actions;
                    self.action_selected = 0;
                    self.sync_action_state();
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.apply_filter(None);
            }
            KeyCode::Char(ch) if editable_modifiers(key.modifiers) => {
                self.query.push(ch);
                self.apply_filter(None);
            }
            _ => {}
        }
    }

    fn handle_action_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.focus = Focus::Journeys,
            KeyCode::Char('q') if key.modifiers.is_empty() => self.focus = Focus::Journeys,
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.dialog = Dialog::NewTitle {
                    input: String::new(),
                };
            }
            KeyCode::Up => {
                self.action_selected = self.action_selected.saturating_sub(1);
                self.sync_action_state();
            }
            KeyCode::Down => {
                self.action_selected = (self.action_selected + 1).min(ACTIONS.len() - 1);
                self.sync_action_state();
            }
            KeyCode::Enter => self.execute_selected_action(),
            _ => {}
        }
    }

    fn apply_dialog_action(&mut self, action: DialogAction) -> Result<()> {
        match action {
            DialogAction::None => {}
            DialogAction::Cancel => self.dialog = Dialog::None,
            DialogAction::SubmitNewTitle => {
                let Dialog::NewTitle { input } = &self.dialog else {
                    return Ok(());
                };
                let title = input.trim();
                if title.is_empty() {
                    self.notice_error("title is required");
                } else {
                    self.dialog = Dialog::NewDescription {
                        title: title.to_string(),
                        input: String::new(),
                    };
                }
            }
            DialogAction::SubmitNewDescription => self.create_journey_from_dialog()?,
            DialogAction::SubmitWorktreeBranch => self.open_worktree_path_dialog(),
            DialogAction::SubmitWorktreePath => self.create_worktree_from_dialog()?,
            DialogAction::SubmitUnlink => self.unlink_selected_repo()?,
        }
        Ok(())
    }

    fn create_journey_from_dialog(&mut self) -> Result<()> {
        let Dialog::NewDescription { title, input } = &self.dialog else {
            return Ok(());
        };
        let title = title.clone();
        let description = input.trim().to_string();
        let description = if description.is_empty() {
            None
        } else {
            Some(description)
        };

        let result = app::new_journey(&self.home, &title, description);
        self.dialog = Dialog::None;
        self.record_mutation(result, None)
    }

    fn open_worktree_path_dialog(&mut self) {
        let Dialog::WorktreeBranch {
            journey_id,
            input,
            default_branch,
        } = &self.dialog
        else {
            return;
        };
        let branch = if input.trim().is_empty() {
            default_branch.clone()
        } else {
            input.trim().to_string()
        };
        match git::discover_repo(&self.cwd) {
            Ok(discovered) => {
                let default_path = discovered.root.join(format!(".worktrees/{branch}"));
                self.dialog = Dialog::WorktreePath {
                    journey_id: journey_id.clone(),
                    branch,
                    input: String::new(),
                    default_path,
                };
            }
            Err(err) => {
                self.dialog = Dialog::None;
                self.notice_error(format!("cannot create worktree: {err}"));
            }
        }
    }

    fn create_worktree_from_dialog(&mut self) -> Result<()> {
        let Dialog::WorktreePath {
            journey_id,
            branch,
            input,
            default_path,
        } = &self.dialog
        else {
            return Ok(());
        };
        let journey_id = journey_id.clone();
        let branch = branch.clone();
        let path = if input.trim().is_empty() {
            default_path.clone()
        } else {
            PathBuf::from(input.trim())
        };
        let result = create_and_link_worktree(&self.home, &self.cwd, &journey_id, &branch, &path);
        self.dialog = Dialog::None;
        self.record_mutation(result, Some(journey_id))
    }

    fn execute_selected_action(&mut self) {
        let Some(journey_id) = self.selected_id() else {
            self.notice_error("no Journey selected");
            return;
        };
        let action = ACTIONS[self.action_selected];

        match action {
            JourneyAction::Cd => {
                let dir = storage::journey_dir(&self.home, &journey_id);
                let output = if app::shell_integration_active() {
                    format!("{}{}", app::SHELL_CD_PREFIX, dir.display())
                } else {
                    format!(
                        "selected Journey path: {} (enable parent-shell cd with: eval \"$(journey shell-init)\")",
                        dir.display()
                    )
                };
                self.output = Some(output);
                self.should_quit = true;
            }
            JourneyAction::Resume => {
                let result = app::resume(&self.home, &self.cwd, Some(&journey_id));
                let _ = self.record_mutation(result, Some(journey_id));
            }
            JourneyAction::Worktree => {
                let default_branch = storage::slugify(&journey_id);
                self.dialog = Dialog::WorktreeBranch {
                    journey_id,
                    input: String::new(),
                    default_branch,
                };
            }
            JourneyAction::Link => {
                let result =
                    app::link_repo(&self.home, &self.cwd, Some(&journey_id), &self.cwd, None);
                let _ = self.record_mutation(result, Some(journey_id));
            }
            JourneyAction::Unlink => self.open_unlink_dialog(&journey_id),
            JourneyAction::Pause => {
                let result = app::set_status(
                    &self.home,
                    &self.cwd,
                    Some(&journey_id),
                    JourneyStatus::Paused,
                );
                let _ = self.record_mutation(result, Some(journey_id));
            }
            JourneyAction::Archive => {
                let result = app::set_status(
                    &self.home,
                    &self.cwd,
                    Some(&journey_id),
                    JourneyStatus::Archived,
                );
                let _ = self.record_mutation(result, Some(journey_id));
            }
            JourneyAction::Abandon => {
                let result = app::set_status(
                    &self.home,
                    &self.cwd,
                    Some(&journey_id),
                    JourneyStatus::Abandoned,
                );
                let _ = self.record_mutation(result, Some(journey_id));
            }
        }
    }

    fn open_unlink_dialog(&mut self, journey_id: &str) {
        match storage::resolve_context(&self.home, Some(journey_id), &self.cwd) {
            Ok(ctx) if ctx.journey.repos.is_empty() => self.notice_error("no repos linked"),
            Ok(ctx) => {
                let mut state = ListState::default();
                state.select(Some(0));
                self.dialog = Dialog::Unlink {
                    journey_id: journey_id.to_string(),
                    repos: ctx.journey.repos,
                    selected: 0,
                    state,
                };
            }
            Err(err) => self.notice_error(err.to_string()),
        }
    }

    fn unlink_selected_repo(&mut self) -> Result<()> {
        let Dialog::Unlink {
            journey_id,
            repos,
            selected,
            ..
        } = &self.dialog
        else {
            return Ok(());
        };
        let Some(repo) = repos.get(*selected) else {
            self.notice_error("no repo selected");
            return Ok(());
        };
        let journey_id = journey_id.clone();
        let repo_name = repo.name.clone();
        let result = app::unlink_repo(&self.home, &self.cwd, Some(&journey_id), &repo_name);
        self.dialog = Dialog::None;
        self.record_mutation(result, Some(journey_id))
    }

    fn record_mutation(
        &mut self,
        result: Result<String>,
        preferred_id: Option<String>,
    ) -> Result<()> {
        match result {
            Ok(message) => {
                self.notice_success(message);
                self.focus = Focus::Journeys;
                self.reload_preserving(preferred_id.as_deref())?;
            }
            Err(err) => self.notice_error(err.to_string()),
        }
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        self.reload_preserving(None)
    }

    fn reload_preserving(&mut self, preferred_id: Option<&str>) -> Result<()> {
        storage::ensure_home(&self.home)?;
        let mut rows = storage::load_index(&self.home)?.journeys;
        rows.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
        self.rows = rows;
        self.apply_filter(preferred_id);
        Ok(())
    }

    fn reload_with_notice(&mut self) {
        let selected = self.selected_id();
        match self.reload_preserving(selected.as_deref()) {
            Ok(()) => self.notice_success("reloaded Journeys"),
            Err(err) => self.notice_error(format!("reload failed: {err}")),
        }
    }

    fn apply_filter(&mut self, preferred_id: Option<&str>) {
        self.filtered = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| self.entry_matches(entry).then_some(idx))
            .collect();

        if self.filtered.is_empty() {
            self.selected = 0;
            self.table_state.select(None);
            return;
        }

        self.selected = preferred_id
            .and_then(|id| {
                self.filtered
                    .iter()
                    .position(|idx| self.rows[*idx].id == id)
            })
            .unwrap_or_else(|| self.selected.min(self.filtered.len() - 1));
        self.sync_table_state();
    }

    fn entry_matches(&self, entry: &IndexEntry) -> bool {
        if let StatusFilter::Status(status) = self.filter {
            if entry.status != status {
                return false;
            }
        }

        let query = self.query.trim();
        if query.is_empty() {
            return true;
        }

        let haystack = format!(
            "{} {} {} {} {}",
            entry.id,
            entry.title,
            entry.description.as_deref().unwrap_or(""),
            entry.status,
            entry.repos.join(" ")
        )
        .to_lowercase();

        query
            .to_lowercase()
            .split_whitespace()
            .all(|term| haystack.contains(term))
    }

    fn selected_entry(&self) -> Option<&IndexEntry> {
        let idx = *self.filtered.get(self.selected)?;
        self.rows.get(idx)
    }

    fn selected_id(&self) -> Option<String> {
        self.selected_entry().map(|entry| entry.id.clone())
    }

    fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as isize;
        let next = (self.selected as isize + delta).clamp(0, len - 1);
        self.selected = next as usize;
        self.sync_table_state();
    }

    fn cycle_filter(&mut self, reverse: bool) {
        self.filter = self.filter.cycle(reverse);
        self.apply_filter(None);
    }

    fn sync_table_state(&mut self) {
        if self.filtered.is_empty() {
            self.table_state.select(None);
        } else {
            self.table_state.select(Some(self.selected));
        }
    }

    fn sync_action_state(&mut self) {
        self.action_state.select(Some(self.action_selected));
    }

    fn notice_success(&mut self, message: impl Into<String>) {
        self.notice = Some(Notice {
            message: message.into(),
            is_error: false,
        });
    }

    fn notice_error(&mut self, message: impl Into<String>) {
        self.notice = Some(Notice {
            message: message.into(),
            is_error: true,
        });
    }
}

fn render(frame: &mut Frame<'_>, app: &mut JourneyApp) {
    let area = frame.area();
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    render_header(frame, app, root[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(root[1]);
    render_journey_list(frame, app, body[0]);
    render_details(frame, app, body[1]);
    render_footer(frame, app, root[2]);
    render_dialog(frame, app, area);
}

fn render_header(frame: &mut Frame<'_>, app: &JourneyApp, area: Rect) {
    let repo_hint = git::discover_repo(&app.cwd)
        .ok()
        .and_then(|repo| {
            repo.root
                .file_name()
                .map(|name| format!("git: {}, {}", name.to_string_lossy(), repo.branch))
        })
        .unwrap_or_else(|| "no git repo detected".to_string());

    let lines = vec![
        Line::from(vec![
            Span::styled("Journey", accent().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(repo_hint, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled("filter: ", dim()),
            Span::styled(app.filter.label(), accent()),
            Span::raw("  "),
            Span::styled("search: ", dim()),
            Span::raw(if app.query.is_empty() {
                String::from("")
            } else {
                app.query.clone()
            }),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_journey_list(frame: &mut Frame<'_>, app: &mut JourneyApp, area: Rect) {
    let rows = if app.filtered.is_empty() {
        vec![Row::new([
            Cell::from("No matching Journeys").style(dim()),
            Cell::from(""),
            Cell::from(""),
        ])]
    } else {
        app.filtered
            .iter()
            .map(|idx| journey_row(&app.rows[*idx]))
            .collect()
    };

    let title = match app.focus {
        Focus::Journeys => " Journeys ",
        Focus::Actions => " Journeys ",
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(match app.focus {
            Focus::Journeys => accent(),
            Focus::Actions => Style::default().fg(Color::DarkGray),
        });
    let header = Row::new([
        Cell::from("Journey"),
        Cell::from("Status"),
        Cell::from("Repos"),
    ])
    .style(dim())
    .bottom_margin(1);
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(54),
            Constraint::Length(11),
            Constraint::Min(10),
        ],
    )
    .block(block)
    .header(header)
    .column_spacing(2)
    .row_highlight_style(selected_row())
    .highlight_symbol(selection_stroke())
    .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn journey_row(entry: &IndexEntry) -> Row<'static> {
    let repos = if entry.repos.is_empty() {
        "no repos".to_string()
    } else if entry.repos.len() == 1 {
        entry.repos[0].clone()
    } else {
        format!("{} repos", entry.repos.len())
    };

    Row::new([
        Cell::from(entry.title.clone()).style(Style::default().fg(Color::White)),
        Cell::from(entry.status.to_string()).style(status_style(entry.status)),
        Cell::from(repos).style(dim()),
    ])
}

fn render_details(frame: &mut Frame<'_>, app: &mut JourneyApp, area: Rect) {
    match app.focus {
        Focus::Journeys => {
            let block = Block::default().title(" Details ").borders(Borders::ALL);
            let text = Text::from(detail_lines(app));
            frame.render_widget(
                Paragraph::new(text).block(block).wrap(Wrap { trim: false }),
                area,
            );
        }
        Focus::Actions => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
                .split(area);
            let block = Block::default().title(" Details ").borders(Borders::ALL);
            frame.render_widget(
                Paragraph::new(Text::from(detail_lines(app)))
                    .block(block)
                    .wrap(Wrap { trim: false }),
                chunks[0],
            );
            render_actions(frame, app, chunks[1]);
        }
    }
}

fn render_actions(frame: &mut Frame<'_>, app: &mut JourneyApp, area: Rect) {
    let items = ACTIONS
        .iter()
        .map(|action| {
            ListItem::new(Line::from(vec![
                Span::styled(action.label(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(action.description(), dim()),
            ]))
        })
        .collect::<Vec<_>>();
    let block = Block::default()
        .title(" Actions ")
        .borders(Borders::ALL)
        .border_style(accent());
    let list = List::new(items)
        .block(block)
        .highlight_style(selected())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut app.action_state);
}

fn render_footer(frame: &mut Frame<'_>, app: &JourneyApp, area: Rect) {
    let notice = app.notice.as_ref().map(|notice| {
        let style = if notice.is_error {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::Green)
        };
        Line::from(Span::styled(notice.message.clone(), style))
    });
    let help = match app.focus {
        Focus::Journeys => "Enter actions  Ctrl-N new  Tab filter  Ctrl-R reload  Esc quit",
        Focus::Actions => "Enter run  Esc back  Ctrl-N new",
    };
    let lines = vec![
        notice.unwrap_or_else(|| Line::from("")),
        Line::from(Span::styled(help, dim())),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::TOP)),
        area,
    );
}

fn render_dialog(frame: &mut Frame<'_>, app: &mut JourneyApp, area: Rect) {
    match &mut app.dialog {
        Dialog::None => {}
        Dialog::NewTitle { input } => {
            render_input_dialog(
                frame,
                area,
                "New Journey",
                "Title",
                input,
                "Enter next  Esc cancel",
            );
        }
        Dialog::NewDescription { title, input } => render_input_dialog(
            frame,
            area,
            &format!("New Journey: {title}"),
            "Description",
            input,
            "Enter create  Esc cancel",
        ),
        Dialog::WorktreeBranch {
            input,
            default_branch,
            ..
        } => render_input_dialog(
            frame,
            area,
            "New Branch + Worktree",
            &format!("Branch (default: {default_branch})"),
            input,
            "Enter next  Esc cancel",
        ),
        Dialog::WorktreePath {
            input,
            default_path,
            ..
        } => render_input_dialog(
            frame,
            area,
            "New Branch + Worktree",
            &format!("Path (default: {})", default_path.display()),
            input,
            "Enter create  Esc cancel",
        ),
        Dialog::Unlink { repos, state, .. } => {
            let area = centered_rect(68, 60, area);
            frame.render_widget(Clear, area);
            let items = repos
                .iter()
                .map(|repo| {
                    ListItem::new(vec![
                        Line::from(Span::styled(
                            repo.name.clone(),
                            Style::default().fg(Color::White),
                        )),
                        Line::from(Span::styled(repo.worktree.display().to_string(), dim())),
                    ])
                })
                .collect::<Vec<_>>();
            let list = List::new(items)
                .block(
                    Block::default()
                        .title(" Unlink Repo ")
                        .borders(Borders::ALL)
                        .border_style(accent()),
                )
                .highlight_style(Style::default().fg(Color::Black).bg(Color::Red))
                .highlight_symbol("> ");
            frame.render_stateful_widget(list, area, state);
        }
    }
}

fn render_input_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    label: &str,
    input: &str,
    help: &str,
) {
    let area = centered_rect(64, 32, area);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled(label.to_string(), dim())),
        Line::from(Span::styled(
            if input.is_empty() {
                " ".to_string()
            } else {
                input.to_string()
            },
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(help.to_string(), dim())),
    ];
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(accent());
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn detail_lines(app: &JourneyApp) -> Vec<Line<'static>> {
    let Some(entry) = app.selected_entry() else {
        return vec![
            Line::from(Span::styled("No Journeys", accent())),
            Line::from(""),
            Line::from(Span::styled("Create one with Ctrl-N.", dim())),
        ];
    };

    let journey_path = storage::journey_dir(&app.home, &entry.id);
    let mut lines = vec![Line::from(Span::styled(
        entry.title.clone(),
        accent().add_modifier(Modifier::BOLD),
    ))];
    if let Some(description) = &entry.description {
        lines.push(label_value("description:", description.clone()));
    }
    lines.push(label_value("id:", entry.id.clone()));
    lines.push(Line::from(vec![
        Span::styled("status:", dim()),
        Span::raw(" "),
        Span::styled(entry.status.to_string(), status_style(entry.status)),
    ]));
    lines.push(label_value("updated:", entry.updated.clone()));
    lines.push(label_value("path:", journey_path.display().to_string()));

    let event_count = events::read_events(&journey_path)
        .map(|events| events.len().to_string())
        .unwrap_or_else(|_| "unavailable".to_string());
    lines.push(label_value("events:", event_count));
    lines.push(Line::from(""));
    lines.extend(repo_lines(&app.home, &entry.id));
    lines.push(Line::from(""));
    lines.extend(doc_lines(&journey_path));
    lines.extend(readme_lines(&journey_path));
    lines
}

fn repo_lines(home: &Path, id: &str) -> Vec<Line<'static>> {
    let journey_path = storage::journey_dir(home, id);
    let Ok(journey) = storage::load_journey(&journey_path) else {
        return vec![Line::from(Span::styled("repos: unavailable", dim()))];
    };
    if journey.repos.is_empty() {
        return vec![Line::from(vec![
            Span::styled("repos:", dim()),
            Span::raw(" none"),
        ])];
    }

    let mut lines = vec![Line::from(Span::styled("repos:", dim()))];
    for repo in journey.repos {
        lines.push(Line::from(vec![
            Span::raw("- "),
            Span::styled(repo.name, accent()),
            Span::raw("  "),
            Span::styled(repo.branch, dim()),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(repo.worktree.display().to_string(), dim()),
        ]));
    }
    lines
}

fn doc_lines(journey_path: &Path) -> Vec<Line<'static>> {
    let docs_dir = journey_path.join(storage::DOCS_DIR);
    let Ok(entries) = fs::read_dir(&docs_dir) else {
        return vec![Line::from(vec![
            Span::styled("docs:", dim()),
            Span::raw(" none"),
        ])];
    };

    let mut docs = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.is_file() {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    docs.sort();

    if docs.is_empty() {
        return vec![Line::from(vec![
            Span::styled("docs:", dim()),
            Span::raw(" none"),
        ])];
    }

    let mut lines = vec![Line::from(Span::styled("docs:", dim()))];
    for doc in docs.iter().take(8) {
        lines.push(Line::from(vec![
            Span::raw("- "),
            Span::styled(format!("docs/{doc}"), accent()),
        ]));
    }
    if docs.len() > 8 {
        lines.push(Line::from(Span::styled(
            format!("- ... {} more", docs.len() - 8),
            dim(),
        )));
    }
    lines
}

fn readme_lines(journey_path: &Path) -> Vec<Line<'static>> {
    let path = journey_path.join(storage::README_FILE);
    if !path.exists() {
        return Vec::new();
    }

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled("README.md:", dim())),
    ];
    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            lines.push(Line::from(Span::styled("(empty)", dim())));
        }
        Ok(content) => {
            lines.extend(render_markdown_lines(&content));
        }
        Err(err) => {
            lines.push(Line::from(Span::styled(
                format!("unavailable: {err}"),
                dim(),
            )));
        }
    }
    lines
}

fn render_markdown_lines(content: &str) -> Vec<Line<'static>> {
    let mut renderer = MarkdownRenderer::default();
    let parser = Parser::new_ext(
        content,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES,
    );

    for event in parser {
        renderer.handle(event);
    }

    renderer.finish()
}

#[derive(Default)]
struct MarkdownRenderer {
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    list_stack: Vec<ListMarker>,
    current_item_prefix: Option<String>,
    blockquote_depth: usize,
    code_block: bool,
    pending_link_destinations: Vec<String>,
}

impl MarkdownRenderer {
    fn handle(&mut self, event: MarkdownEvent<'_>) {
        match event {
            MarkdownEvent::Start(tag) => self.start_tag(tag),
            MarkdownEvent::End(tag) => self.end_tag(tag),
            MarkdownEvent::Text(text)
            | MarkdownEvent::Html(text)
            | MarkdownEvent::InlineHtml(text) => {
                self.push_text(text.as_ref());
            }
            MarkdownEvent::Code(code) | MarkdownEvent::InlineMath(code) => {
                self.push_span(Span::styled(code.to_string(), inline_code()));
            }
            MarkdownEvent::DisplayMath(math) => {
                self.flush_current();
                self.push_span(Span::styled(format!("    {math}"), code_block_style()));
                self.flush_current();
            }
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => self.flush_current(),
            MarkdownEvent::Rule => {
                self.flush_current();
                self.lines
                    .push(Line::from(Span::styled("----------------", dim())));
            }
            MarkdownEvent::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                self.push_span(Span::styled(marker, dim()));
            }
            MarkdownEvent::FootnoteReference(reference) => {
                self.push_span(Span::styled(format!("[{reference}]"), dim()));
            }
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_current();
                self.push_style(heading_style(level));
            }
            Tag::BlockQuote(_) => {
                self.flush_current();
                self.blockquote_depth += 1;
                self.push_style(blockquote_style());
            }
            Tag::CodeBlock(kind) => {
                self.flush_current();
                self.code_block = true;
                if let CodeBlockKind::Fenced(language) = kind {
                    if !language.is_empty() {
                        self.lines.push(Line::from(vec![
                            Span::styled("code", dim()),
                            Span::styled(format!(" {language}"), dim()),
                        ]));
                    }
                }
            }
            Tag::List(start) => self.list_stack.push(ListMarker::new(start)),
            Tag::Item => self.start_list_item(),
            Tag::Emphasis => self.push_style(self.current_style().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(self.current_style().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => {
                self.push_style(self.current_style().add_modifier(Modifier::CROSSED_OUT));
            }
            Tag::Link { dest_url, .. } => {
                self.pending_link_destinations.push(dest_url.to_string());
                self.push_style(accent().add_modifier(Modifier::UNDERLINED));
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                let label = if title.is_empty() {
                    dest_url.to_string()
                } else {
                    format!("{title} ({dest_url})")
                };
                self.push_span(Span::styled(format!("[image: {label}]"), dim()));
                self.push_style(dim());
            }
            Tag::Table(_) | Tag::TableHead | Tag::TableRow | Tag::TableCell => {}
            Tag::FootnoteDefinition(name) => {
                self.flush_current();
                self.push_span(Span::styled(format!("[{name}] "), dim()));
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph
            | TagEnd::FootnoteDefinition
            | TagEnd::Table
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => self.flush_current(),
            TagEnd::Heading(_) => {
                self.flush_current();
                self.pop_style();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_current();
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
                self.pop_style();
            }
            TagEnd::CodeBlock => {
                self.flush_current();
                self.code_block = false;
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Item => {
                self.flush_current();
                self.current_item_prefix = None;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link => {
                self.pop_style();
                if let Some(destination) = self.pending_link_destinations.pop() {
                    if !destination.is_empty() {
                        self.push_span(Span::styled(format!(" ({destination})"), dim()));
                    }
                }
            }
            TagEnd::Image => self.pop_style(),
            TagEnd::HtmlBlock | TagEnd::TableCell => {}
        }
    }

    fn start_list_item(&mut self) {
        self.flush_current();
        let depth = self.list_stack.len().saturating_sub(1);
        let indent = "  ".repeat(depth);
        let marker = self
            .list_stack
            .last_mut()
            .map(ListMarker::next)
            .unwrap_or_else(|| "- ".to_string());
        self.current_item_prefix = Some(format!("{indent}{marker}"));
    }

    fn push_text(&mut self, text: &str) {
        if self.code_block {
            for (idx, line) in text.split('\n').enumerate() {
                if idx > 0 {
                    self.flush_current();
                }
                if !line.is_empty() {
                    self.push_span(Span::styled(format!("    {line}"), code_block_style()));
                }
            }
            return;
        }

        for (idx, part) in text.split('\n').enumerate() {
            if idx > 0 {
                self.flush_current();
            }
            if !part.is_empty() {
                self.push_span(Span::styled(part.to_string(), self.current_style()));
            }
        }
    }

    fn push_span(&mut self, span: Span<'static>) {
        if self.current.is_empty() {
            self.push_prefix();
        }
        self.current.push(span);
    }

    fn push_prefix(&mut self) {
        for _ in 0..self.blockquote_depth {
            self.current.push(Span::styled("> ", dim()));
        }
        if let Some(prefix) = &self.current_item_prefix {
            self.current.push(Span::styled(prefix.clone(), dim()));
        }
    }

    fn flush_current(&mut self) {
        if !self.current.is_empty() {
            self.lines
                .push(Line::from(std::mem::take(&mut self.current)));
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_current();
        self.lines
    }

    fn current_style(&self) -> Style {
        self.style_stack
            .last()
            .copied()
            .unwrap_or_else(Style::default)
    }

    fn push_style(&mut self, style: Style) {
        self.style_stack.push(style);
    }

    fn pop_style(&mut self) {
        self.style_stack.pop();
    }
}

struct ListMarker {
    next: Option<u64>,
}

impl ListMarker {
    fn new(next: Option<u64>) -> Self {
        Self { next }
    }

    fn next(&mut self) -> String {
        match self.next {
            Some(value) => {
                self.next = Some(value + 1);
                format!("{value}. ")
            }
            None => "- ".to_string(),
        }
    }
}

fn label_value(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(label.to_string(), dim()),
        Span::raw(" "),
        Span::raw(value),
    ])
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn handle_text_input(input: &mut String, key: KeyEvent, target: DialogKeyTarget) -> DialogAction {
    match key.code {
        KeyCode::Esc => DialogAction::Cancel,
        KeyCode::Enter => target.submit_action(),
        KeyCode::Backspace => {
            input.pop();
            DialogAction::None
        }
        KeyCode::Char(ch) if editable_modifiers(key.modifiers) => {
            input.push(ch);
            DialogAction::None
        }
        _ => DialogAction::None,
    }
}

fn editable_modifiers(modifiers: KeyModifiers) -> bool {
    modifiers.is_empty() || modifiers == KeyModifiers::SHIFT
}

fn sync_list_state(state: &mut ListState, selected: usize, len: usize) {
    if len == 0 {
        state.select(None);
    } else {
        state.select(Some(selected.min(len - 1)));
    }
}

fn create_and_link_worktree(
    home: &Path,
    cwd: &Path,
    journey_id: &str,
    branch: &str,
    path: &Path,
) -> Result<String> {
    let discovered = git::discover_repo(cwd)?;
    git::create_worktree(&discovered.root, path, branch, true)?;
    let repo_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "worktree".to_string());
    let linked = app::link_repo(home, cwd, Some(journey_id), path, Some(repo_name))?;
    Ok(format!(
        "created worktree {} on branch `{}`\n{}",
        path.display(),
        branch,
        linked
    ))
}

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn heading_style(level: HeadingLevel) -> Style {
    let style = accent().add_modifier(Modifier::BOLD);
    match level {
        HeadingLevel::H1 | HeadingLevel::H2 => style.add_modifier(Modifier::UNDERLINED),
        _ => style,
    }
}

fn blockquote_style() -> Style {
    dim().add_modifier(Modifier::ITALIC)
}

fn inline_code() -> Style {
    Style::default().fg(Color::Yellow)
}

fn code_block_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn accent() -> Style {
    Style::default().fg(ACCENT)
}

fn selected() -> Style {
    Style::default().fg(ACCENT_TEXT).bg(ACCENT)
}

fn selected_row() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn selection_stroke() -> Text<'static> {
    Text::from(Line::from(vec![
        Span::styled("|", accent()),
        Span::raw(" "),
    ]))
}

fn status_style(status: JourneyStatus) -> Style {
    match status {
        JourneyStatus::Active => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        JourneyStatus::Paused => Style::default().fg(Color::Yellow),
        JourneyStatus::Archived => Style::default().fg(Color::DarkGray),
        JourneyStatus::Abandoned => Style::default().fg(Color::Red),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Journeys,
    Actions,
}

#[derive(Clone, Copy)]
enum JourneyAction {
    Cd,
    Resume,
    Worktree,
    Link,
    Unlink,
    Pause,
    Archive,
    Abandon,
}

impl JourneyAction {
    fn label(self) -> &'static str {
        match self {
            JourneyAction::Cd => "cd journey",
            JourneyAction::Resume => "Resume",
            JourneyAction::Worktree => "New branch + worktree",
            JourneyAction::Link => "Link current worktree",
            JourneyAction::Unlink => "Unlink a repo",
            JourneyAction::Pause => "Pause",
            JourneyAction::Archive => "Archive",
            JourneyAction::Abandon => "Abandon",
        }
    }

    fn description(self) -> &'static str {
        match self {
            JourneyAction::Cd => "shell integration aware",
            JourneyAction::Resume => "mark active",
            JourneyAction::Worktree => "git worktree add -b",
            JourneyAction::Link => "attach cwd repo",
            JourneyAction::Unlink => "detach linked repo",
            JourneyAction::Pause => "lifecycle only",
            JourneyAction::Archive => "release worktrees",
            JourneyAction::Abandon => "release worktrees",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusFilter {
    Status(JourneyStatus),
    All,
}

impl StatusFilter {
    fn label(self) -> &'static str {
        match self {
            StatusFilter::Status(JourneyStatus::Active) => "active",
            StatusFilter::Status(JourneyStatus::Paused) => "paused",
            StatusFilter::Status(JourneyStatus::Archived) => "archived",
            StatusFilter::Status(JourneyStatus::Abandoned) => "abandoned",
            StatusFilter::All => "all",
        }
    }

    fn cycle(self, reverse: bool) -> Self {
        let values = [
            StatusFilter::Status(JourneyStatus::Active),
            StatusFilter::Status(JourneyStatus::Paused),
            StatusFilter::Status(JourneyStatus::Archived),
            StatusFilter::Status(JourneyStatus::Abandoned),
            StatusFilter::All,
        ];
        let current = values.iter().position(|value| *value == self).unwrap_or(0);
        let next = if reverse {
            current.checked_sub(1).unwrap_or(values.len() - 1)
        } else {
            (current + 1) % values.len()
        };
        values[next]
    }
}

enum Dialog {
    None,
    NewTitle {
        input: String,
    },
    NewDescription {
        title: String,
        input: String,
    },
    WorktreeBranch {
        journey_id: String,
        input: String,
        default_branch: String,
    },
    WorktreePath {
        journey_id: String,
        branch: String,
        input: String,
        default_path: PathBuf,
    },
    Unlink {
        journey_id: String,
        repos: Vec<RepoRef>,
        selected: usize,
        state: ListState,
    },
}

struct Notice {
    message: String,
    is_error: bool,
}

#[derive(Clone, Copy)]
enum DialogKeyTarget {
    NewTitle,
    NewDescription,
    WorktreeBranch,
    WorktreePath,
}

impl DialogKeyTarget {
    fn submit_action(self) -> DialogAction {
        match self {
            DialogKeyTarget::NewTitle => DialogAction::SubmitNewTitle,
            DialogKeyTarget::NewDescription => DialogAction::SubmitNewDescription,
            DialogKeyTarget::WorktreeBranch => DialogAction::SubmitWorktreeBranch,
            DialogKeyTarget::WorktreePath => DialogAction::SubmitWorktreePath,
        }
    }
}

enum DialogAction {
    None,
    Cancel,
    SubmitNewTitle,
    SubmitNewDescription,
    SubmitWorktreeBranch,
    SubmitWorktreePath,
    SubmitUnlink,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_readme_markdown_as_terminal_lines() {
        let lines = render_markdown_lines(
            "# Overview\n\nA **short** note with [docs](https://example.com).\n\n- first\n- second\n\n```sh\ncargo test\n```",
        );
        let text = lines.iter().map(plain_line).collect::<Vec<_>>().join("\n");

        assert!(text.contains("Overview"));
        assert!(text.contains("A short note with docs (https://example.com)."));
        assert!(text.contains("- first"));
        assert!(text.contains("- second"));
        assert!(text.contains("code sh"));
        assert!(text.contains("    cargo test"));
    }

    fn plain_line(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }
}
