//! Agent skills: folders of instructions an agent can read.
//!
//! A skill is untrusted content, so this module does three things and nothing
//! else: it validates what is on disk, it never executes anything a skill
//! contains, and every path it builds is proven to stay inside the skills
//! directory. Installing a skill copies files; removing one deletes only a
//! validated child of that directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::{DwError, DwResult, ErrorCode};
use crate::store::Store;

const MANIFEST: &str = "SKILL.md";
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 5 * 1024 * 1024;
const MAX_FILES: usize = 60;
const DISABLED_KEY: &str = "agent.skills.disabled";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: String,
    pub enabled: bool,
    pub files: usize,
    pub bytes: u64,
    /// Present when the folder is there but its manifest is not usable. The
    /// skill is listed so the user can see and fix it, not silently ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid: Option<String>,
}

fn skills_root(data_dir: &Path) -> PathBuf {
    data_dir.join("skills")
}

/// Folder names are ids. Anything that could escape the directory is rejected
/// before a path is built.
pub fn validate_id(id: &str) -> DwResult<String> {
    let trimmed = id.trim();
    let valid = !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if valid {
        Ok(trimmed.to_string())
    } else {
        Err(DwError::new(
            ErrorCode::InvalidPath,
            format!("“{id}” is not a valid skill id."),
        )
        .detail("A skill id uses lowercase letters, digits, dashes and underscores."))
    }
}

fn disabled_ids(store: &Store) -> Vec<String> {
    store
        .setting(DISABLED_KEY)
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn set_enabled(data_dir: &Path, store: &Store, id: &str, enabled: bool) -> DwResult<()> {
    let id = validate_id(id)?;
    if !skills_root(data_dir).join(&id).is_dir() {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            format!("There is no skill called {id}."),
        ));
    }
    let mut disabled = disabled_ids(store);
    disabled.retain(|value| value != &id);
    if !enabled {
        disabled.push(id);
    }
    store.set_setting(DISABLED_KEY, &disabled.join(","))
}

pub fn discover(data_dir: &Path, store: &Store) -> DwResult<Vec<SkillInfo>> {
    let root = skills_root(data_dir);
    let disabled = disabled_ids(store);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut skills = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        if !entry.path().is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if validate_id(&id).is_err() {
            continue;
        }
        skills.push(read_skill(data_dir, &id, &disabled));
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(skills)
}

fn read_skill(data_dir: &Path, id: &str, disabled: &[String]) -> SkillInfo {
    let dir = skills_root(data_dir).join(id);
    let (files, bytes) = tree_size(&dir).unwrap_or((0, 0));
    let mut info = SkillInfo {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        path: dir.display().to_string(),
        enabled: !disabled.iter().any(|value| value == id),
        files,
        bytes,
        invalid: None,
    };
    match std::fs::read_to_string(dir.join(MANIFEST)) {
        Ok(text) => match parse_manifest(&text) {
            Ok((name, description)) => {
                info.name = name;
                info.description = description;
            }
            Err(reason) => info.invalid = Some(reason.summary),
        },
        Err(_) => info.invalid = Some(format!("{MANIFEST} is missing or unreadable.")),
    }
    info
}

/// Read one skill's instructions for an agent. Bounded, and confined to the
/// skills directory by construction.
pub fn read_instructions(data_dir: &Path, id: &str) -> DwResult<String> {
    let id = validate_id(id)?;
    let path = skills_root(data_dir).join(&id).join(MANIFEST);
    let meta = std::fs::metadata(&path).map_err(|e| {
        DwError::io(
            ErrorCode::InvalidPath,
            format!("There is no skill called {id}."),
            &e,
        )
    })?;
    if meta.len() > MAX_MANIFEST_BYTES {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            format!("{id}'s instructions are larger than a skill may be."),
        ));
    }
    std::fs::read_to_string(&path).map_err(|e| {
        DwError::io(
            ErrorCode::BackendFailure,
            format!("{id}'s instructions could not be read."),
            &e,
        )
    })
}

