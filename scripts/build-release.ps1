[CmdletBinding()]
param(
    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string] $Distribution
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptDirectory = Split-Path -Parent $PSCommandPath
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDirectory '..')).Path
$isLinuxHost = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::Linux
)

if ($isLinuxHost) {
    & bash (Join-Path $scriptDirectory 'build-release.sh')
    if ($LASTEXITCODE -ne 0) {
        throw "The Linux release build failed with exit code $LASTEXITCODE."
    }
    exit 0
}

$isWindowsHost = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::Windows
)
if (-not $isWindowsHost) {
    throw 'Ubuntu release bundles must be built on Linux or from Windows through WSL.'
}

$wsl = Get-Command -Name 'wsl.exe' -ErrorAction SilentlyContinue
if ($null -eq $wsl) {
    throw 'WSL is unavailable. Build the Ubuntu bundle on a Linux host or configure WSL first.'
}

$wslArguments = @()
if ($PSBoundParameters.ContainsKey('Distribution')) {
    $wslArguments += @('--distribution', $Distribution)
}

$linuxPathOutput = & $wsl.Source @wslArguments --exec wslpath -a -u $repositoryRoot
if ($LASTEXITCODE -ne 0) {
    throw 'WSL could not resolve the Helix repository path. Confirm that the selected distribution is running and can access this drive.'
}

$linuxRepositoryRoot = ($linuxPathOutput | Out-String).Trim()
if ([string]::IsNullOrWhiteSpace($linuxRepositoryRoot)) {
    throw 'WSL returned an empty repository path.'
}

& $wsl.Source @wslArguments --exec bash -lc 'cd -- "$1" && exec bash scripts/build-release.sh' bash $linuxRepositoryRoot
if ($LASTEXITCODE -ne 0) {
    throw "The WSL release build failed with exit code $LASTEXITCODE."
}
