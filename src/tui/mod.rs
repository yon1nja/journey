mod dialog;
mod markdown;
mod render;

use std::fs;
use std::io::{self, IsTerminal, Stderr, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

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
    Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
};
use ratatui::{Frame, Terminal};
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    FontStyle as SyntectFontStyle, Style as SyntectStyle, Theme, ThemeSet,
};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::app;
use crate::config::{Command as ShortcutCommand, ShortcutConfig, TuiAction};
use crate::events;
use crate::git;
use crate::models::{IndexEntry, JourneyStatus, RepoRef};
use crate::storage;

use dialog::*;
use markdown::*;
use render::*;

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
        (Ok(AppOutput::None), Ok(())) => Ok(None),
        (
            Ok(AppOutput::RunInJourney {
                command,
                journey_path,
            }),
            Ok(()),
        ) => {
            run_command_in_journey(command, &journey_path)?;
            Ok(None)
        }
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
    screen: Screen,
    shortcuts: ShortcutConfig,
    details_scroll: usize,
    details_viewport_height: usize,
    doc_tab_index: usize,
    action_selected: usize,
    action_state: ListState,
    dialog: Dialog,
    notice: Option<Notice>,
    doc_render_cache: DocRenderCache,
    output: Option<AppOutput>,
    should_quit: bool,
}

