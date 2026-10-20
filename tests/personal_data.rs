//! No tracked file holds personal data: a mail address outside the reserved example domains, a person's home
//! path, a telephone number, a token or a private network address (`PERSONAL-DATA.md`). A finding names the
//! file, the line and the kind, never the value. The check carries the patterns alone; no list of the values it
//! forbids stands in the repository.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use regex::{Captures, Regex};

/// The home folders a neutral example writes, so a person's own is not among them.
const NEUTRAL_HOME: [&str; 5] = ["user", "tester", "qa-admin", "placeholder", "example"];

fn root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every file git tracks or would track: the committed ones and the new ones it does not ignore.
fn tracked() -> Vec<String> {
  let output = Command::new("git")
    .args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
    .current_dir(root())
    .stdin(Stdio::null())
    .output()
    .expect("git lists the files");

  assert!(
    output.status.success(),
    "git ls-files failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );

  String::from_utf8(output.stdout)
    .expect("the paths are text")
    .split('\0')
    .filter(|path| !path.is_empty() && root().join(path).is_file())
    .map(str::to_owned)
    .collect()
}

/// The tracked files that read as text, with their text.
fn tracked_texts() -> Vec<(String, String)> {
  tracked()
    .into_iter()
    .filter_map(|path| std::fs::read_to_string(root().join(&path)).ok().map(|text| (path, text)))
    .collect()
}

/// One kind of personal data: the name a finding carries and the pattern that finds it.
struct Rule {
  kind: &'static str,
  pattern: Regex,
  /// Whether a match is a neutral form the rule leaves alone.
  allowed: fn(&Captures<'_>) -> bool,
}

/// The five kinds this check looks for.
fn rules() -> [Rule; 5] {
  [
    Rule {
      kind: "mail address",
      pattern: Regex::new(concat!(r"\b[A-Za-z0-9._%+-]+@", r"[A-Za-z][A-Za-z0-9.-]*\.[A-Za-z]{2,}\b"))
        .expect("the mail pattern reads"),
      allowed: |captures| captures.get(0).is_some_and(|address| is_example_address(address.as_str())),
    },
    Rule {
      kind: "home path",
      pattern: Regex::new(r"(?:/home/|/Users/|[A-Za-z]:\\Users\\)([A-Za-z0-9._-]+)")
        .expect("the home path pattern reads"),
      allowed: |captures| captures.get(1).is_some_and(|folder| is_neutral_home(folder.as_str())),
    },
    Rule {
      kind: "telephone number",
      pattern: Regex::new(concat!(
        r"\b01[016789][-. ]?\d{3,4}[-. ]?\d{4}\b|",
        r"\+\d{1,3}[-. ]?\d{2,4}[-. ]?\d{3,4}[-. ]?\d{4}\b|",
        r"\+\d{9,15}\b"
      ))
      .expect("the telephone pattern reads"),
      allowed: |_| false,
    },
    Rule {
      kind: "token",
      pattern: Regex::new(concat!(
        r"gh[pousr]_[A-Za-z0-9]{20,}|",
        r"github_pat_[A-Za-z0-9_]{20,}|",
        r"sk-[A-Za-z0-9_-]{20,}|",
        r"xox[baprs]-[A-Za-z0-9-]{10,}|",
        r"AKIA[0-9A-Z]{16}|",
        r"AIza[0-9A-Za-z_-]{35}"
      ))
      .expect("the token pattern reads"),
      allowed: |_| false,
    },
    Rule {
      kind: "private network address",
      pattern: Regex::new(concat!(
        r"(?i)\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3}|",
        r"192\.168\.\d{1,3}\.\d{1,3}|",
        r"172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3})\b|",
        r"\b(?:f[cd][0-9a-f]{2}|fe80):"
      ))
      .expect("the private address pattern reads"),
      allowed: |_| false,
    },
  ]
}

/// Whether a mail address stands at a reserved example domain.
fn is_example_address(address: &str) -> bool {
  address.rsplit_once('@').is_some_and(|(_, domain)| is_example_domain(domain))
}

/// Whether a domain is the reserved example space, where a neutral example address stands.
fn is_example_domain(domain: &str) -> bool {
  let domain = domain.to_lowercase();

  ["example.com", "example.net", "example.org"]
    .iter()
    .any(|example| domain == *example || domain.ends_with(&format!(".{example}")))
    || [".example", ".test", ".invalid", ".localhost"]
      .iter()
      .any(|suffix| domain.ends_with(suffix))
}

/// Whether a home path's own folder is a neutral example.
fn is_neutral_home(folder: &str) -> bool {
  NEUTRAL_HOME.contains(&folder) || folder.starts_with("agent-")
}

/// The line a byte offset falls on, counting from one.
fn line_at(text: &str, offset: usize) -> usize {
  text[..offset].bytes().filter(|byte| *byte == b'\n').count() + 1
}

/// Every finding in one file, each naming the file, the line and the kind.
fn findings(path: &str, text: &str, rules: &[Rule]) -> Vec<String> {
  let mut found = Vec::new();

  for rule in rules {
    for captures in rule.pattern.captures_iter(text) {
      if (rule.allowed)(&captures) {
        continue;
      }

      let matched = captures.get(0).expect("a match has its whole text");
      found.push(format!("{path}:{}: {}", line_at(text, matched.start()), rule.kind));
    }
  }

  found
}

#[test]
fn no_personal_data_in_a_tracked_file() {
  let rules = rules();
  let mut found = Vec::new();

  for (path, text) in tracked_texts() {
    found.extend(findings(&path, &text, &rules));
  }

  assert!(found.is_empty(), "personal data in tracked files:\n{}", found.join("\n"));
}

#[test]
fn a_finding_names_the_file_the_line_and_the_kind() {
  let address = format!("someone@{}", "ahas.io");
  let text = format!("ordinary words\n{address}\n");
  let found = findings("notes.txt", &text, &rules());

  assert_eq!(found, ["notes.txt:2: mail address"]);
  assert!(!found[0].contains(&address));
}

#[test]
fn the_neutral_forms_pass() {
  assert!(is_example_address("tester@example.com"));
  assert!(is_example_address("qa-admin@mail.example.org"));
  assert!(is_example_domain("host.test"));
  assert!(is_example_domain("mail.example.invalid"));
  assert!(is_neutral_home("tester"));
  assert!(is_neutral_home("agent-a"));

  let elsewhere = format!("tester@{}", "ahas.io");
  assert!(!is_example_address(&elsewhere));
  assert!(!is_example_domain("ahas.io"));
  assert!(!is_neutral_home("someone"));
}