/// Copy a folder the user chose into the skills directory.
pub fn install(data_dir: &Path, source: &Path) -> DwResult<SkillInfo> {
    if !source.is_dir() {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            "Choose a folder that contains a SKILL.md file.",
        )
        .detail(format!("{} is not a folder.", source.display())));
    }
    if !source.join(MANIFEST).is_file() {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            "That folder has no SKILL.md, so it is not a skill.",
        ));
    }
    let text = std::fs::read_to_string(source.join(MANIFEST)).unwrap_or_default();
    parse_manifest(&text)?;

    let raw_id = source
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let id = validate_id(&raw_id.to_ascii_lowercase().replace(' ', "-"))?;
    let root = skills_root(data_dir);
    std::fs::create_dir_all(&root).map_err(|e| {
        DwError::io(
            ErrorCode::StorageFailure,
            "Cannot create the skills folder.",
            &e,
        )
    })?;
    let target = root.join(&id);
    if target.exists() {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            format!("A skill called {id} is already installed."),
        )
        .detail("Remove it first if you want to install this version."));
    }

    // Copy into a staging folder, then rename: a failed copy cannot leave a
    // half-installed skill that looks usable.
    let staging = root.join(format!(".{id}.staging"));
    let _ = std::fs::remove_dir_all(&staging);
    let copied = copy_tree(source, &staging, 0, &mut 0u64, &mut 0usize);
    if let Err(err) = copied {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }
    std::fs::rename(&staging, &target).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staging);
        DwError::io(
            ErrorCode::StorageFailure,
            "The skill could not be installed.",
            &e,
        )
    })?;
    Ok(read_skill(data_dir, &id, &[]))
}

fn copy_tree(
    from: &Path,
    to: &Path,
    depth: usize,
    total: &mut u64,
    files: &mut usize,
) -> DwResult<()> {
    if depth > 4 {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "That skill has folders nested deeper than a skill may have.",
        ));
    }
    std::fs::create_dir_all(to).map_err(|e| {
        DwError::io(
            ErrorCode::StorageFailure,
            "The skill could not be copied.",
            &e,
        )
    })?;
    for entry in std::fs::read_dir(from)
        .map_err(|e| {
            DwError::io(
                ErrorCode::InvalidPath,
                "The skill folder could not be read.",
                &e,
            )
        })?
        .filter_map(Result::ok)
    {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| {
            DwError::io(
                ErrorCode::InvalidPath,
                "A file in that skill could not be read.",
                &e,
            )
        })?;
        if meta.file_type().is_symlink() {
            // A link can point anywhere; a skill is copied, not followed.
            continue;
        }
        *files += 1;
        if *files > MAX_FILES {
            return Err(DwError::new(
                ErrorCode::InvalidPackage,
                format!("That skill has more than {MAX_FILES} files."),
            ));
        }
        let target = to.join(entry.file_name());
        if meta.is_dir() {
            copy_tree(&path, &target, depth + 1, total, files)?;
            continue;
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(DwError::new(
                ErrorCode::InvalidPackage,
                format!(
                    "{} is larger than 1 MB.",
                    entry.file_name().to_string_lossy()
                ),
            ));
        }
        *total += meta.len();
        if *total > MAX_TOTAL_BYTES {
            return Err(DwError::new(
                ErrorCode::InvalidPackage,
                "That skill is larger than 5 MB.",
            ));
        }
        std::fs::copy(&path, &target).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "A file in that skill could not be copied.",
                &e,
            )
        })?;
    }
    Ok(())
}

pub fn remove(data_dir: &Path, store: &Store, id: &str) -> DwResult<()> {
    let id = validate_id(id)?;
    let root = skills_root(data_dir);
    let target = root.join(&id);
    let canonical_root = root.canonicalize().map_err(|e| {
        DwError::io(
            ErrorCode::InvalidPath,
            "The skills folder is not available.",
            &e,
        )
    })?;
    let canonical_target = target.canonicalize().map_err(|e| {
        DwError::io(
            ErrorCode::InvalidPath,
            format!("There is no skill called {id}."),
            &e,
        )
    })?;
    if !canonical_target.starts_with(&canonical_root) || canonical_target == canonical_root {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            "That path is not a skill folder.",
        ));
    }
    std::fs::remove_dir_all(&canonical_target).map_err(|e| {
        DwError::io(
            ErrorCode::StorageFailure,
            "The skill folder could not be removed.",
            &e,
        )
    })?;
    let mut disabled = disabled_ids(store);
    disabled.retain(|value| value != &id);
    let _ = store.set_setting(DISABLED_KEY, &disabled.join(","));
    Ok(())
}

