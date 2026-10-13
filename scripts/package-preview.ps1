param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
function Assert-Amd64([string]$FilePath, [bool]$ExpectedDll) {
    if (!(Test-Path -LiteralPath $FilePath -PathType Leaf)) { throw "缺少发布产物：$FilePath" }
    if ((Get-Item -LiteralPath $FilePath).Length -gt 64MB) { throw "发布产物过大：$FilePath" }
    $peBytes = [IO.File]::ReadAllBytes($FilePath)
    if ($peBytes.Length -lt 64 -or $peBytes[0] -ne 77 -or $peBytes[1] -ne 90) { throw "无效 PE：$FilePath" }
    $peOffset = [BitConverter]::ToUInt32($peBytes, 60)
    if ($peOffset -gt ($peBytes.Length - 26) -or [BitConverter]::ToUInt32($peBytes, $peOffset) -ne 17744 -or [BitConverter]::ToUInt16($peBytes, $peOffset + 4) -ne 34404 -or [BitConverter]::ToUInt16($peBytes, $peOffset + 24) -ne 523) { throw "产物不是 AMD64 PE32+：$FilePath" }
    $isDll = ([BitConverter]::ToUInt16($peBytes, $peOffset + 22) -band 8192) -ne 0
    if ($isDll -ne $ExpectedDll) { throw "EXE / DLL 类型不匹配：$FilePath" }
}
$appBinary = Join-Path $projectRoot 'target/release/gmod-dglab-desktop.exe'
$collectorBinary = Join-Path $projectRoot 'target/injector-collector/release/gmod_dglab_collector.dll'
if (!$SkipBuild) {
    $cargoTool = (Get-Command cargo.exe -ErrorAction Stop).Source
    $npmTool = (Get-Command npm.cmd -ErrorAction Stop).Source
    Push-Location (Join-Path $projectRoot 'desktop')
    try { & $npmTool run tauri -- build --no-bundle; if ($LASTEXITCODE -ne 0) { throw '桌面构建失败' } } finally { Pop-Location }
    Push-Location $projectRoot
    try { & $cargoTool build --release -p gmod-dglab-collector --target-dir (Join-Path $projectRoot 'target/injector-collector'); if ($LASTEXITCODE -ne 0) { throw '采样 DLL 构建失败' } } finally { Pop-Location }
}
Assert-Amd64 $appBinary $false
Assert-Amd64 $collectorBinary $true
$releaseRoot = Join-Path $projectRoot 'target/releases'
$packageName = 'gmod-dglab-0.1.0-windows-x64-preview-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')
$packageDir = Join-Path $releaseRoot $packageName
if (Test-Path -LiteralPath $packageDir) { throw '发布目录已经存在，请稍后重试；不会覆盖旧包' }
New-Item -ItemType Directory -Path (Join-Path $packageDir 'bin') -Force | Out-Null
Copy-Item -LiteralPath $appBinary -Destination (Join-Path $packageDir 'gmod-dglab.exe')
Copy-Item -LiteralPath $collectorBinary -Destination (Join-Path $packageDir 'bin/gmod_dglab_collector.dll')
foreach ($licenseFile in @('README.md','LICENSE','LICENSE-MIT','THIRD_PARTY_NOTICES.md')) { Copy-Item -LiteralPath (Join-Path $projectRoot $licenseFile) -Destination (Join-Path $packageDir $licenseFile) }
$hashLines = foreach ($fileName in @('gmod-dglab.exe','bin/gmod_dglab_collector.dll')) { (Get-FileHash -LiteralPath (Join-Path $packageDir $fileName) -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + $fileName }
[IO.File]::WriteAllLines((Join-Path $packageDir 'SHA256SUMS.txt'), $hashLines, [Text.UTF8Encoding]::new($false))
$binaryZip = Join-Path $releaseRoot ($packageName + '.zip')
Compress-Archive -LiteralPath $packageDir -DestinationPath $binaryZip
Add-Type -AssemblyName System.IO.Compression.FileSystem
$sourceZip = Join-Path $releaseRoot ($packageName + '-source.zip')
$archive = [IO.Compression.ZipFile]::Open($sourceZip, [IO.Compression.ZipArchiveMode]::Create)
$excluded = @('docs','.git','target','node_modules','dist','.codex','.agents','.aws')
$sourceExtensions = @('.rs','.toml','.json','.ts','.vue','.css','.html','.ico','.png','.svg','.ps1','.md')
function Add-SourceFile([string]$FilePath) {
    $sourceFullPath = [IO.Path]::GetFullPath($FilePath)
    $sourcePrefix = $projectRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if (!$sourceFullPath.StartsWith($sourcePrefix, [StringComparison]::OrdinalIgnoreCase)) { throw '源码路径越出项目目录' }
    $relative = $sourceFullPath.Substring($sourcePrefix.Length).Replace('\','/')
    foreach ($part in $relative.Split('/')) { if ($excluded -contains $part) { throw "源码包包含私有或生成目录：$relative" } }
    [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $FilePath, $relative, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
}
function Add-SourceTree([string]$DirectoryPath) {
    foreach ($sourceItem in Get-ChildItem -LiteralPath $DirectoryPath) {
        if (($sourceItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { continue }
        if ($sourceItem.PSIsContainer) { if ($excluded -notcontains $sourceItem.Name) { Add-SourceTree $sourceItem.FullName } }
        elseif ($sourceExtensions -contains $sourceItem.Extension.ToLowerInvariant()) { Add-SourceFile $sourceItem.FullName }
    }
}
try {
    foreach ($rootFile in @('Cargo.toml','Cargo.lock','.gitignore','README.md','LICENSE','LICENSE-MIT','THIRD_PARTY_NOTICES.md')) { Add-SourceFile (Join-Path $projectRoot $rootFile) }
    foreach ($sourceDirectory in @('crates','desktop','scripts')) { Add-SourceTree (Join-Path $projectRoot $sourceDirectory) }
} finally { $archive.Dispose() }
$zipHashes = foreach ($archivePath in @($binaryZip, $sourceZip)) { (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + [IO.Path]::GetFileName($archivePath) }
[IO.File]::WriteAllLines((Join-Path $releaseRoot ($packageName + '-SHA256SUMS.txt')), $zipHashes, [Text.UTF8Encoding]::new($false))
[pscustomobject]@{Directory=$packageDir;ApplicationZip=$binaryZip;SourceZip=$sourceZip}
