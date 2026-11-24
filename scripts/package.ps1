# 打包给别人用的 zip：dist\AGY-Switch-<版本>-win-x64.zip，内含 agy-switch.exe 与 使用说明.txt。
# 编译时把本机用户目录映射成 ~，exe 里不留你的用户名；单独的 target-dist 目录，不影响平时的增量构建。
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$ws = Split-Path $PSScriptRoot -Parent
$ver = (Get-Content (Join-Path $ws 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
$targetDir = Join-Path $ws 'src-tauri\target-dist'

if (-not $SkipBuild) {
    # 依赖库路径用 remap 改成 ~；tauri 宏会把项目目录原样写进 exe，remap 管不到，
    # 所以借一个临时盘符（subst）在不含用户名的路径下编译，结束即撤销。
    $drive = [char[]]'PQRSTUVWXYZ' | Where-Object { -not (Test-Path "${_}:\") } | Select-Object -First 1
    if (-not $drive) { throw '找不到空闲盘符' }
    subst "${drive}:" $ws
    if ($LASTEXITCODE -ne 0) { throw "subst ${drive}: 失败" }
    $env:CARGO_ENCODED_RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~"
    try {
        cargo build --locked --release --manifest-path "${drive}:\src-tauri\Cargo.toml" --target-dir "${drive}:\src-tauri\target-dist"
        if ($LASTEXITCODE -ne 0) { throw "cargo build 失败 ($LASTEXITCODE)" }
    } finally {
        Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
        subst "${drive}:" /D
    }
}
$exe = Join-Path $targetDir 'release\agy-switch.exe'

# 发出去之前确认 exe 里没有本机用户目录
$bytes = [IO.File]::ReadAllBytes($exe)
foreach ($enc in [Text.Encoding]::ASCII, [Text.Encoding]::Unicode) {
    if ($enc.GetString($bytes).IndexOf($env:USERPROFILE, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        throw "exe 中仍含本机用户目录 $env:USERPROFILE，已中止打包"
    }
}

$name = "AGY-Switch-$ver-win-x64"
$dist = Join-Path $ws 'dist'
$stage = Join-Path $dist $name
if (-not ([IO.Path]::GetFullPath($stage).StartsWith([IO.Path]::GetFullPath($dist) + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase))) {
    throw '打包目录超出了 dist 范围'
}
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
[IO.Directory]::CreateDirectory($stage) | Out-Null
. (Join-Path $PSScriptRoot 'write-release-exe.ps1')
Write-ReleaseExecutable -Source $exe -Destination (Join-Path $stage 'agy-switch.exe')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'share-readme.txt') -Destination (Join-Path $stage '使用说明.txt')
$zip = Join-Path $dist "$name.zip"
if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
$hash = (Get-FileHash -LiteralPath (Join-Path $stage 'agy-switch.exe') -Algorithm SHA256).Hash
Write-Output $zip
Write-Output "agy-switch.exe SHA256 $hash"
