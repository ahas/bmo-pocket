//! The host's console face served in this process (docs/api.md, 「The console face」): the manifest it answers from
//! its declaration, read through bmo-ui's own reading from the release `console/vendor/` holds, the console token
//! and the screen's role on the control socket's reads, and those reads answered as the issuer is answered them. The
//! refusals of the bundle's own files are bmo-chat's `bmo-console-face` tests.

use std::path::{Path, PathBuf};

use bmo_console_face::{ConsoleFace, read_manifest};
use bmo_pocket_client::transport::request;
use bmo_pocket_client::{Granted, Pocket, SessionRequest};
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use mcp_host::console;
use serde_json::{Value, json};
use tokio::net::TcpStream;

use crate::units::fixture::{CONSOLE, Fixture, ISSUER};

const MODULE: &str = "export default { routes: [] };\n";

// The one release of bmo-ui `console/vendor/` holds, and the version its file is named by.
fn vendored_release() -> (PathBuf, String) {
  let vendor = Path::new(env!("CARGO_MANIFEST_DIR")).join("../console/vendor");
  let mut releases: Vec<PathBuf> = std::fs::read_dir(&vendor)
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .filter(|path| {
      path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("bmo-ui-"))
    })
    .collect();

  assert_eq!(releases.len(), 1, "console/vendor/ holds one release of bmo-ui: {releases:?}");
  let release = releases.remove(0);
  let name = release.file_name().unwrap().to_str().unwrap();
  let version = name
    .strip_prefix("bmo-ui-")
    .and_then(|rest| rest.strip_suffix(".tgz"))
    .unwrap()
    .to_owned();

  (release, version)
}

