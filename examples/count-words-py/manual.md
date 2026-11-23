# count_words_py

Hand it the text as `text`. It answers the count twice: as a line of JSON text, and as the object
`{"words": …, "characters": …}`.

- A word is a piece of the text between whitespace, so `"one two  three"` holds three.
- A character is a Unicode code point, so an accented letter written as one code point counts once.
