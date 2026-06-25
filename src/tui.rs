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
use crate::config::{InsertShortcut, NormalShortcut, ShortcutAction, ShortcutConfig};
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

const ACTIONS: [JourneyAction; 14] = [
    JourneyAction::OpenClaude,
    JourneyAction::OpenNvim,
    JourneyAction::Worktree,
    JourneyAction::ExistingBranchWorktree,
    JourneyAction::Link,
    JourneyAction::Unlink,
    JourneyAction::Capture,
    JourneyAction::DeleteWorktree,
    JourneyAction::Done,
    JourneyAction::Pause,
    JourneyAction::Archive,
    JourneyAction::CopyCd,
    JourneyAction::Resume,
    JourneyAction::Abandon,
];

const ACCENT: Color = Color::Indexed(75);
const ACCENT_TEXT: Color = Color::Black;
const CODE_THEME: &str = "base16-ocean.dark";

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

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
    focus: Focus,
    input_mode: InputMode,
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
            focus: Focus::Journeys,
            input_mode: InputMode::Normal,
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

    fn handle_main_key(&mut self, key: KeyEvent) {
        if self.handle_shortcut(key) {
            return;
        }

        match self.focus {
            Focus::Journeys => self.handle_journey_key(key),
            Focus::Details => self.handle_details_key(key),
            Focus::Actions => self.handle_action_key(key),
        }
    }

    fn handle_shortcut(&mut self, key: KeyEvent) -> bool {
        match self.input_mode {
            InputMode::Normal => {
                if key.code == KeyCode::Esc && key.modifiers.is_empty() {
                    self.should_quit = true;
                    return true;
                }

                match self.shortcuts.normal_command(key) {
                    Some(NormalShortcut::NewJourney) => {
                        self.open_new_journey_dialog();
                        true
                    }
                    Some(NormalShortcut::SwitchToInsert) => {
                        self.input_mode = InputMode::Insert;
                        self.focus = Focus::Journeys;
                        true
                    }
                    Some(NormalShortcut::Action(action)) => {
                        self.execute_shortcut_action(action);
                        true
                    }
                    None => false,
                }
            }
            InputMode::Insert => match self.shortcuts.insert_command(key) {
                Some(InsertShortcut::NewJourney) => {
                    self.open_new_journey_dialog();
                    true
                }
                Some(InsertShortcut::SwitchToNormal) => {
                    self.input_mode = InputMode::Normal;
                    self.focus = Focus::Journeys;
                    true
                }
                None => false,
            },
        }
    }

    fn open_new_journey_dialog(&mut self) {
        self.dialog = Dialog::NewTitle {
            input: String::new(),
        };
    }

    fn handle_journey_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                if self.input_mode == InputMode::Normal && !self.query.is_empty() {
                    self.query.clear();
                    self.apply_filter(None);
                }
            }
            KeyCode::Char('q')
                if self.input_mode == InputMode::Normal && key.modifiers.is_empty() =>
            {
                self.should_quit = true;
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.reload_with_notice();
            }
            KeyCode::Tab => self.cycle_filter(false),
            KeyCode::BackTab => self.cycle_filter(true),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Right => {
                if self.selected_entry().is_some() {
                    self.focus = Focus::Details;
                }
            }
            KeyCode::Enter => {
                if self.selected_entry().is_some() {
                    self.focus = Focus::Actions;
                    self.action_selected = 0;
                    self.sync_action_state();
                }
            }
            KeyCode::Backspace if self.input_mode == InputMode::Insert => {
                self.query.pop();
                self.apply_filter(None);
            }
            KeyCode::Char(ch)
                if self.input_mode == InputMode::Insert && editable_modifiers(key.modifiers) =>
            {
                self.query.push(ch);
                self.apply_filter(None);
            }
            _ => {}
        }
    }

    fn handle_details_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.focus = Focus::Journeys,
            KeyCode::Left => {
                if self.doc_tab_index > 0 {
                    self.doc_tab_index -= 1;
                    self.details_scroll = 0;
                } else {
                    self.focus = Focus::Journeys;
                }
            }
            KeyCode::Right => {
                let tab_count = self.doc_tab_count();
                if tab_count > 0 && self.doc_tab_index < tab_count - 1 {
                    self.doc_tab_index += 1;
                    self.details_scroll = 0;
                }
            }
            KeyCode::Tab => {
                let tab_count = self.doc_tab_count();
                if tab_count > 0 {
                    self.doc_tab_index = (self.doc_tab_index + 1) % tab_count;
                    self.details_scroll = 0;
                }
            }
            KeyCode::BackTab => {
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
            KeyCode::Char('q') if key.modifiers.is_empty() => self.focus = Focus::Journeys,
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.reload_with_notice();
            }
            KeyCode::Up => self.scroll_details_up(1),
            KeyCode::Down => self.scroll_details_down(1),
            KeyCode::PageUp => self.scroll_details_up(self.details_page_step()),
            KeyCode::PageDown => self.scroll_details_down(self.details_page_step()),
            KeyCode::Home => self.details_scroll = 0,
            KeyCode::End => self.details_scroll = usize::MAX,
            KeyCode::Enter => {
                if self.selected_entry().is_some() {
                    self.focus = Focus::Actions;
                    self.action_selected = 0;
                    self.sync_action_state();
                }
            }
            _ => {}
        }
    }

    fn handle_action_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.focus = Focus::Journeys,
            KeyCode::Char('q') if key.modifiers.is_empty() => self.focus = Focus::Journeys,
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
        self.execute_action(ACTIONS[self.action_selected]);
    }

    fn execute_shortcut_action(&mut self, action: ShortcutAction) {
        self.execute_action(match action {
            ShortcutAction::OpenClaude => JourneyAction::OpenClaude,
            ShortcutAction::OpenNvim => JourneyAction::OpenNvim,
            ShortcutAction::NewBranchWorktree => JourneyAction::Worktree,
            ShortcutAction::ExistingBranchWorktree => JourneyAction::ExistingBranchWorktree,
            ShortcutAction::LinkCurrent => JourneyAction::Link,
            ShortcutAction::UnlinkRepo => JourneyAction::Unlink,
            ShortcutAction::Capture => JourneyAction::Capture,
            ShortcutAction::DeleteWorktree => JourneyAction::DeleteWorktree,
            ShortcutAction::Done => JourneyAction::Done,
            ShortcutAction::Pause => JourneyAction::Pause,
            ShortcutAction::Archive => JourneyAction::Archive,
        });
    }

    fn execute_action(&mut self, action: JourneyAction) {
        let Some(journey_id) = self.selected_id() else {
            self.notice_error("no Journey selected");
            return;
        };

        match action {
            JourneyAction::CopyCd => {
                let dir = storage::journey_dir(&self.home, &journey_id);
                let command = cd_command(&dir);
                match copy_to_clipboard(&command) {
                    Ok(()) => self.notice_success("copied cd command to clipboard"),
                    Err(err) => self.notice_error(format!("clipboard failed: {err}")),
                }
            }
            JourneyAction::OpenNvim => {
                self.output = Some(AppOutput::RunInJourney {
                    command: "nvim",
                    journey_path: storage::journey_dir(&self.home, &journey_id),
                });
                self.should_quit = true;
            }
            JourneyAction::OpenClaude => {
                self.output = Some(AppOutput::RunInJourney {
                    command: "claude",
                    journey_path: storage::journey_dir(&self.home, &journey_id),
                });
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
            JourneyAction::ExistingBranchWorktree => {
                self.open_existing_branch_dialog(journey_id);
            }
            JourneyAction::Link => {
                let result =
                    app::link_repo(&self.home, &self.cwd, Some(&journey_id), &self.cwd, None);
                let _ = self.record_mutation(result, Some(journey_id));
            }
            JourneyAction::Unlink => self.open_unlink_dialog(&journey_id),
            JourneyAction::Capture => {
                self.dialog = Dialog::Capture {
                    journey_id,
                    input: String::new(),
                };
            }
            JourneyAction::DeleteWorktree => self.open_delete_worktree_dialog(&journey_id),
            JourneyAction::Done => self.open_done_dialog(&journey_id),
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
            Span::styled("mode: ", dim()),
            Span::styled(app.input_mode.label(), app.input_mode.style()),
            Span::raw("  "),
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
        Focus::Details => " Journeys ",
        Focus::Actions => " Journeys ",
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(match app.focus {
            Focus::Journeys => accent(),
            Focus::Details | Focus::Actions => Style::default().fg(Color::DarkGray),
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
        Focus::Journeys => render_details_pane(frame, app, area, false),
        Focus::Details => render_details_pane(frame, app, area, true),
        Focus::Actions => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
                .split(area);
            render_details_pane(frame, app, chunks[0], false);
            render_actions(frame, app, chunks[1]);
        }
    }
}

fn render_details_pane(frame: &mut Frame<'_>, app: &mut JourneyApp, area: Rect, focused: bool) {
    let block = Block::default()
        .title(" Details ")
        .borders(Borders::ALL)
        .border_style(if focused { accent() } else { Style::default() });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let meta = metadata_lines(app);

    let journey_path = app
        .selected_entry()
        .map(|e| storage::journey_dir(&app.home, &e.id));
    let tabs = journey_path
        .as_deref()
        .map(discover_doc_tabs)
        .unwrap_or_default();

    if tabs.is_empty() {
        let mut content_area = inner;
        let mut content_height = wrapped_line_count(&meta, content_area.width);
        let viewport_height = usize::from(content_area.height);
        let is_scrollable = content_height > viewport_height;
        if is_scrollable && content_area.width > 1 {
            content_area.width = content_area.width.saturating_sub(1);
            content_height = wrapped_line_count(&meta, content_area.width);
        }
        app.details_viewport_height = usize::from(content_area.height).max(1);
        let max_scroll = content_height.saturating_sub(app.details_viewport_height);
        app.details_scroll = app.details_scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(Text::from(meta))
                .wrap(Wrap { trim: false })
                .scroll((scroll_offset(app.details_scroll), 0)),
            content_area,
        );
        render_details_scrollbar(
            frame,
            inner,
            app.details_scroll,
            app.details_viewport_height,
            max_scroll,
            focused,
        );
        return;
    }

    let meta_height = wrapped_line_count(&meta, inner.width) as u16;
    // ponytail: +1 blank line between metadata and tab bar, +1 separator below tab bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(meta_height + 1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Text::from(meta)).wrap(Wrap { trim: false }),
        chunks[0],
    );

    let tab_line = tab_bar_line(&tabs, app.doc_tab_index, usize::from(chunks[1].width));
    frame.render_widget(Paragraph::new(Text::from(vec![tab_line])), chunks[1]);

    let sep = "\u{2500}".repeat(usize::from(chunks[2].width));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(sep, dim()))),
        chunks[2],
    );

    let doc_lines = doc_content_lines(app, &tabs);
    let mut doc_area = chunks[3];
    let mut content_height = wrapped_line_count(&doc_lines, doc_area.width);
    let viewport_height = usize::from(doc_area.height);
    let is_scrollable = content_height > viewport_height;
    if is_scrollable && doc_area.width > 1 {
        doc_area.width = doc_area.width.saturating_sub(1);
        content_height = wrapped_line_count(&doc_lines, doc_area.width);
    }

    app.details_viewport_height = usize::from(doc_area.height).max(1);
    let max_scroll = content_height.saturating_sub(app.details_viewport_height);
    app.details_scroll = app.details_scroll.min(max_scroll);

    frame.render_widget(
        Paragraph::new(Text::from(doc_lines))
            .wrap(Wrap { trim: false })
            .scroll((scroll_offset(app.details_scroll), 0)),
        doc_area,
    );

    render_details_scrollbar(
        frame,
        chunks[3],
        app.details_scroll,
        app.details_viewport_height,
        max_scroll,
        focused,
    );
}

