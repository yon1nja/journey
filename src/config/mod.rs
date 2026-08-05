mod actions;

pub(crate) use actions::TuiAction;
use actions::{
    apply_key, parse_optional_action_binding, resolve_action_order, validate_unique, ConfigFile,
};

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

pub const CONFIG_FILE: &str = "config.toml";

pub const DEFAULT_CONFIG_TOML: &str = r#"# ===========================================================================
#  Journey — config.toml
# ===========================================================================
#
#  This file configures the Journey interactive TUI.
#  Location: ~/.journey/config.toml
#
#  Journey reads this file once at startup. To reload after editing,
#  press Ctrl+R inside the TUI.
#
#  -----------------------------------------------------------------------
#  KEY SYNTAX
#  -----------------------------------------------------------------------
#
#  Each shortcut is a quoted string using one of these forms:
#
#    "c"           A single character key.
#    "ctrl+n"      Control + a character ("ctrl-n" and "control+n" also work).
#    "esc"         The Escape key ("escape" also works).
#    "enter"       The Enter/Return key ("return" also works).
#    "none"        Disable this shortcut entirely.
#
#  Keys are case-insensitive: "C" and "c" are the same binding.
#  Two shortcuts in the same context cannot share the same key —
#  Journey will refuse to start and print which keys conflict.
#
#  -----------------------------------------------------------------------
#  TUI SCREENS
#  -----------------------------------------------------------------------
#
#  The TUI has four screens. Which shortcuts are active depends on the
#  current screen:
#
#    List      The main Journey list. Navigation keys move the selection;
#              action shortcuts trigger on the selected Journey.
#    Search    A search/filter bar is active. Typing goes into the search
#              field; most shortcuts are suspended.
#    Details   The right pane is focused, showing README or docs for the
#              selected Journey. Scroll with navigation keys.
#    Actions   A popup listing available actions for the selected Journey.
#              Navigate and confirm to execute.
#
#  -----------------------------------------------------------------------
#  HARDCODED KEYS (not configurable)
#  -----------------------------------------------------------------------
#
#    Ctrl+C          Quit immediately (works on every screen).
#    Ctrl+N          Toggle notification history overlay.
#    Ctrl+R          Reload Journeys from disk.
#    Tab / Shift+Tab Cycle the status filter (Active → Paused → All → …).
#    Arrow keys      Same as nav_up / nav_down / nav_left / nav_right.
#


# ===========================================================================
#  ACTIONS
# ===========================================================================
#
#  Actions are operations you can perform on the selected Journey.
#  They appear in the actions popup (opened with `open_actions`).
#
#  `order` controls which actions are visible and in what order.
#  Only actions listed here appear in the popup and respond to shortcuts.
#  Remove an action from `order` to hide it completely.
#
#  `disabled` hides actions and unbinds their shortcuts, without removing
#  them from `order`. This is useful when you want to temporarily suppress
#  an action but keep its position for later.
#
#  Available actions and what they do:
#
#    open_claude               Open Claude Code in the Journey's directory.
#    open_nvim                 Open Neovim in the Journey's directory.
#    new_branch_worktree       Create a new git branch + worktree and link it.
#    existing_branch_worktree  Link an existing branch as a worktree.
#    link_current              Link the current working directory's repo.
#    unlink_repo               Detach a linked repository from the Journey.
#    capture                   Append a quick text capture to a Journey doc.
#    delete_worktree           Remove a linked worktree from disk and unlink it.
#    done                      Mark the Journey as done.
#    pause                     Mark the Journey as paused.
#    archive                   Mark the Journey as archived.
#    copy_cd                   Copy a `cd` command for the Journey to clipboard.
#    resume                    Resume a paused/done Journey (mark active).
#    abandon                   Mark the Journey as abandoned.
#

[actions]
order = [
  "open_claude",
  "open_nvim",
  "new_branch_worktree",
  "existing_branch_worktree",
  "link_current",
  "unlink_repo",
  "capture",
  "delete_worktree",
  "done",
  "pause",
  "archive",
  "copy_cd",
  "resume",
  "abandon",
]
disabled = []

