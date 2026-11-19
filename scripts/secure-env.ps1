$ErrorActionPreference = "Stop"
$path = Join-Path (Get-Location) ".env"
if (-not (Test-Path $path)) { throw ".env not found. Copy .env.example to .env first." }
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
icacls $path /inheritance:r | Out-Null
icacls $path /grant:r "${identity}:(F)" | Out-Null
Write-Host "Restricted .env ACL to $identity. Note: the file remains plaintext."
