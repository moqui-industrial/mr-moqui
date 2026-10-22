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

//! The skill index the `skill` tool reads (plans/LLM-A2A-Assist.md 4.9 and 7.5), as far as phase 13.3 needs it: the
//! Markdown files under each component's `skill/` directory (the framework's `skill/` first, then the components in
//! load order), read with their front matter (`name` and `description` required; `title`, `kind`, `aliases`,
//! `unlocks`, `requires`, `locked`) and split into `##` sections with slugs. A file that does not read is logged and
//! left out; a second skill of a name is logged and left out. Each component may also ship `skill/aliases.json`
//! (`{"term": ["alias", …]}`), the domain words `search` and retrieval add (the ERP terms come from Mantle's, not the
//! framework's).
//!
//! [`SkillIndex::load`] is the seam phase 13.6 replaces: the files synced into `AgentSkill` rows with personal and site
//! skills, the catalog and the weighted retrieval. Until then [`SkillIndex::find`] scores by name, title, aliases and
//! description words.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::context::ExecutionContextFactory;

/// A body under this size is returned whole by `skill(name)`.
pub const WHOLE_BODY: usize = 6_000;

/// One skill.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skill {
    pub name: String,
    pub title: String,
    pub description: String,
    /// procedure, reference or guide.
    pub kind: String,
    pub aliases: Vec<String>,
    pub unlocks: Vec<String>,
    pub requires: Vec<String>,
    pub locked: bool,
    /// The file it came from.
    pub location: String,
    /// The text after the front matter.
    pub body: String,
}

/// A `##` section of a skill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub slug: String,
    pub title: String,
    /// The section's text, its heading line included.
    pub text: String,
}

impl Skill {
    /// The text before the first `##` heading, and the sections.
    pub fn sections(&self) -> (String, Vec<Section>) {
        let mut intro = String::new();
        let mut sections: Vec<Section> = Vec::new();
        let mut fenced = false;
        for line in self.body.lines() {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
            }
            if !fenced && line.starts_with("## ") {
                let title = line[3..].trim().to_string();
                sections.push(Section {
                    slug: slug(&title),
                    title,
                    text: String::new(),
                });
            }
            let target = match sections.last_mut() {
                Some(section) => &mut section.text,
                None => &mut intro,
            };
            target.push_str(line);
            target.push('\n');
        }
        (intro.trim().to_string(), sections)
    }
}

/// A heading as a slug: lower case words joined by `-`.
pub fn slug(title: &str) -> String {
    title
        .to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// The skills a run may load and the domain aliases of the components.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkillIndex {
    pub skills: Vec<Skill>,
    /// A term and the words that mean it.
    pub aliases: BTreeMap<String, Vec<String>>,
}

impl SkillIndex {
    /// The skills of the factory's components (the framework's first).
    pub fn load(factory: &ExecutionContextFactory) -> Self {
        let mut dirs: Vec<PathBuf> = vec![factory.framework_dir().join("skill")];
        for component in factory.components() {
            let dir = component.location.join("skill");
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        Self::from_dirs(&dirs)
    }

    /// The skills of these `skill/` directories, in order.
    pub fn from_dirs(dirs: &[PathBuf]) -> Self {
        let mut index = Self::default();
        for dir in dirs {
            if !dir.is_dir() {
                continue;
            }
            let mut files = Vec::new();
            markdown_files(dir, &mut files);
            files.sort();
            for file in files {
                match read_skill(&file) {
                    Ok(skill) if index.skills.iter().any(|known| known.name == skill.name) => tracing::error!(
                        "skill {} in {} is already defined, so it is left out",
                        skill.name,
                        file.display()
                    ),
                    Ok(skill) => index.skills.push(skill),
                    Err(problem) => tracing::error!("skill file {} is left out: {problem}", file.display()),
                }
            }
            let aliases = dir.join("aliases.json");
            if aliases.is_file() {
                match std::fs::read_to_string(&aliases)
                    .map_err(|error| error.to_string())
                    .and_then(|text| {
                        serde_json::from_str::<BTreeMap<String, Vec<String>>>(&text).map_err(|error| error.to_string())
                    }) {
                    Ok(found) => {
                        for (term, words) in found {
                            let known = index.aliases.entry(term.to_lowercase()).or_default();
                            for word in words {
                                let word = word.to_lowercase();
                                if !known.contains(&word) {
                                    known.push(word);
                                }
                            }
                        }
                    }
                    Err(problem) => tracing::error!("skill aliases {} are left out: {problem}", aliases.display()),
                }
            }
        }
        index
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|skill| skill.name == name.trim())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.skills.iter().map(|skill| skill.name.as_str())
    }

    /// The words a query word also stands for: its aliases, and the terms it is an alias of.
    pub fn alias_words(&self, word: &str) -> Vec<String> {
        let word = word.to_lowercase();
        let mut out: Vec<String> = self.aliases.get(&word).cloned().unwrap_or_default();
        for (term, words) in &self.aliases {
            if words.contains(&word) && !out.contains(term) {
                out.push(term.clone());
            }
        }
        out
    }