fn render_details_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    scroll: usize,
    viewport_height: usize,
    max_scroll: usize,
    focused: bool,
) {
    if max_scroll > 0 && area.width > 0 && area.height > 0 {
        let scrollbar_area = Rect {
            x: area.x + area.width.saturating_sub(1),
            y: area.y,
            width: 1,
            height: area.height,
        };
        let mut scrollbar_state = ScrollbarState::new(details_scrollbar_content_length(max_scroll))
            .position(scroll)
            .viewport_content_length(viewport_height);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(None)
                .thumb_style(if focused { accent() } else { dim() }),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }
}

fn wrapped_line_count(lines: &[Line<'_>], width: u16) -> usize {
    let width = usize::from(width.max(1));
    lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum()
}

fn scroll_offset(offset: usize) -> u16 {
    offset.min(usize::from(u16::MAX)) as u16
}

fn details_scrollbar_content_length(max_scroll: usize) -> usize {
    max_scroll.saturating_add(1)
}

fn render_actions(frame: &mut Frame<'_>, app: &mut JourneyApp, area: Rect) {
    let items = ACTIONS
        .iter()
        .map(|action| {
            let mut spans = Vec::new();
            if let Some(shortcut) = action.shortcut_action() {
                spans.push(Span::styled(
                    format!(
                        "[{}] ",
                        app.shortcuts.binding_for_action(shortcut).display()
                    ),
                    dim(),
                ));
            }
            spans.extend([
                Span::styled(action.label(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(action.description(), dim()),
            ]);
            ListItem::new(Line::from(spans))
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
    let help = match app.input_mode {
        InputMode::Insert => format!(
            "INSERT  type search  {} normal  {} new  Tab filter  Up/Down select",
            app.shortcuts.normal_mode.display(),
            app.shortcuts.new_journey.display()
        ),
        InputMode::Normal => match app.focus {
            Focus::Journeys => normal_help(app),
            Focus::Details => {
                "NORMAL  Tab/S-Tab switch doc  Up/Down scroll  Esc back  Enter actions".to_string()
            }
            Focus::Actions => {
                format!(
                    "NORMAL  Enter run  Esc back  {} new  shortcuts active",
                    app.shortcuts.new_journey.display()
                )
            }
        },
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

fn normal_help(app: &JourneyApp) -> String {
    format!(
        "NORMAL  {} search  {} new  {} claude  {} nvim  {}/{} worktree  {}/{} link  {} capture  {} delete  {} done  {} pause  {} archive  Esc/q quit",
        app.shortcuts.insert_mode.display(),
        app.shortcuts.new_journey.display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::OpenClaude)
            .display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::OpenNvim)
            .display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::NewBranchWorktree)
            .display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::ExistingBranchWorktree)
            .display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::LinkCurrent)
            .display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::UnlinkRepo)
            .display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::Capture)
            .display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::DeleteWorktree)
            .display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::Done)
            .display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::Pause)
            .display(),
        app.shortcuts
            .binding_for_action(ShortcutAction::Archive)
            .display()
    )
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
        Dialog::ExistingBranch {
            branches,
            query,
            selected,
            state,
            ..
        } => render_branch_selection_dialog(frame, area, branches, query, *selected, state),
        Dialog::ExistingWorktreePath {
            input,
            default_path,
            ..
        } => render_input_dialog(
            frame,
            area,
            "Existing Branch + Worktree",
            &format!("Path (default: {})", default_path.display()),
            input,
            "Enter create  Esc cancel",
        ),
        Dialog::ExistingWorktreeLinkConfirm {
            branch, worktree, ..
        } => render_confirm_dialog(
            frame,
            area,
            " Link Existing Worktree ",
            vec![
                Line::from(Span::styled(
                    "A worktree for this branch already exists at:",
                    dim(),
                )),
                Line::from(Span::styled(worktree.display().to_string(), accent())),
                Line::from(""),
                Line::from(vec![
                    Span::raw("Branch: "),
                    Span::styled(branch.clone(), Style::default().fg(Color::White)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Would you like to link this worktree instead of creating a new one?",
                    dim(),
                )),
                Line::from(Span::styled("Enter/y link  n/Esc cancel", dim())),
            ],
        ),
        Dialog::Unlink { repos, state, .. } => {
            render_repo_selection_dialog(frame, area, " Unlink Repo ", repos, state);
        }
        Dialog::DeleteWorktree { repos, state, .. } => {
            render_repo_selection_dialog(frame, area, " Delete Worktree ", repos, state);
        }
        Dialog::Capture { input, .. } => render_input_dialog(
            frame,
            area,
            "Capture Thought",
            "Text",
            input,
            "Enter capture  Esc cancel",
        ),
        Dialog::DoneConfirm {
            journey_id,
            repo_count,
        } => render_confirm_dialog(
            frame,
            area,
            " Done ",
            vec![
                Line::from(vec![
                    Span::raw("Archive "),
                    Span::styled(journey_id.clone(), accent()),
                    Span::raw(format!(" and remove {repo_count} worktrees?")),
                ]),
                Line::from(Span::styled(
                    "Git will refuse dirty or main worktrees.",
                    dim(),
                )),
                Line::from(""),
                Line::from(Span::styled("Enter confirm  Esc cancel", dim())),
            ],
        ),
    }
}

