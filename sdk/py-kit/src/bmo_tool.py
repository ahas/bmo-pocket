"""What a Python tool reads its call with and answers it by, on the protocol the host speaks
(docs/api.md, 「The call protocol」): the handed object off standard input, the entry it names answered
on standard output, and another entry asked through the call's call-back socket.

A tool's executable is its handlers by full entry name, handed to `answer`:

    answer({"voice/encoder": encode})

Standard library alone, so a tool runs on whichever `python3` the clean environment's PATH finds.
"""

import json
import socket
import sys

PROTOCOL = 1


def text(said):
    """An answer of plain text."""
    return {"content": [{"type": "text", "text": said}], "isError": False}


def refusal(said):
    """A refusal, an answer marked as an error that the model reads as any other."""
    return {"content": [{"type": "text", "text": said}], "isError": True}


def failure(reason):
    """A tool that could not read what it answers out of: the reason, marked as the verbs mark it."""
    return refusal(f"ERROR: {reason}")


def structured(obj):
    """An object as an entry with an output schema answers it: the object, and the same object as its text."""
    said = json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
    return {"content": [{"type": "text", "text": said}], "isError": False, "structuredContent": obj}


class CallBackError(Exception):
    """The call-back socket could not be asked, or answered nothing that reads."""


class Call:
    """One call as the host handed it: the entry, its arguments, the session record and the sockets."""

    def __init__(self, handed):
        self.entry = handed["entry"]
        self.arguments = handed.get("arguments") or {}
        self.session = handed["session"]
        self.callback = handed.get("callback")
        self.shared = handed.get("shared") or {}

    def call_back(self, entry, arguments=None):
        """Another entry called under the same record, by its full name; answered with its tool result."""
        return self._ask({"kind": "call", "entry": entry, "arguments": arguments or {}})

    def claim(self, address):
        """Which entry the record holds claims this address, and the arguments it reads the page with."""
        return self._ask({"kind": "claim", "address": address})

    def _ask(self, asked):
        if not self.callback:
            raise CallBackError("this call was handed no call-back socket")
        return ask_line(self.callback, asked)


def ask_line(path, asked):
    """One JSON object asked on one line of a socket, and the one line it is answered with, read."""
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as conn:
            conn.connect(path)
            conn.sendall((json.dumps(asked, ensure_ascii=False) + "\n").encode())
            said = conn.makefile("rb").readline().decode()
    except OSError as why:
        raise CallBackError(f"{path} could not be asked: {why}") from why
    if not said.strip():
        raise CallBackError(f"{path} answered nothing")
    try:
        return json.loads(said)
    except ValueError as why:
        raise CallBackError(f"the answer of {path} does not read: {why}") from why


def answer_handed(handlers, handed):
    """What the handed object asks, answered by the handlers: a call by its entry's handler, a claim with
    none, and a resource request as an executable serving none answers it."""
    kind = handed.get("kind")
    if kind == "claim":
        return {"arguments": None}
    if kind == "resources_list":
        return {"resources": []}
    if kind == "resources_read":
        return {"error": "this executable serves no resources"}
    if kind != "call":
        return failure("this executable answers calls alone")
    handler = handlers.get(handed.get("entry"))
    if handler is None:
        return failure(f"this executable holds no entry {handed.get('entry')}")
    try:
        return handler(Call(handed))
    except Exception as why:
        return failure(why)


def answer(handlers, given=None, said=None):
    """The whole of a Python tool's main: the call read off standard input and answered on standard
    output; a call that does not read is said on standard error and ends unclean."""
    # Bytes both ways, read and written as UTF-8 whatever locale the clean environment leaves.
    given = given or sys.stdin.buffer
    said = said or sys.stdout.buffer
    try:
        handed = json.loads(given.read().decode())
        if not isinstance(handed, dict) or not isinstance(handed.get("session"), dict):
            raise ValueError("it is no object holding a session")
        if handed.get("protocol") != PROTOCOL:
            raise ValueError(f"protocol {handed.get('protocol')} is not {PROTOCOL}")
    except ValueError as why:
        print(f"the call does not read: {why}", file=sys.stderr)
        sys.exit(1)
    said.write(json.dumps(answer_handed(handlers, handed), ensure_ascii=False).encode())
    said.flush()
