$state=@{}
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$script = Join-Path $root 'scripts/install.ps1'
$python = Get-Command python -ErrorAction SilentlyContinue
$suite = Join-Path (Join-Path $root 'test/install') ('.ps-suite-'+[guid]::NewGuid().ToString('N'))
$null = [IO.Directory]::CreateDirectory($suite)
foreach ($key in @('HOME','USERPROFILE','LOCALAPPDATA','APPDATA','XDG_DATA_HOME','XDG_CONFIG_HOME','XDG_CACHE_HOME','XDG_STATE_HOME','TMPDIR','TEMP','TMP','BUN_INSTALL','GH_CONFIG_DIR')) {
    $path=Join-Path $suite $key; $null=[IO.Directory]::CreateDirectory($path); [Environment]::SetEnvironmentVariable($key,$path,'Process')
}
$env:HTTPS_PROXY='http://127.0.0.1:9'; $env:NO_PROXY='127.0.0.1,localhost,::1'
$env:SHELL='powershell'; $env:GH_TOKEN=$null; $env:GITHUB_TOKEN=$null
$env:ENVSWITCH_INSTALL_TEST_MODE='install'; $env:ENVSWITCH_INSTALL_TEST_USER_PATH_FILE=Join-Path $env:USERPROFILE user-path.txt
$env:POWERSHELL_TELEMETRY_OPTOUT='1'; $env:DOTNET_CLI_TELEMETRY_OPTOUT='1'
$env:PATH = if ($env:SystemRoot) { $env:SystemRoot+'\System32;'+$env:SystemRoot+'\System32\WindowsPowerShell\v1.0' } else { '/usr/bin:/bin' }
try {
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($script, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
# Load declarations only: Linux must never bypass the Windows platform gate.
$ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $true) | ForEach-Object { . ([scriptblock]::Create($_.Extent.Text)) }
$state.Utf8 = New-Object Text.UTF8Encoding($false, $true)
$state.count = 0
function Assert-Equal($Actual, $Expected) {
    if ($Actual -cne $Expected) { throw "expected <$Expected>, got <$Actual>" }
    $state.count++
}
function Assert-Fails([scriptblock]$Body, $Text) {
    try { & $Body; throw 'expected failure' } catch { if (-not $_.Exception.Message.Contains($Text)) { throw }; $state.count++ }
}
$marker=$ast.FindAll({ param($node) $node -is [Management.Automation.Language.ExpandableStringExpressionAst] -and $node.Extent.Text.Contains('$state.Tag') -and $node.Extent.Text.Contains('`n') },$true)[0]
$state.Tag='v0.0.42'
Assert-Equal (& ([scriptblock]::Create($marker.Extent.Text))) "v0.0.42`n"
$unwrapped=$ast.FindAll({ param($node) $node -is [Management.Automation.Language.ExpandableStringExpressionAst] -and $node.Extent.Text.Contains('$state.') -and @($node.NestedExpressions | Where-Object { $_ -is [Management.Automation.Language.VariableExpressionAst] -and $_.VariablePath.UserPath -eq 'state' }).Count -gt 0 },$true)
Assert-Equal $unwrapped.Count 0
Assert-Equal (Convert-Tag '0.0.42' Version) 'v0.0.42'
Assert-Equal (Convert-Tag 'v0.0.0' CurrentVersion) 'v0.0.0'
foreach ($bad in @(' v0.0.1','0.0.01','v','1.0.0','0.0.1/evil',"0.0.1`n")) {
    Assert-Fails { Convert-Tag $bad Version } 'invalid Version:'
}
Assert-Fails { Read-Json '{"kind":1,"kind":2}' } 'JSON'
Assert-Equal (Read-Json '{"tag_name":"v0.0.42","assets":[]}').tag_name 'v0.0.42'
$manifest = ('a' * 64) + "  install.sh`n" + ('b' * 64) + "  install.ps1`n"
Assert-Equal (Read-Sums ([Text.Encoding]::UTF8.GetBytes($manifest)) 'v0.0.42' @('install.sh','install.ps1'))['install.sh'] ('a'*64)
foreach ($bad in @($manifest.TrimEnd("`n"), ($manifest + $manifest), $manifest.Replace('  ',' '), ([char]0xfeff + $manifest))) {
    Assert-Fails { Read-Sums ([Text.Encoding]::UTF8.GetBytes($bad)) 'v0.0.42' @('install.sh') } 'invalid SHA256SUMS'
}
$launcher = New-Launcher native ('C:\'+[char]0x96ea+' %!&\current\bin\envswitch.exe')
Assert-Equal ($launcher.Contains(('"C:\'+[char]0x96ea+' %%!&\current\bin\envswitch.exe" %*'))) $true
Assert-Equal ($launcher.Contains('setlocal DisableDelayedExpansion')) $true
Assert-Equal ($launcher.Contains("`r`n")) $true
Assert-Equal ($launcher.Contains('call ')) $false
Assert-Equal (($launcher -split "`r`n")[2].Contains('tokens=2 delims=:.')) $true
Assert-Equal $launcher.Contains('chcp %_ENVSWITCH_CP% >nul 2>&1') $true
Assert-Equal (('Aktive Codepage: 850.' -split '[:.]')[1].Trim()) '850'
Assert-Equal (Test-ManagedLauncher ($launcher.Replace('tokens=2 delims=:.','tokens=2 delims=:').Replace('chcp %_ENVSWITCH_CP% >nul 2>&1','chcp %_ENVSWITCH_CP% >nul')) native ('C:\'+[char]0x96ea+' %!&\current\bin\envswitch.exe')) $true
Assert-Equal (Get-Target AMD64) 'x86_64-pc-windows-msvc'
Assert-Equal (Get-Target X64) 'x86_64-pc-windows-msvc'
foreach ($cpu in @('Arm64','X86','Unknown')) { Assert-Fails { Get-Target $cpu } 'unsupported architecture' }
foreach ($name in @('../escape','/absolute','C:/drive','scripts\evil','scripts/./x','scripts//x','envswitch/src/NUL','envswitch/src/COM1.txt',"scripts/a`nb")) {
    Assert-Fails { Assert-Member $name $false native } 'archive'
}
Assert-Fails { Assert-Member 'envswitch/src/NUL' $false source } 'archive'
Assert-Equal (Assert-Member 'scripts/install.sh' $false native) 'scripts/install.sh'
Assert-Fails { Assert-Member '.env' $false native } 'archive'
# Archive tests exercise real .NET readers. Only directory ACL setup is platform specific.
if (-not (Test-Windows)) {
    function New-PrivateDirectory([string]$Path) { $null=[IO.Directory]::CreateDirectory($Path) }
}
$archiveFixtures=Read-Json ([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'install/archive-fixtures.json')))
foreach ($fixture in $archiveFixtures.PSObject.Properties) {
    $archivePath=Join-Path $suite ($fixture.Name+'.archive'); $out=Join-Path $suite ($fixture.Name+'-out')
    [IO.File]::WriteAllBytes($archivePath,[Convert]::FromBase64String($fixture.Value))
    $kind=if ($fixture.Name.StartsWith('native')) { 'native' } else { 'source' }
    if ($fixture.Name -in @('source-valid','source-unicode','native-valid')) {
        Expand-Safe $archivePath $out $kind
        Assert-Equal ([IO.Directory]::Exists($out)) $true
    } else { Assert-Fails { Expand-Safe $archivePath $out $kind } 'unsafe or invalid archive' }
}
if (-not (Test-Windows)) {
    $command="Get-Content -LiteralPath '"+$script.Replace("'","''")+"' -Raw | Invoke-Expression; [Console]::Out.WriteLine('session-alive:'+ `$global:LASTEXITCODE)"
    $engine=(Get-Process -Id $PID).Path
    $result=Invoke-Captured $engine ('-NoProfile -Command "'+$command.Replace('"','\"')+'"') 60000
    Assert-Equal ([Text.Encoding]::UTF8.GetString($result.Out).Contains('session-alive:1')) $true
}
if (-not (Test-Windows)) {
    $previousBase=$env:ENVSWITCH_INSTALL_TEST_API_BASE_URL
    try {
        $env:ENVSWITCH_INSTALL_TEST_API_BASE_URL='http://127.0.0.1:9'
        $arguments='-NoProfile -File "'+$script+'" -Repo demo/suite -Version 0.0.42 -WhatIf'
        $result=Invoke-Captured $engine $arguments 60000
        Assert-Equal $result.Code 1
        Assert-Equal ([Text.Encoding]::UTF8.GetString($result.Err)) "error: unsupported OS for this installer. Use scripts/install.sh on macOS, Linux, or WSL.`n"
    } finally { $env:ENVSWITCH_INSTALL_TEST_API_BASE_URL=$previousBase }
}
$engine=(Get-Process -Id $PID).Path
Push-Location -LiteralPath $suite
try {
    $result=Invoke-Captured $engine '-NoProfile -Command "[Console]::Out.Write([Environment]::CurrentDirectory)"' 60000
    Assert-Equal ([Text.Encoding]::UTF8.GetString($result.Out)) $suite
} finally { Pop-Location }
# Mock the registry boundary itself to verify value type without opening real User PATH.
$openEnvironment=${function:Open-UserEnvironment}; $notifyEnvironment=${function:Send-EnvironmentChange}
$state.RegistryMock=[pscustomobject]@{Value='%USERPROFILE%\tools';Kind=[Microsoft.Win32.RegistryValueKind]::ExpandString;Options=$null;Writes=0}
$state.RegistryMock | Add-Member ScriptMethod GetValueNames { return @('Path') }
$state.RegistryMock | Add-Member ScriptMethod GetValueKind { param($name) return $this.Kind }
$state.RegistryMock | Add-Member ScriptMethod GetValue { param($name,$fallback,$options) $this.Options=$options; return $this.Value }
$state.RegistryMock | Add-Member ScriptMethod SetValue { param($name,$value,$kind) $this.Value=$value; $this.Kind=$kind; $this.Writes++ }
$state.RegistryMock | Add-Member ScriptMethod Dispose { }
function Open-UserEnvironment { param([switch]$Writable) return $state.RegistryMock }
function Send-EnvironmentChange { }
try {
    $state.TestMode=$false
    Assert-Equal (Get-UserPath) '%USERPROFILE%\tools'
    Assert-Equal $state.RegistryMock.Options ([Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    Set-UserPath '%USERPROFILE%\tools;C:\envswitch'
    Assert-Equal $state.RegistryMock.Kind ([Microsoft.Win32.RegistryValueKind]::ExpandString)
    Assert-Equal $state.RegistryMock.Writes 1
    function Send-EnvironmentChange { throw 'notification unavailable' }
    Set-UserPath '%USERPROFILE%\tools;C:\envswitch-again'
    Assert-Equal $state.RegistryMock.Writes 2
    Assert-Equal $state.RegistryMock.Value '%USERPROFILE%\tools;C:\envswitch-again'

} finally {
    Set-Item Function:Open-UserEnvironment $openEnvironment
    Set-Item Function:Send-EnvironmentChange $notifyEnvironment
    $state.TestMode=$true
}
Assert-Equal (Get-RetryDelay 2 '999999999999999999999') 60
Assert-Equal (Get-RetryDelay 2 '61') 60
Assert-Equal (Get-RetryDelay 2 '0') 2
Assert-Equal (Get-RetryDelay 1 ([DateTimeOffset]::UtcNow.AddHours(1).ToString('r'))) 60
# Raw iex must neither overwrite caller variables/preferences nor publish installer helpers.
$command="`$Repo='caller-repo'; `$Version='caller-version'; `$ErrorActionPreference='Continue'; Get-Content -LiteralPath '"+$script.Replace("'","''")+"' -Raw | iex; if (`$Repo -ne 'caller-repo' -or `$Version -ne 'caller-version' -or `$ErrorActionPreference -ne 'Continue' -or (Get-Command Fail -ErrorAction SilentlyContinue)) { throw 'session polluted' }; [Console]::Out.Write('session-clean')"
$result=Invoke-Captured $engine ('-NoProfile -Command "'+$command.Replace('"','\"')+'"') 60000
Assert-Equal $result.Code 0
Assert-Equal ([Text.Encoding]::UTF8.GetString($result.Out)) 'session-clean'
Write-Host "PowerShell helper assertions: $($state.count) passed"
if ((Test-Windows)) {
    $env:ENVSWITCH_TEST_POWERSHELL = (Get-Process -Id $PID).Path
    & $python.Source (Join-Path $PSScriptRoot 'install/test_windows.py')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

} finally { Remove-Item -LiteralPath $suite -Recurse -Force }