#  Examples:
#
#  Show only a few actions:
#    order = ["open_claude", "capture", "pause", "resume", "done"]
#
#  Keep all actions but disable destructive ones:
#    disabled = ["delete_worktree", "abandon"]


# ===========================================================================
#  SHORTCUTS
# ===========================================================================
#
#  Shortcuts are split into two groups: navigation and actions.
#
#  Navigation shortcuts control movement and screen transitions.
#  Action shortcuts trigger Journey operations from the list screen.
#
#  Set any shortcut to "none" to disable it without affecting others.
#

[shortcuts]

# ---------------------------------------------------------------------------
#  Navigation
# ---------------------------------------------------------------------------

# Quit the TUI (from the list screen; on other screens, goes back first).
quit = "q"

# Go back: close popup, leave search, unfocus details, or quit from list.
back = "esc"

# Confirm: select an action in the popup, or open details from the list.
confirm = "enter"

# Focus the search bar. Type to filter Journeys by title. Esc to leave.
focus_search = "/"

# Focus the details pane (README / docs) for the selected Journey.
focus_details = "l"

# Open the actions popup for the selected Journey.
open_actions = "enter"

# Vim-style navigation. Also works with arrow keys (always available).
nav_up = "k"
nav_down = "j"
nav_left = "h"
nav_right = "l"

# ---------------------------------------------------------------------------
#  Action shortcuts
# ---------------------------------------------------------------------------
#
#  These fire directly from the list screen without opening the actions
#  popup. Only actions that appear in `actions.order` (and are not in
#  `actions.disabled`) respond to their shortcut.
#
#  Set a shortcut to "none" to require using the actions popup instead.
#

new_journey = "ctrl+j"
open_claude = "c"
open_nvim = "n"
new_branch_worktree = "b"
existing_branch_worktree = "w"
link_current = "none"
unlink_repo = "u"
capture = "t"
delete_worktree = "d"
done = "f"
pause = "p"
archive = "x"
copy_cd = "none"
resume = "none"
abandon = "none"

#  Examples:
#
#  Remap open_claude to "o":
#    open_claude = "o"
#
#  Add a shortcut for copy_cd:
#    copy_cd = "y"
#
#  Disable a shortcut (action is still available in the popup):
#    delete_worktree = "none"
"#;

#[derive(Debug, Clone)]
pub(crate) struct ShortcutConfig {
    pub(crate) new_journey: KeyBinding,
    quit: KeyBinding,
    back: KeyBinding,
    confirm: KeyBinding,
    focus_search: KeyBinding,
    focus_details: KeyBinding,
    open_actions: KeyBinding,
    nav_up: KeyBinding,
    nav_down: KeyBinding,
    nav_left: KeyBinding,
    nav_right: KeyBinding,
    actions: Vec<TuiAction>,
    action_bindings: HashMap<TuiAction, Option<KeyBinding>>,
}

