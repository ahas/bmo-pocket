//! The console face served in this process (docs/api.md, 「The console face」): the console token on every route,
//! the screen's role on every route the screen uses, the manifest naming the bundle's files by their digests, and
//! the control socket's reads answered as the issuer is answered them.

use std::path::Path;

use bmo_pocket_client::transport::request;
use bmo_pocket_client::{Granted, Pocket, SessionRequest};
use http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use mcp_host::console::Bundle;
use serde_json::{Value, json};
use tokio::net::TcpStream;

use crate::units::fixture::{CONSOLE, Fixture, ISSUER};

const MODULE: &str = "export default { routes: [] };\n";
const STYLESHEET: &str = ".pocket {}\n";
const WORDS: &str = r#"{"en":{"pocket":{"title":{"screen":"Tools"}}}}"#;

// A bundle as the build of `console/` writes it, read from a folder of its own.
fn bundle(stylesheet: bool) -> Bundle {
  let folder = tempfile::tempdir().unwrap();

  std::fs::write(folder.path().join("module.js"), MODULE).unwrap();
  std::fs::write(folder.path().join("words.json"), WORDS).unwrap();

  if stylesheet {
    std::fs::write(folder.path().join("style.css"), STYLESHEET).unwrap();
  }

  Bundle::read(folder.path()).unwrap().unwrap()
}

struct Answer {
  status: StatusCode,
  headers: HeaderMap,
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
  let headers = answered.headers().clone();
  let body = answered.into_body().collect().await.unwrap().to_bytes();

  Answer {
    status,
    headers,
    body,
  }
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
async fn the_console_token_alone_reaches_the_face_and_the_screens_routes_need_the_administrator() {
  let fixture = Fixture::new();
  let (_, port) = fixture.serve_with_console(Some(bundle(true)));

  for path in ["/console-manifest", "/console-bundle/words", "/namespaces", "/nowhere"] {
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
  }

  // The manifest and the words to the token with no role; the screen's routes to the administrator alone.
  assert_eq!(
    ask(port, Method::GET, "/console-manifest", Some(CONSOLE), None).await.status,
    StatusCode::OK
  );
  assert_eq!(
    ask(port, Method::GET, "/console-bundle/words", Some(CONSOLE), None).await.status,
    StatusCode::OK
  );

  let module = bundle(true).manifest().bundle;

  for path in [module.as_str(), "/namespaces", "/schedules", "/tool-sessions", "/servers"] {
    for role in [None, Some("member"), Some("Administrator")] {
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

#[tokio::test]
async fn the_manifest_names_the_bundles_files_by_their_digests_and_each_is_answered() {
  let fixture = Fixture::new();
  let (_, port) = fixture.serve_with_console(Some(bundle(true)));
  let answered = ask(port, Method::GET, "/console-manifest", Some(CONSOLE), None).await;
  let manifest = answered.json();

  assert_eq!(answered.headers["cache-control"], "no-cache");
  assert_eq!(
    manifest,
    json!({
      "name": "pocket",
      "title": "pocket.title.screen",
      "icon": "i-material-symbols-home-repair-service-outline",
      "screenPath": "/pocket",
      "serviceName": "pocket",
      "bundle": manifest["bundle"],
      "stylesheet": manifest["stylesheet"],
      "words": "/console-bundle/words",
      "apiPrefix": "/",
      "role": "administrator",
      "ownerKey": false,
      "events": false,
      "websocket": false,
      "pushKinds": [],
      "calls": [],
    })
  );

  let module = manifest["bundle"].as_str().unwrap();
  let stylesheet = manifest["stylesheet"].as_str().unwrap();

  assert!(module.starts_with("/console-bundle/module.") && module.ends_with(".js"), "{module}");
  assert_eq!(module.len(), "/console-bundle/module.".len() + 16 + ".js".len());
  assert!(
    stylesheet.starts_with("/console-bundle/style.") && stylesheet.ends_with(".css"),
    "{stylesheet}"
  );

  let loaded = read(port, module).await;

  assert_eq!(loaded.body, MODULE.as_bytes());
  assert_eq!(loaded.headers["content-type"], "text/javascript; charset=utf-8");
  assert_eq!(loaded.headers["cache-control"], "private, max-age=31536000, immutable");

  let styled = read(port, stylesheet).await;

  assert_eq!(styled.body, STYLESHEET.as_bytes());
  assert_eq!(styled.headers["content-type"], "text/css; charset=utf-8");

  let words = ask(port, Method::GET, "/console-bundle/words", Some(CONSOLE), None).await;

  assert_eq!(words.body, WORDS.as_bytes());
  assert_eq!(words.headers["content-type"], "application/json");

  let other = read(port, "/console-bundle/module.0000000000000000.js").await;

  assert_eq!((other.status, other.code().as_str()), (StatusCode::NOT_FOUND, "not-found"));

  // A bundle the build wrote no stylesheet for is named with none.
  let plain = Fixture::new();
  let (_, bare) = plain.serve_with_console(Some(bundle(false)));

  assert_eq!(
    ask(bare, Method::GET, "/console-manifest", Some(CONSOLE), None).await.json()["stylesheet"],
    Value::Null
  );
}

#[tokio::test]
async fn a_host_holding_no_bundle_answers_no_manifest_and_still_reads() {
  let fixture = Fixture::new();
  let (_, port) = fixture.serve_with_console(None);

  for path in ["/console-manifest", "/console-bundle/words"] {
    let absent = ask(port, Method::GET, path, Some(CONSOLE), None).await;

    assert_eq!((absent.status, absent.code().as_str()), (StatusCode::NOT_FOUND, "not-found"), "{path}");
  }

  assert_eq!(read(port, "/namespaces").await.status, StatusCode::OK);

  let folder = tempfile::tempdir().unwrap();

  assert!(Bundle::read(folder.path()).unwrap().is_none());
  std::fs::write(folder.path().join("module.js"), MODULE).unwrap();
  std::fs::write(folder.path().join("words.json"), "[]").unwrap();
  assert!(Bundle::read(folder.path()).is_err(), "words that are not one object do not read");
}

#[tokio::test]
async fn every_read_answers_what_the_control_socket_answers_the_issuer() {
  let fixture = Fixture::new();
  let (socket, port) = fixture.serve_with_console(Some(bundle(true)));
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
