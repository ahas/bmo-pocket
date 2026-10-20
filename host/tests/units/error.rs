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
async fn an_unknown_token_is_refused_as_unauthorised_in_the_common_error_shape() {
  let (status, kind, body) = answered(HostError::UnknownToken).await;

  assert_eq!(status, StatusCode::UNAUTHORIZED);
  assert_eq!(kind, "application/json");
  assert_eq!(body.as_object().unwrap().len(), 2, "{body}");
  assert_eq!(body["code"], "unknown-token");
  assert_eq!(body["error"], "the token names no session");
}

#[tokio::test]
async fn each_case_answers_its_own_status() {
  assert_eq!(answered(HostError::NoToken).await.0, StatusCode::UNAUTHORIZED);
  assert_eq!(answered(HostError::UnknownNamespace("garden".into())).await.0, StatusCode::NOT_FOUND);
  assert_eq!(
    answered(HostError::MalformedRecord("no holder".into())).await.0,
    StatusCode::BAD_REQUEST
  );
  assert_eq!(answered(HostError::GrantRefused("no grant".into())).await.0, StatusCode::FORBIDDEN);
}
