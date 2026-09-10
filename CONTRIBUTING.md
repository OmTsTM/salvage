# Contributing

Salvage decides where a user's data is allowed to live on failing hardware. A
bug here does not produce a wrong pixel; it produces a partition that silently
eats files. The rules below exist for that reason.

## The invariant

> No user data may ever land on a sector that was not explicitly approved.

Never-inspected sectors stay out as firmly as failed ones — absence of proof is
not proof of absence. `PartitionPlan::validate` enforces this and is called
again immediately before the partition table is written. A change that relaxes
it will not be merged.

Two mechanisms deliver that promise, and `Containment` says which one a plan
uses: a partition boundary that admits only approved sectors, or a FAT32
allocation table that withholds every cluster touching an unapproved one.
Adding a third means teaching `validate` how to prove it, not adding a case that
skips the check.

The diagnosis carries the same rule: while any sector remains uninspected there
is no verdict, however clean the inspected part looked.

Related, and equally non-negotiable: there is no "guaranteed" assurance level,
and `assurance_never_claims_a_guarantee` fails the build if one is introduced.
The Flash Translation Layer remaps addresses on every write; a guarantee would
be a lie the code cannot back.

## Layering

```
salvage-core   pure domain — no I/O, no OS, no dependencies
salvage-app    use cases and ports (traits) — no OS
salvage-win32  the only place unsafe exists, and only in sys.rs
src-tauri, ui  presentation — no business rules
```

Dependencies point one way only. `salvage-core` and `salvage-app` declare
`#![forbid(unsafe_code)]`, and CI builds both on Linux to keep the "no OS
dependency" claim honest rather than aspirational.

**The domain returns classifications and numbers, never sentences.** Wording
lives in the presentation layers: English in `salvage-win32/src/text.rs` for the
CLI tools, and `ui/i18n.js` for the window — Brazilian Portuguese, English,
Spanish and Simplified Chinese, one flat dictionary each. If you find yourself
writing a user-facing sentence inside `salvage-core` or `salvage-app`, that is
the signal you are in the wrong layer.

That rule is what made four languages a matter of one file. Two corollaries
hold it in place, and both are easy to break by accident:

- **The bridge returns keys, not prose.** A command's `Err` is a lookup key
  (`err.no_device`), and a report detail is a kind plus its numbers, never an
  assembled sentence. `t()` falls through to whatever it was handed, so a
  technical error from a lower layer still shows up — but anything a user is
  expected to act on needs a key and four entries.
- **Numbers are formatted in the window, never below it.** `src-tauri` sends
  byte counts and sector counts; `ui/i18n.js` renders them through `Intl`. A
  thousands separator is a period in Brazil and a comma in the United States,
  and a string the backend already formatted cannot be re-formatted here. There
  is no `format::bytes` call left in `src-tauri/src/main.rs`, and adding one
  back is how this quietly regresses.

## Before opening a pull request

```powershell
node --check ui/app.js
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All four run in CI and all four must be clean. The parse check is there because
nothing compiles `ui/app.js`: a syntax error in it costs nothing at build time
and everything at run time, since the window draws from `index.html` and then
sits there with no script behind it.

`cargo test --workspace` needs an elevated shell. `salvage-gui` embeds a
manifest requesting Administrator, its test binary inherits it, and Windows
refuses to start such a binary otherwise (os error 740). From a normal shell,
run `cargo test --workspace --exclude salvage-gui`.

## Tests

Anything touching the decision of where data may be placed is tested against
`salvage-app::simulator`, which reproduces counterfeit capacity, read and write
errors, silent corruption, stuck cells, transient failures, and a card that
acknowledges every write and retains nothing. Hardware is not required, and a
pull request that changes scanning or planning without a simulator test covering
the new behaviour is incomplete.

Name tests after the property they defend, not after the function they call:
`a_read_only_scan_is_never_reported_as_healthy` says what breaks if it fails.

## Comments

Comments in this repository are in technical English and explain **why**, not
what. `// increments the counter` adds nothing. `// bisection does not advance
the phase counter` prevents the next reader from removing the line.

## Unsafe

New `unsafe` belongs in `crates/salvage-win32/src/sys.rs` and nowhere else, and
every use carries a `SAFETY:` comment stating the invariant that makes it sound.
Prefer taking `&OwnedHandle` over a raw `HANDLE`: holding the borrow is what
proves the handle is open, which is why the wrappers in that module are safe
functions at all.
