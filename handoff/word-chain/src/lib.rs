//! The word chain as a game (docs/word-chain.md): the Hangul arithmetic, the word book a game judges by and its
//! robot plays from, its installation, and the game itself — players, turns, judging, rounds, scores and the
//! robot. It names no room, road or agent: whoever hosts a game says how each event reads, and keeps its time.

pub mod book;
pub mod game;
pub mod hangul;
pub mod install;
pub mod policy;

pub use book::{ChainRules, SqliteWordBook, WordBook, WordEntry, WordMeaning, openings_after};
pub use game::{
  Alarm, Difficulty, Elimination, Ending, GameEvent, GameSettings, Opening, Phase, Player, Rejection,
  RobotSeat, Said, WordChainGame,
};
pub use hangul::is_hangul_word;
