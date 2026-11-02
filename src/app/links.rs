//! Links between Journeys, side-quest promotion, and side-quest outcomes.

use super::*;

use crate::models::JourneyKind;

/// Heading of the README section that collects side-quest outcomes.
const SIDE_QUESTS_HEADING: &str = "## Side-quests";

pub(crate) fn relate(home: &Path, cwd: &Path, id: Option<&str>, other: &str) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    let other = other.trim();
    if other == ctx.journey.id {
        bail!("a Journey cannot be related to itself");
    }
    if !storage::journey_dir(home, other)
        .join(storage::JOURNEY_FILE)
        .exists()
    {
        bail!("unknown Journey id: {other}");
    }
    if ctx.journey.links.iter().any(|link| link == other) {
        bail!(
            "Journey `{}` is already related to `{other}`",
            ctx.journey.id
        );
    }

    ctx.journey.links.push(other.to_string());
    storage::save_journey(&ctx.path, &ctx.journey)?;
    let record = events::append_event(
        &ctx.path,
        EventKind::Relate {
            journey: other.to_string(),
        },
    )?;
    finish_mutation(&ctx, Some(record.ts))?;
    Ok(format!("related Journey `{}` to `{other}`", ctx.journey.id))
}

pub(crate) fn unrelate(home: &Path, cwd: &Path, id: Option<&str>, other: &str) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    let other = other.trim();
    let Some(position) = ctx.journey.links.iter().position(|link| link == other) else {
        bail!("Journey `{}` is not related to `{other}`", ctx.journey.id);
    };

    ctx.journey.links.remove(position);
    storage::save_journey(&ctx.path, &ctx.journey)?;
    let record = events::append_event(
        &ctx.path,
        EventKind::Unrelate {
            journey: other.to_string(),
        },
    )?;
    finish_mutation(&ctx, Some(record.ts))?;
    Ok(format!(
        "unrelated Journey `{}` from `{other}`",
        ctx.journey.id
    ))
}

pub(crate) fn promote(home: &Path, cwd: &Path, id: Option<&str>) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    if ctx.journey.kind != JourneyKind::SideQuest {
        bail!("Journey `{}` is not a side-quest", ctx.journey.id);
    }

    ctx.journey.kind = JourneyKind::Journey;
    storage::save_journey(&ctx.path, &ctx.journey)?;
    let record = events::append_event(&ctx.path, EventKind::Promote)?;
    finish_mutation(&ctx, Some(record.ts))?;
    Ok(format!(
        "promoted side-quest `{}` to a journey",
        ctx.journey.id
    ))
}

/// Writes a closed side-quest's outcome into the README of every Journey it is
/// linked to. Returns the ids that were updated.
pub(crate) fn record_side_quest_outcome(
    home: &Path,
    side_quest: &JourneyContext,
    outcome: &str,
    now: &str,
) -> Result<Vec<String>> {
    let date = now.get(..10).unwrap_or(now);
    let line = format!("- `{}` ({date}): {outcome}", side_quest.journey.id);
    let mut updated = Vec::new();
    for link in &side_quest.journey.links {
        let path = storage::journey_dir(home, link);
        if !path.join(storage::JOURNEY_FILE).exists() {
            continue;
        }
        let journey = storage::load_journey(&path)?;
        let readme = path.join(storage::README_FILE);
        let content = match fs::read_to_string(&readme) {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                format!("# {}\n", journey.title)
            }
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", readme.display()))
            }
        };
        storage::write_string_atomic(&readme, &append_to_side_quests(&content, &line))?;
        storage::update_index_entry(home, &journey, now)?;
        updated.push(journey.id);
    }
    Ok(updated)
}

/// Appends `line` to the end of the side-quests section, creating the section
/// at the end of the document when it does not exist.
fn append_to_side_quests(content: &str, line: &str) -> String {
    let mut lines = content.lines().map(str::to_string).collect::<Vec<_>>();
    match lines
        .iter()
        .position(|existing| existing.trim_end() == SIDE_QUESTS_HEADING)
    {
        Some(heading) => {
            let section_end = lines[heading + 1..]
                .iter()
                .position(|existing| existing.starts_with("# ") || existing.starts_with("## "))
                .map(|offset| heading + 1 + offset)
                .unwrap_or(lines.len());
            let mut insert_at = section_end;
            while insert_at > heading + 1 && lines[insert_at - 1].trim().is_empty() {
                insert_at -= 1;
            }
            if insert_at == heading + 1 {
                lines.insert(insert_at, String::new());
                insert_at += 1;
            }
            lines.insert(insert_at, line.to_string());
        }
        None => {
            while lines.last().is_some_and(|last| last.trim().is_empty()) {
                lines.pop();
            }
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(SIDE_QUESTS_HEADING.to_string());
            lines.push(String::new());
            lines.push(line.to_string());
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_side_quests_section_at_the_end() {
        let out = append_to_side_quests("# Title\n\nOverview.\n\n", "- a");
        assert_eq!(out, "# Title\n\nOverview.\n\n## Side-quests\n\n- a\n");
    }

    #[test]
    fn appends_inside_an_existing_section() {
        let content = "# Title\n\n## Side-quests\n\n- a\n\n## Notes\n\nKeep.\n";
        let out = append_to_side_quests(content, "- b");
        assert_eq!(
            out,
            "# Title\n\n## Side-quests\n\n- a\n- b\n\n## Notes\n\nKeep.\n"
        );
    }

    #[test]
    fn fills_an_empty_section() {
        let out = append_to_side_quests("# Title\n\n## Side-quests\n", "- a");
        assert_eq!(out, "# Title\n\n## Side-quests\n\n- a\n");
    }
}