fn render_branch_selection_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    branches: &[String],
    query: &str,
    selected_index: usize,
    state: &mut ListState,
) {
    let area = centered_rect(74, 72, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Existing Branch + Worktree ")
        .borders(Borders::ALL)
        .border_style(accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(inner);

    let query_line = if query.is_empty() {
        Line::from(" ")
    } else {
        Line::from(Span::styled(
            query.to_string(),
            Style::default().fg(Color::White),
        ))
    };
    frame.render_widget(
        Paragraph::new(query_line).block(
            Block::default()
                .title(" Filter ")
                .borders(Borders::ALL)
                .border_style(dim()),
        ),
        chunks[0],
    );

    let filtered = branch_filter_indices(branches, query);
    let items = if filtered.is_empty() {
        state.select(None);
        vec![ListItem::new(Line::from(Span::styled(
            "No matching branches",
            dim(),
        )))]
    } else {
        sync_list_state(state, selected_index, filtered.len());
        filtered
            .iter()
            .map(|branch_idx| {
                let branch = &branches[*branch_idx];
                ListItem::new(Line::from(Span::styled(
                    branch.clone(),
                    Style::default().fg(Color::White),
                )))
            })
            .collect::<Vec<_>>()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" Branches {}/{} ", filtered.len(), branches.len()))
                .borders(Borders::ALL)
                .border_style(dim()),
        )
        .highlight_style(selected())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], state);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Enter select  Backspace edit  Esc cancel",
            dim(),
        ))),
        chunks[2],
    );
}

