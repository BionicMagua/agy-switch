# 在普通临时目录创建文件，再移动到目标位置，避免继承构建 / dist 目录的 Low 标签。
# 不修改任何目录或现有文件的安全描述符。
function Test-ReleaseIntegrity {
    param([string]$Path)
    $startInfo = [Diagnostics.ProcessStartInfo]::new($Path)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.Arguments = '--check-integrity'
    $probe = [Diagnostics.Process]::Start($startInfo)
    try {
        if (-not $probe.WaitForExit(10000)) {
            $probe.Kill()
            throw '程序完整性检查超时'
        }
        return $probe.ExitCode -eq 0
    } finally {
        $probe.Dispose()
    }
}

function Write-ReleaseExecutable {
    param([string]$Source, [string]$Destination)
    $temporaryExe = Join-Path ([IO.Path]::GetTempPath()) ('agy-release-' + [guid]::NewGuid().ToString('N') + '.exe')
    try {
        [IO.File]::WriteAllBytes($temporaryExe, [IO.File]::ReadAllBytes($Source))
        if (-not (Test-ReleaseIntegrity -Path $temporaryExe)) {
            throw '发布进程生成了低完整性程序。请在普通用户 PowerShell（构建沙箱外）运行发布脚本。'
        }
        # 跨卷移动会继承目标权限，移动后必须再检查；普通 CI 目录可正常通过。
        Move-Item -LiteralPath $temporaryExe -Destination $Destination -Force
        if (-not (Test-ReleaseIntegrity -Path $Destination)) {
            throw '发布后程序仍为低完整性。请使用普通用户目录，或将 TEMP 设为目标磁盘上的普通临时目录后重试。'
        }
    } finally {
        if (Test-Path -LiteralPath $temporaryExe) { Remove-Item -LiteralPath $temporaryExe }
    }
}
