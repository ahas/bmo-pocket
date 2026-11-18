# Personal data

No personal data stands in this repository: not the names, handles, account ids, mail addresses, host names,
home paths, domains, devices or network details of the owner and the people they know. None of them stands in
code, a comment, a document, a test fixture or a commit message.

- **A value belongs to the installation that holds it.** An address, a host, a path, a port and a credential
  are read from a settings file or the environment; the code that reads one never carries its value.
- **An example is neutral.** A person is `tester` or `qa-admin`, an agent is `agent-a` or `agent-b`, a mailbox
  stands at `example.com`, and a home path names `user`.
- **A tool placed under the host carries no value of one installation.** It reads what the host hands it or
  what its own settings file states, and never the values of the sessions and the people it serves.
- **A test's fixtures carry no real person, host or address.** A value a test reads is written for the test.

The testbed's `personal-data` check, which the declaration enables, holds it: it reads every tracked file for a
mail address, a home path, a telephone number, a token and a private network address before anything is built. A finding names the file, the line and the
kind, never the value. The forms are the testbed's policy; no list of the values it forbids is kept here.
