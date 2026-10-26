//! `bmo-pocket`'s `tool` nouns and its `nouns` answer as a person and `bmo` meet them (docs/api.md, 「The CLI」):
//! the five nouns declared, `tool` alone answering its usage, the catalogue listed and narrowed, one entry
//! described and called, and the site tools read, against a real host on a scratch data root. Every process the host starts goes down with the test.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};

use tempfile::TempDir;

const MANIFEST: &str = r#"
[tool]
name = "garden"
protocol = 1
run = ["python3", "garden.py"]
timeout = 20

[[entry]]
namespace = "garden"
name = "echo"
title = "Echo"
description = "Says the text back. It reads one field."
input = "any.json"
capabilities = ["garden.read"]
grant = "garden.use"
hints = { read_only = true }

[entry.listing]
line = "- echo: says the text back"
order = 1

[[entry]]
namespace = "web"
name = "web_page"
description = "Reads a page."
input = "any.json"
capabilities = ["web.sites"]
hints = { read_only = true }

[entry.facts.lists.sites]
what = "a page read"
site = "Example (example.org)"
since = "2026-10-20"
arguments = ["url"]
answers = "example.org/<path>"
description = "Reads the page an address names."
"#;

const SCRIPT: &str = r#"
import json, sys

handed = json.load(sys.stdin)
arguments = handed["arguments"]

print(json.dumps({
    "content": [{"type": "text", "text": arguments.get("text", "")}],
    "isError": bool(arguments.get("fail")),
}))
"#;

// A host running on a scratch data root, with its control socket in a directory short enough to bind.
struct Running {
  child: Child,
  control: PathBuf,
  data: PathBuf,
  _data: TempDir,
  _sockets: TempDir,
}