fn branch_filter_indices(branches: &[String], query: &str) -> Vec<usize> {
    let terms = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return (0..branches.len()).collect();
    }

    branches
        .iter()
        .enumerate()
        .filter_map(|(idx, branch)| {
            let branch = branch.to_lowercase();
            terms
                .iter()
                .all(|term| branch.contains(term))
                .then_some(idx)
        })
        .collect()
}

fn render_repo_selection_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    repos: &[RepoRef],
    state: &mut ListState,
) {
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
                .title(title)
                .borders(Borders::ALL)
                .border_style(accent()),
        )
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Red))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, state);
}

fn render_confirm_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
) {
    let area = centered_rect(64, 32, area);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
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

fn metadata_lines(app: &JourneyApp) -> Vec<Line<'static>> {
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
    lines
}

fn tab_bar_line(tabs: &[DocTab], selected: usize, width: usize) -> Line<'static> {
    let sep = " \u{2502} ";
    // Build label positions so we can scroll to keep the selected tab visible
    let mut positions: Vec<(usize, usize)> = Vec::new(); // (start, end) for each tab
    let mut pos = 1usize; // leading space
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            pos += sep.chars().count();
        }
        let start = pos;
        pos += tab.label.len();
        positions.push((start, pos));
    }
    let total_width = pos + 1; // trailing space

    // Determine horizontal scroll offset to keep selected tab visible
    let scroll = if total_width <= width {
        0
    } else if let Some(&(_start, end)) = positions.get(selected) {
        // Ensure the selected tab fits in the viewport
        if end > width {
            // Scroll right so the selected tab's end is at the right edge
            (end.saturating_sub(width)).min(total_width.saturating_sub(width))
        } else {
            0
        }
    } else {
        0
    };

    let mut spans = Vec::new();
    let mut col = 0usize;

    let push_visible =
        |text: String, style: Style, col: &mut usize, spans: &mut Vec<Span<'static>>| {
            let len = text.len();
            let start = *col;
            let end = start + len;
            *col = end;

            if end <= scroll || start >= scroll + width {
                return; // entirely outside viewport
            }

            let visible_start = scroll.saturating_sub(start);
            let visible_end = if end > scroll + width {
                scroll + width - start
            } else {
                len
            };
            let visible: String = text
                .chars()
                .skip(visible_start)
                .take(visible_end - visible_start)
                .collect();
            if !visible.is_empty() {
                spans.push(Span::styled(visible, style));
            }
        };

    push_visible(" ".to_string(), Style::default(), &mut col, &mut spans);
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            push_visible(sep.to_string(), dim(), &mut col, &mut spans);
        }
        let style = if i == selected {
            accent().add_modifier(Modifier::BOLD)
        } else {
            dim()
        };
        push_visible(tab.label.clone(), style, &mut col, &mut spans);
    }
    push_visible(" ".to_string(), Style::default(), &mut col, &mut spans);

    // Show scroll indicators
    if scroll > 0 {
        spans.insert(0, Span::styled("\u{25c2}", dim()));
    }
    if scroll + width < total_width {
        spans.push(Span::styled("\u{25b8}", dim()));
    }

    Line::from(spans)
}

