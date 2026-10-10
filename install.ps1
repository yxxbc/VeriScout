# VeriScout (ves) installer — Windows (x86_64).
#
#   irm https://raw.githubusercontent.com/yxxbc/VeriScout/main/install.ps1 | iex
#
# Options: -Version <tag>, -InstallDir <path>, -Repo <owner/repo>
# Env overrides: VES_VERSION, VES_INSTALL_DIR, VES_REPO, VES_BASE_URL
param(
    [string]$Version = $env:VES_VERSION,
    [string]$InstallDir = $env:VES_INSTALL_DIR,
    [string]$Repo = $(if ($env:VES_REPO) { $env:VES_REPO } else { 'yxxbc/VeriScout' }),
    [string]$BaseUrl = $env:VES_BASE_URL
)

$ErrorActionPreference = 'Stop'
$Target = 'x86_64-pc-windows-msvc'
$BinName = 'ves.exe'

if (-not $Version) { $Version = 'latest' }

# Prefer the `/releases/latest` redirect (no auth, no API quota); the API is the
# fallback because anonymous callers share one small quota per IP.
function Get-LatestTag {
    param([string]$Repo)
    $final = $null
    try {
        $r = Invoke-WebRequest -Uri "https://github.com/$Repo/releases/latest" -MaximumRedirection 5 -UseBasicParsing -ErrorAction Stop
        if ($r.BaseResponse.ResponseUri) { $final = $r.BaseResponse.ResponseUri.AbsoluteUri }
        elseif ($r.BaseResponse.RequestMessage -and $r.BaseResponse.RequestMessage.RequestUri) { $final = $r.BaseResponse.RequestMessage.RequestUri.AbsoluteUri }
    } catch { }
    if ($final -match '/releases/tag/([^/]+)/?$') { return $Matches[1] }
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
        -Headers @{ 'User-Agent' = 'ves-installer' }
    return $release.tag_name
}

if ($Version -eq 'latest') {
    Write-Host "→ 查询最新版本（${Repo}）"
    $Version = Get-LatestTag -Repo $Repo
    if (-not $Version) { throw 'could not resolve the latest release tag' }
}

if (-not $BaseUrl) { $BaseUrl = "https://github.com/$Repo/releases/download/$Version" }
if (-not $InstallDir) { $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\ves' }

$asset = "ves-$Version-$Target.zip"
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("ves-install-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work -Force | Out-Null

try {
    Write-Host "→ 下载 $asset"
    $zip = Join-Path $work $asset
    Invoke-WebRequest -Uri "$BaseUrl/$asset" -OutFile $zip -UseBasicParsing

    # Checksum: verify against SHA256SUMS.txt when the release ships one.
    $sums = Join-Path $work 'SHA256SUMS.txt'
    try {
        Invoke-WebRequest -Uri "$BaseUrl/SHA256SUMS.txt" -OutFile $sums -UseBasicParsing
        $line = (Get-Content $sums) | Where-Object { $_ -match [regex]::Escape($asset) } | Select-Object -First 1
        if ($line) {
            $expected = ($line -split '\s+')[0].ToUpperInvariant()
            $actual = (Get-FileHash -Path $zip -Algorithm SHA256).Hash
            if ($actual -ne $expected) {
                throw "checksum mismatch for ${asset}: expected $expected, got $actual"
            }
            Write-Host "✓ 校验通过 sha256 $($actual.Substring(0,12))…"
        } else {
            Write-Warning "$asset 不在 SHA256SUMS.txt 里，跳过校验"
        }
    } catch [System.Net.WebException] {
        Write-Warning '没有 SHA256SUMS.txt，跳过校验'
    }

    Expand-Archive -Path $zip -DestinationPath $work -Force
    $src = Join-Path $work "ves-$Version-$Target\$BinName"
    if (-not (Test-Path $src)) { throw "压缩包里没有找到 $BinName" }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $dest = Join-Path $InstallDir $BinName
    if (Test-Path $dest) { Write-Host "→ 覆盖已有安装（${dest}）" }
    Copy-Item -Path $src -Destination $dest -Force

    Write-Host ''
    Write-Host "✓ 已安装 $(& $dest --version) → $dest"

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $InstallDir) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$InstallDir", 'User')
        Write-Host ''
        Write-Host "! 已把 $InstallDir 加进用户 PATH —— 重开一个终端窗口后生效"
    }

    Write-Host ''
    Write-Host '下一步：'
    Write-Host '  ves config init          # 生成配置，填入 api_key（或设 DEEPSEEK_API_KEY）'
    Write-Host '  ves "一次性纸杯到底能不能回收"'
    Write-Host '  ves --html "一次性纸杯到底能不能回收"   # 想要一份离线 HTML 报告再加这个'
    Write-Host '  ves --help               # 全部命令'
    Write-Host ''
    Write-Host '默认只在终端把结论说清楚，不写任何文件；加了 --html 才会生成报告到 visualizations\。'
    Write-Host '（注意：scripts_tool 的 Python 沙箱需要 python 在 PATH 里；Windows 上沙箱限制是降级模式。）'
} finally {
    Remove-Item -Path $work -Recurse -Force -ErrorAction SilentlyContinue
}