impl Drop for Running {
  fn drop(&mut self) {
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

impl Running {
  // `bmo-pocket` asked on this host's control socket.
  fn bmo_pocket(&self, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bmo-pocket"))
      .args(["--data-root", self.data.to_str().unwrap()])
      .args(args)
      .env("BMO_MCP_HOST_SOCKET", &self.control)
      .output()
      .unwrap()
  }

  // What `bmo-pocket` wrote, held to having ended well.
  fn stdout(&self, args: &[&str]) -> String {
    let output = self.bmo_pocket(args);

    assert!(
      output.status.success(),
      "bmo-pocket {args:?} ended {:?}: {}",
      output.status.code(),
      String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).unwrap()
  }
}

// The host started over a placed entry of a garden namespace and a site tool of the web namespace.
fn start() -> Running {
  let data = tempfile::tempdir().unwrap();
  let sockets = tempfile::Builder::new().prefix("cm").tempdir_in("/tmp").unwrap();
  let tools = data.path().join("tools");
  let staged = tools.join("garden/.placing");

  fs::create_dir_all(&staged).unwrap();
  fs::write(staged.join("tool.toml"), MANIFEST).unwrap();
  fs::write(staged.join("any.json"), r#"{ "type": "object" }"#).unwrap();
  fs::write(staged.join("garden.py"), SCRIPT).unwrap();
  fs::rename(&staged, tools.join("garden/.placed")).unwrap();

  let control = sockets.path().join("control.sock");
  let mut child = Command::new(env!("CARGO_BIN_EXE_mcp-host"))
    .args(["serve", "--port", "0"])
    .arg("--data-root")
    .arg(data.path())
    .arg("--control")
    .arg(&control)
    .arg("--tools")
    .arg(&tools)
    .args(["--settle-ms", "50"])
    .stdout(Stdio::piped())
    .stderr(fs::File::create(data.path().join("host.log")).unwrap())
    .spawn()
    .unwrap();
  let mut line = String::new();

  BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();

  assert!(line.contains("mcp-host listening"), "the host did not start: {line}");

  Running {
    child,
    control,
    data: data.path().to_owned(),
    _data: data,
    _sockets: sockets,
  }
}

#[test]
fn the_nouns_are_declared_one_a_line() {
  let output = Command::new(env!("CARGO_BIN_EXE_bmo-pocket")).arg("nouns").output().unwrap();

  assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "tool\ntool list\ntool describe\ntool call\ntool sites\n"
  );
}

#[test]
fn tool_alone_answers_its_usage_naming_the_four_nouns_and_its_help_the_same_on_standard_output() {
  let pocket = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_bmo-pocket")).args(args).output().unwrap();
  let alone = pocket(&["tool"]);
  let usage = String::from_utf8_lossy(&alone.stderr).into_owned();

  assert_eq!(alone.status.code(), Some(2));
  assert!(alone.stdout.is_empty());

  for noun in ["list", "describe", "call", "sites"] {
    assert!(usage.lines().any(|line| line.trim_start().starts_with(noun)), "{noun}: {usage}");
  }

  let help = pocket(&["tool", "--help"]);

  assert_eq!(help.status.code(), Some(0));
  assert_eq!(String::from_utf8_lossy(&help.stdout), usage);

  let noun = pocket(&["tool", "call", "--help"]);

  assert_eq!(noun.status.code(), Some(0));
  assert!(String::from_utf8_lossy(&noun.stdout).contains("Usage: bmo-pocket tool call"));
}

#[test]
fn the_catalogue_is_listed_and_narrowed_by_the_words_given() {
  let host = start();

  assert_eq!(
    host.stdout(&["tool", "list"]),
    "garden/echo: Says the text back.\nweb/web_page: Reads a page.\n"
  );
  assert_eq!(host.stdout(&["tool", "list", "PAGE"]), "web/web_page: Reads a page.\n");
  assert_eq!(host.stdout(&["tool", "list", "echo", "field"]), "garden/echo: Says the text back.\n");
}

#[test]
fn an_entry_is_described_with_what_it_holds_and_declares() {
  let host = start();

  assert_eq!(
    host.stdout(&["tool", "describe", "garden/echo"]),
    [
      "garden/echo: Says the text back. It reads one field.",
      "Title: Echo",
      "Capabilities: [\"garden.read\"]",
      "Grant: garden.use",
      "Listing: - echo: says the text back",
      "",
    ]
    .join("\n")
  );

  let none = host.bmo_pocket(&["tool", "describe", "garden/none"]);

  assert_eq!(none.status.code(), Some(1));
  assert_eq!(
    String::from_utf8(none.stderr).unwrap(),
    "bmo-pocket: the tool host serves no garden/none\n"
  );
}

#[test]
fn an_entry_is_called_under_a_session_of_its_own_and_answers_its_result() {
  let host = start();

  assert_eq!(host.stdout(&["tool", "call", "garden/echo", r#"{"text":"season 2"}"#]), "season 2\n");

  let failed = host.bmo_pocket(&["tool", "call", "garden/echo", r#"{"text":"no","fail":true}"#]);

  assert_eq!(failed.status.code(), Some(1));
  assert_eq!(String::from_utf8(failed.stdout).unwrap(), "no\n");
}

#[test]
fn the_site_tools_are_read_off_the_web_namespaces_facts() {
  let host = start();

  assert_eq!(
    host.stdout(&["tool", "sites"]),
    [
      "1 site tools",
      "",
      "web_page: a page read",
      "  site: Example (example.org)",
      "  since: 2026-10-20",
      "  arguments: url",
      "  answers: example.org/<path>",
      "",
    ]
    .join("\n")
  );
  assert!(
    host
      .stdout(&["tool", "sites", "--detail"])
      .contains("  description: Reads the page an address names.\n")
  );
}

#[test]
fn words_that_do_not_read_end_with_two() {
  let host = start();

  assert_eq!(host.bmo_pocket(&["tool", "describe", "garden"]).status.code(), Some(2));
  assert_eq!(host.bmo_pocket(&["tool", "call", "garden/echo", "[]"]).status.code(), Some(2));
}
