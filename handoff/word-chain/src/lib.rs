//! The word chain (docs/word-chain.md): the game itself — players, turns, judging, rounds, scores and the robot —
//! with its Hangul arithmetic and the word book it judges by; the games standing in the rooms and what each says,
//! as codes; the service that reads the rooms' messages and says the games' lines; its control routes; and the
//! tool that reaches them. No agent, room or road is named in code: who the robot is, where lines go and how they
//! read are the configuration's and the other units'.

pub mod book;
pub mod config;
pub mod control;
pub mod game;
pub mod hangul;
pub mod install;
pub mod lines;
pub mod policy;
pub mod rooms;
pub mod serve;
pub mod tool;

pub use book::{ChainRules, SqliteWordBook, WordBook, WordEntry, WordMeaning, openings_after};
pub use game::{
  Alarm, Difficulty, Elimination, Ending, GameEvent, GameSettings, Opening, Phase, Player, Rejection,
  RobotSeat, Said, WordChainGame,
};
pub use hangul::is_hangul_word;
