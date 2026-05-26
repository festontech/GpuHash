#requires -Version 5.1
<#
.SYNOPSIS
  Phase 6 smoke test for `gpuhash` — exercises --json, exit codes, and the
  full session lifecycle (save/list/load/show/delete).

.DESCRIPTION
  Builds the CLI once, then invokes the binary directly (no `cargo run`) so
  PowerShell 5.1's NativeCommandError quirk around `2>&1` doesn't trip the
  script. Validates:

    * NDJSON shape (Started -> Progress* -> Match* -> Finished)
    * Exit code 1 when matches are found (audit "failed open" — ARCHITECTURE.md §7.4)
    * Exit code 0 when no matches
    * Exit code 2 when --i-own-these-hashes is missing (refusal)
    * session save / list / load / show / delete round-trip

  Sessions are isolated by setting GPUHASH_SESSIONS_DIR to a temp dir for the
  duration of the run.

.EXAMPLE
  pwsh scripts/smoke.ps1
#>

[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string] $CargoProfile = 'release'
)

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

function Fail($msg) {
    Write-Host "[FAIL] $msg" -ForegroundColor Red
    exit 1
}

function Pass($msg) {
    Write-Host "[ OK ] $msg" -ForegroundColor Green
}

# Build once, then call the binary directly so we don't have to fight
# PS5.1's NativeCommandError behavior around cargo's stderr.
Write-Host "Building $CargoProfile..." -ForegroundColor Cyan
if ($CargoProfile -eq 'release') {
    & cargo build --release -p gpuhash-cli
} else {
    & cargo build -p gpuhash-cli
}
if ($LASTEXITCODE -ne 0) { Fail "cargo build failed (exit $LASTEXITCODE)" }

# CARGO_TARGET_DIR override is honored by cargo; default lands under
# %LOCALAPPDATA%\cargo-target on this machine. Ask cargo where the binary is.
$metaJson = & cargo metadata --no-deps --format-version 1
if ($LASTEXITCODE -ne 0) { Fail "cargo metadata failed" }
$meta = ($metaJson -join "`n") | ConvertFrom-Json
$gpuhash = Join-Path $meta.target_directory "$CargoProfile/gpuhash.exe"
if (-not (Test-Path $gpuhash)) { Fail "could not find built binary at $gpuhash" }
Write-Host "Binary: $gpuhash" -ForegroundColor DarkGray

function Invoke-Gpuhash {
    param([string[]] $CliArgs)
    $out = & $gpuhash @CliArgs
    return @{ stdout = $out; exit = $LASTEXITCODE }
}

# Isolate sessions in a temp dir.
$tmp = Join-Path $env:TEMP "gpuhash-smoke-$([guid]::NewGuid().Guid.Substring(0,8))"
New-Item -ItemType Directory -Force $tmp | Out-Null
$env:GPUHASH_SESSIONS_DIR = $tmp
Write-Host "Sessions dir: $tmp" -ForegroundColor DarkGray

