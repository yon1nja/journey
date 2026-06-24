use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

pub const CONFIG_FILE: &str = "config.toml";

pub const DEFAULT_CONFIG_TOML: &str = r#"# Journey configuration.
#
# Shortcuts use single keys like "c", or control keys like "ctrl+n".
[shortcuts]
new_journey = "ctrl+n"
open_claude = "c"
open_nvim = "n"
new_branch_worktree = "b"
existing_branch_worktree = "w"
link_current = "l"
unlink_repo = "u"
capture = "t"
delete_worktree = "d"
done = "f"
pause = "p"
archive = "x"
insert_mode = "a"
normal_mode = "esc"
"#;

#[derive(Debug, Clone)]
pub(crate) struct ShortcutConfig {
    pub(crate) new_journey: KeyBinding,
    pub(crate) insert_mode: KeyBinding,
    pub(crate) normal_mode: KeyBinding,
    open_claude: KeyBinding,
    open_nvim: KeyBinding,
    new_branch_worktree: KeyBinding,
    existing_branch_worktree: KeyBinding,
    link_current: KeyBinding,
    unlink_repo: KeyBinding,
    capture: KeyBinding,
    delete_worktree: KeyBinding,
    done: KeyBinding,
    pause: KeyBinding,
    archive: KeyBinding,
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

        apply_key(
            &mut config.new_journey,
            shortcuts.new_journey,
            "shortcuts.new_journey",
        )?;
        apply_key(
            &mut config.open_claude,
            shortcuts.open_claude,
            "shortcuts.open_claude",
        )?;
        apply_key(
            &mut config.open_nvim,
            shortcuts.open_nvim,
            "shortcuts.open_nvim",
        )?;
        apply_key(
            &mut config.new_branch_worktree,
            shortcuts.new_branch_worktree,
            "shortcuts.new_branch_worktree",
        )?;
        apply_key(
            &mut config.existing_branch_worktree,
            shortcuts.existing_branch_worktree,
            "shortcuts.existing_branch_worktree",
        )?;
        apply_key(
            &mut config.link_current,
            shortcuts.link_current,
            "shortcuts.link_current",
        )?;
        apply_key(
            &mut config.unlink_repo,
            shortcuts.unlink_repo,
            "shortcuts.unlink_repo",
        )?;
        apply_key(&mut config.capture, shortcuts.capture, "shortcuts.capture")?;
        apply_key(
            &mut config.delete_worktree,
            shortcuts.delete_worktree,
            "shortcuts.delete_worktree",
        )?;
        apply_key(&mut config.done, shortcuts.done, "shortcuts.done")?;
        apply_key(&mut config.pause, shortcuts.pause, "shortcuts.pause")?;
        apply_key(&mut config.archive, shortcuts.archive, "shortcuts.archive")?;
        apply_key(
            &mut config.insert_mode,
            shortcuts.insert_mode,
            "shortcuts.insert_mode",
        )?;
        apply_key(
            &mut config.normal_mode,
            shortcuts.normal_mode,
            "shortcuts.normal_mode",
        )?;