// The host's face over a bundle as the build of `console/` writes it, on the layer `console/vendor/` holds.
fn face() -> ConsoleFace {
  let folder = tempfile::tempdir().unwrap();
  let (_, layer) = vendored_release();

  std::fs::write(folder.path().join("module.js"), MODULE).unwrap();
  std::fs::write(folder.path().join("style.css"), ".pocket {}\n").unwrap();
  std::fs::write(folder.path().join("words.json"), r#"{"en":{"pocket":{"title":{"screen":"Tools"}}}}"#)
    .unwrap();
  std::fs::write(folder.path().join("layer.json"), json!({ "layer": layer }).to_string()).unwrap();

  console::face(folder.path(), CONSOLE).unwrap_or_else(|why| panic!("{why}"))
}

struct Answer {
  status: StatusCode,
  body: Bytes,
}

impl Answer {
  fn json(&self) -> Value {
    serde_json::from_slice(&self.body).unwrap_or(Value::Null)
  }

  fn code(&self) -> String {
    self.json()["code"].as_str().unwrap_or_default().to_owned()
  }
}

// One request on the console face, with the token and the role where each is given.
async fn ask(port: u16, method: Method, path: &str, token: Option<&str>, role: Option<&str>) -> Answer {
  let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  let (mut link, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();

  tokio::spawn(connection);

  let mut asked = Request::builder().method(method).uri(path).header("host", "127.0.0.1");

  if let Some(token) = token {
    asked = asked.header("authorization", format!("Bearer {token}"));
  }

  if let Some(role) = role {
    asked = asked.header("owner-role", role);
  }

  let answered = link.send_request(asked.body(Empty::<Bytes>::new()).unwrap()).await.unwrap();
  let status = answered.status();
  let body = answered.into_body().collect().await.unwrap().to_bytes();

  Answer { status, body }
}

async fn read(port: u16, path: &str) -> Answer {
  ask(port, Method::GET, path, Some(CONSOLE), Some("administrator")).await
}

// What the control socket answers the issuer at the path.
async fn issuers(socket: &Path, path: &str) -> Value {
  let reply = request(socket, Method::GET, path, Some(ISSUER), None).await.unwrap();

  assert_eq!(reply.status, StatusCode::OK, "{}", String::from_utf8_lossy(&reply.body));
  reply.json().unwrap()
}

#[tokio::test]
async fn the_served_manifest_reads_through_the_contract_and_names_files_the_face_answers() {
  let fixture = Fixture::new();
  let (_, port) = fixture.serve_with_console(face());
  let answered = ask(port, Method::GET, "/console-manifest", Some(CONSOLE), None).await;

  assert_eq!(answered.status, StatusCode::OK);

  let (release, layer) = vendored_release();
  let manifest = read_manifest(&answered.json(), &release).unwrap_or_else(|why| panic!("{why}"));

  assert_eq!(manifest["name"], "pocket");
  assert_eq!(manifest["screenPath"], "/pocket");
  assert_eq!(manifest["role"], "administrator");
  assert_eq!(manifest["layer"], layer.as_str());

  let module = read(port, manifest["bundle"].as_str().unwrap()).await;

  assert_eq!((module.status, module.body.as_ref()), (StatusCode::OK, MODULE.as_bytes()));
  assert_eq!(
    ask(port, Method::GET, manifest["words"].as_str().unwrap(), Some(CONSOLE), None)
      .await
      .status,
    StatusCode::OK
  );
}

#[tokio::test]
async fn the_reads_answer_the_console_token_alone_and_the_administrator() {
  let fixture = Fixture::new();
  let (_, port) = fixture.serve_with_console(face());

  for path in ["/namespaces", "/schedules", "/tool-sessions", "/servers", "/uses"] {
    let bare = ask(port, Method::GET, path, None, Some("administrator")).await;

    assert_eq!(
      (bare.status, bare.code().as_str()),
      (StatusCode::UNAUTHORIZED, "unauthorized"),
      "{path}"
    );

    for other in [ISSUER, "nobody's"] {
      let refused = ask(port, Method::GET, path, Some(other), Some("administrator")).await;

      assert_eq!(refused.status, StatusCode::UNAUTHORIZED, "{path}");
      assert_eq!(refused.json()["error"], "the console token alone reaches the console face");
    }

    for role in [None, Some("member")] {
      let refused = ask(port, Method::GET, path, Some(CONSOLE), role).await;

      assert_eq!(
        (refused.status, refused.code().as_str()),
        (StatusCode::FORBIDDEN, "forbidden"),
        "{path} {role:?}"
      );
    }

    assert_eq!(read(port, path).await.status, StatusCode::OK, "{path}");
  }

  let elsewhere = read(port, "/tool-calls").await;

  assert_eq!((elsewhere.status, elsewhere.code().as_str()), (StatusCode::NOT_FOUND, "no-route"));

  let written = ask(port, Method::POST, "/tool-sessions", Some(CONSOLE), Some("administrator")).await;

  assert_eq!((written.status, written.code().as_str()), (StatusCode::NOT_FOUND, "no-route"));
}

#[test]
fn a_bundle_folder_that_does_not_read_builds_no_face() {
  let folder = tempfile::tempdir().unwrap();

  assert!(console::face(folder.path(), CONSOLE).is_err());
}

#[tokio::test]
async fn every_read_answers_what_the_control_socket_answers_the_issuer() {
  let fixture = Fixture::new();
  let (socket, port) = fixture.serve_with_console(face());
  let issuer = Pocket::at(&socket, ISSUER);
  let issued = issuer
    .issue(&SessionRequest {
      holder_name: "tester".to_owned(),
      namespaces: ["notes".to_owned()].into(),
      capabilities: ["notes.read".to_owned()].into(),
      grants: vec![Granted {
        resource: "notes.keep".to_owned(),
      }],
      variables: [("PLOT".to_owned(), "a secret".to_owned())].into(),
      ttl_ms: Some(60_000),
      ..SessionRequest::default()
    })
    .await
    .unwrap();

  for path in [
    "/namespaces",
    "/namespaces?limit=1",
    "/namespaces/notes",
    "/namespaces/notes/tools",
    "/namespaces/notes/tools/broken",
    "/schedules",
    "/tool-sessions",
    "/servers",
  ] {
    let answered = read(port, path).await;

    assert_eq!(answered.status, StatusCode::OK, "{path}");
    assert_eq!(answered.json(), issuers(&socket, path).await, "{path}");
  }

  let sessions = read(port, "/tool-sessions").await.json();

  assert_eq!(sessions["items"][0]["holderName"], "tester");
  assert_eq!(sessions["items"][0]["variables"], json!(["PLOT"]));
  assert!(!String::from_utf8_lossy(&read(port, "/tool-sessions").await.body).contains(&issued.token));
  assert!(!sessions.to_string().contains("a secret"));

  let missing = read(port, "/namespaces/garden").await;

  assert_eq!((missing.status, missing.code().as_str()), (StatusCode::NOT_FOUND, "not-found"));
}

#[tokio::test]
async fn the_kept_calls_read_as_the_issuer_reads_them_narrowed_by_state_and_agent() {
  let fixture = Fixture::new();
  let (socket, port) = fixture.serve_with_console(face());

  fixture.kept_call(
    "first",
    ("bmo/wait", "agent-a", 1),
    json!({ "note": "the fetch ended", "check": "true", "every": 900 }),
    None,
  );
  fixture.kept_call(
    "second",
    ("bmo/wait", "agent-b", 2),
    json!({ "note": "a reminder", "seconds": 60 }),
    Some(("done", "waited", 5)),
  );
  fixture.kept_call("third", ("notes/keep", "agent-a", 3), json!({}), Some(("failed", "it broke", 6)));

  let every = read(port, "/uses").await.json();
  let ids = |page: &Value| {
    page["items"]
      .as_array()
      .unwrap()
      .iter()
      .map(|call| call["id"].clone())
      .collect::<Vec<_>>()
  };

  assert_eq!(every, issuers(&socket, "/uses").await, "the face reads what the issuer reads");
  assert_eq!(ids(&every), [json!("first"), json!("second"), json!("third")]);
  assert_eq!(
    every["items"][0]["state"], "lost",
    "a call no host holds running and with no end reads lost"
  );
  assert_eq!(every["items"][0]["arguments"]["note"], "the fetch ended");
  assert_eq!(every["items"][1]["summary"], "waited");

  assert_eq!(ids(&read(port, "/uses?state=done").await.json()), [json!("second")]);
  assert_eq!(ids(&read(port, "/uses?agent=agent-a").await.json()), [json!("first"), json!("third")]);
  assert_eq!(ids(&read(port, "/uses?agent=agent-a&state=failed").await.json()), [json!("third")]);

  let unknown = read(port, "/uses?holder=agent-a").await;

  assert_eq!(unknown.status, StatusCode::BAD_REQUEST, "a field the read does not take is refused");
}