impl ShortcutConfig {
    pub(crate) fn load(home: &Path) -> Result<Self> {
        let path = home.join(CONFIG_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Self::from_toml(&content).with_context(|| format!("failed to parse {}", path.display()))
    }

    fn from_toml(content: &str) -> Result<Self> {
        let file: ConfigFile = toml::from_str(content)?;
        let mut config = Self::default();
        let shortcuts = file.shortcuts;
        config.actions = resolve_action_order(file.actions)?;

        apply_key(
            &mut config.new_journey,
            shortcuts.new_journey,
            "shortcuts.new_journey",
        )?;
        config.apply_action_key(
            TuiAction::OpenClaude,
            shortcuts.open_claude,
            "shortcuts.open_claude",
        )?;
        config.apply_action_key(
            TuiAction::OpenNvim,
            shortcuts.open_nvim,
            "shortcuts.open_nvim",
        )?;
        config.apply_action_key(
            TuiAction::NewBranchWorktree,
            shortcuts.new_branch_worktree,
            "shortcuts.new_branch_worktree",
        )?;
        config.apply_action_key(
            TuiAction::ExistingBranchWorktree,
            shortcuts.existing_branch_worktree,
            "shortcuts.existing_branch_worktree",
        )?;
        config.apply_action_key(
            TuiAction::LinkCurrent,
            shortcuts.link_current,
            "shortcuts.link_current",
        )?;
        config.apply_action_key(
            TuiAction::UnlinkRepo,
            shortcuts.unlink_repo,
            "shortcuts.unlink_repo",
        )?;
        config.apply_action_key(TuiAction::Capture, shortcuts.capture, "shortcuts.capture")?;
        config.apply_action_key(
            TuiAction::DeleteWorktree,
            shortcuts.delete_worktree,
            "shortcuts.delete_worktree",
        )?;
        config.apply_action_key(TuiAction::Done, shortcuts.done, "shortcuts.done")?;
        config.apply_action_key(TuiAction::Pause, shortcuts.pause, "shortcuts.pause")?;
        config.apply_action_key(TuiAction::Archive, shortcuts.archive, "shortcuts.archive")?;
        config.apply_action_key(TuiAction::CopyCd, shortcuts.copy_cd, "shortcuts.copy_cd")?;
        config.apply_action_key(TuiAction::Resume, shortcuts.resume, "shortcuts.resume")?;
        config.apply_action_key(TuiAction::Abandon, shortcuts.abandon, "shortcuts.abandon")?;
        apply_key(&mut config.quit, shortcuts.quit, "shortcuts.quit")?;
        apply_key(&mut config.back, shortcuts.back, "shortcuts.back")?;
        apply_key(&mut config.confirm, shortcuts.confirm, "shortcuts.confirm")?;
        apply_key(
            &mut config.focus_search,
            shortcuts.focus_search,
            "shortcuts.focus_search",
        )?;
        apply_key(
            &mut config.focus_details,
            shortcuts.focus_details,
            "shortcuts.focus_details",
        )?;
        apply_key(
            &mut config.open_actions,
            shortcuts.open_actions,
            "shortcuts.open_actions",
        )?;
        apply_key(&mut config.nav_up, shortcuts.nav_up, "shortcuts.nav_up")?;
        apply_key(
            &mut config.nav_down,
            shortcuts.nav_down,
            "shortcuts.nav_down",
        )?;
        apply_key(
            &mut config.nav_left,
            shortcuts.nav_left,
            "shortcuts.nav_left",
        )?;
        apply_key(
            &mut config.nav_right,
            shortcuts.nav_right,
            "shortcuts.nav_right",
        )?;

        config.validate()?;
        Ok(config)
    }

    pub(crate) fn command(&self, key: KeyEvent) -> Option<Command> {
        if self.quit.matches(key) {
            return Some(Command::Quit);
        }
        if self.back.matches(key) {
            return Some(Command::Back);
        }
        if self.confirm.matches(key) {
            return Some(Command::Confirm);
        }
        if self.focus_search.matches(key) {
            return Some(Command::FocusSearch);
        }
        if self.new_journey.matches(key) {
            return Some(Command::NewJourney);
        }
        if self.focus_details.matches(key) {
            return Some(Command::FocusDetails);
        }
        if self.open_actions.matches(key) {
            return Some(Command::OpenActions);
        }
        if self.nav_up.matches(key) {
            return Some(Command::NavUp);
        }
        if self.nav_down.matches(key) {
            return Some(Command::NavDown);
        }
        if self.nav_left.matches(key) {
            return Some(Command::NavLeft);
        }
        if self.nav_right.matches(key) {
            return Some(Command::NavRight);
        }
        if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Some(Command::Reload);
        }
        if key.code == KeyCode::Tab && key.modifiers.is_empty() {
            return Some(Command::CycleFilter);
        }
        if key.code == KeyCode::BackTab {
            return Some(Command::CycleFilterBack);
        }
        self.action_for_key(key).map(Command::Action)
    }

    pub(crate) fn quit_display(&self) -> String {
        self.quit.display()
    }

    pub(crate) fn back_display(&self) -> String {
        self.back.display()
    }

    pub(crate) fn confirm_display(&self) -> String {
        self.confirm.display()
    }

    pub(crate) fn focus_search_display(&self) -> String {
        self.focus_search.display()
    }

    pub(crate) fn focus_details_display(&self) -> String {
        self.focus_details.display()
    }