    /// The skills that match words, best first: a name or title word 100, a whole phrase in the title 40, an alias 20,
    /// a description word 8.
    pub fn find(&self, text: &str) -> Vec<&Skill> {
        let words: Vec<String> = text
            .to_lowercase()
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|word| word.len() > 1)
            .map(str::to_string)
            .collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(i64, &Skill)> = Vec::new();
        for skill in &self.skills {
            let named: Vec<String> = format!("{} {}", skill.name, skill.title)
                .to_lowercase()
                .split(|ch: char| !ch.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .map(str::to_string)
                .collect();
            let described = skill.description.to_lowercase();
            let mut score = 0;
            for word in &words {
                if named.iter().any(|known| known.starts_with(word.as_str())) {
                    score += 100;
                } else if skill.aliases.iter().any(|alias| alias.eq_ignore_ascii_case(word)) {
                    score += 20;
                } else if described.contains(word.as_str()) {
                    score += 8;
                }
            }
            if words.len() > 1 && skill.title.to_lowercase().contains(&text.to_lowercase()) {
                score += 40;
            }
            if score > 0 {
                scored.push((score, skill));
            }
        }
        scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.name.cmp(&right.1.name)));
        scored.into_iter().map(|(_, skill)| skill).collect()
    }
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        tracing::error!("skill directory {} could not be read", dir.display());
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

fn read_skill(file: &Path) -> Result<Skill, String> {
    let text = std::fs::read_to_string(file).map_err(|error| error.to_string())?;
    let mut skill = parse_skill(&text)?;
    skill.location = file.display().to_string();
    Ok(skill)
}

/// A skill from its Markdown: the front matter between `---` lines, then the body.
///
/// # Errors
///
/// Returns why it is not a skill: no front matter, no name, a name that is not kebab-case, no description.
pub fn parse_skill(text: &str) -> Result<Skill, String> {
    let text = text.trim_start_matches('\u{feff}');
    let rest = text
        .strip_prefix("---")
        .ok_or("it does not start with front matter (---)")?;
    let end = rest.find("\n---").ok_or("its front matter has no closing ---")?;
    let (front, body) = rest.split_at(end);
    let body = body[4..].trim_start_matches(['-']).trim_start_matches(['\r', '\n']);
    let mut skill = Skill {
        body: body.to_string(),
        ..Skill::default()
    };
    for line in front.lines() {
        let line = match line.find(" #") {
            Some(at) => &line[..at],
            None => line,
        };
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']).to_string();
        let list = || -> Vec<String> {
            value
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|item| item.trim().trim_matches(['"', '\'']).to_string())
                .filter(|item| !item.is_empty())
                .collect()
        };
        match key.trim() {
            "name" => skill.name = value.clone(),
            "title" => skill.title = value.clone(),
            "description" | "summary" => skill.description = value.clone(),
            "kind" => skill.kind = value.clone(),
            "aliases" => skill.aliases = list(),
            "unlocks" => skill.unlocks = list(),
            "requires" => skill.requires = list(),
            "locked" => skill.locked = value == "true",
            _ => {}
        }
    }
    if skill.name.is_empty() {
        return Err("its front matter has no name".to_string());
    }
    if !skill
        .name
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(format!("its name {} is not kebab-case", skill.name));
    }
    if skill.description.is_empty() {
        return Err(format!("skill {} has no description", skill.name));
    }
    if skill.title.is_empty() {
        skill.title = skill.name.replace('-', " ");
    }
    Ok(skill)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL: &str = "---\nname: data-reads   # required\ntitle: Reading lists\ndescription: Use when reading rows.\n\
        kind: guide\naliases: [rows, table]\n---\nRead lists with data.\n\n## Where\nfield values.\n```\n## not a heading\n```\n\n## Artifacts and grep\nartifact=A1\n";

    #[test]
    fn a_skill_reads_its_front_matter_and_sections() {
        let skill = parse_skill(SKILL).unwrap();
        assert_eq!(skill.name, "data-reads");
        assert_eq!(skill.title, "Reading lists");
        assert_eq!(skill.kind, "guide");
        assert_eq!(skill.aliases, ["rows", "table"]);
        let (intro, sections) = skill.sections();
        assert_eq!(intro, "Read lists with data.");
        let slugs: Vec<&str> = sections.iter().map(|section| section.slug.as_str()).collect();
        assert_eq!(slugs, ["where", "artifacts-and-grep"]);
        assert!(sections[0].text.contains("## not a heading"));
        assert!(parse_skill("no front matter").is_err());
        assert!(parse_skill("---\nname: Bad Name\ndescription: x\n---\n").is_err());
        assert!(parse_skill("---\nname: no-description\n---\n").is_err());
    }

    #[test]
    fn the_index_reads_skill_dirs_finds_by_words_and_reads_aliases() {
        let dir = std::env::temp_dir().join(format!("moqui-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("more")).unwrap();
        std::fs::write(dir.join("data-reads.md"), SKILL).unwrap();
        std::fs::write(
            dir.join("more/dates.md"),
            "---\nname: dates\ndescription: Use when a date is a phrase.\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(dir.join("broken.md"), "no front matter").unwrap();
        std::fs::write(dir.join("aliases.json"), r#"{"customer": ["client", "buyer"]}"#).unwrap();
        let index = SkillIndex::from_dirs(&[dir.clone(), dir.join("missing")]);
        let _ = std::fs::remove_dir_all(&dir);
        let names: Vec<&str> = index.names().collect();
        assert_eq!(names, ["data-reads", "dates"]);
        assert_eq!(index.find("reading rows")[0].name, "data-reads");
        assert_eq!(index.find("date phrase")[0].name, "dates");
        assert!(index.find("zz").is_empty());
        assert_eq!(index.alias_words("Customer"), ["client", "buyer"]);
        assert_eq!(index.alias_words("buyer"), ["customer"]);
    }
}
