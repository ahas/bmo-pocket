//! The example tool's executable: one call read off standard input, answered on standard output.

use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
  let mut said = String::new();
  let answered = std::io::stdin()
    .read_to_string(&mut said)
    .map_err(|why| why.to_string())
    .and_then(|_| serde_json::from_str(&said).map_err(|why| why.to_string()))
    .and_then(|handed| count_words_rust::answer(&handed));

  match answered {
    Ok(answer) => {
      println!("{answer}");
      ExitCode::SUCCESS
    }
    Err(why) => {
      eprintln!("the call does not read: {why}");
      ExitCode::FAILURE
    }
  }
}
