<#
.SYNOPSIS
    Builds the distributable Salvage installer and portable executable.

.DESCRIPTION
    One command from a clean checkout to something a person can double-click.
    The icon is regenerated first, because it is produced by a script rather
    than committed art and the Tauri build caches it; then the same four checks
    CI runs are run locally, so a release is never cut from a tree that would
    fail CI; then the bundle is built and both artifacts are collected in dist/.

    Passing -Version rewrites the version in the workspace manifest, which is the
    single source of truth: tauri.conf.json deliberately does not carry one, so
    the installer, the window and the crate can never disagree.

.PARAMETER Version
    New version, as x.y.z. Omit to build the version already in Cargo.toml.

.PARAMETER SkipChecks
    Skips the script check, fmt, clippy and tests. For iterating on packaging
    only — never for a release you intend to hand to somebody.

.EXAMPLE
    pwsh tools/build_release.ps1
    pwsh tools/build_release.ps1 -Version 0.2.0
#>
[CmdletBinding()]
param(
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$Version,

    [switch]$SkipChecks
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

function Step($text) { Write-Host "`n=== $text" -ForegroundColor Cyan }

try {
    if ($Version) {
        Step "Setting version to $Version"
        $manifest = Join-Path $root 'Cargo.toml'
        $text = Get-Content $manifest -Raw
        $field = '(?m)^(version\s*=\s*")\d+\.\d+\.\d+(")'
        # The absence of the field is the failure. Text that comes back
        # unchanged is not: re-running the same release command finds the
        # version already set, and treating that as a missing field made the
        # second attempt at a build fail for a reason that was not true.
        if (-not [regex]::IsMatch($text, $field)) {
            throw "could not find the version field in $manifest"
        }
        Set-Content $manifest ([regex]::Replace($text, $field, "`${1}$Version`${2}", 1)) -NoNewline
    }

    $cargoVersion = ([regex]::Match(
        (Get-Content (Join-Path $root 'Cargo.toml') -Raw),
        '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"')).Groups[1].Value
    Write-Host "Building Salvage $cargoVersion"

    Step 'Reinstalling the icon'
    # Rendered from the committed source every time, so the shipped sizes can
    # never drift from it. The Tauri build script embeds the icon and does not
    # track the file, so a stale one would otherwise survive a clean rebuild.
    python tools/install_icon.py assets/icon-source.png
    if ($LASTEXITCODE -ne 0) { throw 'the icon could not be installed' }
    # --release matters: without it this cleans the debug profile, the release
    # build script never re-runs, and the linker quietly reuses the resource
    # compiled from the previous icon.
    cargo clean -p salvage-gui --release

    if (-not $SkipChecks) {
        # The window's script is never compiled, so nothing else in this build
        # would notice a syntax error in it. Version 0.3.0 shipped with a
        # duplicated `const` block: the script failed to parse, no code ran,
        # and the device list sat on its placeholder text forever. A parse
        # check costs a second and is the only thing standing between that
        # class of mistake and an installer.
        Step 'Window scripts'
        # Every one of them, found rather than listed: the check existed and
        # covered only app.js when i18n.js and splash-boot.js arrived, which is
        # the same hole in a smaller shape.
        foreach ($script in Get-ChildItem 'ui' -Filter '*.js') {
            node --check $script.FullName
            if ($LASTEXITCODE -ne 0) { throw "ui/$($script.Name) does not parse" }
        }

        Step 'Formatting'
        cargo fmt --all -- --check
        if ($LASTEXITCODE -ne 0) { throw 'cargo fmt reported differences' }

        Step 'Clippy'
        cargo clippy --workspace --all-targets -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'clippy reported warnings' }

        Step 'Tests'
        # The window crate embeds a manifest demanding Administrator, and
        # Windows refuses to launch anything carrying it from an unelevated
        # shell — the test harness included, which fails as os error 740. From
        # a normal shell the rest of the workspace is still tested and the two
        # helpers in the window crate wait for an elevated run, rather than
        # taking the whole checks block down with them: a script that always
        # fails here is a script people start passing -SkipChecks to, and that
        # is how a window that could not parse once reached an installer.
        $elevated = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()
            ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        if ($elevated) {
            cargo test --workspace
        } else {
            cargo test --workspace --exclude salvage-gui
        }
        if ($LASTEXITCODE -ne 0) { throw 'the test suite failed' }
        if (-not $elevated) {
            Write-Warning 'salvage-gui tests skipped: they need an elevated shell (os error 740)'
        }
    }

    Step 'Bundling'
    # Old installers are removed first. The bundle directory accumulates one per
    # version built, and picking a file out of it by pattern alone once shipped
    # a previous build under the current version's name.
    $bundleDir = Join-Path $root 'target/release/bundle/nsis'
    if (Test-Path $bundleDir) {
        Get-ChildItem $bundleDir -Filter '*-setup.exe' | Remove-Item -Force
    }

    cargo tauri build
    if ($LASTEXITCODE -ne 0) { throw 'the bundle step failed' }

    Step 'Collecting artifacts'
    $dist = Join-Path $root 'dist'
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    # Stale artifacts are cleared best-effort. A previously built portable exe
    # may be running right now — plausibly the very scan this build is meant to
    # speed up — and a locked leftover must not fail a build that succeeded.
    foreach ($old in Get-ChildItem $dist -File) {
        try { Remove-Item $old.FullName -Force -ErrorAction Stop }
        catch { Write-Warning "kept $($old.Name): in use" }
    }

    # Matched on the version being built, not on whatever the pattern finds
    # first: the bundler names the file after the version, and selecting by
    # anything looser silently ships the wrong build under the right name.
    $expected = "Salvage_${cargoVersion}_x64-setup.exe"
    $installer = Get-ChildItem $bundleDir -Filter $expected -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $installer) {
        throw "the bundler produced no $expected"
    }
    Copy-Item $installer.FullName (Join-Path $dist "Salvage-$cargoVersion-setup.exe")

    # The raw executable, for people who cannot or will not run an installer.
    # It needs the WebView2 runtime, which the installer would have supplied.
    $portable = Join-Path $root 'target/release/salvage-gui.exe'
    if (Test-Path $portable) {
        Copy-Item $portable (Join-Path $dist "Salvage-$cargoVersion-portable.exe")
    }

    Step 'Done'
    Get-ChildItem $dist | Select-Object Name, @{
        Name = 'Size'
        Expression = { '{0:N1} MB' -f ($_.Length / 1MB) }
    } | Format-Table -AutoSize
}
finally {
    Pop-Location
}
