# vault 0.6.0

- Website and API service addresses have an open-in-browser action. API credentials also have an optional documentation URL, separate from their request endpoints.
- Desktop and MCP share one Rust backend, with bound caller identities and core-owned permissions. Window and file insertion use write_credential; the obsolete fill_credential tool is removed. The current 11-tool MCP interface retains batch and direct-single CRUD inputs.
- Current-only data contract: schema=4 vaults, tags only, explicit services arrays and OpenAI / Anthropic / Gemini / custom protocols. Runtime migrations, group aliases, historical protocol variants and old update formats are removed. Existing vaults require an external, backed-up and verified conversion before installation; this release does not auto-migrate older vaults.
- Updates accept signed schema=2 ZIP manifests from the canonical lowercase repository and member path. Installation handoff drains active MCP calls, restores connections, and preserves current-format data; failed startup rolls back the program.
- Distribution contains a portable ZIP and a Setup.exe installer. WebView2 is a system prerequisite. Code is licensed under Apache-2.0.

Local validation covers 61 Rust tests, MCP permissions and operations, real desktop interactions, shared-backend lifecycle, native editors and file insertion, link actions, signed update success and rollback. Tests use synthetic data; existing local vault conversion is verified separately without publishing credentials or backups.