fn doc_content_lines(app: &mut JourneyApp, tabs: &[DocTab]) -> Vec<Line<'static>> {
    if tabs.is_empty() {
        return Vec::new();
    }
    let idx = app.doc_tab_index.min(tabs.len().saturating_sub(1));
    app.doc_render_cache.lines(&tabs[idx].path)
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

struct DocTab {
    label: String,
    path: PathBuf,
}

fn discover_doc_tabs(journey_path: &Path) -> Vec<DocTab> {
    let mut tabs = Vec::new();

    let readme_path = journey_path.join(storage::README_FILE);
    if readme_path.is_file() {
        tabs.push(DocTab {
            label: "README".to_string(),
            path: readme_path,
        });
    }

    let docs_dir = journey_path.join(storage::DOCS_DIR);
    if let Ok(entries) = fs::read_dir(&docs_dir) {
        let mut doc_files: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .collect();
        doc_files.sort_by_key(|e| e.file_name());

        for entry in doc_files {
            let path = entry.path();
            let label = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            tabs.push(DocTab { label, path });
        }
    }

    tabs
}

#[derive(Clone, PartialEq, Eq)]
struct DocCacheKey {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
}

#[derive(Default)]
struct DocRenderCache {
    key: Option<DocCacheKey>,
    lines: Vec<Line<'static>>,
}

impl DocRenderCache {
    fn lines(&mut self, path: &Path) -> Vec<Line<'static>> {
        let Ok(metadata) = fs::metadata(path) else {
            return vec![Line::from(Span::styled("unavailable", dim()))];
        };

        let key = DocCacheKey {
            path: path.to_path_buf(),
            modified: metadata.modified().ok(),
            len: metadata.len(),
        };

        if self.key.as_ref() != Some(&key) {
            self.lines = render_doc_file(path);
            self.key = Some(key);
        }

        self.lines.clone()
    }
}

fn render_doc_file(path: &Path) -> Vec<Line<'static>> {
    match fs::read_to_string(path) {
        Ok(content) if content.trim().is_empty() => {
            vec![Line::from(Span::styled("(empty)", dim()))]
        }
        Ok(content) => render_markdown_lines(&content),
        Err(err) => vec![Line::from(Span::styled(
            format!("unavailable: {err}"),
            dim(),
        ))],
    }
}

fn render_markdown_lines(content: &str) -> Vec<Line<'static>> {
    let mut renderer = MarkdownRenderer::default();
    let parser = Parser::new_ext(
        content,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES,
    );

    for (event, range) in parser.into_offset_iter() {
        renderer.preserve_source_gap(content, range.start);
        renderer.handle(event);
        renderer.mark_source_end(range.end);
    }

    renderer.preserve_source_gap(content, content.len());
    renderer.finish()
}

fn source_blank_lines(gap: &str) -> usize {
    if !gap.trim().is_empty() {
        return 0;
    }

    gap.bytes().filter(|byte| *byte == b'\n').count()
}

#[derive(Default)]
struct MarkdownRenderer {
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    list_stack: Vec<ListMarker>,
    current_item_prefix: Option<String>,
    blockquote_depth: usize,
    code_block: Option<MarkdownCodeBlock>,
    pending_link_destinations: Vec<String>,
    source_offset: usize,
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

