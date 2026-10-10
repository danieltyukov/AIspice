//! What a netlist is allowed to do before aispice hands it to a simulator.
//!
//! Netlists are written by people and by language models, and a model can be
//! steered by text it reads (a comment in a schematic, a datasheet, a log).
//! Simulators can do more than simulate: ngspice `.control` blocks run shell
//! commands and write files, `.include` reads any file and echoes bad lines
//! into the log the model then reads, and several dialects write output to a
//! path of the netlist's choosing. This policy is checked on every deck before
//! it runs, whatever produced it.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Policy {
    /// Folder the deck runs in; relative paths resolve against it.
    pub base_dir: PathBuf,
    /// Folders absolute paths may point into: the project, configured model
    /// folders, the simulator's own library, aispice's data folder.
    pub allowed_dirs: Vec<PathBuf>,
    /// ngspice/Xyce `.control` blocks. Off unless the user turns it on.
    pub allow_control_blocks: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    pub line: String,
    pub reason: String,
}

/// Directives whose first argument is a file path.
const FILE_DIRECTIVES: &[&str] = &[
    ".include",
    ".inc",
    ".lib",
    ".libfile",
    ".wave",
    ".loadbias",
    ".savebias",
];
/// `key=path` parameters that name files, on any line.
const FILE_PARAMS: &[&str] = &[
    "file",
    "wavefile",
    "rawfile",
    "input_file",
    "output_file",
    "filename",
];

/// Check netlist text. Returns every violation; an empty list means it may run.
pub fn check(text: &str, policy: &Policy) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut logical: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(cont) = line.strip_prefix('+')
            && let Some(last) = logical.last_mut() {
                last.push(' ');
                last.push_str(cont.trim());
                continue;
            }
        logical.push(line.to_string());
    }
    let mut in_control = false;
    for line in logical.iter().skip(1) {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with('*') || lower.is_empty() {
            continue;
        }
        let first = lower.split_whitespace().next().unwrap_or("");
        if first == ".control" {
            in_control = true;
            if !policy.allow_control_blocks {
                out.push(Violation { line: line.clone(), reason: "`.control` blocks can run shell commands and write files; they are off unless enabled in settings".into() });
            }
            continue;
        }
        if first == ".endc" {
            in_control = false;
            continue;
        }
        if in_control {
            // Already reported once for the block.
            continue;
        }
        if FILE_DIRECTIVES.contains(&first)
            && let Some(arg) = path_argument(line)
                && let Err(reason) = path_allowed(&arg, policy) {
                    out.push(Violation {
                        line: line.clone(),
                        reason,
                    });
                }
        for (key, value) in key_values(line) {
            if FILE_PARAMS.contains(&key.as_str())
                && let Err(reason) = path_allowed(&value, policy) {
                    out.push(Violation {
                        line: line.clone(),
                        reason,
                    });
                }
        }
    }
    out
}

/// The first argument after the directive keyword, unquoted. For `.lib` with
/// a section (`.lib models.lib tt`) that is still the file.
fn path_argument(line: &str) -> Option<String> {
    let rest = line.split_once(char::is_whitespace)?.1.trim();
    if let Some(q) = rest.strip_prefix('"') {
        return q.split('"').next().map(str::to_string);
    }
    if let Some(q) = rest.strip_prefix('\'') {
        return q.split('\'').next().map(str::to_string);
    }
    rest.split_whitespace().next().map(str::to_string)
}

fn key_values(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while let Some(eq) = line[i..].find('=') {
        let eq = i + eq;
        let key_start = line[..eq]
            .trim_end()
            .rfind(|c: char| c.is_whitespace() || c == '(' || c == ',')
            .map(|p| p + 1)
            .unwrap_or(0);
        let key = line[key_start..eq].trim().to_ascii_lowercase();
        let mut j = eq + 1;
        while j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        let value = if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
            let q = bytes[j] as char;
            line[j + 1..].split(q).next().unwrap_or("").to_string()
        } else {
            line[j..]
                .split(|c: char| c.is_whitespace() || c == ')' || c == ',')
                .next()
                .unwrap_or("")
                .to_string()
        };
        out.push((key, value));
        i = eq + 1;
    }
    out
}

/// A path may be relative without `..` (it resolves inside the deck folder or
/// the simulator's library search path), or absolute inside an allowed folder.
fn path_allowed(path: &str, policy: &Policy) -> Result<(), String> {
    let p = Path::new(path.trim());
    if path.trim().is_empty() {
        return Ok(());
    }
    let windows_abs =
        path.len() > 2 && path.as_bytes()[1] == b':' && path.as_bytes()[0].is_ascii_alphabetic();
    if p.components().any(|c| matches!(c, Component::ParentDir)) || path.contains("..\\") {
        return Err(format!(
            "`{path}` climbs out of the folder with `..`; reference files inside the project or a configured model folder"
        ));
    }
    if !p.is_absolute() && !windows_abs && !path.starts_with('\\') && !path.starts_with('~') {
        return Ok(());
    }
    if windows_abs || path.starts_with('\\') || path.starts_with('~') {
        return Err(format!(
            "`{path}` is outside the project and the configured model folders"
        ));
    }
    let resolved = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let inside = policy
        .allowed_dirs
        .iter()
        .chain(std::iter::once(&policy.base_dir))
        .any(|d| {
            let d = std::fs::canonicalize(d).unwrap_or_else(|_| d.clone());
            resolved.starts_with(&d)
        });
    if inside {
        Ok(())
    } else {
        Err(format!(
            "`{path}` is outside the project and the configured model folders"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy {
            base_dir: std::env::temp_dir(),
            allowed_dirs: vec![std::env::temp_dir()],
            allow_control_blocks: false,
        }
    }

    #[test]
    fn plain_decks_pass() {
        let deck = "t\nR1 a 0 1k\n.lib opamp.sub\n.include models/2n3904.mod\n.tran 1m\n.end\n";
        assert!(check(deck, &policy()).is_empty());
    }

    #[test]
    fn control_blocks_are_refused_by_default() {
        let deck = "t\nR1 a 0 1k\n.control\nshell rm -rf ~\n.endc\n.end\n";
        let v = check(deck, &policy());
        assert_eq!(v.len(), 1);
        assert!(v[0].reason.contains(".control"));
        let mut p = policy();
        p.allow_control_blocks = true;
        assert!(check(deck, &p).is_empty());
    }

    #[test]
    fn includes_outside_allowed_folders_are_refused() {
        for deck in [
            "t\n.include /etc/passwd\n",
            "t\n.inc \"../../.ssh/id_rsa\"\n",
            "t\n.lib C:\\Users\\me\\secret.lib\n",
            "t\n.include ~/.aws/credentials\n",
            "t\nV1 a 0 PWL file=/home/someone/.netrc\n",
            "t\n.print tran file=/etc/cron.d/x V(a)\n",
            "t\nV1 a 0 PWL\n+ file=../escape.txt\n",
        ] {
            assert_eq!(check(deck, &policy()).len(), 1, "{deck}");
        }
    }

    #[test]
    fn absolute_paths_inside_allowed_folders_pass() {
        let dir = std::env::temp_dir().join("aispice-policy-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("m.lib");
        std::fs::write(&file, "* models\n").unwrap();
        let deck = format!("t\n.lib {}\n", file.display());
        assert!(check(&deck, &policy()).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
