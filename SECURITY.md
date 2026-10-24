# Security Policy

## Supported versions

Security fixes go into the latest release only. Please update before reporting.

## Reporting a vulnerability

Please report security problems **privately**, not in a public issue:
use **[Report a vulnerability](https://github.com/eneskocamaan/savepoint/security/advisories/new)** on this repository's Security tab.

Include what you found, how to reproduce it and what an attacker could do with it. You will get an answer as soon as possible, and you will be credited in the fix unless you prefer not to be.

## Scope

Savepoint runs as the signed-in user without administrator rights. Its window talks to its service over a local named pipe that only accepts connections from the same PC, and its only network access is downloading the game manifest from `raw.githubusercontent.com`. Reports about any of these are especially welcome, for example:

- another user or program on the PC controlling the service through the pipe,
- a crafted manifest or `backup.json` making Savepoint read or write files outside the save and backup folders,
- a restore writing somewhere it should not.