    fn preserve_source_gap(&mut self, source: &str, next_start: usize) {
        if self.code_block.is_some() || next_start <= self.source_offset {
            return;
        }

        let gap = &source[self.source_offset..next_start];
        let blank_lines = source_blank_lines(gap);
        if blank_lines == 0 {
            return;
        }

        self.flush_current();
        for _ in 0..blank_lines {
            self.lines.push(Line::from(""));
        }
    }

    fn mark_source_end(&mut self, end: usize) {
        self.source_offset = self.source_offset.max(end);
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
                self.code_block = Some(MarkdownCodeBlock::new(kind));
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
                if let Some(block) = self.code_block.take() {
                    self.push_code_block(block);
                }
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
        if let Some(block) = &mut self.code_block {
            block.content.push_str(text);
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
        self.current.extend(self.prefix_spans());
    }

    fn prefix_spans(&self) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        for _ in 0..self.blockquote_depth {
            spans.push(Span::styled("> ", dim()));
        }
        if let Some(prefix) = &self.current_item_prefix {
            spans.push(Span::styled(prefix.clone(), dim()));
        }
        spans
    }

    fn push_code_block(&mut self, block: MarkdownCodeBlock) {
        if let Some(language) = block.language.as_deref() {
            self.push_line_with_prefix(vec![
                Span::styled("code", dim()),
                Span::styled(format!(" {language}"), dim()),
            ]);
        }

        for spans in highlight_code_lines(&block.content, block.language.as_deref()) {
            self.push_line_with_prefix(spans);
        }
    }

    fn push_line_with_prefix(&mut self, spans: Vec<Span<'static>>) {
        let mut line = self.prefix_spans();
        line.extend(spans);
        self.lines.push(Line::from(line));
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

struct MarkdownCodeBlock {
    language: Option<String>,
    content: String,
}

impl MarkdownCodeBlock {
    fn new(kind: CodeBlockKind<'_>) -> Self {
        let language = match kind {
            CodeBlockKind::Fenced(info) => parse_code_language(info.as_ref()),
            CodeBlockKind::Indented => None,
        };
        Self {
            language,
            content: String::new(),
        }
    }
}

fn parse_code_language(info: &str) -> Option<String> {
    let token = info
        .split_whitespace()
        .next()?
        .trim_matches(|ch| ch == '{' || ch == '}' || ch == '.');
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn highlight_code_lines(code: &str, language: Option<&str>) -> Vec<Vec<Span<'static>>> {
    let Some(language) = language else {
        return plain_code_lines(code);
    };
    let Some(syntax) = syntax_for_language(language) else {
        return plain_code_lines(code);
    };
    let Some(theme) = code_theme() else {
        return plain_code_lines(code);
    };

    let mut highlighter = HighlightLines::new(syntax, theme);
    code.lines()
        .map(|line| {
            let mut spans = vec![Span::styled("    ", code_block_style())];
            match highlighter.highlight_line(line, &SYNTAX_SET) {
                Ok(ranges) => {
                    for (style, text) in ranges {
                        if !text.is_empty() {
                            spans.push(Span::styled(text.to_string(), syntax_style(style)));
                        }
                    }
                }
                Err(_) => spans.push(Span::styled(line.to_string(), code_block_style())),
            }
            spans
        })
        .collect()
}

fn plain_code_lines(code: &str) -> Vec<Vec<Span<'static>>> {
    code.lines()
        .map(|line| vec![Span::styled(format!("    {line}"), code_block_style())])
        .collect()
}

fn syntax_for_language(language: &str) -> Option<&'static SyntaxReference> {
    let syntax_set = &*SYNTAX_SET;
    for candidate in language_candidates(language) {
        if let Some(syntax) = syntax_set
            .find_syntax_by_token(&candidate)
            .or_else(|| syntax_set.find_syntax_by_extension(&candidate))
        {
            return Some(syntax);
        }
    }
    None
}

fn language_candidates(language: &str) -> Vec<String> {
    let token = language.trim().trim_start_matches('.').to_ascii_lowercase();
    let alias = match token.as_str() {
        "rs" => Some("rust"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"),
        "sh" | "zsh" | "shell" => Some("bash"),
        "yml" => Some("yaml"),
        "md" => Some("markdown"),
        "rb" => Some("ruby"),
        "py" => Some("python"),
        _ => None,
    };

    let mut candidates = vec![token];
    if let Some(alias) = alias {
        candidates.push(alias.to_string());
    }
    candidates
}

fn code_theme() -> Option<&'static Theme> {
    THEME_SET
        .themes
        .get(CODE_THEME)
        .or_else(|| THEME_SET.themes.values().next())
}

fn syntax_style(style: SyntectStyle) -> Style {
    let mut result = Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));
    if style.font_style.contains(SyntectFontStyle::BOLD) {
        result = result.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(SyntectFontStyle::ITALIC) {
        result = result.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(SyntectFontStyle::UNDERLINE) {
        result = result.add_modifier(Modifier::UNDERLINED);
    }
    result
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
    create_branch: bool,
) -> Result<String> {
    let discovered = git::discover_repo(cwd)?;
    git::create_worktree(&discovered.root, path, branch, create_branch)?;
    let repo_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "worktree".to_string());
    let linked = app::link_repo(home, cwd, Some(journey_id), path, Some(repo_name))?;
    let action = if create_branch {
        "created worktree"
    } else {
        "checked out worktree"
    };
    Ok(format!(
        "{action} {} on branch `{}`\n{}",
        path.display(),
        branch,
        linked
    ))
}

