# The word chain in a room

- **What this says:** how a room plays the word chain (끝말잇기) together with its agent — the word book and
  where it comes from, the rules a move is judged by, how a game is opened, run and ended, how the robot plays,
  what a game says, and why a move costs no model run. Every route, call and code is stated in `docs/api.md`.

## The word book

- **The words are KKuTu's.** KKuTu (`https://github.com/JJoriping/KKuTu`) is an open-source Korean word-game
  server; its repository ships a PostgreSQL dump, `db.sql`, whose `kkutu_ko` table lists about 431,000 Korean
  words with a part-of-speech code and flags for each.
- **Only what a chain game judges by is kept:** the word, and whether it is a loanword. A word is kept where it
  is a noun, is two syllables or more, is spelled in Hangul alone, and carries no flag but the loanword one — so
  no dialect word, old form, North Korean standard word, word written with spaces, or word KKuTu accepts outside
  the dictionary. About 209,000 words remain. The dump's meanings are placeholders and are not read.
- **The book is a SQLite file under the data root,** `word-chain/ko.sqlite`, with each word's first and last
  syllable, and a count per opening syllable of the words that start with it, all and native only, so asking
  what can follow a word is one indexed read.
- **Installing it** is `bmo-word-chain install`: it fetches `db.sql` from the upstream commit the reader was
  written against, then the NIKL dictionaries below, and writes the book beside the old one before moving it into
  place. `--from <db.sql>` reads a dump already on the machine, and `--nikl-from <folder>` a copy of the mirror's
  `stdict/` and `krdict/` parts. The book records each source, commit, licence and size in itself.
  - The sources, their commits and licences, and the dictionaries' own words the reading goes by — a noun's part
    of speech, the kinds and fields of sense passed over — stand in `policy/word-chain.toml`.
- **Each word's meaning comes from the National Institute of Korean Language.** The installer also reads the
  standard dictionary (표준국어대사전) and the basic dictionary for learners (한국어기초사전), both as the NIKL
  publishes them for download, from the mirror `https://github.com/spellcheck-ko/korean-dict-nikl` at a pinned
  commit. A word of the book keeps the standard dictionary's first plain noun sense of its lowest-numbered
  homograph, its field before it and never a person, place or book, and whether the basic dictionary lists it; a
  word the basic dictionary lists is a basic word.
- **Looking a word up** is `bmo-word-chain word <word>`: whether the book has it, whether it is a loanword,
  whether it is a basic word, its meaning, and how many words can follow it, in English lines.
- **The licence.** KKuTu's repository is GPL-3.0; its images and sounds are CC BY, and its English words come
  from WordNet. The book is built on the machine from the upstream dump and is never committed or published, so
  nothing of KKuTu is redistributed; no KKuTu code is used — the rules below are implemented here from the Korean
  orthography rules, and only the word list is read. Anyone the book is ever handed to is handed KKuTu's GPL-3.0
  with it and told where it came from. The NIKL dictionaries are CC BY-SA 2.0 KR; the book records them as its
  meanings' source, and the same holds for them.

## The rules

- **A word follows** where it starts with the last syllable of the word before it, or with that syllable's form
  under the initial-sound rule (두음법칙): rieul before a, ae, o, oe, u and eu reads nieun, rieul before ya, yeo,
  ye, yo, yu and i reads ieung, and nieun before yeo, yo, yu and i reads ieung. The rule runs one way: a syllable
  turns into its form, never the form back. The consonants and vowels stand in `policy/word-chain.toml`
  (`initial_sound`).
- **Only a word is a move.** A message from the player whose turn it is counts as a move only where it is one
  word of two Hangul syllables or more; anything else they say — a one-syllable reply, a sentence, letters alone —
  is talk.
- **No move is dropped in silence.** Every move is either taken or refused in the room with the reason, so a
  player never waits on a game that heard nothing.
- **A move is refused** where the word does not start where the chain stands — the refusal names the syllables
  it must start with — is not in the book, was already played in this game, is a loanword in a game that turned
  loanwords off, or — in a game played with the manner rule — leaves nothing to follow it. A refused player keeps
  the turn and tries again, within the time left where the game has a limit.
- **The hanbang.** A word nothing in the book can follow puts the next player out at once rather than leaving
  them a turn nothing answers; the player who played it scores a bonus. Under the manner rule such a word is
  refused instead. A player alone who plays one scores the bonus and goes on from a new word.
- **Time.** A move has no limit: the game waits for the player whose turn it is. A limit is set only where the
  room asks for one when the game is opened, from 10 to 120 seconds a move, and then a player whose time runs
  down is out.
- **A game nobody plays closes.** Where a turn has no limit and ten minutes pass with no word taken in that game,
  it ends and announces the scores so far; nobody is put out for it. Each game in a room keeps its own ten
  minutes.
- **Rounds.** A game opens on the word the person who asked gave, where they gave one, and the player after them
  moves first; otherwise it opens on a word drawn from the book that leaves at least twenty answers. Every time
  the player whose move it was goes out, play goes on from a fresh drawn word with the next player.
- **Scores.** A word scores ten points a syllable; a hanbang scores thirty; the last player standing in a game
  of two or more scores fifty. The game is over when one player is left, or — for a player alone — when they go
  out. The ranking puts those still standing first, best score first, then those who went out, the last out
  first.
