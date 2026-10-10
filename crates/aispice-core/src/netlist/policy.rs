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

/// First words that switch a deck into another simulator's own language or
/// run programs there (Spectre's `simulator lang=spectre`, `ahdl_include`,
/// `shell`, bare `include`). A SPICE deck has no business using them.
const FOREIGN: &[&str] = &[
    "simulator",
    "ahdl_include",
    "include",
    "shell",
    "library",
    "section",
    "endsection",
    "pre_osdi",
    "codemodel",
];

/// Included files larger than this are refused rather than read.
const MAX_INCLUDE_BYTES: u64 = 16 * 1024 * 1024;

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
    // C-based simulators stop reading a line at NUL, and other control
    // characters are handled differently by each reader. Either can make the
    // simulator see a line this check did not, so they are refused outright.
    if let Some(c) = text
        .chars()
        .find(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{85}'))
    {
        out.push(Violation {
            line: String::new(),
            reason: format!(
                "the deck contains a control character (U+{:04X}); remove it",
                c as u32
            ),
        });
        return;
    }
    let mut logical: Vec<String> = Vec::new();
    for raw in text.split(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}']) {
        let line = raw.trim();
        if let Some(cont) = line.strip_prefix('+')
            && let Some(last) = logical.last_mut()
        {
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
        if FOREIGN.contains(&first) {
            out.push(Violation { line: line.clone(), reason: format!("`{first}` belongs to another simulator's language and can load code or run programs; it is not allowed in a SPICE deck") });
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
            && let Some(arg) = path_argument(line)
        {
            match resolve(&arg, base, policy) {
                Err(reason) => out.push(Violation {
                    line: line.clone(),
                    reason,
                }),
                Ok(Some(file)) if !seen.contains(&file) => {
                    seen.push(file.clone());
                    if depth >= MAX_INCLUDE_DEPTH {
                        out.push(Violation {
                            line: line.clone(),
                            reason: format!("includes nest deeper than {MAX_INCLUDE_DEPTH} levels"),
                        });
                        continue;
                    }
                    let size = std::fs::metadata(&file)
                        .map(|m| m.len())
                        .unwrap_or(u64::MAX);
                    let bytes = if size <= MAX_INCLUDE_BYTES {
                        std::fs::read(&file).ok()
                    } else {
                        None
                    };
                    let Some(bytes) = bytes else {
                        out.push(Violation {
                                line: line.clone(),
                                reason: format!(
                                    "{} could not be read for checking (no permission, or larger than 16 MiB)",
                                    file.display()
                                ),
                            });
                        continue;
                    };
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
                Ok(_) => {}
            }
        }
        // Any value that names a file, on any line: `file=...`, `wavefile=...`,
        // and anything containing a path separator.
        if lower
            .split(|c: char| c.is_whitespace() || c == ',')
            .any(|t| t.starts_with("cmd=") || t == "cmd")
        {
            out.push(Violation {
                line: line.clone(),
                reason: "`cmd=` runs a program in some simulators; it is not allowed".into(),
            });
        }
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
    let is_sep = |c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')';
    // Tokens with the character just before each, so `V(out)/V(in)` can be
    // told apart from a path: there `/V` follows `)` and is a division.
    let mut tokens: Vec<(&str, Option<char>)> = Vec::new();
    let mut start = None;
    for (i, c) in line
        .char_indices()
        .chain(std::iter::once((line.len(), ' ')))
    {
        match (start, is_sep(c)) {
            (None, false) => start = Some(i),
            (Some(st), true) => {
                tokens.push((&line[st..i], line[..st].chars().next_back()));
                start = None;
            }
            _ => {}
        }
    }
    for (i, &(t, before)) in tokens.iter().enumerate() {
        let divides = before == Some(')') && is_division(t, line);
        let (key, value) = match t.split_once('=') {
            Some((k, "")) => (
                k.to_ascii_lowercase(),
                tokens.get(i + 1).map_or("", |&(t, _)| t).to_string(),
            ),
            Some((k, v)) => (k.to_ascii_lowercase(), v.to_string()),
            None => (String::new(), t.to_string()),
        };
        let value = value.trim_matches(|c| c == '"' || c == '\'').to_string();
        let looks_like_path = value.contains('/')
            || value.contains('\\')
            || value.starts_with('~')
            || is_drive_path(&value);
        if FILE_KEYS.contains(&key.as_str())
            || (looks_like_path && !divides && !value.contains('{'))
        {
            out.push(value);
        }
    }
    out
}

/// Whether a token that follows `)` is the rest of a division, as in the
/// `/V` of `V(out)/V(in)`, rather than a path. Deliberately narrow, so no
/// simulator could read it as a file: one slash, then a single number or a
/// plain identifier (no further slash, no backslash, no dot except in a
/// number), and never on a line that includes files.
fn is_division(token: &str, line: &str) -> bool {
    let first = line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if INCLUDES.contains(&first.as_str()) {
        return false;
    }
    // `{V(a)/2}` leaves `/2}`: the brace closes the expression.
    let Some(rest) = token.strip_prefix('/').map(|r| r.trim_end_matches('}')) else {
        return false;
    };
    let identifier = rest
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let number = rest
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '.')
        && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
        && rest.matches('.').count() <= 1;
    identifier || number
}

/// `C:\x`, `C:/x` and also drive-relative `C:x`, which Windows (and Wine)
/// resolve against the current folder of that drive.
fn is_drive_path(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
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
        // allowed folder. A bare file name is a lookup on the simulator's own
        // library path, which only holds allowed folders. A relative path with
        // folders in it must exist, so the simulator cannot resolve it
        // somewhere this check did not look.
        Err(_) if p.is_absolute() => {
            let inside = std::iter::once(&policy.base_dir)
                .chain(policy.allowed_dirs.iter())
                .any(|d| p.starts_with(d));
            if inside { Ok(None) } else { outside() }
        }
        Err(_) if path.contains(['/', '\\']) => Err(format!(
            "`{path}` does not exist in the project; reference an existing file or a bare library name"
        )),
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
    fn foreign_language_statements_and_control_chars_are_refused() {
        let d = temp("foreign");
        for deck in [
            "t\nsimulator lang=spectre\n",
            "t\nahdl_include \"x.va\"\n",
            "t\nshell1 shell cmd=\"id\"\n",
            "t\ninclude \"x.scs\"\n",
            "t\nR1 a 0 1k\u{0}\n",
            "t\nR1 a 0 1k\u{1b}[2J\n",
        ] {
            assert!(!check(deck, &policy_at(&d)).is_empty(), "{deck:?}");
        }
    }

    #[test]
    fn drive_relative_and_variable_paths_are_refused() {
        let d = temp("drive");
        for deck in [
            "t\n.lib C:secret.lib\n",
            "t\n.include $HOME/x.lib\n",
            "t\n.include %USERPROFILE%\\x.lib\n",
            "t\nV1 a 0 PWL file=D:data.txt\n",
        ] {
            assert!(!check(deck, &policy_at(&d)).is_empty(), "{deck}");
        }
    }

    #[test]
    fn missing_relative_paths_with_folders_are_refused() {
        let d = temp("missing");
        assert!(!check("t\n.include models/none.lib\n", &policy_at(&d)).is_empty());
        assert!(!check("t\n.lib sub\\none.lib\n", &policy_at(&d)).is_empty());
        assert!(check("t\n.lib none.lib\n", &policy_at(&d)).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_includes_fail_closed() {
        use std::os::unix::fs::PermissionsExt;
        let d = temp("unreadable");
        let f = d.join("locked.lib");
        std::fs::write(&f, ".control\n.endc\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable_anyway = std::fs::read(&f).is_ok(); // running as root
        let v = check("t\n.include locked.lib\n", &policy_at(&d));
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!v.is_empty() || readable_anyway);
    }

    #[test]
    fn line_separators_cannot_hide_a_control_block() {
        let d = temp("sep");
        assert!(!check("t\nR1 a 0 1\r.control\rshell id\r.endc\n", &policy_at(&d)).is_empty());
        assert!(!check("t\nR1 a 0 1\u{2028}.control\u{2028}.endc\n", &policy_at(&d)).is_empty());
    }

    /// Was a false positive: in `V(out)/V(in)` the tokenizer saw `/V` and
    /// refused it as an absolute path, so a live model could not write a
    /// gain measurement. Real file references must still be refused.
    #[test]
    fn division_in_expressions_is_not_a_path() {
        for ok in [
            ".meas AC gain FIND mag(V(out))/mag(V(in)) AT 1k",
            ".meas TRAN r PARAM V(a)/V(b)",
            "B1 x 0 V=V(a)/V(b)",
            "E1 x 0 value={V(a)/2}",
        ] {
            assert!(
                check_lexical(ok).is_empty(),
                "{ok}: {:?}",
                check_lexical(ok)
            );
        }
        for bad in [
            ".include /etc/passwd",
            "V1 a 0 wavefile=/etc/passwd",
            ".lib (/etc/passwd)",
            "V1 a 0 PWL file=/etc/x",
            // After `)`, still a path unless it is a plain identifier or number.
            ".meas TRAN r PARAM V(a)/etc/passwd",
            ".meas TRAN r PARAM V(a)/models.lib",
            ".meas TRAN r PARAM V(a)/..",
            ".inc x)/etc",
            "R1 a b R=V(a)/C:x",
        ] {
            assert!(!check_lexical(bad).is_empty(), "{bad} was allowed");
        }
    }
}
