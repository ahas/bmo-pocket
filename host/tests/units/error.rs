use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use http_body_util::BodyExt;
use mcp_host::error::HostError;

async fn answered(error: HostError) -> (StatusCode, String, serde_json::Value) {
  let response = error.into_response();
  let status = response.status();
  let kind = response.headers()[CONTENT_TYPE].to_str().unwrap().to_owned();
  let body = response.into_body().collect().await.unwrap().to_bytes();

  (status, kind, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn an_unknown_token_is_refused_as_unauthorised_in_problem_details() {
  let (status, kind, body) = answered(HostError::UnknownToken).await;

  assert_eq!(status, StatusCode::UNAUTHORIZED);
  assert_eq!(kind, "application/problem+json");
  assert_eq!(body["code"], "mcp-host.unknown-token");
  assert_eq!(body["status"], 401);
  assert_eq!(body["type"], "urn:sumida:error:mcp-host.unknown-token");
  assert_eq!(body["title"], "Unauthorized");
}

#[tokio::test]
async fn each_case_answers_its_own_status() {
  assert_eq!(answered(HostError::NoToken).await.0, StatusCode::UNAUTHORIZED);
  assert_eq!(answered(HostError::UnknownNamespace("graph".into())).await.0, StatusCode::NOT_FOUND);
  assert_eq!(answered(HostError::MalformedRecord("no line".into())).await.0, StatusCode::BAD_REQUEST);
}
