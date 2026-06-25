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
#
# Actions are shown in this order. Remove an action from order, or add it to
# disabled, to hide it and disable its normal-mode shortcut.
[actions]
order = [
  "open_claude",
  "open_nvim",
  "new_branch_worktree",
  "existing_branch_worktree",
  "link_current",
  "unlink_repo",
  "delete_worktree",
  "done",
  "pause",
  "archive",
  "copy_cd",
  "resume",
  "abandon",
]
disabled = []

[shortcuts]
new_journey = "ctrl+n"
open_claude = "c"
open_nvim = "n"
new_branch_worktree = "b"
existing_branch_worktree = "w"
link_current = "l"
unlink_repo = "u"
delete_worktree = "d"
done = "f"
pause = "p"
archive = "x"
copy_cd = "none"
resume = "none"
abandon = "none"
insert_mode = "a"
normal_mode = "esc"
"#;

#[derive(Debug, Clone)]
pub(crate) struct ShortcutConfig {
    pub(crate) new_journey: KeyBinding,
    pub(crate) insert_mode: KeyBinding,
    pub(crate) normal_mode: KeyBinding,
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
        let mut normal_bindings = vec![
            ("new_journey".to_string(), self.new_journey),
            ("insert_mode".to_string(), self.insert_mode),
        ];
        for action in &self.actions {
            if let Some(binding) = self.binding_for_action(*action) {
                normal_bindings.push((action.config_key().to_string(), binding));
            }
        }
        validate_unique("normal mode", normal_bindings)?;
        validate_unique(
            "insert mode",
            vec![
                ("new_journey".to_string(), self.new_journey),
                ("normal_mode".to_string(), self.normal_mode),
            ],
        )
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
            new_journey: KeyBinding::control('n'),
            insert_mode: KeyBinding::plain('a'),
            normal_mode: KeyBinding::escape(),
            actions: TuiAction::all().to_vec(),
            action_bindings,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NormalShortcut {
    NewJourney,
    SwitchToInsert,
    Action(TuiAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InsertShortcut {
    NewJourney,
    SwitchToNormal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TuiAction {
    OpenClaude,
    OpenNvim,
    NewBranchWorktree,
    ExistingBranchWorktree,
    LinkCurrent,
    UnlinkRepo,
    DeleteWorktree,
    Done,
    Pause,
    Archive,
    CopyCd,
    Resume,
    Abandon,
}

impl TuiAction {
    pub(crate) const fn all() -> [Self; 13] {
        [
            Self::OpenClaude,
            Self::OpenNvim,
            Self::NewBranchWorktree,
            Self::ExistingBranchWorktree,
            Self::LinkCurrent,
            Self::UnlinkRepo,
            Self::DeleteWorktree,
            Self::Done,
            Self::Pause,
            Self::Archive,
            Self::CopyCd,
            Self::Resume,
            Self::Abandon,
        ]
    }

    pub(crate) fn config_key(self) -> &'static str {
        match self {
            Self::OpenClaude => "open_claude",
            Self::OpenNvim => "open_nvim",
            Self::NewBranchWorktree => "new_branch_worktree",
            Self::ExistingBranchWorktree => "existing_branch_worktree",
            Self::LinkCurrent => "link_current",
            Self::UnlinkRepo => "unlink_repo",
            Self::DeleteWorktree => "delete_worktree",
            Self::Done => "done",
            Self::Pause => "pause",
            Self::Archive => "archive",
            Self::CopyCd => "copy_cd",
            Self::Resume => "resume",
            Self::Abandon => "abandon",
        }
    }

    fn default_binding(self) -> Option<KeyBinding> {
        match self {
            Self::OpenClaude => Some(KeyBinding::plain('c')),
            Self::OpenNvim => Some(KeyBinding::plain('n')),
            Self::NewBranchWorktree => Some(KeyBinding::plain('b')),
            Self::ExistingBranchWorktree => Some(KeyBinding::plain('w')),
            Self::LinkCurrent => Some(KeyBinding::plain('l')),
            Self::UnlinkRepo => Some(KeyBinding::plain('u')),
            Self::DeleteWorktree => Some(KeyBinding::plain('d')),
            Self::Done => Some(KeyBinding::plain('f')),
            Self::Pause => Some(KeyBinding::plain('p')),
            Self::Archive => Some(KeyBinding::plain('x')),
            Self::CopyCd | Self::Resume | Self::Abandon => None,
        }
    }
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
    #[serde(default)]
    actions: ActionOverrides,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct ActionOverrides {
    order: Option<Vec<TuiAction>>,
    disabled: Vec<TuiAction>,
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
    delete_worktree: Option<String>,
    done: Option<String>,
    pause: Option<String>,
    archive: Option<String>,
    copy_cd: Option<String>,
    resume: Option<String>,
    abandon: Option<String>,
    insert_mode: Option<String>,
    normal_mode: Option<String>,
}

fn apply_key(target: &mut KeyBinding, value: Option<String>, field: &str) -> Result<()> {
    if let Some(value) = value {
        *target = KeyBinding::parse(&value).with_context(|| format!("invalid {field}"))?;
    }
    Ok(())
}

fn parse_optional_action_binding(value: &str) -> Result<Option<KeyBinding>> {
    let raw = value.trim();
    if raw.eq_ignore_ascii_case("none") || raw.eq_ignore_ascii_case("disabled") {
        return Ok(None);
    }
    Ok(Some(KeyBinding::parse(raw)?))
}

fn resolve_action_order(actions: ActionOverrides) -> Result<Vec<TuiAction>> {
    let mut resolved = actions.order.unwrap_or_else(|| TuiAction::all().to_vec());
    validate_action_list("actions.order", &resolved)?;
    validate_action_list("actions.disabled", &actions.disabled)?;
    resolved.retain(|action| !actions.disabled.contains(action));
    Ok(resolved)
}

fn validate_action_list(field: &str, actions: &[TuiAction]) -> Result<()> {
    let mut seen = HashMap::new();
    for action in actions {
        if let Some(previous) = seen.insert(*action, action.config_key()) {
            bail!(
                "{field} contains duplicate action `{}` and `{}`",
                previous,
                action.config_key()
            );
        }
    }
    Ok(())
}

fn validate_unique(mode: &str, bindings: Vec<(String, KeyBinding)>) -> Result<()> {
    let mut seen = HashMap::new();
    for (name, binding) in bindings {
        if let Some(previous) = seen.insert(binding, name.clone()) {
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
            Some(NormalShortcut::Action(TuiAction::OpenClaude))
        );
        assert_eq!(
            config.normal_command(key('n')),
            Some(NormalShortcut::Action(TuiAction::OpenNvim))
        );
        assert_eq!(
            config.normal_command(key('b')),
            Some(NormalShortcut::Action(TuiAction::NewBranchWorktree))
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
            Some(NormalShortcut::Action(TuiAction::OpenClaude))
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

    #[test]
    fn toml_configures_action_order_and_disabled_actions() {
        let config = ShortcutConfig::from_toml(
            r#"
            [actions]
            order = ["pause", "copy_cd", "open_claude"]
            disabled = ["open_claude"]

            [shortcuts]
            copy_cd = "y"
            "#,
        )
        .unwrap();

        assert_eq!(config.actions(), &[TuiAction::Pause, TuiAction::CopyCd]);
        assert_eq!(
            config.normal_command(key('y')),
            Some(NormalShortcut::Action(TuiAction::CopyCd))
        );
        assert_eq!(config.normal_command(key('c')), None);
    }

    #[test]
    fn none_disables_action_shortcut_without_hiding_action() {
        let config = ShortcutConfig::from_toml(
            r#"
            [shortcuts]
            open_claude = "none"
            "#,
        )
        .unwrap();

        assert!(config.actions().contains(&TuiAction::OpenClaude));
        assert_eq!(config.binding_for_action(TuiAction::OpenClaude), None);
        assert_eq!(config.normal_command(key('c')), None);
    }
}
