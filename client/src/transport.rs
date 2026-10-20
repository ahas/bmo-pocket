//! One HTTP request over a local socket, as a caller on this machine asks the host's control socket.

use std::io;
use std::path::Path;

use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::UnixStream;

/// What a request came back with: its status and its body.
pub struct Reply {
  pub status: StatusCode,
  pub body: Bytes,
}

impl Reply {
  /// The body read as JSON, or none where it is not.
  pub fn json(&self) -> Option<Value> {
    serde_json::from_slice(&self.body).ok()
  }
}

/// A request to `path` on the server listening at `socket`, with the bearer token and a JSON body where each is
/// given. Failing to reach the socket at all is an error of its own kind, which a caller waiting for a
/// restarting server tells apart from a server that answered badly.
pub async fn request(
  socket: &Path,
  method: Method,
  path: &str,
  token: Option<&str>,
  body: Option<&Value>,
) -> io::Result<Reply> {
  let stream = UnixStream::connect(socket).await?;
  let (mut link, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
    .await
    .map_err(io::Error::other)?;

  tokio::spawn(connection);

  let mut asked = Request::builder().method(method).uri(path).header(http::header::HOST, "localhost");

  if let Some(token) = token {
    asked = asked.header(http::header::AUTHORIZATION, format!("Bearer {token}"));
  }

  let bytes = match body {
    Some(body) => {
      asked = asked.header(http::header::CONTENT_TYPE, "application/json");
      Bytes::from(serde_json::to_vec(body)?)
    }
    None => Bytes::new(),
  };
  let response = link
    .send_request(asked.body(Full::new(bytes)).map_err(io::Error::other)?)
    .await
    .map_err(io::Error::other)?;
  let status = response.status();
  let body = response.into_body().collect().await.map_err(io::Error::other)?.to_bytes();

  Ok(Reply { status, body })
}