fn cd_command(path: &Path) -> String {
    format!("cd -- {}", shell_quote(path))
}

fn shell_quote(path: &Path) -> String {
    let value = path.display().to_string();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn copy_to_clipboard(text: &str) -> Result<()> {
    let mut last_error = None;
    for mut command in clipboard_commands() {
        match run_clipboard_command(&mut command, text) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no clipboard command is available")))
}

fn clipboard_commands() -> Vec<Command> {
    let mut commands = Vec::new();

    #[cfg(target_os = "macos")]
    {
        commands.push(Command::new("pbcopy"));
    }

    #[cfg(target_os = "windows")]
    {
        commands.push(Command::new("clip"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        commands.push(Command::new("wl-copy"));

        let mut xclip = Command::new("xclip");
        xclip.args(["-selection", "clipboard"]);
        commands.push(xclip);

        let mut xsel = Command::new("xsel");
        xsel.args(["--clipboard", "--input"]);
        commands.push(xsel);
    }

    commands
}

fn run_clipboard_command(command: &mut Command, text: &str) -> Result<()> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to start clipboard command")?;
    let Some(mut stdin) = child.stdin.take() else {
        anyhow::bail!("clipboard command did not open stdin");
    };
    stdin
        .write_all(text.as_bytes())
        .context("failed to write clipboard text")?;
    drop(stdin);

    let status = child
        .wait()
        .context("failed to wait for clipboard command")?;
    if !status.success() {
        anyhow::bail!("clipboard command exited with {status}");
    }
    Ok(())
}

fn run_command_in_journey(command: &str, journey_path: &Path) -> Result<()> {
    let status = Command::new(command)
        .current_dir(journey_path)
        .status()
        .with_context(|| {
            format!(
                "failed to start {command} in Journey folder {}",
                journey_path.display()
            )
        })?;
    if !status.success() {
        anyhow::bail!("{command} exited with {status}");
    }
    Ok(())
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
    Details,
    Actions,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Insert,
}

impl InputMode {
    fn label(self) -> &'static str {
        match self {
            InputMode::Normal => "normal",
            InputMode::Insert => "insert",
        }
    }

    fn style(self) -> Style {
        match self {
            InputMode::Normal => Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            InputMode::Insert => accent().add_modifier(Modifier::BOLD),
        }
    }
}

#[derive(Clone, Copy)]
enum JourneyAction {
    CopyCd,
    OpenNvim,
    OpenClaude,
    Resume,
    Worktree,
    ExistingBranchWorktree,
    Link,
    Unlink,
    Capture,
    DeleteWorktree,
    Done,
    Pause,
    Archive,
    Abandon,
}

impl JourneyAction {
    fn label(self) -> &'static str {
        match self {
            JourneyAction::CopyCd => "Copy cd to path",
            JourneyAction::OpenNvim => "Open nvim in Journey folder",
            JourneyAction::OpenClaude => "Open Claude Code in Journey folder",
            JourneyAction::Resume => "Resume",
            JourneyAction::Worktree => "New branch + worktree",
            JourneyAction::ExistingBranchWorktree => "Existing branch + worktree",
            JourneyAction::Link => "Link current worktree",
            JourneyAction::Unlink => "Unlink a repo",
            JourneyAction::Capture => "Capture thought",
            JourneyAction::DeleteWorktree => "Delete worktree",
            JourneyAction::Done => "Done",
            JourneyAction::Pause => "Pause",
            JourneyAction::Archive => "Archive",
            JourneyAction::Abandon => "Abandon",
        }
    }

    fn description(self) -> &'static str {
        match self {
            JourneyAction::CopyCd => "copy cd command",
            JourneyAction::OpenNvim => "cd journey + run nvim",
            JourneyAction::OpenClaude => "cd journey + run claude",
            JourneyAction::Resume => "mark active",
            JourneyAction::Worktree => "git worktree add -b",
            JourneyAction::ExistingBranchWorktree => "select branch",
            JourneyAction::Link => "attach cwd repo",
            JourneyAction::Unlink => "detach linked repo",
            JourneyAction::Capture => "append docs/capture.md",
            JourneyAction::DeleteWorktree => "git remove + unlink",
            JourneyAction::Done => "archive + remove worktrees",
            JourneyAction::Pause => "lifecycle only",
            JourneyAction::Archive => "release worktrees",
            JourneyAction::Abandon => "release worktrees",
        }
    }

    fn shortcut_action(self) -> Option<ShortcutAction> {
        match self {
            JourneyAction::OpenClaude => Some(ShortcutAction::OpenClaude),
            JourneyAction::OpenNvim => Some(ShortcutAction::OpenNvim),
            JourneyAction::Worktree => Some(ShortcutAction::NewBranchWorktree),
            JourneyAction::ExistingBranchWorktree => Some(ShortcutAction::ExistingBranchWorktree),
            JourneyAction::Link => Some(ShortcutAction::LinkCurrent),
            JourneyAction::Unlink => Some(ShortcutAction::UnlinkRepo),
            JourneyAction::Capture => Some(ShortcutAction::Capture),
            JourneyAction::DeleteWorktree => Some(ShortcutAction::DeleteWorktree),
            JourneyAction::Done => Some(ShortcutAction::Done),
            JourneyAction::Pause => Some(ShortcutAction::Pause),
            JourneyAction::Archive => Some(ShortcutAction::Archive),
            JourneyAction::CopyCd | JourneyAction::Resume | JourneyAction::Abandon => None,
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
    ExistingBranch {
        journey_id: String,
        branches: Vec<String>,
        query: String,
        selected: usize,
        state: ListState,
    },
    ExistingWorktreePath {
        journey_id: String,
        branch: String,
        input: String,
        default_path: PathBuf,
    },
    ExistingWorktreeLinkConfirm {
        journey_id: String,
        branch: String,
        worktree: PathBuf,
    },
    Unlink {
        journey_id: String,
        repos: Vec<RepoRef>,
        selected: usize,
        state: ListState,
    },
    DeleteWorktree {
        journey_id: String,
        repos: Vec<RepoRef>,
        selected: usize,
        state: ListState,
    },
    Capture {
        journey_id: String,
        input: String,
    },
    DoneConfirm {
        journey_id: String,
        repo_count: usize,
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
    Capture,
    ExistingWorktreePath,
}

impl DialogKeyTarget {
    fn submit_action(self) -> DialogAction {
        match self {
            DialogKeyTarget::NewTitle => DialogAction::SubmitNewTitle,
            DialogKeyTarget::NewDescription => DialogAction::SubmitNewDescription,
            DialogKeyTarget::WorktreeBranch => DialogAction::SubmitWorktreeBranch,
            DialogKeyTarget::WorktreePath => DialogAction::SubmitWorktreePath,
            DialogKeyTarget::Capture => DialogAction::SubmitCapture,
            DialogKeyTarget::ExistingWorktreePath => DialogAction::SubmitExistingWorktreePath,
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
    SubmitCapture,
    SubmitExistingBranch,
    SubmitExistingWorktreePath,
    SubmitExistingWorktreeLink,
    SubmitUnlink,
    SubmitDeleteWorktree,
    SubmitDone,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::widgets::StatefulWidget;

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

    #[test]
    fn renders_fenced_code_with_syntax_coloring() {
        let lines = render_markdown_lines("```rust\nfn main() {}\n```");
        let code_line = lines
            .iter()
            .find(|line| plain_line(line).contains("fn main"))
            .expect("rendered Rust code line");

        assert!(code_line
            .spans
            .iter()
            .any(|span| matches!(span.style.fg, Some(Color::Rgb(_, _, _)))));
    }

    #[test]
    fn preserves_blank_lines_between_markdown_blocks() {
        let lines =
            render_markdown_lines("Observed error:\n\n\n```js\nconsole.log('broken');\n```");
        let plain = lines.iter().map(plain_line).collect::<Vec<_>>();

        assert_eq!(
            plain,
            vec![
                "Observed error:",
                "",
                "",
                "code js",
                "    console.log('broken');"
            ]
        );
    }

    #[test]
    fn counts_wrapped_detail_lines() {
        let lines = vec![Line::from("abcdef"), Line::from("")];

        assert_eq!(wrapped_line_count(&lines, 3), 3);
        assert_eq!(wrapped_line_count(&lines, 0), 7);
    }

    #[test]
    fn details_scrollbar_reaches_bottom_at_max_scroll() {
        let viewport_height = 10;
        let content_height = 30;
        let max_scroll = content_height - viewport_height;
        let mut state = ScrollbarState::new(details_scrollbar_content_length(max_scroll))
            .position(max_scroll)
            .viewport_content_length(viewport_height);
        let area = Rect::new(0, 0, 1, viewport_height as u16);
        let mut buffer = Buffer::empty(area);

        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None)
            .render(area, &mut buffer, &mut state);

        assert_eq!(
            buffer
                .cell((0, viewport_height as u16 - 1))
                .expect("bottom scrollbar cell")
                .symbol(),
            "\u{2588}"
        );
    }

    #[test]
    fn cd_command_quotes_journey_paths() {
        let command = cd_command(Path::new("/tmp/my journey/it's here"));

        assert_eq!(command, "cd -- '/tmp/my journey/it'\\''s here'");
    }

    #[test]
    fn branch_filter_matches_all_query_terms() {
        let branches = vec![
            "main".to_string(),
            "feature/editor-esm".to_string(),
            "fix/editor-cjs".to_string(),
        ];

        assert_eq!(branch_filter_indices(&branches, ""), vec![0, 1, 2]);
        assert_eq!(branch_filter_indices(&branches, "editor esm"), vec![1]);
        assert!(branch_filter_indices(&branches, "editor missing").is_empty());
    }

    fn plain_line(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }
}
