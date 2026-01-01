# Contributing to Ward

Thanks for your interest in contributing to Ward.

Ward is a pragmatic scripting toolchain: a Lua-first user experience backed by
a Rust runtime. The project optimizes for predictable scripting semantics,
good diagnostics, and a stable surface area for daily automation.

This document explains how to propose changes, set up a dev environment, and get
a PR merged.

---

## Project Principles

When contributing, please align with these principles:

1. **Lua-native flavor**

   * Prefer APIs and naming that feel natural in Lua.
   * Avoid imposing "non-Lua" abstractions unless they demonstrably improve ergonomics.

2. **Small, well-scoped modules**

   * Ward favors focused modules (`ward.fs`, `ward.process`, `ward.async`, etc.)
   over monolithic APIs.
   * Prefer composable primitives.

3. **Safety and predictability**

   * Ward provides resource limits (time/memory/instructions) to prevent runaway
   scripts.
   * Ward is not a hardened sandbox for untrusted code. Do not represent it as one.

4. **Explicit scope**

   * LuaRocks support: **pure Lua packages are OK**; **C extensions are out of
   scope** for now.
   * Keep the core portable and dependency-light.

---

## Ways to Contribute

* Bug reports (repro steps + expected vs actual)
* Documentation improvements (API guide, examples, clarity)
* New modules / features (please read "Proposing features" below)
* Tests (unit tests and integration tests)
* CI and release hygiene (workflows, packaging, install script)

---

## Before You Start

### Check existing issues/PRs

If you have GitHub Issues enabled, search for related work first to avoid duplication.

### Discuss non-trivial changes

For anything beyond a small bugfix or doc tweak, please open an issue describing:

* the problem statement
* proposed API (Lua-side)
* runtime implications (Rust-side)
* alternatives considered
* testing approach

---

## Development Setup

### Requirements

* Rust toolchain (stable)
* `cargo`
* A working C toolchain for building dependencies (varies per distro)
* Optional: `git` (required for `ward.module.git` functionality)

### Build

```sh
cargo build
```

### Run

```sh
cargo run -- <args>
```

### Tests

```sh
cargo test
```

### Formatting

Use Rustfmt:

```sh
cargo fmt
```

### Lints

Use Clippy:

```sh
cargo clippy --all-targets -- -D warnings
```

---

## Documentation

Docs live under `docs/`.

* If you change user-facing behavior or public APIs, update the relevant
page(s) under `docs/src/` and ensure the mdBook builds (`mdbook build docs`).
* Prefer docs that include:

  * purpose / constraints
  * minimal example
  * error behavior (what is returned/raised)

### Examples

If adding a feature, include at least one short example script showing:

* typical usage
* failure mode or edge case (if relevant)

---

## Testing Expectations

### What to test

* **Correctness:** main happy path, plus edge cases.
* **Error handling:** missing args, invalid types, OS errors.
* **Async behavior:** cancellation/timeout behavior where applicable.
* **Resource limits:** if your change touches execution limits, add a test to
protect regressions.

### Guidelines

* Prefer deterministic tests (avoid sleeps unless unavoidable).
* Keep tests fast. If something is slow by nature, mark it clearly and keep it minimal.

---

## API and Compatibility Guidelines

Ward is early stage, but we still try to avoid gratuitous breakage.

* Changes to Lua-visible APIs should be deliberate and documented.
* If a breaking change is necessary:

  * explain why in the PR description
  * update docs
  * update examples/tests accordingly

---

## Proposing Features

For new modules or meaningful behavior changes, please include:

1. **Lua API proposal**

   * module name
   * function names and signatures
   * return values + error conventions

2. **Rationale**

   * why this belongs in Ward core vs an external Lua library
   * why the design is Lua-idiomatic

3. **Security/safety considerations**

   * filesystem or process effects
   * whether it increases the "attack surface" (even if Ward isn't a hardened sandbox)

4. **Test plan**

   * how to cover main behavior
   * how to cover error cases

---

## Coding Style

### Rust

* Keep modules focused and readable.
* Use explicit error messages (and preserve source errors where possible).
* Avoid unnecessary allocations in hot paths, but don't micro-optimize at the
expense of clarity.
* Prefer clear lifetimes/ownership; avoid complex patterns unless required.

### Lua-facing semantics

* Be consistent in return conventions:

  * if a function returns `(value, err)` in Lua, keep it consistent for similar functions.
* Keep naming consistent across modules (`wait()`, `spawn()`, channel semantics,
etc.).

---

## Commit and PR Guidelines

### Commits

* Keep commits focused; avoid mixing refactors with behavior changes when possible.
* Write commit messages that describe intent.

Recommended format:

* `module: short summary`
* Example: `async: fix channel close semantics`

### Pull Requests

A PR should contain:

* clear description of the problem and solution
* any API changes (Lua-side) called out explicitly
* tests added/updated
* docs updated if user-visible behavior changed

If your PR changes:

* installer/release workflows: note how it was validated
* docs build: note how it was validated

---

## Release Notes (Optional but Helpful)

If your change is user-visible, include a short "release note" snippet in
the PR description, e.g.:

* Added: `ward.fs.xxx` supporting ...
* Fixed: `ward.async.channel` behavior on ...
* Changed: error message when ...

---

## Reporting Security Issues

If you discover a security-sensitive issue:

* Do not open a public issue with exploit details.
* Prefer a private report channel if available (e.g., a security email).
  If none is available yet, open an issue with minimal details and request a
private follow-up.

---

## License

By contributing, you agree that your contributions will be licensed under the
project's license (see `LICENSE-APACHE` and `LICENSE-MIT`).

---

## Thank You

Ward benefits a lot from careful review, reproducible bug reports, and small
focused PRs. Thanks for helping improve it.
