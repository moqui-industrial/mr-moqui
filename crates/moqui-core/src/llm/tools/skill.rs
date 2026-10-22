/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! `skill` (plans/LLM-A2A-Assist.md 7.5): `skill(name)` answers the text before the first section, a table of contents
//! with each section's size, and the whole body when it is under 6k characters; `skill(name#section)` one section;
//! `skill(find=words)` the skills that match. It reads the [`SkillIndex`]; the lessons of a skill join its answer in
//! phase 13.14.

use serde_json::Value as Json;

use super::format::ToolError;
use super::schema::{object, Property};
use super::suggest;
use super::{AgentTool, Classification, ToolOutcome, ToolSpec};
use crate::llm::run::context::TurnContext;
use crate::llm::skills::{self, Skill, SkillIndex};

/// Skills a find lists.
const FIND_LIMIT: usize = 8;

pub struct SkillTool(ToolSpec);

impl Default for SkillTool {
    fn default() -> Self {
        Self(ToolSpec::new(
            "skill",
            "Load a skill by name, or one section as name#section. find lists skills that match words.",
            object(&[
                Property::optional("name", "string", "name or name#section"),
                Property::optional("find", "string", "Words to match"),
            ]),
            Classification::Read,
        ))
    }
}

impl AgentTool for SkillTool {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn call(&self, _ctx: &mut TurnContext<'_>, args: Json) -> ToolOutcome {
        let index = match super::apps::factory() {
            Ok(factory) => SkillIndex::load(&factory),
            Err(_) => SkillIndex::default(),
        };
        let text = |key: &str| {
            args.get(key)
                .and_then(Json::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
        };
        let outcome = match (text("name"), text("find")) {
            (Some(name), _) => load(&index, name),
            (None, Some(words)) => Ok(find(&index, words)),
            (None, None) => Err(ToolError::new(
                "no_name",
                "Call skill with name (a name from the catalog, or name#section), or with find=words.",
            )),
        };
        match outcome {
            Ok(text) => ToolOutcome::text(text),
            Err(error) => error.outcome(),
        }
    }
}

/// The skills that match `words`, one line each.
pub fn find(index: &SkillIndex, words: &str) -> String {
    let found = index.find(words);
    if found.is_empty() {
        return format!("No skill matches \"{words}\". Work from the screens; search finds them.");
    }
    let mut lines: Vec<String> = found.iter().take(FIND_LIMIT).map(|skill| catalog_line(skill)).collect();
    if found.len() > FIND_LIMIT {
        lines.push(format!(
            "… {} more: skill find with more words",
            found.len() - FIND_LIMIT
        ));
    }
    lines.join("\n")
}

/// `- name: description (kind)`, as the catalog writes a skill.
pub fn catalog_line(skill: &Skill) -> String {
    if skill.kind.is_empty() {
        format!("- {}: {}", skill.name, skill.description)
    } else {
        format!("- {}: {} ({})", skill.name, skill.description, skill.kind)
    }
}

/// The answer of `skill(name)` or `skill(name#section)`.
///
/// # Errors
///
/// Returns an error the model reads when there is no such skill or section, with the nearest names.
pub fn load(index: &SkillIndex, wanted: &str) -> Result<String, ToolError> {
    let (name, section) = match wanted.split_once('#') {
        Some((name, section)) => (name.trim(), Some(section.trim())),
        None => (wanted, None),
    };
    let Some(skill) = index.get(name) else {
        return Err(ToolError::new(
            "unknown_skill",
            format!("There is no skill named {name}. Use a name from the catalog, or call skill with find=words."),
        )
        .did_you_mean(suggest::nearest(name, index.names(), 3)));
    };
    let (intro, sections) = skill.sections();
    if let Some(wanted) = section.filter(|section| !section.is_empty()) {
        let slug = skills::slug(wanted);
        let Some(found) = sections.iter().find(|section| section.slug == slug) else {
            let slugs: Vec<&str> = sections.iter().map(|section| section.slug.as_str()).collect();
            let near = suggest::nearest(&slug, slugs.iter().copied(), 3);
            return Err(ToolError::new(
                "unknown_section",
                format!("Skill {name} has no section {wanted}. Call skill with name \"{name}\" for its contents."),
            )
            .did_you_mean(if near.is_empty() {
                slugs.iter().map(|slug| format!("{name}#{slug}")).collect()
            } else {
                near.into_iter().map(|slug| format!("{name}#{slug}")).collect()
            }));
        };
        return Ok(format!("skill {name}#{}\n{}", found.slug, found.text.trim_end()));
    }
    let mut lines = vec![format!("skill {name}: {}", skill.title)];
    if skill.body.chars().count() < skills::WHOLE_BODY {
        lines.push(skill.body.trim_end().to_string());
        return Ok(lines.join("\n"));
    }
    if !intro.is_empty() {
        lines.push(intro);
    }
    lines.push("sections (load one with skill name=\"name#section\"):".to_string());
    for section in &sections {
        lines.push(format!(
            "- {name}#{}: {} ({} chars)",
            section.slug,
            section.title,
            section.text.chars().count()
        ));
    }
    Ok(lines.join("\n"))
}

crate::submit_tool!(SkillTool);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::skills::parse_skill;

    fn index() -> SkillIndex {
        let long = "x".repeat(skills::WHOLE_BODY);
        SkillIndex {
            skills: vec![
                parse_skill("---\nname: dates\ndescription: Use when a date is a phrase.\nkind: reference\n---\nPhrases.\n\n## Phrases\ntoday\n")
                    .unwrap(),
                parse_skill(&format!(
                    "---\nname: openui-lang\ndescription: Use when you write a canvas.\nkind: guide\n---\nCore rules.\n\n## Syntax\n{long}\n\n## Query\nQuery(\"request\")\n"
                ))
                .unwrap(),
            ],
            ..SkillIndex::default()
        }
    }

    #[test]
    fn a_small_skill_comes_whole_and_a_large_one_as_contents() {
        let index = index();
        let dates = load(&index, "dates").unwrap();
        assert!(
            dates.starts_with("skill dates: dates\nPhrases.") && dates.contains("## Phrases\ntoday"),
            "{dates}"
        );
        let guide = load(&index, "openui-lang").unwrap();
        assert!(guide.contains("Core rules."), "{guide}");
        assert!(guide.contains("- openui-lang#syntax: Syntax ("), "{guide}");
        assert!(guide.contains("- openui-lang#query: Query ("), "{guide}");
        assert!(!guide.contains("xxxxxxxxxx"), "the large body is not sent");
        let query = load(&index, "openui-lang#query").unwrap();
        assert_eq!(query, "skill openui-lang#query\n## Query\nQuery(\"request\")");
    }

    #[test]
    fn unknown_names_and_sections_name_the_nearest() {
        let index = index();
        let error = load(&index, "openui").unwrap_err().to_text();
        assert!(
            error.contains("\"unknown_skill\"") && error.contains("\"did_you_mean\":[\"openui-lang\"]"),
            "{error}"
        );
        let section = load(&index, "openui-lang#qeury").unwrap_err().to_text();
        assert!(section.contains("openui-lang#query"), "{section}");
        assert!(find(&index, "canvas").starts_with("- openui-lang: Use when you write a canvas. (guide)"));
        assert!(find(&index, "nothing here").starts_with("No skill matches"));
    }
}
