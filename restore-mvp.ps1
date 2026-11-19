$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
foreach ($entry in @('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'src', 'tests', 'migrations', 'scripts', '.vscode', '.gitignore', '.env.example', 'meme-hunter-src.tar.gz')) {
    if (Test-Path -LiteralPath (Join-Path $root $entry)) {
        throw "Refusing to overwrite '$entry'. The bundle is a historical snapshot; use an empty source tree."
    }
}
$bundle = Join-Path $root "bundle"
$parts = @()

for ($i = 0; $i -lt 9; $i++) {
    $name = "meme-hunter-src.tar.gz.b64.part{0:D2}" -f $i
    $path = Join-Path $bundle $name
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Missing bundle part: $path"
    }
    $parts += [System.IO.File]::ReadAllText($path, [System.Text.Encoding]::ASCII)
}

$encoded = [string]::Concat($parts)
try {
    $bytes = [Convert]::FromBase64String($encoded)
}
catch {
    throw "The bundle parts are not valid base64: $($_.Exception.Message)"
}

$archive = Join-Path $root "meme-hunter-src.tar.gz"
[System.IO.File]::WriteAllBytes($archive, $bytes)

$expectedSha256 = "81a1315ee42828b752d6a3ff866371e7abd7c79f4223a1419117b9e4dfc4d8a5"
$actualSha256 = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $expectedSha256) {
    Remove-Item -LiteralPath $archive -Force -ErrorAction SilentlyContinue
    throw "Archive integrity check failed. Expected $expectedSha256, got $actualSha256."
}

$tar = Get-Command tar -ErrorAction SilentlyContinue
if (-not $tar) {
    throw "The archive is verified, but 'tar' was not found. Install Windows tar or extract meme-hunter-src.tar.gz manually."
}

& $tar.Source -xkzf $archive -C $root
if ($LASTEXITCODE -ne 0) {
    throw "tar extraction failed with exit code $LASTEXITCODE."
}

Remove-Item -LiteralPath $archive
Write-Host "Meme Hunter historical Rust source restored successfully." -ForegroundColor Green
Write-Host "Open this folder in VS Code and run: cargo check; cargo test; cargo fmt --check"
