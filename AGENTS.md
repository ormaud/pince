See [denv/AGENTS.md](denv/AGENTS.md) for dev environment rules. If missing, run `denv/update.sh` to pull it from trame-tools.

<!-- Project-specific instructions -->

## Project: pince

Pince is a local-first, single-user AI agent framework written in Rust.

- **Supervisor**: trusted async Rust process (tokio), never contacts an LLM
- **Sub-agents**: untrusted sandboxed OS processes that run LLM interaction loops
- **Frontends**: external client processes (CLI, messaging bots) connecting via frontend protocol
- **Memory**: markdown files on disk, indexed by qmd via MCP

### Build & Test

```bash
cargo build
cargo test
cargo clippy
```
