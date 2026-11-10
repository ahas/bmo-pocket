use bmo_pocket_client::ReachedBy;
use mcp_host::external::{Registration, entries, entry_name_of};
use serde_json::json;

const STDIO: &str = r#"
name = "stand-in"
namespace = "outer"
capabilities = ["outer.use"]
timeout = 5

[stdio]
command = ["stand-in"]
"#;

#[test]
fn a_listed_name_is_made_an_entrys_name() {
  assert_eq!(entry_name_of("echo_text"), "echo_text");
  assert_eq!(entry_name_of("echoText"), "echo_text");
  assert_eq!(entry_name_of("getUserID"), "get_user_id");
  assert_eq!(entry_name_of("HTTPGet"), "httpget");
  assert_eq!(entry_name_of("write-note.v2"), "write_note_v2");
}

#[test]
fn a_registration_reads_with_act_by_default_and_names_what_does_not_read() {
  let read = Registration::parse(STDIO).unwrap();

  assert_eq!(read.reached_by, ReachedBy::Act);
  assert_eq!(read.address(), "stdio `stand-in`");

  let refused = |text: &str| Registration::parse(text).unwrap_err();

  assert!(refused(&STDIO.replace("[stdio]\ncommand = [\"stand-in\"]", "")).contains("neither"));
  assert!(refused(&format!("{STDIO}\n[http]\nurl = \"http://[::1]:1/mcp\"\n")).contains("both"));
  assert!(refused(&STDIO.replace("[\"stand-in\"]", "[]")).contains("names nothing to run"));
  assert!(refused(&STDIO.replace("\"outer\"", "\"toolbook\"")).contains("no namespace's name"));
  assert!(refused(&STDIO.replace("[\"outer.use\"]", "[]")).contains("asks for no capability"));
  assert!(Registration::parse(&STDIO.replace("timeout = 5", "timeout = 0")).is_ok());
  assert!(Registration::parse(&STDIO.replace("timeout = 5\n", "")).is_ok());
  assert!(
    refused(&format!("{STDIO}\n[tools.echo]\nmanual = \"../echo.md\"\n")).contains("does not stand inside")
  );
  assert!(refused(&format!("{STDIO}\nwhatever = 1\n")).contains("whatever"));

  let http = STDIO.replace("[stdio]\ncommand = [\"stand-in\"]", "[http]\nurl = \"ftp://127.0.0.1/mcp\"");

  assert!(refused(&http).contains("no http or https address"));
}

#[test]
fn a_listed_tool_reads_as_an_entry_its_read_only_hint_where_carried_else_the_registrations_default() {
  let listed: Vec<rmcp::model::Tool> = serde_json::from_value(json!([
    { "name": "echoText", "description": "Says it back.", "inputSchema": { "type": "object" },
      "annotations": { "readOnlyHint": true } },
    { "name": "write-note", "inputSchema": { "type": "object" }, "title": "Write a note" },
    { "name": "echo_text", "description": "Takes a name another holds.", "inputSchema": { "type": "object" } },
    { "name": "broken", "description": "Its schema is none.", "inputSchema": { "type": 5 } },
  ]))
  .unwrap();
  let registration = Registration::parse(STDIO).unwrap();
  let (manifest, names, left) = entries(&registration, &listed);
  let read: Vec<(&str, bool, bool, &str)> = manifest
    .entries
    .iter()
    .map(|entry| {
      (
        entry.name.as_str(),
        entry.hints.read_only,
        entry.hints.destructive,
        entry.description.as_str(),
      )
    })
    .collect();

  assert_eq!(
    read,
    [("echo_text", true, false, "Says it back."), ("write_note", false, true, "Write a note")]
  );
  assert_eq!(names["write_note"], "write-note");
  assert!(
    manifest
      .entries
      .iter()
      .all(|entry| entry.namespace == "outer" && entry.capabilities == ["outer.use"])
  );
  assert_eq!(left.len(), 2, "{left:?}");

  let calling =
    Registration::parse(&STDIO.replace("timeout = 5", "timeout = 5\nreached_by = \"call\"")).unwrap();
  let (manifest, _, _) = entries(&calling, &listed);

  assert!(manifest.entries[1].hints.read_only);
}