- **The numbers are the policy's:** how long a game recruits, waits for a move and lets the robot think, how many
  answers a round's opening word must leave and the syllables it is drawn from, how many words the robot weighs,
  the range a turn's seconds are held to, how many players a game's name lists, and the points.

## A game in a room

- **Opening.** The room's agent opens a game when a room asks, with the tool's `word_chain_start`; the same verb
  is `bmo-word-chain start --chat <chatId>`. The person who asked joins at once, and the robot joins as a player
  unless asked to sit out. A word the person who asked already said to begin with goes to the tool as the opening
  word.
- **An opening word that cannot open waits for another.** Where the book lacks it, it is a loanword in a game
  without them, or nothing follows it, no word is drawn in its place: the room is told why, and the person who
  asked is asked for another. Their next message of one Hangul word of two syllables or more is judged the same
  way, and the game opens on the first that can open it. The word may come after a begin or an offer word, and a
  bare begin or offer word is answered with a reminder that a word is wanted; anything else they say is talk.
  Ten minutes with no usable word fold the game. A start that names nobody who asked, with such a word, is
  refused with the reason.
- **Recruiting.** The room is told the rules of this game and how to play; anyone who says a join word joins,
  and play starts after a minute or when a player says a begin word. A game nobody but the robot joined folds.
  The words a room says to a game stand in `policy/word-chain.toml` (`words`).
- **Several games in a room.** A room may hold any number of games at once, each with its own players, turn,
  scores and clock. A game standing never refuses a start; a start is refused where the person who asked is
  already in a game.
- **One game a person.** A person is in one game of a room at a time, from joining it until it ends, going out
  included; the robot may be in every game at once. Whose word it is is read from who sent it: a person's move,
  begin word, give-up word or stop word goes to the one game they are in, and a join word from someone already in
  a game is not a join.
- **Which game a join is for.** With one game recruiting, a join word joins it. With several, the join word is
  followed by the name of a player of the one meant, matched on the start of the name, spaces and a closing game
  word aside. A bare join word, or a name that fits more than one, is answered with the games recruiting and how
  to name one; a name that fits none leaves the message as talk. A call for players says so while another game
  is recruiting beside it.
- **Playing.** The order is the order of joining, with the robot last. Only the player whose move it is is heard,
  and only with a move as 「The rules」 says, trailing punctuation aside; everything else in the room is talk. A
  player says a give-up word to give up.
- **Stopping.** A player saying a stop word stops the game they are in and no other. The owner, where they are in
  no game, stops the room's one game with it, or with several names the one meant as a join does. The tool's
  `word_chain_stop`, given a player — whoever asked, or one of the game someone in no game named — stops that
  player's game; given nobody, it stops the room's only game and is refused where several stand.
  `bmo-word-chain stop --chat <chatId> --player <id>` is the same verb. A stopped game announces the scores so
  far.

## The robot

- **The robot's moves come from the book, not a model.** After a short pause it draws up to sixty unplayed words
  that follow and picks by its difficulty:
  - **easy** — the word leaving the most answers, shortest first;
  - **normal** — any of them;
  - **hard** — the word leaving the fewest answers, which is how it plays hanbang words where the rules allow them.
- **The robot goes out** where nothing it may play is left.
- **It plays under the room's agent's name,** which the configuration names (`robot_name`); a game with nobody
  but the robot goes by that name.

## How a game speaks

- **What a game says is codes, never sentences.** Each thing a game says is a line of a code and the facts it
  reads — a player who joined and the count, a turn's player, the syllables a word must start with and the
  seconds it has, a word taken and its points, a refusal and its reason, the ranking — which the web unit's
  language pack renders as the room reads it, choosing the particles and the marks around words and names
  (`docs/api.md`, 「The codes」).
- **Everything one step says goes out as one message:** a word taken and the next turn, a hanbang and the new
  round, the lines of a step rendered one by one and joined with line breaks, then said in the room as the room's
  agent through the receiver. A line the web unit renders no text for goes out as its code and fields, so a room
  is never silent.
- **A game is named while another stands beside it.** Where a room holds more than one game, every message of a
  game opens on a line naming it by the people in it, the robot aside, in the order they joined, the first three
  shown and the rest counted; a room's only game is not named.
- **A word that is not basic comes with its meaning.** Where the word a round opens on or a word a player — the
  robot included — played is not a basic word and the book has its meaning, the line after it gives that meaning,
  cut to about a hundred characters.

## The service

- **A game costs one model run: the turn that opened it.** The games stand in the service's own process,
  `bmo-word-chain serve`, which reads the receiver's event stream; while a game stands in a room, each message
  the room says is read and judged against the book in well under a millisecond, the robot's own move taking tens
  of milliseconds. A restart of the service ends every standing game without an announcement.
- **The tool and the command line reach the games** through the service's control routes, at the address and
  with the token its configuration names.
- **The game names no room, road or agent.** The game itself is `game`'s, keeping time with a clock it is handed
  and saying what happened as events; `rooms` holds which room each game stands in and which game each person is
  in, the wait for a usable opening word, which words of a room are joins, starts, give-ups and stops and which
  game each is for, and what each event says (`lines`).
