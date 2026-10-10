//! What a netlist is allowed to do before aispice hands it to a simulator.
//!
//! Netlists are written by people and by language models, and a model can be
//! steered by text it reads (a comment in a schematic, a datasheet, a log).
//! Simulators can do more than simulate: ngspice `.control` blocks run shell
//! commands, some versions load compiled model libraries, LTspice's `.ferret`
//! downloads files, `.include` reads any file and echoes bad lines into the
//! log the model then reads, and several dialects write output wherever the
//! deck says.
//!
//! So the policy is an allowlist. Only known dot-commands may appear; every
//! file a deck references, directly or through an include, must resolve
//! inside the project or a configured model folder (symlinks are followed
//! before the check); included files are checked by the same rules. Anything
//! not understood is refused rather than allowed. The policy runs on every
//! deck, whatever produced it.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Policy {
    /// Folder relative paths resolve against (the deck's folder).
    pub base_dir: PathBuf,
    /// Folders files may live in: the project, configured model folders,
    /// the simulator's own library, aispice's data folder. `base_dir` is
    /// always allowed.
    pub allowed_dirs: Vec<PathBuf>,
    /// ngspice/Xyce `.control` blocks. Off unless the user turns it on.
    pub allow_control_blocks: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    pub line: String,
    pub reason: String,
}

/// Dot-commands a deck may use. Anything else is refused.
const ALLOWED: &[&str] = &[
    ".ac",
    ".backanno",
    ".csparam",
    ".data",
    ".dc",
    ".disto",
    ".else",
    ".elseif",
    ".end",
    ".endd",
    ".enddata",
    ".endif",
    ".endl",
    ".endm",
    ".endmachine",
    ".ends",
    ".four",
    ".fourier",
    ".func",
    ".global",
    ".ic",
    ".if",
    ".inc",
    ".include",
    ".lib",
    ".machine",
    ".meas",
    ".measure",
    ".model",
    ".net",
    ".nodeset",
    ".noise",
    ".op",
    ".opt",
    ".option",
    ".options",
    ".param",
    ".params",
    ".plot",
    ".print",
    ".probe",
    ".pz",
    ".save",
    ".sens",
    ".step",
    ".subckt",
    ".temp",
    ".tf",
    ".title",
    ".tran",
    ".width",
];

/// Dot-commands whose first argument is a file to read.
const INCLUDES: &[&str] = &[".include", ".inc", ".lib"];

/// Includes are followed this deep.
const MAX_INCLUDE_DEPTH: usize = 8;

/// Check netlist text. An empty result means the deck may run.
pub fn check(text: &str, policy: &Policy) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    check_inner(text, policy, &policy.base_dir, true, 0, &mut seen, &mut out);
    let mut unique: Vec<Violation> = Vec::with_capacity(out.len());
    for v in out {
        if !unique.contains(&v) {
            unique.push(v);
        }
    }
    unique
}

fn check_inner(
    text: &str,
    policy: &Policy,
    base: &Path,
    has_title: bool,
    depth: usize,
    seen: &mut Vec<PathBuf>,
    out: &mut Vec<Violation>,
) {
    let mut logical: Vec<String> = Vec::new();
    for raw in text.split(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}']) {
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
    let skip = usize::from(has_title);
    for line in logical.iter().skip(skip) {
        let lower = line.to_ascii_lowercase();
        if lower.is_empty() || lower.starts_with('*') {
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
            continue;
        }
        if first.starts_with('.') && !ALLOWED.contains(&first) {
            out.push(Violation {
                line: line.clone(),
                reason: format!("`{first}` is not on aispice's list of allowed directives"),
            });
            continue;
        }
        if INCLUDES.contains(&first)
            && let Some(arg) = path_argument(line) {
                match resolve(&arg, base, policy) {
                    Err(reason) => out.push(Violation {
                        line: line.clone(),
                        reason,
                    }),
                    Ok(Some(file)) if depth < MAX_INCLUDE_DEPTH && !seen.contains(&file) => {
                        seen.push(file.clone());
                        if let Ok(bytes) = std::fs::read(&file) {
                            let (inc, _) = crate::encoding::decode(&bytes);
                            let inc_base = file
                                .parent()
                                .map(Path::to_path_buf)
                                .unwrap_or_else(|| base.to_path_buf());
                            let before = out.len();
                            check_inner(&inc, policy, &inc_base, false, depth + 1, seen, out);
                            for v in &mut out[before..] {
                                v.reason = format!("in {}: {}", file.display(), v.reason);
                            }
                        }
                    }
                    Ok(_) => {}
                }
            }
        // Any value that names a file, on any line: `file=...`, `wavefile=...`,
        // and anything containing a path separator.
        for value in path_like_values(line) {
            if let Err(reason) = resolve(&value, base, policy) {
                out.push(Violation {
                    line: line.clone(),
                    reason,
                });
            }
        }
    }
}

