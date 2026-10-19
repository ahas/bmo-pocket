//! Prints the control socket's OpenAPI document as the repository keeps it:
//! `cargo run -p bmo-pocket-client --example openapi > docs/openapi.json`.

fn main() {
  print!("{}", bmo_pocket_client::openapi_json());
}