    pub(crate) fn open_actions_display(&self) -> String {
        self.open_actions.display()
    }

    pub(crate) fn nav_up_display(&self) -> String {
        self.nav_up.display()
    }

    pub(crate) fn nav_down_display(&self) -> String {
        self.nav_down.display()
    }

    pub(crate) fn actions(&self) -> &[TuiAction] {
        &self.actions
    }

    pub(crate) fn binding_for_action(&self, action: TuiAction) -> Option<KeyBinding> {
        self.action_bindings.get(&action).copied().flatten()
    }

    fn action_for_key(&self, key: KeyEvent) -> Option<TuiAction> {
        self.actions.iter().copied().find(|action| {
            self.binding_for_action(*action)
                .is_some_and(|binding| binding.matches(key))
        })
    }

    fn validate(&self) -> Result<()> {
        let mut bindings = vec![
            ("new_journey".to_string(), self.new_journey),
            ("quit".to_string(), self.quit),
            ("focus_search".to_string(), self.focus_search),
            ("nav_up".to_string(), self.nav_up),
            ("nav_down".to_string(), self.nav_down),
        ];
        for action in &self.actions {
            if let Some(binding) = self.binding_for_action(*action) {
                bindings.push((action.config_key().to_string(), binding));
            }
        }
        validate_unique("shortcuts", bindings)
    }