/// Check text without touching the file system: allowed directives only, no
/// absolute paths, no `..`. Used when a directive is added, before any
/// project folder is known; the full [`check`] still runs before simulation.
pub fn check_lexical(text: &str) -> Vec<Violation> {
    let policy = Policy {
        base_dir: PathBuf::from("/nonexistent/aispice-lexical"),
        allowed_dirs: Vec::new(),
        allow_control_blocks: false,
    };
    check(&format!("* directive\n{text}"), &policy)
}

/// The first argument after the directive keyword, unquoted.
fn path_argument(line: &str) -> Option<String> {
    let rest = line.split_once(char::is_whitespace)?.1.trim();
    for q in ['"', '\''] {
        if let Some(inner) = rest.strip_prefix(q) {
            return inner.split(q).next().map(str::to_string);
        }
    }
    rest.split_whitespace().next().map(str::to_string)
}

const FILE_KEYS: &[&str] = &[
    "file",
    "wavefile",
    "rawfile",
    "input_file",
    "output_file",
    "filename",
    "path",
    "lib",
    "library",
];

/// Values on a line that are, or look like, file paths.
fn path_like_values(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let tokens: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .filter(|t| !t.is_empty())
        .collect();
    for (i, t) in tokens.iter().enumerate() {
        let (key, value) = match t.split_once('=') {
            Some((k, "")) => (
                k.to_ascii_lowercase(),
                tokens.get(i + 1).copied().unwrap_or("").to_string(),
            ),
            Some((k, v)) => (k.to_ascii_lowercase(), v.to_string()),
            None => (String::new(), t.to_string()),
        };
        let value = value.trim_matches(|c| c == '"' || c == '\'').to_string();
        let looks_like_path = value.contains('/')
            || value.contains('\\')
            || value.starts_with('~')
            || is_drive_path(&value);
        if FILE_KEYS.contains(&key.as_str()) || (looks_like_path && !value.contains('{')) {
            out.push(value);
        }
    }
    out
}

