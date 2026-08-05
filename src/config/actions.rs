use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TuiAction {
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
    CopyCd,
    Resume,
    Abandon,
}

impl TuiAction {
    pub(crate) const fn all() -> [Self; 14] {
        [
            Self::OpenClaude,
            Self::OpenNvim,
            Self::NewBranchWorktree,
            Self::ExistingBranchWorktree,
            Self::LinkCurrent,
            Self::UnlinkRepo,
            Self::Capture,
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
            Self::Capture => "capture",
            Self::DeleteWorktree => "delete_worktree",
            Self::Done => "done",
            Self::Pause => "pause",
            Self::Archive => "archive",
            Self::CopyCd => "copy_cd",
            Self::Resume => "resume",
            Self::Abandon => "abandon",
        }
    }

    pub(super) fn default_binding(self) -> Option<KeyBinding> {
        match self {
            Self::OpenClaude => Some(KeyBinding::plain('c')),
            Self::OpenNvim => Some(KeyBinding::plain('n')),
            Self::NewBranchWorktree => Some(KeyBinding::plain('b')),
            Self::ExistingBranchWorktree => Some(KeyBinding::plain('w')),
            Self::UnlinkRepo => Some(KeyBinding::plain('u')),
            Self::Capture => Some(KeyBinding::plain('t')),
            Self::DeleteWorktree => Some(KeyBinding::plain('d')),
            Self::Done => Some(KeyBinding::plain('f')),
            Self::Pause => Some(KeyBinding::plain('p')),
            Self::Archive => Some(KeyBinding::plain('x')),
            Self::LinkCurrent | Self::CopyCd | Self::Resume | Self::Abandon => None,
        }
    }
}
#[derive(Debug, Deserialize, Default)]
pub(super) struct ConfigFile {
    #[serde(default)]
    pub(super) shortcuts: ShortcutOverrides,
    #[serde(default)]
    pub(super) actions: ActionOverrides,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ActionOverrides {
    pub(super) order: Option<Vec<TuiAction>>,
    pub(super) disabled: Vec<TuiAction>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(super) struct ShortcutOverrides {
    pub(super) new_journey: Option<String>,
    pub(super) quit: Option<String>,
    pub(super) back: Option<String>,
    pub(super) confirm: Option<String>,
    pub(super) focus_search: Option<String>,
    pub(super) focus_details: Option<String>,
    pub(super) open_actions: Option<String>,
    pub(super) nav_up: Option<String>,
    pub(super) nav_down: Option<String>,
    pub(super) nav_left: Option<String>,
    pub(super) nav_right: Option<String>,
    pub(super) open_claude: Option<String>,
    pub(super) open_nvim: Option<String>,
    pub(super) new_branch_worktree: Option<String>,
    pub(super) existing_branch_worktree: Option<String>,
    pub(super) link_current: Option<String>,
    pub(super) unlink_repo: Option<String>,
    pub(super) capture: Option<String>,
    pub(super) delete_worktree: Option<String>,
    pub(super) done: Option<String>,
    pub(super) pause: Option<String>,
    pub(super) archive: Option<String>,
    pub(super) copy_cd: Option<String>,
    pub(super) resume: Option<String>,
    pub(super) abandon: Option<String>,
}

pub(super) fn apply_key(target: &mut KeyBinding, value: Option<String>, field: &str) -> Result<()> {
    if let Some(value) = value {
        *target = KeyBinding::parse(&value).with_context(|| format!("invalid {field}"))?;
    }
    Ok(())
}

pub(super) fn parse_optional_action_binding(value: &str) -> Result<Option<KeyBinding>> {
    let raw = value.trim();
    if raw.eq_ignore_ascii_case("none") || raw.eq_ignore_ascii_case("disabled") {
        return Ok(None);
    }
    Ok(Some(KeyBinding::parse(raw)?))
}

pub(super) fn resolve_action_order(actions: ActionOverrides) -> Result<Vec<TuiAction>> {
    let mut resolved = actions.order.unwrap_or_else(|| TuiAction::all().to_vec());
    validate_action_list("actions.order", &resolved)?;
    validate_action_list("actions.disabled", &actions.disabled)?;
    resolved.retain(|action| !actions.disabled.contains(action));
    Ok(resolved)
}

pub(super) fn validate_action_list(field: &str, actions: &[TuiAction]) -> Result<()> {
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

pub(super) fn validate_unique(mode: &str, bindings: Vec<(String, KeyBinding)>) -> Result<()> {
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
            config.command(key('y')),
            Some(Command::Action(TuiAction::CopyCd))
        );
        assert_eq!(config.command(key('c')), None);
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
        assert_eq!(config.command(key('c')), None);
    }
}
