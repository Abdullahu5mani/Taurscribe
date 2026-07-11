param(
    [int]$LatencyLimit = 300,
    [int]$WerLimit = 0,
    [switch]$RunFullWer,
    [switch]$SkipBuild,
    [switch]$Force
)

$ErrorActionPreference = "Stop"

function Resolve-RepoRoot {
    Split-Path -Parent $PSScriptRoot
}

function Get-GpuUsedMiB {
    try {
        $value = & nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>$null |
            Select-Object -First 1
        if ([string]::IsNullOrWhiteSpace($value)) {
            return $null
        }
        return [int]$value.Trim()
    } catch {
        return $null
    }
}

function Get-DirSizeBytes([string]$Path) {
    if (-not (Test-Path $Path)) {
        return 0
    }
    return (Get-ChildItem $Path -Recurse -File | Measure-Object Length -Sum).Sum
}

function Read-LatencySummary([string]$Path) {
    if (-not (Test-Path $Path)) {
        return $null
    }
    $rows = @(Import-Csv $Path)
    if ($rows.Count -eq 0) {
        return $null
    }
    $wer = @($rows | ForEach-Object { [double]$_.wer } | Sort-Object)
    $transcribe = @($rows | ForEach-Object { [double]$_.transcribe_sec } | Sort-Object)
    $rtf = @($rows | ForEach-Object { [double]$_.rtf } | Sort-Object)
    $mid = [int]($rows.Count / 2)
    $p95 = [int][Math]::Round(($rows.Count - 1) * 0.95)

    [pscustomobject]@{
        Rows = $rows.Count
        WerMean = [Math]::Round((($wer | Measure-Object -Average).Average), 6)
        WerMedian = [Math]::Round($wer[$mid], 6)
        TranscribeMeanSec = [Math]::Round((($transcribe | Measure-Object -Average).Average), 6)
        TranscribeMedianSec = [Math]::Round($transcribe[$mid], 6)
        TranscribeP95Sec = [Math]::Round($transcribe[$p95], 6)
        RtfMean = [Math]::Round((($rtf | Measure-Object -Average).Average), 6)
        RtfMedian = [Math]::Round($rtf[$mid], 6)
        RtfP95 = [Math]::Round($rtf[$p95], 6)
    }
}

function Read-WerSummary([string]$Path) {
    if (-not (Test-Path $Path)) {
        return $null
    }
    $rows = @(Import-Csv $Path)
    if ($rows.Count -eq 0) {
        return $null
    }
    $wer = @($rows | ForEach-Object { [double]$_.wer } | Sort-Object)
    $mid = [int]($rows.Count / 2)
    [pscustomobject]@{
        Rows = $rows.Count
        WerMean = [Math]::Round((($wer | Measure-Object -Average).Average), 6)
        WerMedian = [Math]::Round($wer[$mid], 6)
    }
}