fn is_drive_path(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// Resolve a referenced file. Ok(Some(path)) when it exists inside an allowed
/// folder, Ok(None) for a bare library name that does not exist next to the
/// deck (the simulator finds it on its own library path), Err otherwise.
fn resolve(path: &str, base: &Path, policy: &Policy) -> Result<Option<PathBuf>, String> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    let outside = || {
        Err(format!(
            "`{path}` is outside the project and the configured model folders"
        ))
    };
    if path.starts_with('~') || path.starts_with('\\') || is_drive_path(path) {
        return outside();
    }
    let p = Path::new(path);
    if p.components().any(|c| matches!(c, Component::ParentDir)) || path.contains("..\\") {
        return Err(format!(
            "`{path}` climbs out of its folder with `..`; reference files inside the project or a configured model folder"
        ));
    }
    let candidate = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    };
    match std::fs::canonicalize(&candidate) {
        Ok(real) => {
            let inside = std::iter::once(&policy.base_dir)
                .chain(policy.allowed_dirs.iter())
                .any(|d| {
                    let d = std::fs::canonicalize(d).unwrap_or_else(|_| d.clone());
                    real.starts_with(&d)
                });
            if inside { Ok(Some(real)) } else { outside() }
        }
        // Not there: an absolute path must still be lexically inside an
        // allowed folder; a bare relative name is a library-path lookup.
        Err(_) if p.is_absolute() => {
            let inside = std::iter::once(&policy.base_dir)
                .chain(policy.allowed_dirs.iter())
                .any(|d| p.starts_with(d));
            if inside { Ok(None) } else { outside() }
        }
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aispice-policy-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn policy_at(dir: &Path) -> Policy {
        Policy {
            base_dir: dir.to_path_buf(),
            allowed_dirs: vec![],
            allow_control_blocks: false,
        }
    }

    #[test]
    fn plain_decks_pass() {
        let d = temp("plain");
        let deck = "t\nR1 a 0 1k\n.lib opamp.sub\n.param r=1k\n.step param r list 1k 2k\n.meas tran v max V(a)\n.tran 1m\n.end\n";
        assert_eq!(check(deck, &policy_at(&d)), vec![]);
    }

    #[test]
    fn unknown_and_dangerous_directives_are_refused() {
        let d = temp("unknown");
        for deck in [
            "t\n.ferret http://example.com/x.lib\n",
            "t\n.osdi /tmp/x.osdi\n",
            "t\n.wave out.wav 16 44.1k V(a)\n",
            "t\n.control\nshell id\n.endc\n",
            "t\n.CONTROL\n.endc\n",
            "t\n.hdl x.va\n",
        ] {
            assert!(!check(deck, &policy_at(&d)).is_empty(), "{deck}");
        }
    }

    #[test]
    fn includes_outside_allowed_folders_are_refused() {
        let d = temp("outside");
        for deck in [
            "t\n.include /etc/passwd\n",
            "t\n.inc \"../../.ssh/id_rsa\"\n",
            "t\n.lib C:\\Users\\me\\secret.lib\n",
            "t\n.include ~/.aws/credentials\n",
            "t\nV1 a 0 PWL file=/home/someone/.netrc\n",
            "t\n.print tran file=/etc/cron.d/x V(a)\n",
            "t\nV1 a 0 PWL\n+ file=../escape.txt\n",
            "t\nA1 a d_src\n.model d_src d_source(input_file=\"/etc/shadow\")\n",
        ] {
            assert!(!check(deck, &policy_at(&d)).is_empty(), "{deck}");
        }
    }

    #[test]
    fn included_files_are_checked_too() {
        let d = temp("nested");
        std::fs::write(d.join("good.lib"), "* ok\n.model D1 D(Is=1n)\n").unwrap();
        std::fs::write(
            d.join("bad.lib"),
            "* sneaky\n.control\nshell curl evil\n.endc\n",
        )
        .unwrap();
        assert!(check("t\n.include good.lib\n", &policy_at(&d)).is_empty());
        let v = check("t\n.include bad.lib\n", &policy_at(&d));
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].reason.contains("bad.lib"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_out_of_the_project_are_refused() {
        let d = temp("symlink");
        let secret = temp("secret-target");
        std::fs::write(secret.join("key"), "secret").unwrap();
        let link = d.join("models");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        let v = check("t\n.include models/key\n", &policy_at(&d));
        assert_eq!(v.len(), 1, "{v:?}");
    }

    #[test]
    fn line_separators_cannot_hide_a_control_block() {
        let d = temp("sep");
        assert!(!check("t\nR1 a 0 1\r.control\rshell id\r.endc\n", &policy_at(&d)).is_empty());
        assert!(!check("t\nR1 a 0 1\u{2028}.control\u{2028}.endc\n", &policy_at(&d)).is_empty());
    }
}
