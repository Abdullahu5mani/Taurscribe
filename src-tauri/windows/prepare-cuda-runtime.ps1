param(
    [Parameter(Mandatory=$true)][string]$Manifest,
    [Parameter(Mandatory=$true)][string]$StageDirectory,
    [string]$InstallDirectory,
    [switch]$DisableLocalReuse
)

# Called before NSIS copies the app. Download only the pinned cuBLAS component,
# verify the archive and individual DLLs, and stage the two required files.
# Never install the toolkit/driver or keep a second runtime cache on disk.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$work = $null
try {
    $spec = Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json
    if ($spec.schemaVersion -ne 1 -or $spec.files.Count -ne 2) { throw 'Invalid CUDA runtime manifest' }
    $uri = [Uri]$spec.url
    if ($uri.Scheme -ne 'https' -or $uri.Host -ne 'developer.download.nvidia.com') { throw 'Unexpected CUDA download source' }
    if ($spec.archiveSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw 'Invalid CUDA archive checksum' }
    $names = @($spec.files | ForEach-Object { $_.name })
    if (@($names | Sort-Object -Unique).Count -ne 2 -or
        @($names | Where-Object { $_ -notmatch '^cublas(Lt)?64_\d+\.dll$' }).Count -ne 0) { throw 'Invalid CUDA DLL names' }
    foreach ($file in $spec.files) {
        if ($file.bytes -le 0 -or $file.sha256 -notmatch '^[0-9a-fA-F]{64}$') { throw 'Invalid CUDA DLL checksum' }
    }
    function Test-PinnedFile([string]$Path, $File) {
        if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
        if ((Get-Item -LiteralPath $Path).Length -ne $File.bytes) { return $false }
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -eq $File.sha256
    }

    New-Item -ItemType Directory -Force -Path $StageDirectory | Out-Null
    $work = Join-Path $StageDirectory ([Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work | Out-Null
    $missing = @()
    foreach ($file in $spec.files) {
        $source = $null
        if (-not $DisableLocalReuse) {
            $candidates = @()
            if ($InstallDirectory) { $candidates += Join-Path $InstallDirectory $file.name }
            if ($env:CUDA_PATH) {
                $candidates += Join-Path $env:CUDA_PATH "bin\x64\$($file.name)"
                $candidates += Join-Path $env:CUDA_PATH "bin\$($file.name)"
            }
            foreach ($candidate in $candidates) {
                if (Test-PinnedFile $candidate $file) { $source = $candidate; break }
            }
        }
        if ($source) {
            Copy-Item -LiteralPath $source -Destination (Join-Path $work $file.name)
            Write-Output "Reused verified $($file.name)"
        } else { $missing += $file }
    }

    if ($missing.Count -gt 0) {
        Write-Output "Downloading $($spec.component) from NVIDIA..."
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $zip = Join-Path $work 'runtime.zip'
        $client = New-Object Net.WebClient
        try { $client.DownloadFile($uri, $zip) } finally { $client.Dispose() }
        if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash -ne $spec.archiveSha256) { throw 'CUDA archive checksum mismatch' }
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $archive = [IO.Compression.ZipFile]::OpenRead($zip)
        try {
            foreach ($file in $missing) {
                $entries = @($archive.Entries | Where-Object { $_.Name -ceq $file.name })
                if ($entries.Count -ne 1) { throw "Missing or ambiguous $($file.name) in CUDA archive" }
                [IO.Compression.ZipFileExtensions]::ExtractToFile($entries[0], (Join-Path $work $file.name), $true)
            }
        } finally { $archive.Dispose() }
    }

    # Validate everything before publishing any DLL. Never accept an arbitrary
    # same-named DLL from a toolkit, previous install, or downloaded archive.
    foreach ($file in $spec.files) {
        if (-not (Test-PinnedFile (Join-Path $work $file.name) $file)) { throw "CUDA DLL checksum mismatch: $($file.name)" }
    }
    foreach ($file in $spec.files) {
        Move-Item -LiteralPath (Join-Path $work $file.name) -Destination (Join-Path $StageDirectory $file.name) -Force
    }
    Write-Output 'NVIDIA runtime ready.'
    exit 0
} catch {
    [Console]::Error.WriteLine("NVIDIA runtime setup failed: $($_.Exception.Message)")
    exit 1
} finally {
    if ($work -and (Test-Path -LiteralPath $work)) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
}