function Invoke-MonitoredProcess {
    param(
        [string]$Exe,
        [string[]]$ArgumentList,
        [string]$WorkingDirectory,
        [string]$Stdout,
        [string]$Stderr
    )

    Remove-Item $Stdout, $Stderr -ErrorAction SilentlyContinue
    $gpuBase = Get-GpuUsedMiB
    $start = Get-Date
    $process = Start-Process `
        -FilePath $Exe `
        -ArgumentList $ArgumentList `
        -WorkingDirectory $WorkingDirectory `
        -WindowStyle Hidden `
        -RedirectStandardOutput $Stdout `
        -RedirectStandardError $Stderr `
        -PassThru

    $peakWs = 0L
    $gpuPeak = $gpuBase
    while (-not $process.HasExited) {
        try {
            $p = Get-Process -Id $process.Id -ErrorAction Stop
            if ($p.WorkingSet64 -gt $peakWs) {
                $peakWs = $p.WorkingSet64
            }
        } catch {
        }

        $gpu = Get-GpuUsedMiB
        if ($null -ne $gpu -and ($null -eq $gpuPeak -or $gpu -gt $gpuPeak)) {
            $gpuPeak = $gpu
        }
        Start-Sleep -Milliseconds 500
    }
    $process.WaitForExit()
    $end = Get-Date

    $gpuDelta = $null
    if ($null -ne $gpuBase -and $null -ne $gpuPeak) {
        $gpuDelta = [Math]::Max(0, $gpuPeak - $gpuBase)
    }

    [pscustomobject]@{
        ExitCode = $process.ExitCode
        WallSec = [Math]::Round(($end - $start).TotalSeconds, 2)
        PeakRAMMiB = [Math]::Round($peakWs / 1MB, 1)
        GpuBaseMiB = $gpuBase
        GpuPeakMiB = $gpuPeak
        ApproxVRAMDeltaMiB = $gpuDelta
    }
}

$repoRoot = Resolve-RepoRoot
$tauriRoot = Join-Path $repoRoot "src-tauri"
$runtimeRoot = Join-Path $repoRoot "taurscribe-runtime"
$librispeechRoot = Join-Path $runtimeRoot "librispeech"
$testCleanRoot = Join-Path $librispeechRoot "LibriSpeech\test-clean"
$outRoot = Join-Path $librispeechRoot "granite_benchmarks"
New-Item -ItemType Directory -Force -Path $outRoot | Out-Null

$manifestAll = Join-Path $librispeechRoot "eval_manifest_all.jsonl"
$manifestLatency = Join-Path $outRoot "eval_manifest_latency_$LatencyLimit.jsonl"
$manifestWer = if ($WerLimit -gt 0) {
    Join-Path $outRoot "eval_manifest_wer_$WerLimit.jsonl"
} else {
    $manifestAll
}

if (-not (Test-Path $testCleanRoot)) {
    throw "Missing LibriSpeech test-clean: $testCleanRoot"
}

Push-Location $tauriRoot
try {
    if (-not $SkipBuild) {
        cargo build --release --bin granite_latency_bench --bin librispeech_eval
    }

    if (-not (Test-Path $manifestAll) -or $Force) {
        cargo run --release --bin librispeech_manifest -- --root $testCleanRoot --out $manifestAll --shuffle-seed 42
    }

    Get-Content $manifestAll -TotalCount $LatencyLimit | Set-Content $manifestLatency
    if ($WerLimit -gt 0) {
        Get-Content $manifestAll -TotalCount $WerLimit | Set-Content $manifestWer
    }

    $models = @(
        [pscustomobject]@{
            Name = "int4"
            Dir = Join-Path $env:LOCALAPPDATA "Taurscribe\models\granite-speech-4.1-2b-nar-int4-matmul"
        },
        [pscustomobject]@{
            Name = "int8"
            Dir = Join-Path $env:LOCALAPPDATA "Taurscribe\models\granite-speech-4.1-2b-nar"
        },
        [pscustomobject]@{
            Name = "fp32"
            Dir = Join-Path $env:LOCALAPPDATA "Taurscribe\models\granite-speech-4.1-2b-nar-fp32-backup"
        }
    )

    $summary = @()
    foreach ($model in $models) {
        if (-not (Test-Path $model.Dir)) {
            Write-Warning "Skipping $($model.Name): missing $($model.Dir)"
            continue
        }

        $latencyCsv = Join-Path $outRoot "granite_latency_$($model.Name)_$LatencyLimit.csv"
        $latencyStdout = Join-Path $outRoot "granite_latency_$($model.Name)_$LatencyLimit.stdout.log"
        $latencyStderr = Join-Path $outRoot "granite_latency_$($model.Name)_$LatencyLimit.stderr.log"

        Write-Host "Running latency benchmark: $($model.Name)"
        Remove-Item $latencyCsv -ErrorAction SilentlyContinue
        $latencyRun = Invoke-MonitoredProcess `
            -Exe (Join-Path $tauriRoot "target\release\granite_latency_bench.exe") `
            -ArgumentList @("--manifest", $manifestLatency, "--out", $latencyCsv, "--model-dir", $model.Dir) `
            -WorkingDirectory $tauriRoot `
            -Stdout $latencyStdout `
            -Stderr $latencyStderr
        $latency = Read-LatencySummary $latencyCsv

        $werCsv = $null
        $wer = $null
        $werRun = $null
        if ($RunFullWer -or $WerLimit -gt 0) {
            $werName = if ($WerLimit -gt 0) { $WerLimit } else { "all" }
            $werCsv = Join-Path $outRoot "granite_wer_$($model.Name)_$werName.csv"
            $werStdout = Join-Path $outRoot "granite_wer_$($model.Name)_$werName.stdout.log"
            $werStderr = Join-Path $outRoot "granite_wer_$($model.Name)_$werName.stderr.log"

            Write-Host "Running WER benchmark: $($model.Name)"
            Remove-Item $werCsv -ErrorAction SilentlyContinue
            $previous = $env:TAURSCRIBE_GRANITE_MODEL_ID
            $env:TAURSCRIBE_GRANITE_MODEL_ID = $model.Dir
            try {
                $werRun = Invoke-MonitoredProcess `
                    -Exe "cargo" `
                    -ArgumentList @("run", "--release", "--bin", "librispeech_eval", "--", "--manifest", $manifestWer, "--out", $werCsv, "--engines", "granite") `
                    -WorkingDirectory $tauriRoot `
                    -Stdout $werStdout `
                    -Stderr $werStderr
            } finally {
                if ($null -eq $previous) {
                    Remove-Item Env:\TAURSCRIBE_GRANITE_MODEL_ID -ErrorAction SilentlyContinue
                } else {
                    $env:TAURSCRIBE_GRANITE_MODEL_ID = $previous
                }
            }
            $wer = Read-WerSummary $werCsv
        }

        $summary += [pscustomobject]@{
            Model = $model.Name
            ModelDir = $model.Dir
            DiskGiB = [Math]::Round((Get-DirSizeBytes $model.Dir) / 1GB, 3)
            LatencyRun = $latencyRun
            Latency = $latency
            WerRun = $werRun
            Wer = $wer
            LatencyCsv = $latencyCsv
            WerCsv = $werCsv
        }

        $summary | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $outRoot "granite_benchmark_summary.json")
    }

    $summary | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $outRoot "granite_benchmark_summary.json")
    $summary | ConvertTo-Json -Depth 8
} finally {
    Pop-Location
}