        config.validate()?;
        Ok(config)
    }

    pub(crate) fn normal_command(&self, key: KeyEvent) -> Option<NormalShortcut> {
        if self.new_journey.matches(key) {
            return Some(NormalShortcut::NewJourney);
        }
        if self.insert_mode.matches(key) {
            return Some(NormalShortcut::SwitchToInsert);
        }
        self.action_for_key(key).map(NormalShortcut::Action)
    }

    pub(crate) fn insert_command(&self, key: KeyEvent) -> Option<InsertShortcut> {
        if self.new_journey.matches(key) {
            return Some(InsertShortcut::NewJourney);
        }
        if self.normal_mode.matches(key) {
            return Some(InsertShortcut::SwitchToNormal);
        }
        None
    }

    pub(crate) fn binding_for_action(&self, action: ShortcutAction) -> KeyBinding {
        match action {
            ShortcutAction::OpenClaude => self.open_claude,
            ShortcutAction::OpenNvim => self.open_nvim,
            ShortcutAction::NewBranchWorktree => self.new_branch_worktree,
            ShortcutAction::ExistingBranchWorktree => self.existing_branch_worktree,
            ShortcutAction::LinkCurrent => self.link_current,
            ShortcutAction::UnlinkRepo => self.unlink_repo,
            ShortcutAction::Capture => self.capture,
            ShortcutAction::DeleteWorktree => self.delete_worktree,
            ShortcutAction::Done => self.done,
            ShortcutAction::Pause => self.pause,
            ShortcutAction::Archive => self.archive,
        }
    }

    fn action_for_key(&self, key: KeyEvent) -> Option<ShortcutAction> {
        [
            ShortcutAction::OpenClaude,
            ShortcutAction::OpenNvim,
            ShortcutAction::NewBranchWorktree,
            ShortcutAction::ExistingBranchWorktree,
            ShortcutAction::LinkCurrent,
            ShortcutAction::UnlinkRepo,
            ShortcutAction::Capture,
            ShortcutAction::DeleteWorktree,
            ShortcutAction::Done,
            ShortcutAction::Pause,
            ShortcutAction::Archive,
        ]
        .into_iter()
        .find(|action| self.binding_for_action(*action).matches(key))
    }

    fn validate(&self) -> Result<()> {
        validate_unique(
            "normal mode",
            [
                ("new_journey", self.new_journey),
                ("insert_mode", self.insert_mode),
                ("open_claude", self.open_claude),
                ("open_nvim", self.open_nvim),
                ("new_branch_worktree", self.new_branch_worktree),
                ("existing_branch_worktree", self.existing_branch_worktree),
                ("link_current", self.link_current),
                ("unlink_repo", self.unlink_repo),
                ("capture", self.capture),
                ("delete_worktree", self.delete_worktree),
                ("done", self.done),
                ("pause", self.pause),
                ("archive", self.archive),
            ],
        )?;
        validate_unique(
            "insert mode",
            [
                ("new_journey", self.new_journey),
                ("normal_mode", self.normal_mode),
            ],
        )
    }
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            new_journey: KeyBinding::control('n'),
            open_claude: KeyBinding::plain('c'),
            open_nvim: KeyBinding::plain('n'),
            new_branch_worktree: KeyBinding::plain('b'),
            existing_branch_worktree: KeyBinding::plain('w'),
            link_current: KeyBinding::plain('l'),
            unlink_repo: KeyBinding::plain('u'),
            capture: KeyBinding::plain('t'),
            delete_worktree: KeyBinding::plain('d'),
            done: KeyBinding::plain('f'),
            pause: KeyBinding::plain('p'),
            archive: KeyBinding::plain('x'),
            insert_mode: KeyBinding::plain('a'),
            normal_mode: KeyBinding::escape(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NormalShortcut {
    NewJourney,
    SwitchToInsert,
    Action(ShortcutAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InsertShortcut {
    NewJourney,
    SwitchToNormal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShortcutAction {
    OpenClaude,
    OpenNvim,
    NewBranchWorktree,
    ExistingBranchWorktree,
    LinkCurrent,
    UnlinkRepo,
    Capture,
    DeleteWorktree,
    Done,
    Pause,
    Archive,
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
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BindingCode {
    Char(char),
    Esc,
}

#[derive(Debug, Deserialize, Default)]
struct ConfigFile {
    #[serde(default)]
    shortcuts: ShortcutOverrides,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct ShortcutOverrides {
    new_journey: Option<String>,
    open_claude: Option<String>,
    open_nvim: Option<String>,
    new_branch_worktree: Option<String>,
    existing_branch_worktree: Option<String>,
    link_current: Option<String>,
    unlink_repo: Option<String>,
    capture: Option<String>,
    delete_worktree: Option<String>,
    done: Option<String>,
    pause: Option<String>,
    archive: Option<String>,
    insert_mode: Option<String>,
    normal_mode: Option<String>,
}

fn apply_key(target: &mut KeyBinding, value: Option<String>, field: &str) -> Result<()> {
    if let Some(value) = value {
        *target = KeyBinding::parse(&value).with_context(|| format!("invalid {field}"))?;
    }
    Ok(())
}

fn validate_unique<const N: usize>(
    mode: &str,
    bindings: [(&'static str, KeyBinding); N],
) -> Result<()> {
    let mut seen = HashMap::new();
    for (name, binding) in bindings {
        if let Some(previous) = seen.insert(binding, name) {
            bail!(
                "{mode} shortcut `{}` is assigned to both `{previous}` and `{name}`",
                binding.display()
            );
        }
    }
    Ok(())
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
    fn defaults_match_requested_shortcuts() {
        let config = ShortcutConfig::default();

        assert_eq!(
            config.normal_command(key('c')),
            Some(NormalShortcut::Action(ShortcutAction::OpenClaude))
        );
        assert_eq!(
            config.normal_command(key('n')),
            Some(NormalShortcut::Action(ShortcutAction::OpenNvim))
        );
        assert_eq!(
            config.normal_command(key('b')),
            Some(NormalShortcut::Action(ShortcutAction::NewBranchWorktree))
        );
        assert_eq!(
            config.normal_command(key('t')),
            Some(NormalShortcut::Action(ShortcutAction::Capture))
        );
        assert_eq!(
            config.normal_command(key('a')),
            Some(NormalShortcut::SwitchToInsert)
        );
        assert_eq!(
            config.insert_command(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())),
            Some(InsertShortcut::SwitchToNormal)
        );
        assert_eq!(
            config.normal_command(ctrl('n')),
            Some(NormalShortcut::NewJourney)
        );
        assert_eq!(
            config.insert_command(ctrl('n')),
            Some(InsertShortcut::NewJourney)
        );
    }

    #[test]
    fn toml_overrides_default_shortcuts() {
        let config = ShortcutConfig::from_toml(
            r#"
            [shortcuts]
            open_claude = "o"
            insert_mode = "i"
            normal_mode = "ctrl+g"
            "#,
        )
        .unwrap();

        assert_eq!(
            config.normal_command(key('o')),
            Some(NormalShortcut::Action(ShortcutAction::OpenClaude))
        );
        assert_eq!(
            config.normal_command(key('i')),
            Some(NormalShortcut::SwitchToInsert)
        );
        assert_eq!(
            config.insert_command(ctrl('g')),
            Some(InsertShortcut::SwitchToNormal)
        );
    }

    #[test]
    fn rejects_duplicate_bindings_in_same_mode() {
        let error = ShortcutConfig::from_toml(
            r#"
            [shortcuts]
            open_claude = "n"
            "#,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("normal mode shortcut `N`"));
    }
}
