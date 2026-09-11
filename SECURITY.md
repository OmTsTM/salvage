# Security

Salvage runs elevated and writes to raw disks. A defect here does not corrupt a
file — it destroys a drive. This document states what the program guarantees,
how those guarantees are enforced, and how to report a failure of them.

## Reporting a vulnerability

Open a [security advisory](../../security/advisories/new) rather than a public
issue. Include the build version (shown beside the wordmark in the window), what
you observed, and what you expected.

## The guarantee

> No user data is ever written to a sector that the inspection did not prove
> good.

"Prove good" means the sector was written with a self-identifying pattern and
read back byte-identical. Sectors that were never inspected are excluded exactly
as firmly as sectors that failed: absence of proof is not proof of absence.

Two mechanisms enforce it, and `PartitionPlan::validate` checks whichever one a
plan declares, immediately before the partition table is written:

| Mechanism | Enforced by |
|---|---|
| `PartitionBoundary` | The interval contains only approved sectors, dilated away from every defect by a guard band |
| `FilesystemClusterMap` | A FAT32 table in which every cluster touching an unapproved sector is marked `0x0FFFFFF7`, which no driver allocates |

There is deliberately no "guaranteed" assurance level. The flash translation
layer remaps logical addresses to physical cells on every write, so no software
can promise that today's mapping holds tomorrow. A test
(`assurance_never_claims_a_guarantee`) fails the build if such wording returns.

## Remembered results

A card's sector map is stored after an inspection, under
`%LOCALAPPDATA%\Salvage\cards`, keyed to the card's fingerprint.

**A record is never a certificate.** It describes hardware as it was, and flash
degrades. Adopting one restores the picture and the planning and approves
nothing: the apply path refuses to write a data partition from a map that was
not verified in the current session, and says so rather than proceeding.

Records are matched on the fingerprint stored inside them, not on the filename,
so a digest collision or an edited file is refused rather than applied to the
wrong card.

## Releasing a card

Removing a layout restores the partition table the card arrived with, captured
the first time this program overwrote one. It requires the same typed consent as
any other destructive operation.

It restores capacity, which on a failing card is exactly what fencing existed to
withhold. The confirmation states that it removes the protection rather than the
damage, and that the inspection cannot be undone.

## Operational safety

- **Destructive operations require typing the device name.** Consent is a type
  (`DestructiveConsent`) obtainable only from the function that evaluates the
  safety guards, so no code path can reach a write without passing through them.
  The guarantee is structural, not a matter of discipline.
- **Consent is bound to the card's fingerprint** — model, serial, geometry. Swapping
  cards between approval and application aborts the operation. The physical disk
  index is deliberately excluded, because Windows recycles it.
- **System disks are blocked absolutely**, with no override: the disk hosting the
  operating system or the boot partition, buses used only by internal storage,
  capacity beyond what is addressable, and zero capacity.

## Privilege and process boundaries

The program requests Administrator in its manifest, because raw sector access is
not available without it. Everything an elevated process spawns inherits that
elevation, which shapes two rules:

- **External programs are launched by absolute path**, resolved through
  `GetSystemDirectoryW` or `GetWindowsDirectoryW`. Naming a bare executable
  resolves it by walking `PATH`, where a planted file ahead of the real one would
  run as Administrator.
- **Only two programs are ever launched**, both with arguments the window cannot
  influence: the system formatter, and Explorer for opening the diagnostic log
  or the author's page. Explorer is used deliberately — asked by an elevated
  process, it routes the request to the desktop shell running unelevated, so a
  browser or text editor never opens with Administrator rights.

## Memory safety

`unsafe` is **forbidden by the compiler** in the domain, application and window
crates. Every remaining use lives in a single file, `crates/salvage-win32/src/sys.rs`,
which is the border with the Windows API, and each carries a `SAFETY:` comment
naming the invariant that makes it sound. Wrappers there take `&OwnedHandle`
rather than a raw `HANDLE`, so holding the borrow is what proves the handle is
open.

## Dependencies

`cargo audit` runs in CI and the build fails on any advisory that is not
explicitly acknowledged below.

At the time of writing there are **no known vulnerabilities** across the
dependency tree. Seven advisories are acknowledged, all of them transitive
through Tauri and none reachable from this program:

- Six `unmaintained` notices (`proc-macro-error`, `unic-*`) — build-time macro
  and Unicode crates, not present at runtime.
- One `unsound` notice in `glib` — part of the GTK backend Tauri uses on Linux,
  which is not compiled into a Windows build.

## Scope

Salvage inspects storage the operator physically holds and has authorised it to
erase. It is a diagnostic and repair tool: it does not exfiltrate data, does not
communicate over the network, and the only file it writes outside the target
device is its own diagnostic log under `%LOCALAPPDATA%\Salvage`.
