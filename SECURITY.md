# Security

This project has not received an independent security audit.

The application encrypts local credential records using AES-256-GCM and
protects its data key with Windows user-level DPAPI. It is not a security
boundary against malicious software running as the same Windows user.

The MCP interface does not return protected plaintext or notes. Credential
references are resolved locally for the requested target. Files and controls
that receive credentials can subsequently be read by other tools; the
application does not guarantee secrecy outside its own output boundary.

Deleted entries are excluded from ordinary queries and execution. MCP has no
permanent-delete or trash-emptying operation. Permanent deletion requires a
native desktop confirmation, and checks the confirmed trash snapshot again
before deleting. This is not secure erasure of external backups or disk
snapshots.

Do not post real passwords, tokens, vault databases, screenshots containing
secrets, or production diagnostic dumps in public issues. Use synthetic
fixtures when reporting a problem. Where available, use GitHub's private
vulnerability reporting for security issues.

Supported development target: the latest source and release only. Version
0.3 uses database schema 2; back up the complete data directory before
upgrading and do not open the migrated database with older binaries.
