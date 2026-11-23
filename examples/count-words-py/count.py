"""The example tool in Python (the tool host's docs/authoring.md): its one entry, example/count_words_py, answers
how many words and characters a text holds. The host hands the call on standard input and reads the answer on
standard output; the copy of the Python kit beside this file, bmo_tool.py, does both."""

from bmo_tool import answer, structured


def count_words(call):
    """The entry's answer: the count as the object its output schema states, and the same object as its text. The
    host checked the arguments against count.json before the call, so `text` stands and is a string."""
    text = call.arguments["text"]
    return structured({"words": len(text.split()), "characters": len(text)})


if __name__ == "__main__":
    answer({"example/count_words_py": count_words})
