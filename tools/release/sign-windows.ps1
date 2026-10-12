# Authenticode-sign one file with the certificate the release workflow imported, and verify it.
#
#   pwsh -File tools/release/sign-windows.ps1 -Path <file> -Thumbprint <thumbprint>
#
# Used for every Windows binary we ship: the app exe, the uninstaller, and the setup that carries both.
# Moved here from the workflow's inline step so all three get the same checks.
param(
  [Parameter(Mandatory = $true)][string]$Path,
  [Parameter(Mandatory = $true)][string]$Thumbprint
)
$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Path)) { throw "File to sign not found at $Path" }
$certificate = Get-Item "Cert:\CurrentUser\My\$Thumbprint"

# Set-AuthenticodeSignature is built into PowerShell, so this does not depend on locating
# signtool.exe inside whichever Windows SDK the image happens to carry.
Set-AuthenticodeSignature -FilePath $Path -Certificate $certificate `
  -HashAlgorithm SHA256 -TimestampServer "http://timestamp.digicert.com" | Out-Null

$signature = Get-AuthenticodeSignature -LiteralPath $Path
# A self-signed signer makes Get-AuthenticodeSignature report NotTrusted or UnknownError rather than
# Valid, on a runner that does not carry our root. Those two are accepted; HashMismatch and NotSigned
# are not, so an unsigned or modified file still fails here. The signer thumbprint and the
# countersignature are asserted unconditionally -- those say it is OUR certificate and that the signing
# time is attested, neither of which depends on the root being trusted.
$accepted = @("Valid", "NotTrusted", "UnknownError")
if ($accepted -notcontains $signature.Status.ToString()) {
  throw "$Path signature is not usable: $($signature.Status) ($($signature.StatusMessage))"
}
if (-not $signature.SignerCertificate) { throw "$Path carries no Authenticode signer certificate" }
if ($signature.SignerCertificate.Thumbprint -ne $Thumbprint) { throw "$Path was signed by an unexpected certificate" }
if (-not $signature.TimeStamperCertificate) { throw "$Path signature carries no trusted timestamp" }
Write-Host "signed $(Split-Path -Leaf $Path) by $($signature.SignerCertificate.Subject) status=$($signature.Status)"