    fn apply_action_key(
        &mut self,
        action: TuiAction,
        value: Option<String>,
        field: &str,
    ) -> Result<()> {
        if let Some(value) = value {
            let binding = parse_optional_action_binding(&value)
                .with_context(|| format!("invalid {field}"))?;
            self.action_bindings.insert(action, binding);
        }
        Ok(())
    }
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        let action_bindings = TuiAction::all()
            .into_iter()
            .map(|action| (action, action.default_binding()))
            .collect();
        Self {
            new_journey: KeyBinding::control('j'),
            quit: KeyBinding::plain('q'),
            back: KeyBinding::escape(),
            confirm: KeyBinding::enter(),
            focus_search: KeyBinding::plain('/'),
            focus_details: KeyBinding::plain('l'),
            open_actions: KeyBinding::enter(),
            nav_up: KeyBinding::plain('k'),
            nav_down: KeyBinding::plain('j'),
            nav_left: KeyBinding::plain('h'),
            nav_right: KeyBinding::plain('l'),
            actions: TuiAction::all().to_vec(),
            action_bindings,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Quit,
    Back,
    Confirm,
    FocusSearch,
    FocusDetails,
    OpenActions,
    NavUp,
    NavDown,
    NavLeft,
    NavRight,
    NewJourney,
    Reload,
    CycleFilter,
    CycleFilterBack,
    Action(TuiAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct KeyBinding {
    code: BindingCode,
    control: bool,
}

impl KeyBinding {
    fn plain(ch: char) -> Self {
        Self {
            code: BindingCode::Char(ch.to_ascii_lowercase()),
            control: false,
        }
    }

    fn control(ch: char) -> Self {
        Self {
            code: BindingCode::Char(ch.to_ascii_lowercase()),
            control: true,
        }
    }

    fn escape() -> Self {
        Self {
            code: BindingCode::Esc,
            control: false,
        }
    }

    fn enter() -> Self {
        Self {
            code: BindingCode::Enter,
            control: false,
        }
    }

    fn parse(value: &str) -> Result<Self> {
        let raw = value.trim();
        if raw.is_empty() {
            bail!("shortcut cannot be empty");
        }

        let lower = raw.to_ascii_lowercase();
        let normalized = lower.replace('-', "+");
        let (control, key) = if let Some(key) = normalized.strip_prefix("ctrl+") {
            (true, key)
        } else if let Some(key) = normalized.strip_prefix("control+") {
            (true, key)
        } else {
            (false, normalized.as_str())
        };

        let code = match key {
            "esc" | "escape" => BindingCode::Esc,
            "enter" | "return" => BindingCode::Enter,
            key => {
                let mut chars = key.chars();
                let Some(ch) = chars.next() else {
                    bail!("shortcut `{raw}` is missing a key");
                };
                if chars.next().is_some() {
                    bail!(
                        "shortcut `{raw}` must be a single key, `esc`, or a control key like `ctrl+n`"
                    );
                }
                BindingCode::Char(ch)
            }
        };

        Ok(Self { code, control })
    }

    pub(crate) fn display(self) -> String {
        let key = match self.code {
            BindingCode::Char(ch) => ch.to_ascii_uppercase().to_string(),
            BindingCode::Esc => "Esc".to_string(),
            BindingCode::Enter => "Enter".to_string(),
        };
        if self.control {
            format!("Ctrl-{key}")
        } else {
            key
        }
    }

    fn matches(self, key: KeyEvent) -> bool {
        if self.control {
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::ALT)
            {
                return false;
            }
        } else if key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT)
        {
            return false;
        }

        match (self.code, key.code) {
            (BindingCode::Char(expected), KeyCode::Char(actual)) => {
                actual.eq_ignore_ascii_case(&expected)
            }
            (BindingCode::Esc, KeyCode::Esc) => true,
            (BindingCode::Enter, KeyCode::Enter) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BindingCode {
    Char(char),
    Esc,
    Enter,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty())
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    #[test]
    fn defaults_resolve_navigation_commands() {
        let config = ShortcutConfig::default();

        assert_eq!(config.command(key('q')), Some(Command::Quit));
        assert_eq!(
            config.command(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())),
            Some(Command::Back)
        );
        assert_eq!(config.command(key('/')), Some(Command::FocusSearch));
        assert_eq!(config.command(key('l')), Some(Command::FocusDetails));
        assert_eq!(
            config.command(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
            Some(Command::Confirm)
        );
        assert_eq!(config.command(key('k')), Some(Command::NavUp));
        assert_eq!(config.command(key('j')), Some(Command::NavDown));
        assert_eq!(config.command(ctrl('j')), Some(Command::NewJourney));
        assert_eq!(
            config.command(key('c')),
            Some(Command::Action(TuiAction::OpenClaude))
        );
        // Verify LinkCurrent has no default binding (not 'l', which is FocusDetails)
        assert_eq!(config.binding_for_action(TuiAction::LinkCurrent), None);
    }

    #[test]
    fn toml_overrides_default_shortcuts() {
        let config = ShortcutConfig::from_toml(
            r#"
            [shortcuts]
            open_claude = "o"
            quit = "z"
            focus_search = "s"
            "#,
        )
        .unwrap();

        assert_eq!(
            config.command(key('o')),
            Some(Command::Action(TuiAction::OpenClaude))
        );
        assert_eq!(config.command(key('z')), Some(Command::Quit));
        assert_eq!(config.command(key('s')), Some(Command::FocusSearch));
    }

    #[test]
    fn rejects_duplicate_bindings() {
        let error = ShortcutConfig::from_toml(
            r#"
            [shortcuts]
            open_claude = "k"
            "#,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("shortcut `K`"));
    }

    #[test]
    fn ignores_legacy_mode_fields() {
        let config = ShortcutConfig::from_toml(
            r#"
            [shortcuts]
            insert_mode = "a"
            normal_mode = "esc"
            "#,
        );
        assert!(config.is_ok());
    }

    #[test]
    fn parses_enter_binding() {
        let binding = KeyBinding::parse("enter").unwrap();
        let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        assert!(binding.matches(event));
        assert_eq!(binding.display(), "Enter");
    }

    #[test]
    fn parses_return_as_enter() {
        let binding = KeyBinding::parse("return").unwrap();
        let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        assert!(binding.matches(event));
        assert_eq!(binding.display(), "Enter");
    }

    #[test]
    fn enter_does_not_match_other_keys() {
        let binding = KeyBinding::parse("enter").unwrap();
        assert!(!binding.matches(key('a')));
        assert!(!binding.matches(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())));
    }

    #[test]
    fn enter_constructor() {
        let binding = KeyBinding::enter();
        let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        assert!(binding.matches(event));
        assert_eq!(binding.display(), "Enter");
    }

    #[test]
    fn default_config_toml_parses_successfully() {
        ShortcutConfig::from_toml(DEFAULT_CONFIG_TOML).unwrap();
    }
}