try {
    # 1. Refusal path — exit code 2.
    $r = Invoke-Gpuhash @('attack', '--algo', 'md5',
        '--hashes', 'examples/sample_hashes.txt',
        '--wordlist', 'examples/tiny_dict.txt')
    if ($r.exit -ne 2) { Fail "missing --i-own-these-hashes should exit 2; got $($r.exit)" }
    Pass "refuses without --i-own-these-hashes (exit=2)"

    # 2. Match path with --json — exit code 1 + NDJSON shape.
    $r = Invoke-Gpuhash @('attack', '--algo', 'md5',
        '--hashes', 'examples/sample_hashes.txt',
        '--wordlist', 'examples/tiny_dict.txt',
        '--i-own-these-hashes', '--json')
    if ($r.exit -ne 1) { Fail "matches found should exit 1; got $($r.exit)" }

    $events = $r.stdout | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json }
    if (-not $events) { Fail "no NDJSON events parsed" }
    $first = $events | Select-Object -First 1
    $last  = $events | Select-Object -Last 1
    if ($first.type -ne 'Started')  { Fail "first event should be Started; got $($first.type)" }
    if ($last.type  -ne 'Finished') { Fail "last event should be Finished; got $($last.type)" }
    $matchCount = ($events | Where-Object { $_.type -eq 'Match' }).Count
    if ($matchCount -ne 10) { Fail "expected 10 Match events; got $matchCount" }
    Pass "NDJSON: Started + 10 Match + Finished (exit=1)"

    # 3. No-match path — exit code 0.
    $emptyDict = Join-Path $tmp 'empty.txt'
    "no-such-password-xxx" | Set-Content -Encoding ascii $emptyDict
    $r = Invoke-Gpuhash @('attack', '--algo', 'md5',
        '--hashes', 'examples/sample_hashes.txt',
        '--wordlist', $emptyDict,
        '--i-own-these-hashes')
    if ($r.exit -ne 0) { Fail "no matches should exit 0; got $($r.exit)" }
    Pass "no matches -> exit=0"

    # 4. session save (no-run).
    $r = Invoke-Gpuhash @('session', 'save', '--name', 'smoke',
        '--algo', 'md5',
        '--hashes', 'examples/sample_hashes.txt',
        '--wordlist', 'examples/tiny_dict.txt')
    if ($r.exit -ne 0) { Fail "session save failed (exit $($r.exit))" }
    Pass "session save"

    # 5. session list — must include 'smoke' with status 'saved'.
    $r = Invoke-Gpuhash @('session', 'list')
    $listText = ($r.stdout -join "`n")
    if ($listText -notmatch 'smoke\s+saved') { Fail "session list did not mention 'smoke saved': $listText" }
    Pass "session list shows saved entry"

    # 6. session show — JSON parseable.
    $r = Invoke-Gpuhash @('session', 'show', 'smoke')
    $shown = ($r.stdout -join "`n") | ConvertFrom-Json
    if ($shown.name -ne 'smoke')             { Fail "session show: wrong name" }
    if ($shown.status -ne 'saved')           { Fail "session show: wrong status" }
    if ($shown.config.algo -ne 'md5')        { Fail "session show: wrong algo" }
    Pass "session show returns valid JSON"

    # 7. session load — runs the attack from the saved config; exit=1 (matches).
    $r = Invoke-Gpuhash @('session', 'load', 'smoke', '--i-own-these-hashes', '--json')
    if ($r.exit -ne 1) { Fail "session load with matches should exit 1; got $($r.exit)" }
    $events = $r.stdout | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json }
    $matchCount = ($events | Where-Object { $_.type -eq 'Match' }).Count
    if ($matchCount -ne 10) { Fail "session load: expected 10 matches; got $matchCount" }
    Pass "session load executes saved config (10 matches)"

    # 8. session show should now report status=finished + 10 matches stored.
    $r = Invoke-Gpuhash @('session', 'show', 'smoke')
    $shown = ($r.stdout -join "`n") | ConvertFrom-Json
    if ($shown.status -ne 'finished')        { Fail "post-load status should be 'finished', got '$($shown.status)'" }
    if ($shown.matches.Count -ne 10)         { Fail "post-load expected 10 stored matches; got $($shown.matches.Count)" }
    Pass "post-load session file updated (finished, 10 matches)"

    # 9. session delete — idempotent.
    $r = Invoke-Gpuhash @('session', 'delete', 'smoke')
    if ($r.exit -ne 0) { Fail "first delete should succeed" }
    $r = Invoke-Gpuhash @('session', 'delete', 'smoke')
    if ($r.exit -ne 0) { Fail "second delete should also exit 0 (idempotent)" }
    Pass "session delete is idempotent"

    # 10. List should be empty again.
    $r = Invoke-Gpuhash @('session', 'list')
    $listText = ($r.stdout -join "`n")
    if ($listText -match 'smoke') { Fail "deleted session still listed: $listText" }
    Pass "list empty after delete"

    Write-Host ""
    Write-Host "All Phase 6 smoke checks passed." -ForegroundColor Green
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    Remove-Item Env:GPUHASH_SESSIONS_DIR -ErrorAction SilentlyContinue
}