enum AppOutput {
    None,
    RunInJourney {
        command: &'static str,
        journey_path: PathBuf,
    },
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
            screen: Screen::List,
            shortcuts: ShortcutConfig::load(home)?,
            details_scroll: 0,
            details_viewport_height: 1,
            doc_tab_index: 0,
            action_selected: 0,
            action_state: ListState::default(),
            dialog: Dialog::None,
            notice: None,
            doc_render_cache: DocRenderCache::default(),
            output: None,
            should_quit: false,
        };
        app.reload()?;
        Ok(app)
    }

    fn run(&mut self, terminal: &mut JourneyTerminal) -> Result<AppOutput> {
        loop {
            terminal.draw(|frame| render(frame, self))?;

            if self.should_quit {
                return Ok(self.output.take().unwrap_or(AppOutput::None));
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

        if matches!(self.dialog, Dialog::None) {
            match self.screen {
                Screen::List => self.handle_list_key(key),
                Screen::Search => self.handle_search_key(key),
                Screen::Details => self.handle_details_key(key),
                Screen::Actions => self.handle_action_key(key),
            }
            return Ok(());
        }

        let dialog_action = match &mut self.dialog {
            Dialog::None => unreachable!(),
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
            Dialog::Capture { input, .. } => {
                Some(handle_text_input(input, key, DialogKeyTarget::Capture))
            }
            Dialog::ExistingBranch {
                selected,
                branches,
                query,
                state,
                ..
            } => match key.code {
                KeyCode::Esc => Some(DialogAction::Cancel),
                KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                    sync_list_state(
                        state,
                        *selected,
                        branch_filter_indices(branches, query).len(),
                    );
                    None
                }
                KeyCode::Down => {
                    let filtered_len = branch_filter_indices(branches, query).len();
                    if filtered_len > 0 {
                        *selected = (*selected + 1).min(filtered_len.saturating_sub(1));
                    }
                    sync_list_state(state, *selected, filtered_len);
                    None
                }
                KeyCode::Backspace => {
                    query.pop();
                    *selected = 0;
                    sync_list_state(
                        state,
                        *selected,
                        branch_filter_indices(branches, query).len(),
                    );
                    None
                }
                KeyCode::Char(ch) if editable_modifiers(key.modifiers) => {
                    query.push(ch);
                    *selected = 0;
                    sync_list_state(
                        state,
                        *selected,
                        branch_filter_indices(branches, query).len(),
                    );
                    None
                }
                KeyCode::Enter => Some(DialogAction::SubmitExistingBranch),
                _ => None,
            },
            Dialog::ExistingWorktreePath { input, .. } => Some(handle_text_input(
                input,
                key,
                DialogKeyTarget::ExistingWorktreePath,
            )),
            Dialog::ExistingWorktreeLinkConfirm { .. } => match key.code {
                KeyCode::Esc => Some(DialogAction::Cancel),
                KeyCode::Char('n') if key.modifiers.is_empty() => Some(DialogAction::Cancel),
                KeyCode::Char('y') if key.modifiers.is_empty() => {
                    Some(DialogAction::SubmitExistingWorktreeLink)
                }
                KeyCode::Enter => Some(DialogAction::SubmitExistingWorktreeLink),
                _ => None,
            },
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
            Dialog::DeleteWorktree {
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
                KeyCode::Enter => Some(DialogAction::SubmitDeleteWorktree),
                _ => None,
            },
            Dialog::DoneConfirm { .. } => match key.code {
                KeyCode::Esc => Some(DialogAction::Cancel),
                KeyCode::Enter => Some(DialogAction::SubmitDone),
                _ => None,
            },
        };

        if let Some(action) = dialog_action {
            self.apply_dialog_action(action)?;
        }

        Ok(())
    }

    fn open_new_journey_dialog(&mut self) {
        self.dialog = Dialog::NewTitle {
            input: String::new(),
        };
    }

    fn handle_list_key(&mut self, key: KeyEvent) {
        match self.shortcuts.command(key) {
            Some(ShortcutCommand::Quit) | Some(ShortcutCommand::Back) => {
                self.should_quit = true;
            }
            Some(ShortcutCommand::FocusSearch) => {
                self.screen = Screen::Search;
            }
            Some(ShortcutCommand::FocusDetails) | Some(ShortcutCommand::NavRight) => {
                if self.selected_entry().is_some() {
                    self.screen = Screen::Details;
                }
            }
            Some(ShortcutCommand::OpenActions) | Some(ShortcutCommand::Confirm) => {
                if self.selected_entry().is_some() {
                    self.screen = Screen::Actions;
                    self.action_selected = 0;
                    self.sync_action_state();
                }
            }
            Some(ShortcutCommand::NavUp) => self.move_selection(-1),
            Some(ShortcutCommand::NavDown) => self.move_selection(1),
            Some(ShortcutCommand::NewJourney) => self.open_new_journey_dialog(),
            Some(ShortcutCommand::Reload) => self.reload_with_notice(),
            Some(ShortcutCommand::CycleFilter) => self.cycle_filter(false),
            Some(ShortcutCommand::CycleFilterBack) => self.cycle_filter(true),
            Some(ShortcutCommand::Action(action)) => self.execute_shortcut_action(action),
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match self.shortcuts.command(key) {
            Some(ShortcutCommand::Quit) => {
                self.should_quit = true;
            }
            Some(ShortcutCommand::Back) => {
                if self.query.is_empty() {
                    self.screen = Screen::List;
                } else {
                    self.query.clear();
                    self.apply_filter(None);
                }
            }
            Some(ShortcutCommand::Confirm) => {
                self.screen = Screen::List;
            }
            Some(ShortcutCommand::NavUp) => self.move_selection(-1),
            Some(ShortcutCommand::NavDown) => self.move_selection(1),
            Some(ShortcutCommand::NewJourney) => self.open_new_journey_dialog(),
            Some(ShortcutCommand::Reload) => self.reload_with_notice(),
            _ => match key.code {
                KeyCode::Backspace => {
                    self.query.pop();
                    self.apply_filter(None);
                }
                KeyCode::Char(ch) if editable_modifiers(key.modifiers) => {
                    self.query.push(ch);
                    self.apply_filter(None);
                }
                _ => {}
            },
        }
    }

    fn handle_details_key(&mut self, key: KeyEvent) {
        match self.shortcuts.command(key) {
            Some(ShortcutCommand::Quit) => {
                self.should_quit = true;
            }
            Some(ShortcutCommand::Back) | Some(ShortcutCommand::NavLeft) => {
                if self.doc_tab_index > 0 {
                    self.doc_tab_index -= 1;
                    self.details_scroll = 0;
                } else {
                    self.screen = Screen::List;
                }
            }
            Some(ShortcutCommand::OpenActions) | Some(ShortcutCommand::Confirm) => {
                if self.selected_entry().is_some() {
                    self.screen = Screen::Actions;
                    self.action_selected = 0;
                    self.sync_action_state();
                }
            }
            Some(ShortcutCommand::NavUp) => self.scroll_details_up(1),
            Some(ShortcutCommand::NavDown) => self.scroll_details_down(1),
            Some(ShortcutCommand::NavRight) => {
                let tab_count = self.doc_tab_count();
                if tab_count > 0 && self.doc_tab_index < tab_count - 1 {
                    self.doc_tab_index += 1;
                    self.details_scroll = 0;
                }
            }
            Some(ShortcutCommand::CycleFilter) => {
                let tab_count = self.doc_tab_count();
                if tab_count > 0 {
                    self.doc_tab_index = (self.doc_tab_index + 1) % tab_count;
                    self.details_scroll = 0;
                }
            }
            Some(ShortcutCommand::CycleFilterBack) => {
                let tab_count = self.doc_tab_count();
                if tab_count > 0 {
                    self.doc_tab_index = if self.doc_tab_index == 0 {
                        tab_count - 1
                    } else {
                        self.doc_tab_index - 1
                    };
                    self.details_scroll = 0;
                }
            }
            Some(ShortcutCommand::Reload) => self.reload_with_notice(),
            _ => match key.code {
                KeyCode::PageUp => self.scroll_details_up(self.details_page_step()),
                KeyCode::PageDown => self.scroll_details_down(self.details_page_step()),
                KeyCode::Home => self.details_scroll = 0,
                KeyCode::End => self.details_scroll = usize::MAX,
                _ => {}
            },
        }
    }

    fn handle_action_key(&mut self, key: KeyEvent) {
        match self.shortcuts.command(key) {
            Some(ShortcutCommand::Quit) => {
                self.should_quit = true;
            }
            Some(ShortcutCommand::Back) => {
                self.screen = Screen::List;
            }
            Some(ShortcutCommand::Confirm) | Some(ShortcutCommand::OpenActions) => {
                self.execute_selected_action();
            }
            Some(ShortcutCommand::NavUp) => {
                self.action_selected = self.action_selected.saturating_sub(1);
                self.sync_action_state();
            }
            Some(ShortcutCommand::NavDown) => {
                let max = self.shortcuts.actions().len().saturating_sub(1);
                self.action_selected = (self.action_selected + 1).min(max);
                self.sync_action_state();
            }
            Some(ShortcutCommand::Action(action)) => self.execute_shortcut_action(action),
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
            DialogAction::SubmitCapture => self.capture_from_dialog()?,
            DialogAction::SubmitExistingBranch => self.open_existing_worktree_path_dialog(),
            DialogAction::SubmitExistingWorktreePath => {
                self.create_existing_worktree_from_dialog()?
            }
            DialogAction::SubmitExistingWorktreeLink => self.link_existing_branch_worktree()?,
            DialogAction::SubmitUnlink => self.unlink_selected_repo()?,
            DialogAction::SubmitDeleteWorktree => self.delete_selected_worktree()?,
            DialogAction::SubmitDone => self.done_confirmed()?,
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

    fn open_existing_branch_dialog(&mut self, journey_id: String) {
        match git::discover_repo(&self.cwd).and_then(|repo| git::list_branches(&repo.root)) {
            Ok(branches) => {
                let mut state = ListState::default();
                state.select(Some(0));
                self.dialog = Dialog::ExistingBranch {
                    journey_id,
                    branches,
                    query: String::new(),
                    selected: 0,
                    state,
                };
            }
            Err(err) => self.notice_error(format!("cannot list branches: {err}")),
        }
    }

    fn open_existing_worktree_path_dialog(&mut self) {
        let Dialog::ExistingBranch {
            journey_id,
            branches,
            query,
            selected,
            ..
        } = &self.dialog
        else {
            return;
        };
        let filtered = branch_filter_indices(branches, query);
        let Some(branch) = filtered
            .get(*selected)
            .and_then(|branch_idx| branches.get(*branch_idx))
            .cloned()
        else {
            self.notice_error("no branch selected");
            return;
        };

        match git::discover_repo(&self.cwd) {
            Ok(discovered) => {
                match git::worktree_for_branch(&discovered.root, &branch) {
                    Ok(Some(worktree)) => {
                        self.dialog = Dialog::ExistingWorktreeLinkConfirm {
                            journey_id: journey_id.clone(),
                            branch,
                            worktree,
                        };
                        return;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        self.dialog = Dialog::None;
                        self.notice_error(format!("cannot inspect worktrees: {err}"));
                        return;
                    }
                }

                let default_path = discovered
                    .root
                    .join(format!(".worktrees/{}", storage::slugify(&branch)));
                self.dialog = Dialog::ExistingWorktreePath {
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
        let result =
            create_and_link_worktree(&self.home, &self.cwd, &journey_id, &branch, &path, true);
        self.dialog = Dialog::None;
        self.record_mutation(result, Some(journey_id))
    }

    fn create_existing_worktree_from_dialog(&mut self) -> Result<()> {
        let Dialog::ExistingWorktreePath {
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
        match git::discover_repo(&self.cwd)
            .and_then(|discovered| git::worktree_for_branch(&discovered.root, &branch))
        {
            Ok(Some(worktree)) => {
                self.dialog = Dialog::ExistingWorktreeLinkConfirm {
                    journey_id,
                    branch,
                    worktree,
                };
                return Ok(());
            }
            Ok(None) => {}
            Err(err) => {
                self.dialog = Dialog::None;
                self.notice_error(format!("cannot inspect worktrees: {err}"));
                return Ok(());
            }
        }
        let result =
            create_and_link_worktree(&self.home, &self.cwd, &journey_id, &branch, &path, false);
        self.dialog = Dialog::None;
        self.record_mutation(result, Some(journey_id))
    }

    fn link_existing_branch_worktree(&mut self) -> Result<()> {
        let Dialog::ExistingWorktreeLinkConfirm {
            journey_id,
            worktree,
            ..
        } = &self.dialog
        else {
            return Ok(());
        };
        let journey_id = journey_id.clone();
        let worktree = worktree.clone();
        let result = app::link_repo(&self.home, &self.cwd, Some(&journey_id), &worktree, None);
        self.dialog = Dialog::None;
        self.record_mutation(result, Some(journey_id))
    }

    fn execute_selected_action(&mut self) {
        let Some(action) = self.shortcuts.actions().get(self.action_selected).copied() else {
            self.notice_error("no actions configured");
            return;
        };
        self.execute_action(action);
    }

    fn execute_shortcut_action(&mut self, action: TuiAction) {
        self.execute_action(action);
    }

    fn execute_action(&mut self, action: TuiAction) {
        let Some(journey_id) = self.selected_id() else {
            self.notice_error("no Journey selected");
            return;
        };

        match action {
            TuiAction::CopyCd => {
                let dir = storage::journey_dir(&self.home, &journey_id);
                let command = cd_command(&dir);
                match copy_to_clipboard(&command) {
                    Ok(()) => self.notice_success("copied cd command to clipboard"),
                    Err(err) => self.notice_error(format!("clipboard failed: {err}")),
                }
            }
            TuiAction::OpenNvim => {
                self.output = Some(AppOutput::RunInJourney {
                    command: "nvim",
                    journey_path: storage::journey_dir(&self.home, &journey_id),
                });
                self.should_quit = true;
            }
            TuiAction::OpenClaude => {
                self.output = Some(AppOutput::RunInJourney {
                    command: "claude",
                    journey_path: storage::journey_dir(&self.home, &journey_id),
                });
                self.should_quit = true;
            }
            TuiAction::Resume => {
                let result = app::resume(&self.home, &self.cwd, Some(&journey_id));
                let _ = self.record_mutation(result, Some(journey_id));
            }
            TuiAction::NewBranchWorktree => {
                let default_branch = storage::slugify(&journey_id);
                self.dialog = Dialog::WorktreeBranch {
                    journey_id,
                    input: String::new(),
                    default_branch,
                };
            }
            TuiAction::ExistingBranchWorktree => {
                self.open_existing_branch_dialog(journey_id);
            }
            TuiAction::LinkCurrent => {
                let result =
                    app::link_repo(&self.home, &self.cwd, Some(&journey_id), &self.cwd, None);
                let _ = self.record_mutation(result, Some(journey_id));
            }
            TuiAction::UnlinkRepo => self.open_unlink_dialog(&journey_id),
            TuiAction::Capture => {
                self.dialog = Dialog::Capture {
                    journey_id,
                    input: String::new(),
                };
            }
            TuiAction::DeleteWorktree => self.open_delete_worktree_dialog(&journey_id),
            TuiAction::Done => self.open_done_dialog(&journey_id),
            TuiAction::Pause => {
                let result = app::set_status(
                    &self.home,
                    &self.cwd,
                    Some(&journey_id),
                    JourneyStatus::Paused,
                );
                let _ = self.record_mutation(result, Some(journey_id));
            }
            TuiAction::Archive => {
                let result = app::set_status(
                    &self.home,
                    &self.cwd,
                    Some(&journey_id),
                    JourneyStatus::Archived,
                );
                let _ = self.record_mutation(result, Some(journey_id));
            }
            TuiAction::Abandon => {
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

    fn open_delete_worktree_dialog(&mut self, journey_id: &str) {
        match storage::resolve_context(&self.home, Some(journey_id), &self.cwd) {
            Ok(ctx) if ctx.journey.repos.is_empty() => self.notice_error("no worktrees linked"),
            Ok(ctx) => {
                let mut state = ListState::default();
                state.select(Some(0));
                self.dialog = Dialog::DeleteWorktree {
                    journey_id: journey_id.to_string(),
                    repos: ctx.journey.repos,
                    selected: 0,
                    state,
                };
            }
            Err(err) => self.notice_error(err.to_string()),
        }
    }

    fn delete_selected_worktree(&mut self) -> Result<()> {
        let Dialog::DeleteWorktree {
            journey_id,
            repos,
            selected,
            ..
        } = &self.dialog
        else {
            return Ok(());
        };
        let Some(repo) = repos.get(*selected) else {
            self.notice_error("no worktree selected");
            return Ok(());
        };
        let journey_id = journey_id.clone();
        let repo_name = repo.name.clone();
        let result = app::delete_worktree(&self.home, &self.cwd, Some(&journey_id), &repo_name);
        self.dialog = Dialog::None;
        self.record_mutation(result, Some(journey_id))
    }

    fn capture_from_dialog(&mut self) -> Result<()> {
        let Dialog::Capture { journey_id, input } = &self.dialog else {
            return Ok(());
        };
        let journey_id = journey_id.clone();
        let text = input.trim().to_string();
        if text.is_empty() {
            self.notice_error("capture text is required");
            return Ok(());
        }

        let result = app::capture(
            &self.home,
            &self.cwd,
            Some(&journey_id),
            app::DEFAULT_CAPTURE_DOC,
            &text,
        );
        self.dialog = Dialog::None;
        self.record_mutation(result, Some(journey_id))
    }

    fn open_done_dialog(&mut self, journey_id: &str) {
        match storage::resolve_context(&self.home, Some(journey_id), &self.cwd) {
            Ok(ctx) => {
                self.dialog = Dialog::DoneConfirm {
                    journey_id: journey_id.to_string(),
                    repo_count: ctx.journey.repos.len(),
                };
            }
            Err(err) => self.notice_error(err.to_string()),
        }
    }

    fn done_confirmed(&mut self) -> Result<()> {
        let Dialog::DoneConfirm { journey_id, .. } = &self.dialog else {
            return Ok(());
        };
        let journey_id = journey_id.clone();
        let result = app::done(&self.home, &self.cwd, Some(&journey_id));
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
                self.screen = Screen::List;
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
        self.reset_details_scroll();
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
        let previous = self.selected;
        let len = self.filtered.len() as isize;
        let next = (self.selected as isize + delta).clamp(0, len - 1);
        self.selected = next as usize;
        self.sync_table_state();
        if self.selected != previous {
            self.reset_details_scroll();
        }
    }

    fn cycle_filter(&mut self, reverse: bool) {
        self.filter = self.filter.cycle(reverse);
        self.apply_filter(None);
    }

    fn reset_details_scroll(&mut self) {
        self.details_scroll = 0;
        self.doc_tab_index = 0;
    }

    fn doc_tab_count(&self) -> usize {
        let Some(entry) = self.selected_entry() else {
            return 0;
        };
        let journey_path = storage::journey_dir(&self.home, &entry.id);
        discover_doc_tabs(&journey_path).len()
    }

    fn details_page_step(&self) -> usize {
        self.details_viewport_height.saturating_sub(1).max(1)
    }

    fn scroll_details_up(&mut self, amount: usize) {
        self.details_scroll = self.details_scroll.saturating_sub(amount);
    }

    fn scroll_details_down(&mut self, amount: usize) {
        self.details_scroll = self.details_scroll.saturating_add(amount);
    }

    fn sync_table_state(&mut self) {
        if self.filtered.is_empty() {
            self.table_state.select(None);
        } else {
            self.table_state.select(Some(self.selected));
        }
    }

    fn sync_action_state(&mut self) {
        if self.shortcuts.actions().is_empty() {
            self.action_state.select(None);
            return;
        }

        self.action_selected = self
            .action_selected
            .min(self.shortcuts.actions().len().saturating_sub(1));
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    List,
    Search,
    Details,
    Actions,
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

struct Notice {
    message: String,
    is_error: bool,
}