fn tree_size(dir: &Path) -> Option<(usize, u64)> {
    let mut files = 0;
    let mut bytes = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).ok()?.filter_map(Result::ok) {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    Some((files, bytes))
}

/// Parse the frontmatter this project's skills use: a `---` block with `name`
/// and `description`. `>` and `|` folded values are supported; anything else is
/// treated as a plain value.
pub fn parse_manifest(text: &str) -> DwResult<(String, String)> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "SKILL.md must start with a frontmatter block.",
        ));
    }
    let mut name: Option<String> = None;
    let mut description = String::new();
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented {
            if !description.is_empty() {
                description.push(' ');
            }
            description.push_str(line.trim());
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "name" => {
                name = Some(
                    value
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string(),
                )
            }
            "description" => {
                let value = value.trim();
                // `>` and `|` mean the value is folded onto the indented lines
                // below, which the branch above already appends.
                if value != ">" && value != "|" && value != ">-" && value != "|-" {
                    description = value.trim_matches('"').trim_matches('\'').to_string();
                }
            }
            _ => {}
        }
    }
    let name = name
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            DwError::new(
                ErrorCode::InvalidPackage,
                "SKILL.md must name the skill in its frontmatter.",
            )
        })?;
    Ok((name, description.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("droidwin-skills-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn frontmatter_is_read_with_name_and_folded_description() {
        let text = "---\nname: ui-testing\ndescription: >\n  Launch an app and\n  verify its screen.\n---\n\nBody text\n";
        let (name, description) = parse_manifest(text).unwrap();
        assert_eq!(name, "ui-testing");
        assert_eq!(description, "Launch an app and verify its screen.");
    }

    #[test]
    fn a_manifest_without_a_name_is_rejected() {
        assert!(parse_manifest("---\ndescription: x\n---\n").is_err());
        assert!(parse_manifest("no frontmatter").is_err());
    }

    #[test]
    fn ids_cannot_escape_the_skills_directory() {
        for bad in ["../evil", "a/b", "..", "", "Upper", "with space", "dots."] {
            assert!(validate_id(bad).is_err(), "{bad} should be rejected");
        }
        assert_eq!(validate_id("ui-testing_1").unwrap(), "ui-testing_1");
    }

    #[test]
    fn a_skill_installs_reads_and_removes_only_itself() {
        let data = temp_dir("install");
        let source = data.join("source").join("demo-skill");
        std::fs::create_dir_all(source.join("examples")).unwrap();
        std::fs::write(
            source.join("SKILL.md"),
            "---\nname: demo\ndescription: A demo skill.\n---\nUse it.\n",
        )
        .unwrap();
        std::fs::write(source.join("examples").join("flow.txt"), "step").unwrap();

        let store = Store::open_in_memory().unwrap();
        let installed = install(&data, &source).unwrap();
        assert_eq!(installed.id, "demo-skill");
        assert_eq!(installed.name, "demo");
        assert!(installed.enabled);
        assert_eq!(installed.files, 2);

        let instructions = read_instructions(&data, "demo-skill").unwrap();
        assert!(instructions.contains("Use it."));

        set_enabled(&data, &store, "demo-skill", false).unwrap();
        let listed = discover(&data, &store).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].enabled);

        remove(&data, &store, "demo-skill").unwrap();
        assert!(discover(&data, &store).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn installing_the_same_skill_twice_is_refused_instead_of_overwritten() {
        let data = temp_dir("duplicate");
        let source = data.join("kit").join("twice");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "---\nname: twice\n---\n").unwrap();
        install(&data, &source).unwrap();
        assert!(install(&data, &source).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_folder_without_a_manifest_is_not_installable() {
        let data = temp_dir("nomanifest");
        let source = data.join("kit").join("empty");
        std::fs::create_dir_all(&source).unwrap();
        assert!(install(&data, &source).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_broken_skill_is_listed_with_its_problem_instead_of_hidden() {
        let data = temp_dir("invalid");
        let dir = skills_root(&data).join("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "no frontmatter here").unwrap();
        let store = Store::open_in_memory().unwrap();
        let listed = discover(&data, &store).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].invalid.is_some());
        let _ = std::fs::remove_dir_all(&data);
    }
}
