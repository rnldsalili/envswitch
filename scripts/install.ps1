[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$Repo = 'rnldsalili/envswitch',
    [string]$InstallDir,
    [string]$AppRoot,
    [string]$Version,
    [string]$CurrentVersion,
    [switch]$VerifyAttestations
)
$ErrorActionPreference = 'Stop'
$script:Utf8 = New-Object Text.UTF8Encoding($false, $true)
$script:Targets = @('aarch64-apple-darwin','x86_64-apple-darwin','x86_64-unknown-linux-musl','aarch64-unknown-linux-musl','x86_64-pc-windows-msvc')
$script:Journal = New-Object Collections.ArrayList
$script:Created = New-Object Collections.ArrayList
$script:Stage = $null; $script:Locked = $false; $script:Committed = $false

function Fail([string]$Message, [string]$Second) {
    $text = "error: $Message"
    if ($Second) { $text += "`nerror: $Second" }
    throw [InvalidOperationException]::new($text)
}
function Info([string]$Message) { [Console]::Out.WriteLine("info: $Message") }
function Convert-Tag([string]$Value, [string]$Name) {
    if (-not $Value) { return $null }
    if ($Value -cnotmatch '\Av?0\.0\.(0|[1-9][0-9]*)\z') { Fail "invalid ${Name}: $Value; expected 0.0.N or v0.0.N" }
    return 'v' + $Value.TrimStart('v')
}
function Test-Repo([string]$Value) {
    return $Value -cmatch '\A[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+\z' -and @($Value.Split('/') | Where-Object { $_ -in @('.','..') }).Count -eq 0
}
function Unsafe([string]$Path) { Fail "unsafe installation path: $Path" 'choose absolute, non-overlapping envswitch paths without symlinks or reparse points' }
function Assert-NoLinks([string]$Path) {
    $cursor = $Path
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            if (([IO.File]::GetAttributes($cursor) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { Unsafe $cursor }
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }; $cursor = $parent
    }
}
function Normalize-Directory([string]$Path) {
    if ($Path -cnotmatch '\A[A-Za-z]:[\\/]' -or $Path -match '["\x00-\x1f\x7f-\x9f]' -or $Path.Substring(2).Contains(':')) { Unsafe $Path }
    foreach ($component in ($Path.Substring(3) -split '[\\/]')) {
        if ($component -in @('.','..') -or $component -match '[. ]$|^(CON|PRN|AUX|NUL|COM[1-9\u00b9\u00b2\u00b3]|LPT[1-9\u00b9\u00b2\u00b3])(?:\.|$)') { Unsafe $Path }
    }
    $result = [IO.Path]::GetFullPath($Path).TrimEnd('\','/')
    if ($result.Length -le 2) { Unsafe $Path }
    Assert-NoLinks $result
    if ((Test-Path -LiteralPath $result) -and -not [IO.Directory]::Exists($result)) { Unsafe $result }
    return $result
}
function Test-Inside([string]$Path,[string]$Parent) {
    return $Path.Equals($Parent,[StringComparison]::OrdinalIgnoreCase) -or $Path.StartsWith($Parent.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)
}
function Get-Target([string]$Architecture = ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString())) {
    if ($Architecture -cne 'X64') { Fail "unsupported architecture for this installer: $Architecture" }
    return 'x86_64-pc-windows-msvc'
}
function Read-Json([string]$Text) {
    # ConvertFrom-Json accepts duplicate keys; reject them before structural conversion.
    $stack = New-Object Collections.Stack
    $matches = [regex]::Matches($Text, '"(?:[^"\\\x00-\x1f]|\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4}))*"|[{}\[\]:,]|true|false|null|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?')
    $end = 0
    for ($i=0; $i -lt $matches.Count; $i++) {
        $m=$matches[$i]
        if ($Text.Substring($end,$m.Index-$end).Trim().Length) { throw 'invalid JSON' }
        $end=$m.Index+$m.Length; $v=$m.Value
        if ($v -eq '{') { $stack.Push((New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase))) }
        elseif ($v -eq '[') { $stack.Push($null) }
        elseif ($v -in @('}',']')) { if (-not $stack.Count) { throw 'invalid JSON' }; $null=$stack.Pop() }
        elseif ($v.StartsWith('"') -and $i+1 -lt $matches.Count -and $matches[$i+1].Value -eq ':') {
            $key = ConvertFrom-Json ('['+$v+']')
            if (-not $stack.Count -or $null -eq $stack.Peek() -or -not $stack.Peek().Add([string]$key)) { throw 'duplicate JSON key' }
        }
    }
    if ($stack.Count -or $Text.Substring($end).Trim().Length) { throw 'invalid JSON' }
    try { return ConvertFrom-Json -InputObject $Text -ErrorAction Stop } catch { throw 'invalid JSON' }
}
function Read-Sums([byte[]]$Bytes,[string]$Tag,[string[]]$Required) {
    try {
        $text=$script:Utf8.GetString($Bytes)
        if (-not $text.EndsWith("`n")) { throw 'format' }
        $sums=New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::Ordinal)
        foreach ($line in $text.Substring(0,$text.Length-1).Split("`n")) {
            if ($line -cnotmatch '\A([0-9a-f]{64})  ([^/\\\x00-\x20\x7f-\x9f]+)\z') { throw 'format' }
            $name=$Matches[2]; $hash=$Matches[1]
            if ($sums.ContainsKey($name) -or $name -ceq 'SHA256SUMS') { throw 'format' }; $sums.Add($name,$hash)
        }
        foreach ($name in $Required) { if (-not $sums.ContainsKey($name)) { throw 'missing' } }
        return ,$sums
    } catch { Fail "invalid SHA256SUMS for release $Tag" }
}
function New-Launcher([string]$Kind,[string]$Payload) {
    $path=$Payload.Replace('%','%%')
    $lines=@('@echo off','setlocal DisableDelayedExpansion','for /f "tokens=2 delims=:" %%C in (''chcp'') do set "_ENVSWITCH_CP=%%C"','chcp 65001 >nul')
    if ($Kind -eq 'native') {
        $lines += @(('"'+$path+'" %*'),'set "_ENVSWITCH_EXIT=%ERRORLEVEL%"')
    } else {
        $lines += @('where bun >NUL 2>NUL','if %ERRORLEVEL% EQU 0 goto envswitch_bun_path','if exist "%USERPROFILE%\.bun\bin\bun.exe" goto envswitch_bun_home','echo envswitch: bun is not installed or not on PATH 1>&2','set "_ENVSWITCH_EXIT=127"','goto envswitch_finish',':envswitch_bun_path',('bun "'+$path+'" %*'),'set "_ENVSWITCH_EXIT=%ERRORLEVEL%"','goto envswitch_finish',':envswitch_bun_home',('"%USERPROFILE%\.bun\bin\bun.exe" "'+$path+'" %*'),'set "_ENVSWITCH_EXIT=%ERRORLEVEL%"',':envswitch_finish')
    }
    $lines += @('chcp %_ENVSWITCH_CP% >nul','exit /b %_ENVSWITCH_EXIT%')
    return ($lines -join "`r`n")+"`r`n"
}
function Get-Payload([string]$Root,[string]$Kind,[string]$Tag,[AllowNull()][string]$Target) {
    if ($Kind -eq 'source') { return Join-Path $Root 'current\src\cli.ts' }
    return Join-Path $Root ('current\bin\envswitch_'+$Tag.Substring(1)+'_'+$Target+'.exe')
}
function Read-Receipt([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    try {
        $r=Read-Json ([IO.File]::ReadAllText($Path,$script:Utf8))
        foreach ($key in @('schemaVersion','kind','repo','appRoot','installDir','launcherPath','target','tag','payloadPath')) {
            if ($r.PSObject.Properties.Name -cnotcontains $key) { throw 'missing' }
        }
        if (($r.schemaVersion -isnot [int] -and $r.schemaVersion -isnot [long]) -or $r.schemaVersion.ToString() -cne '1' -or $r.kind -cnotin @('source','native') -or -not (Test-Repo $r.repo) -or $r.tag -cnotmatch '\Av0\.0\.(0|[1-9][0-9]*)\z') { throw 'shape' }
        $rr=Normalize-Directory $r.appRoot; $dd=Normalize-Directory $r.installDir
        if ($r.appRoot -cne $rr -or $r.installDir -cne $dd -or -not $rr.Equals((Split-Path (Split-Path $Path -Parent) -Parent),[StringComparison]::OrdinalIgnoreCase)) { throw 'root' }
        if ($rr -eq $dd -or (Test-Inside $dd (Join-Path $rr current)) -or $rr -eq $env:USERPROFILE -or $rr -eq $env:HOME -or $dd -match '[\\/]\.install-') { throw 'paths' }
        if ($r.kind -eq 'source') { if ($null -ne $r.target) { throw 'target' } }
        elseif ($r.target -cne 'x86_64-pc-windows-msvc') { throw 'target' }
        if (-not [string]::Equals($r.launcherPath,(Join-Path $dd envswitch.cmd),[StringComparison]::OrdinalIgnoreCase) -or -not [string]::Equals($r.payloadPath,(Get-Payload $rr $r.kind $r.tag $r.target),[StringComparison]::OrdinalIgnoreCase)) { throw 'payload' }
        Assert-NoLinks $r.launcherPath; Assert-NoLinks $r.payloadPath
        return $r
    } catch { Fail "invalid envswitch install receipt: $Path" 'reinstall envswitch with the install script from the README after checking the install paths' }
}
function Unmanaged([string]$Path) { Fail "refusing to replace an unmanaged path: $Path" 'preserve user files and reinstall using the README instructions with explicit install paths' }
function Assert-Member([string]$Name,[bool]$Directory,[string]$Kind) {
    if ($Name.StartsWith('./')) { $Name=$Name.Substring(2) }
    if ($Directory) { $Name=$Name.TrimEnd('/') }
    if (-not $Name -or $Name -match '[\\:\x00-\x1f\x7f-\x9f]' -or @($Name.Split('/') | Where-Object { $_ -in @('','.','..') -or $_ -match '[. ]$|^(CON|PRN|AUX|NUL|COM[1-9\u00b9\u00b2\u00b3]|LPT[1-9\u00b9\u00b2\u00b3])(?:\.|$)' }).Count) { throw 'unsafe archive member' }
    if ($Kind -eq 'native') {
        if ($Name -notin @('envswitch.exe','scripts','scripts/install.sh','scripts/install.ps1','README.md') -or $Directory -ne ($Name -eq 'scripts')) { throw 'unsafe archive member' }
    } else {
        $parts=$Name.Split('/')
        if ($parts[0] -ne 'envswitch' -or ($parts.Count -gt 1 -and $parts[1] -notin @('src','scripts','package.json','bun.lock','README.md','tsconfig.json'))) { throw 'unsafe archive member' }
        if ($parts.Count -gt 2 -and $parts[1] -notin @('src','scripts')) { throw 'unsafe archive member' }
        if ($Directory -and $parts.Count -gt 1 -and $parts[1] -notin @('src','scripts')) { throw 'unsafe archive member' }
        if (@($parts | Where-Object { $_ -in @('.git','node_modules','.env') -or $_.StartsWith('.env.',[StringComparison]::OrdinalIgnoreCase) }).Count) { throw 'unsafe archive member' }
    }
    return $Name
}
function Expand-Safe([string]$Archive,[string]$Out,[string]$Kind) {
    try {
        $entries=New-Object Collections.ArrayList
        $seen=New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
        if ($Kind -eq 'native') {
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $zip=[IO.Compression.ZipFile]::OpenRead($Archive)
            try {
                foreach ($e in $zip.Entries) {
                    $dir=$e.FullName.EndsWith('/'); $name=Assert-Member $e.FullName $dir $Kind
                    $unixType=($e.ExternalAttributes -shr 16) -band 61440
                    if ($unixType -notin @(0,32768,16384) -or ($unixType -eq 16384 -and -not $dir) -or ($unixType -eq 32768 -and $dir) -or (($e.ExternalAttributes -band 16) -and -not $dir) -or ($e.ExternalAttributes -band 1024) -or -not $seen.Add($name)) { throw 'type/collision' }
                    $memory=New-Object IO.MemoryStream
                    $stream=$e.Open(); try { $stream.CopyTo($memory) } finally { $stream.Dispose() }
                    $null=$entries.Add(@($name,$dir,$memory.ToArray())); $memory.Dispose()
                }
            } finally { $zip.Dispose() }
        } else {
            # Parse tar headers (including PAX paths) before extraction; never delegate unsafe extraction.
            $tarInput=[IO.File]::OpenRead($Archive); $gzip=New-Object IO.Compression.GzipStream($tarInput,[IO.Compression.CompressionMode]::Decompress)
            $memory=New-Object IO.MemoryStream
            try { $gzip.CopyTo($memory); $bytes=$memory.ToArray() } finally { $gzip.Dispose(); $tarInput.Dispose(); $memory.Dispose() }
            $offset=0; $pax=@{}; $globalPax=@{}
            while ($offset+512 -le $bytes.Length) {
                $h=New-Object byte[] 512; [Array]::Copy($bytes,$offset,$h,0,512); $offset+=512
                if (@($h | Where-Object { $_ -ne 0 }).Count -eq 0) { break }
                $name=$script:Utf8.GetString($h,0,100).Split([char]0)[0]
                $prefix=$script:Utf8.GetString($h,345,155).Split([char]0)[0]
                if ($prefix) { $name=$prefix+'/'+$name }
                $sum=0; for ($j=0;$j -lt 512;$j++) { if ($j -ge 148 -and $j -lt 156) { $sum+=32 } else { $sum+=$h[$j] } }
                if ($sum -ne [Convert]::ToInt64(([Text.Encoding]::ASCII.GetString($h,148,8)).Trim([char]0,' '),8)) { throw 'checksum' }
                $size=[Convert]::ToInt64(([Text.Encoding]::ASCII.GetString($h,124,12)).Trim([char]0,' '),8)
                if ($size -lt 0 -or $size -gt [int]::MaxValue -or $offset+$size -gt $bytes.Length) { throw 'size' }
                $body=New-Object byte[] ([int]$size); [Array]::Copy($bytes,$offset,$body,0,[int]$size); $offset += [int64]([Math]::Ceiling($size/512.0)*512)
                $type=[char]$h[156]
                if ($type -in @('x','g')) {
                    $pos=0
                    while ($pos -lt $body.Length) {
                        $space=$pos; while ($space -lt $body.Length -and $body[$space] -ne 32) { $space++ }
                        $lengthText=[Text.Encoding]::ASCII.GetString($body,$pos,$space-$pos)
                        if ($lengthText -cnotmatch '\A[1-9][0-9]*\z') { throw 'pax length' }
                        $length=[int]$lengthText
                        if ($length -le $space-$pos+2 -or $pos+$length -gt $body.Length -or $body[$pos+$length-1] -ne 10) { throw 'pax length' }
                        $record=$script:Utf8.GetString($body,$space+1,$length-($space-$pos)-2); $equal=$record.IndexOf('=')
                        if ($equal -lt 1) { throw 'pax' }; $key=$record.Substring(0,$equal)
                        if ($key -eq 'linkpath' -or $key.StartsWith('GNU.sparse')) { throw 'pax link/sparse' }
                        if ($type -eq 'g') { $globalPax[$key]=$record.Substring($equal+1) } else { $pax[$key]=$record.Substring($equal+1) }
                        $pos+=$length
                    }
                    continue
                }
                if ($type -eq 'L') { $pax.path=$script:Utf8.GetString($body).TrimEnd([char]0); continue }
                if ($type -notin @([char]0,'0','5')) { throw 'type' }
                if ($pax.ContainsKey('path')) { $name=$pax.path } elseif ($globalPax.ContainsKey('path')) { $name=$globalPax.path }; $pax=@{}
                $dir=$type -eq '5'; $name=Assert-Member $name $dir $Kind
                if (-not $seen.Add($name)) { throw 'collision' }; $null=$entries.Add(@($name,$dir,$body))
            }
        }
        $required=if ($Kind -eq 'native') { @('envswitch.exe','README.md','scripts/install.sh','scripts/install.ps1') } else { @('envswitch/package.json','envswitch/bun.lock','envswitch/src/cli.ts','envswitch/scripts/install.sh','envswitch/scripts/install.ps1') }
        foreach ($n in $required) { if (-not @($entries | Where-Object { $_[0] -eq $n -and -not $_[1] }).Count) { throw 'missing' } }
        New-PrivateDirectory $Out
        foreach ($e in $entries) {
            $p=Join-Path $Out $e[0]; Assert-NoLinks $p
            if ($e[1]) { $null=[IO.Directory]::CreateDirectory($p) }
            else { $null=[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($p)); Write-Utf8Bytes $p $e[2] }
        }
        foreach ($f in Get-ChildItem -LiteralPath $Out -Recurse -Force) { Assert-NoLinks $f.FullName }
    } catch { Fail ('unsafe or invalid archive: '+[IO.Path]::GetFileName($Archive)) }
}
function Write-Utf8Bytes([string]$Path,[byte[]]$Bytes) {
    $stream=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try { $stream.Write($Bytes,0,$Bytes.Length) } finally { $stream.Dispose() }
}
function New-PrivateDirectory([string]$Path) {
    if (Test-Path -LiteralPath $Path) { throw [IO.IOException]::new('staging directory already exists') }
    $null=[IO.Directory]::CreateDirectory($Path)
    $acl=New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true,$false)
    foreach ($sid in @([Security.Principal.WindowsIdentity]::GetCurrent().User.Value,'S-1-5-18','S-1-5-32-544')) {
        $identity=New-Object Security.Principal.SecurityIdentifier($sid)
        $rule=New-Object Security.AccessControl.FileSystemAccessRule($identity,'FullControl','ContainerInherit,ObjectInherit','None','Allow')
        $acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $Path -AclObject $acl
}
function Ensure-Directory([string]$Path) {
    Assert-NoLinks $Path
    if ([IO.Directory]::Exists($Path)) { return }
    if (Test-Path -LiteralPath $Path) { Unsafe $Path }
    Ensure-Directory ([IO.Path]::GetDirectoryName($Path))
    $null=[IO.Directory]::CreateDirectory($Path); $null=$script:Created.Add($Path)
}
function Replace-Managed([string]$Path,[byte[]]$Bytes,[string]$Source) {
    Assert-NoLinks $Path; Ensure-Directory ([IO.Path]::GetDirectoryName($Path))
    $backup=Join-Path $script:Stage ('backup-'+$script:Journal.Count)
    $exists=Test-Path -LiteralPath $Path
    if ($exists) { Copy-Item -LiteralPath $Path -Destination $backup -Recurse -Force }
    $null=$script:Journal.Add(@($Path,$(if ($exists) { $backup } else { $null })))
    $temp=Join-Path ([IO.Path]::GetDirectoryName($Path)) ('.install-'+$script:Token+'-'+$script:Journal.Count)
    try {
        if (Test-Path -LiteralPath $temp) { throw [IO.IOException]::new('temporary path already exists') }
        if ($Source) { Copy-Item -LiteralPath $Source -Destination $temp -Recurse } else { Write-Utf8Bytes $temp $Bytes }
        Assert-NoLinks $Path
        if ([IO.Directory]::Exists($Path)) { [IO.Directory]::Move($Path,(Join-Path $script:Stage ('moved-'+$script:Journal.Count))); [IO.Directory]::Move($temp,$Path) }
        elseif ($exists) {
            try { [IO.File]::Replace($temp,$Path,$null) }
            catch { $script:Journal.RemoveAt($script:Journal.Count-1); if ($Path -eq $script:Launcher) { Fail "could not replace envswitch launcher: $Path" 'close processes using the launcher and retry; the previous installation was preserved' }; throw }
        } else { [IO.File]::Move($temp,$Path) }
    } finally { if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force } }
}
function Invoke-Captured([string]$File,[string]$Argument,[int]$Timeout = 0) {
    $psi=New-Object Diagnostics.ProcessStartInfo
    if ($File.EndsWith('.cmd',[StringComparison]::OrdinalIgnoreCase)) {
        $psi.FileName=$env:ComSpec; $psi.EnvironmentVariables['_ENVSWITCH_LAUNCHER']=$File; $psi.Arguments='/d /v:off /s /c ""%_ENVSWITCH_LAUNCHER%" '+$Argument+'"'
    } else { $psi.FileName=$File; $psi.Arguments=$Argument }
    $psi.UseShellExecute=$false; $psi.RedirectStandardOutput=$true; $psi.RedirectStandardError=$true
    $process=New-Object Diagnostics.Process; $process.StartInfo=$psi; $null=$process.Start()
    $out=New-Object IO.MemoryStream; $err=New-Object IO.MemoryStream
    $a=$process.StandardOutput.BaseStream.CopyToAsync($out); $b=$process.StandardError.BaseStream.CopyToAsync($err)
    if ($Timeout -gt 0 -and -not $process.WaitForExit($Timeout)) { $process.Kill(); $process.WaitForExit() } else { $process.WaitForExit() }; $a.GetAwaiter().GetResult(); $b.GetAwaiter().GetResult()
    $result=@{Code=$process.ExitCode; Out=$out.ToArray(); Err=$err.ToArray()}; $process.Dispose(); $out.Dispose(); $err.Dispose(); return $result
}
function Test-Payload([string]$File,[string]$Kind,[string]$Ver,[string]$Target,[switch]$Staged) {
    try {
        if ($Kind -eq 'native') {
            $r=Invoke-Captured $File '--version'
            if ($r.Code -ne 0 -or $r.Err.Length -ne 0 -or [Convert]::ToBase64String($r.Out) -cne [Convert]::ToBase64String($script:Utf8.GetBytes("envswitch $Ver $Target`n"))) {
                if ($Staged) { Fail "native payload verification failed; expected envswitch $Ver $Target" }; return $false
            }
        }
        return (Invoke-Captured $File '--help').Code -eq 0
    } catch { if ($Staged) { Fail "native payload verification failed; expected envswitch $Ver $Target" }; return $false }
}
function Invoke-GhCommand([string]$File,[string[]]$Arguments) {
    $quoted=($Arguments | ForEach-Object { '"'+$_.Replace('"','\"')+'"' }) -join ' '
    return Invoke-Captured $File $quoted 60000
}
function Get-Gh([switch]$Verifier) {
    $command=Get-Command gh -CommandType Application -ErrorAction SilentlyContinue
    if (-not $command) { return $null }
    if ($script:TestMode -and -not (Test-Inside $command.Source $env:USERPROFILE)) { return $null }
    if ($Verifier) { return $command.Source }
    $probe=Invoke-GhCommand $command.Source @('auth','status')
    if ($probe.Code -ne 0) { return $null }; return $command.Source
}
function Get-Http([string]$Url,[string]$Accept) {
    # Manual redirects prevent forwarding bearer headers across origins (including on PS 5.1).
    $deadline=[DateTime]::UtcNow.AddSeconds(60)
    for ($attempt=0; $attempt -lt 3; $attempt++) {
        $next=$Url
        try {
            for ($redirect=0; $redirect -lt 10; $redirect++) {
                $request=[Net.HttpWebRequest]::Create($next); $request.AllowAutoRedirect=$false
                $request.Timeout=[Math]::Max(1,[int]($deadline-[DateTime]::UtcNow).TotalMilliseconds); $request.ReadWriteTimeout=$request.Timeout
                $request.Accept=$Accept; $request.UserAgent='envswitch-installer'
                if ($env:GITHUB_TOKEN -and ([uri]$next).GetLeftPart([UriPartial]::Authority) -ceq $script:ApiBase) { $request.Headers['Authorization']='Bearer '+$env:GITHUB_TOKEN }
                $response=$request.GetResponse()
                try {
                    if ([int]$response.StatusCode -in @(301,302,303,307,308)) {
                        $next=([uri]::new([uri]$next,$response.Headers['Location'])).AbsoluteUri
                        if (-not ((([uri]$next).Scheme -eq 'https' -and ([uri]$next).Host -in @('api.github.com','release-assets.githubusercontent.com','objects.githubusercontent.com','github-releases.githubusercontent.com') -and ([uri]$next).Port -eq 443) -or ($script:TestMode -and ([uri]$next).IsLoopback -and ([uri]$next).Scheme -eq 'http')) -or ([uri]$next).UserInfo) { throw 'unsafe redirect' }
                        continue
                    }
                    $memory=New-Object IO.MemoryStream; $stream=$response.GetResponseStream()
                    try { $copy=$stream.CopyToAsync($memory); if (-not $copy.Wait([Math]::Max(1,[int]($deadline-[DateTime]::UtcNow).TotalMilliseconds))) { throw [TimeoutException]::new() }; $copy.GetAwaiter().GetResult(); return ,$memory.ToArray() } finally { $memory.Dispose(); $stream.Dispose() }
                } finally { $response.Dispose() }
            }
            throw 'redirect limit'
        } catch {
            $resp=$_.Exception.Response
            $code=if ($resp) { [int]$resp.StatusCode } else { 0 }
            if ($attempt -eq 2 -or ([DateTime]::UtcNow -ge $deadline) -or ($code -ne 0 -and $code -notin @(408,429) -and $code -lt 500)) { throw }
        }
    }
}
function Save-Asset([string]$Name) {
    $path=Join-Path $script:Stage $Name
    if ($script:Gh) {
        Info "downloading $Name with GitHub CLI"
        $download=Invoke-GhCommand $script:Gh @('release','download',$script:Tag,'--repo',$Repo,'--pattern',$Name,'--dir',$script:Stage)
        if ($download.Code -eq 0 -and [IO.File]::Exists($path)) { return $path }
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
        Info "GitHub CLI download did not find $Name; trying direct download"
    } else { Info "downloading $Name" }
    try { Write-Utf8Bytes $path (Get-Http $script:Assets[$Name] 'application/octet-stream') }
    catch { Fail "release asset not found or unavailable: $Name" }
    return $path
}
function Verify-Hash([string]$Path,[string]$Name) {
    Info "verifying SHA-256 for $Name"
    if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $script:Sums[$Name]) { Fail "SHA-256 mismatch for $Name" }
}
function Boundary([string]$Name) { if ($script:TestMode -and $env:ENVSWITCH_INSTALL_TEST_FAIL_AT -ceq $Name) { Fail "injected installer test failure: $Name" } }
function Assert-Source([string]$Path,[switch]$Existing) {
    foreach ($f in Get-ChildItem -LiteralPath $Path -Recurse -Force) {
        Assert-NoLinks $f.FullName
        $parts=$f.FullName.Substring($Path.Length+1).Split('\')
        $allowed=@('src','scripts','package.json','bun.lock','README.md','tsconfig.json'); if ($Existing) { $allowed+='node_modules' }
        if ($parts[0] -cnotin $allowed -or @($parts | Where-Object { $_ -in @('.git','.env') -or $_.StartsWith('.env.') }).Count) { Unmanaged $f.FullName }
    }
    try {
        if (((-not $Existing -or [IO.File]::Exists((Join-Path $Path package.json))) -and (Read-Json ([IO.File]::ReadAllText((Join-Path $Path package.json)))).name -cne 'envswitch') -or (-not $Existing -and (-not [IO.File]::Exists((Join-Path $Path 'src\cli.ts')) -or -not [IO.File]::Exists((Join-Path $Path bun.lock))))) { throw 'source' }
    } catch { Fail 'archive did not contain an envswitch source package' }
}
function Get-UserPath {
    if ($script:TestMode) {
        if ([IO.File]::Exists($script:UserPathFile)) { return [IO.File]::ReadAllText($script:UserPathFile,$script:Utf8) }; return ''
    }
    return [Environment]::GetEnvironmentVariable('Path','User')
}
function Set-UserPath([string]$Value) {
    if ($script:TestMode) { Replace-Managed $script:UserPathFile $script:Utf8.GetBytes($Value); return }
    [Environment]::SetEnvironmentVariable('Path',$Value,'User')
}
function Test-PathEntry([string]$Value,[string]$Directory) {
    foreach ($entry in $Value.Split(';')) { if ($entry.TrimEnd('\').Equals($Directory.TrimEnd('\'),[StringComparison]::OrdinalIgnoreCase)) { return $true } }; return $false
}
function Cleanup {
    $rollbackFailed=$false
    if (-not $script:Committed) {
        try {
            for ($i=$script:Journal.Count-1;$i -ge 0;$i--) {
                $p,$backup=$script:Journal[$i]
                if ($backup) {
                    if ([IO.Directory]::Exists($p)) { Remove-Item -LiteralPath $p -Recurse -Force; [IO.Directory]::Move($backup,$p) }
                    elseif ([IO.File]::Exists($p)) { [IO.File]::Replace($backup,$p,$null) }
                    elseif ([IO.Directory]::Exists($backup)) { [IO.Directory]::Move($backup,$p) }
                    else { [IO.File]::Move($backup,$p) }
                } elseif (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Recurse -Force }
            }
            if ($script:PathChanged -and -not $script:TestMode) { [Environment]::SetEnvironmentVariable('Path',$script:OldUserPath,'User') }
            if ($script:OldProcessPath) { $env:Path=$script:OldProcessPath }
        } catch { $rollbackFailed=$true; [Console]::Error.WriteLine("error: rollback incomplete; preserve $script:Stage and reinstall using the README instructions") }
    }
    if (-not $script:Committed -and -not $rollbackFailed -and $script:NewPayload) {
        try { Assert-NoLinks $script:NewPayload; [IO.File]::Delete($script:NewPayload) } catch { Info "deferred cleanup: $script:NewPayload" }
    }
    if ($script:Locked -and [IO.File]::Exists($script:Lock)) {
        Assert-NoLinks $script:Lock
        if ([IO.File]::ReadAllText($script:Lock) -ceq "$script:Token $PID`n") { [IO.File]::Delete($script:Lock) }
    }
    if (-not $rollbackFailed -and $script:Stage -and [IO.Directory]::Exists($script:Stage)) {
        try { Remove-Item -LiteralPath $script:Stage -Recurse -Force } catch { Info "deferred cleanup: $script:Stage" }
    }
    for ($i=$script:Created.Count-1;$i -ge 0;$i--) {
        $p=$script:Created[$i]
        if ([IO.Directory]::Exists($p) -and [IO.Directory]::GetFileSystemEntries($p).Length -eq 0) { [IO.Directory]::Delete($p) }
    }
}

try {
    $script:TestMode=$env:ENVSWITCH_INSTALL_TEST_MODE -ceq 'install'
    if ($env:ENVSWITCH_INSTALL_TEST_MODE -and -not $script:TestMode) { Fail 'invalid installer setting: ENVSWITCH_INSTALL_TEST_MODE' }
    if ($script:TestMode -and $env:ENVSWITCH_INSTALL_TEST_FAIL_AT -and $env:ENVSWITCH_INSTALL_TEST_FAIL_AT -cnotin @('after-payload','after-launcher','after-metadata')) { Fail 'invalid installer setting: ENVSWITCH_INSTALL_TEST_FAIL_AT' }
    if (-not [Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([Runtime.InteropServices.OSPlatform]::Windows)) { Fail 'unsupported OS for this installer. Use scripts/install.sh on macOS, Linux, or WSL.' }
    $target=Get-Target
    if (-not (Test-Repo $Repo)) { Fail "invalid repository format: $Repo. Expected owner/repo." }
    $pin=Convert-Tag $Version Version; $current=Convert-Tag $CurrentVersion CurrentVersion
    if (-not $AppRoot) { if (-not $env:LOCALAPPDATA) { Fail 'LOCALAPPDATA is not set. Pass -InstallDir and -AppRoot explicitly.' }; $AppRoot=Join-Path $env:LOCALAPPDATA 'Programs\envswitch' }
    $AppRoot=Normalize-Directory $AppRoot
    if (-not $InstallDir) { $InstallDir=Join-Path $AppRoot bin }; $InstallDir=Normalize-Directory $InstallDir
    $app=Join-Path $AppRoot current; $script:Launcher=Join-Path $InstallDir envswitch.cmd; $script:Lock=Join-Path $AppRoot '.install.lock'
    if ($AppRoot -eq $env:USERPROFILE -or $AppRoot -eq $env:HOME -or $AppRoot -eq $InstallDir -or (Test-Inside $InstallDir $app) -or (Test-Inside $AppRoot $script:Launcher) -or ($AppRoot+'\'+$InstallDir) -match '[\\/]\.install-') { Unsafe $AppRoot }
    foreach ($p in @($app,$script:Launcher,(Join-Path $app bin),(Join-Path $app scripts),(Join-Path $app 'scripts/install.sh'),(Join-Path $app 'scripts/install.ps1'),(Join-Path $app install.json),(Join-Path $app '.version'))) { Assert-NoLinks $p }
    if ((Test-Path -LiteralPath $app) -and -not [IO.Directory]::Exists($app)) { Unsafe $app }
    foreach ($p in @((Join-Path $AppRoot '.git'),(Join-Path $app '.git'),(Join-Path $InstallDir envswitch.exe))) { if (Test-Path -LiteralPath $p) { Unmanaged $p } }
    $script:ApiBase='https://api.github.com'
    if ($script:TestMode) {
        $script:UserPathFile=$env:ENVSWITCH_INSTALL_TEST_USER_PATH_FILE
        if ($script:UserPathFile) {
            try {
                $parent=Normalize-Directory ([IO.Path]::GetDirectoryName($script:UserPathFile))
                $leaf=[IO.Path]::GetFileName($script:UserPathFile)
                if (-not $leaf -or $leaf -match '["\x00-\x1f\x7f-\x9f:]|[. ]$') { throw 'unsafe test path' }
                $script:UserPathFile=Join-Path $parent $leaf
            } catch { Fail 'invalid installer setting: ENVSWITCH_INSTALL_TEST_USER_PATH_FILE' }
        }
        if (-not $script:UserPathFile -or -not [IO.Path]::IsPathRooted($script:UserPathFile) -or -not (Test-Inside $script:UserPathFile $env:USERPROFILE) -or $script:UserPathFile -eq $env:USERPROFILE) { Fail 'invalid installer setting: ENVSWITCH_INSTALL_TEST_USER_PATH_FILE' }
        Assert-NoLinks $script:UserPathFile
        if ($env:ENVSWITCH_INSTALL_TEST_API_BASE_URL) {
            $u=$null
            if (-not [uri]::TryCreate($env:ENVSWITCH_INSTALL_TEST_API_BASE_URL,[UriKind]::Absolute,[ref]$u) -or $u.Scheme -ne 'http' -or $u.Host -notin @('127.0.0.1','localhost','[::1]') -or $u.UserInfo -or $u.AbsolutePath -ne '/' -or $u.Query -or $u.Fragment) { Fail 'invalid installer setting: ENVSWITCH_INSTALL_TEST_API_BASE_URL' }
            $script:ApiBase=$u.GetLeftPart([UriPartial]::Authority)
        }
    }
    $previous=Read-Receipt (Join-Path $app install.json)
    if ([IO.File]::Exists($script:Launcher)) {
        $content=[IO.File]::ReadAllText($script:Launcher,$script:Utf8); $recognized=$false
        if ($previous -and $content -ceq (New-Launcher $previous.kind $previous.payloadPath)) { $recognized=$true }
        if ($content -ceq (New-Launcher source (Join-Path $app 'src\cli.ts'))) { $recognized=$true }
        $legacy=@('@echo off','setlocal','where bun >NUL 2>NUL','if %ERRORLEVEL% EQU 0 (',('  bun "'+(Join-Path $app 'src\cli.ts')+'" %*'),'  exit /b %ERRORLEVEL%',')','if exist "%USERPROFILE%\.bun\bin\bun.exe" (',('  "%USERPROFILE%\.bun\bin\bun.exe" "'+(Join-Path $app 'src\cli.ts')+'" %*'),'  exit /b %ERRORLEVEL%',')','echo envswitch: bun is not installed or not on PATH 1>&2','exit /b 127') -join "`n"
        if ($content.Replace("`r`n","`n").TrimEnd("`n") -ceq $legacy) { $recognized=$true }
        if ($content -match 'envswitch_(0\.0\.(?:0|[1-9][0-9]*))_x86_64-pc-windows-msvc\.exe') {
            if ($content -ceq (New-Launcher native (Get-Payload $AppRoot native ('v'+$Matches[1]) $target))) { $recognized=$true }
        }
        if (-not $recognized) { Unmanaged $script:Launcher }
    } elseif (Test-Path -LiteralPath $script:Launcher) { Unmanaged $script:Launcher }
    if ([IO.Directory]::Exists($app) -and [IO.Directory]::GetFileSystemEntries($app).Length -and -not $previous -and -not [IO.File]::Exists($script:Launcher)) {
        try { if ((Read-Json ([IO.File]::ReadAllText((Join-Path $app package.json)))).name -cne 'envswitch') { throw 'name' } } catch { Unmanaged $app }
    }
    if ($WhatIfPreference) {
        $selection=if ($pin) { $pin } else { 'latest' }
        foreach ($line in @("What if: Resolve envswitch release $selection from $Repo","What if: Verify SHA-256 and install the release's source or native payload to $app","What if: Install envswitch launcher to $script:Launcher",'What if: Write envswitch install receipt and version marker',"What if: Add $InstallDir to the user PATH if missing")) { [Console]::Out.WriteLine($line) }
        if ($VerifyAttestations) { [Console]::Out.WriteLine('What if: Verify GitHub artifact attestation with gh') }; exit 0
    }
    if (-not $PSCmdlet.ShouldProcess($AppRoot,'Install envswitch')) { exit 0 }
    $script:Gh=Get-Gh; $verifier=Get-Gh -Verifier
    if ($VerifyAttestations -and -not $verifier) { Fail 'gh is required when attestation verification is requested' }
    Ensure-Directory $AppRoot
    $script:Token=[guid]::NewGuid().ToString('N')
    try { Write-Utf8Bytes $script:Lock $script:Utf8.GetBytes("$script:Token $PID`n") }
    catch { Fail "another envswitch installer may be running: $script:Lock" 'retry after it exits; remove the lock only after confirming no installer is running' }
    $script:Locked=$true
    $lockAcl=New-Object Security.AccessControl.FileSecurity; $lockAcl.SetAccessRuleProtection($true,$false)
    foreach ($sid in @([Security.Principal.WindowsIdentity]::GetCurrent().User.Value,'S-1-5-18','S-1-5-32-544')) {
        $identity=New-Object Security.Principal.SecurityIdentifier($sid)
        $lockAcl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($identity,'FullControl','Allow')))
    }
    Set-Acl -LiteralPath $script:Lock -AclObject $lockAcl
    $previous=Read-Receipt (Join-Path $app install.json)
    $script:Stage=Join-Path $AppRoot ('.install-stage-'+$script:Token); New-PrivateDirectory $script:Stage
    if ($PSCommandPath) { Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $script:Stage executing-install.ps1) }
    else { Write-Utf8Bytes (Join-Path $script:Stage executing-install.ps1) $script:Utf8.GetBytes([string]$MyInvocation.MyCommand.Definition) }
    $endpoint='repos/'+$Repo+'/releases/'+$(if ($pin) { 'tags/'+$pin } else { 'latest' }); $raw=$null
    if ($script:Gh) { $response=Invoke-GhCommand $script:Gh @('api',$endpoint); if ($response.Code -eq 0) { $raw=$script:Utf8.GetString($response.Out) } }
    if ($null -eq $raw) {
        try { $raw=$script:Utf8.GetString((Get-Http ($script:ApiBase+'/'+$endpoint) 'application/vnd.github+json')) }
        catch { if ($pin) { Fail "GitHub Release not found or unavailable: $Repo@$pin" } else { Fail "no GitHub Release found for $Repo, or access is denied" } }
    }
    try {
        $release=Read-Json $raw; $script:Tag=$release.tag_name
        if ($script:Tag -cnotmatch '\Av0\.0\.(0|[1-9][0-9]*)\z' -or ($pin -and $pin -cne $script:Tag) -or ($release.id -isnot [int] -and $release.id -isnot [long]) -or $release.id -le 0 -or $release.draft -isnot [bool] -or $release.draft -or $release.prerelease -isnot [bool] -or $release.prerelease -or $release.assets -isnot [array]) { throw 'shape' }
        $script:Assets=New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::Ordinal)
        $duplicates=New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
        foreach ($asset in $release.assets) {
            if ($asset.name -isnot [string] -or $asset.url -isnot [string] -or $asset.url -cnotmatch ('\A'+[regex]::Escape($script:ApiBase+'/repos/'+$Repo+'/releases/assets/')+'[0-9]+\z')) { throw 'asset' }; if ($script:Assets.ContainsKey($asset.name)) { $null=$duplicates.Add($asset.name) }; $script:Assets[$asset.name]=$asset.url
        }
        $ver=$script:Tag.Substring(1); $source='envswitch_'+$ver+'_source.tar.gz'
        $natives=@($script:Targets | ForEach-Object { 'envswitch_'+$ver+'_'+$_+$(if ($_.EndsWith('msvc')) { '.zip' } else { '.tar.gz' }) })
        $kind=if (@($natives | Where-Object { $script:Assets.ContainsKey($_) }).Count) { 'native' } else { 'source' }
        $required=if ($kind -eq 'native') { $natives } else { @($source) }
        if (($kind -eq 'native' -and $script:Assets.ContainsKey($source)) -or @($required | Where-Object { -not $script:Assets.ContainsKey($_) }).Count) { throw 'kind' }
        foreach ($n in ($required+@('SHA256SUMS','install.sh','install.ps1'))) { if ($duplicates.Contains($n)) { throw 'duplicate' } }
    } catch { Fail "invalid envswitch release metadata for $Repo" }
    if (-not $script:Assets.ContainsKey('SHA256SUMS')) { Fail "release $script:Tag is missing SHA256SUMS; refusing unverified installation" }
    foreach ($n in @('install.sh','install.ps1')) { if (-not $script:Assets.ContainsKey($n)) { Fail "invalid envswitch release metadata for $Repo" } }
    $name=if ($kind -eq 'native') { 'envswitch_'+$ver+'_'+$target+'.zip' } else { $source }
    Info "resolved envswitch $script:Tag ($kind)"; if ($kind -eq 'native') { Info "selected target: $target" }
    $manifest=Save-Asset SHA256SUMS; $archive=Save-Asset $name
    $script:Sums=Read-Sums ([IO.File]::ReadAllBytes($manifest)) $script:Tag ($required+@('install.sh','install.ps1')); Verify-Hash $archive $name
    if ($VerifyAttestations) { Info "verifying GitHub attestation for $name"; $attestation=Invoke-GhCommand $verifier @('attestation','verify',$archive,'--repo',$Repo); if ($attestation.Code -ne 0) { Fail "GitHub attestation verification failed for $name" } }
    Info "extracting $kind archive"; $tree=Join-Path $script:Stage extracted; Expand-Safe $archive $tree $kind
    if ($kind -eq 'source') { $tree=Join-Path $tree envswitch }
    foreach ($n in @('install.sh','install.ps1')) { Verify-Hash (Join-Path $tree ('scripts\'+$n)) $n }
    $payload=Get-Payload $AppRoot $kind $script:Tag $target
    if ($kind -eq 'native') {
        $staged=Join-Path $script:Stage ([IO.Path]::GetFileName($payload)); Copy-Item -LiteralPath (Join-Path $tree envswitch.exe) -Destination $staged
        if (-not (Test-Payload $staged $kind $ver $target -Staged)) { Fail "envswitch launcher verification failed: $script:Launcher" }
    } else {
        Assert-Source $tree
        $bunCommand=Get-Command bun -CommandType Application -ErrorAction SilentlyContinue
        $bun=if ($bunCommand) { $bunCommand.Source } else { Join-Path $env:USERPROFILE '.bun\bin\bun.exe' }
        if (-not [IO.File]::Exists($bun)) {
            Info 'Bun is not installed; installing Bun'
            & powershell -NoProfile -Command 'irm bun.sh/install.ps1 | iex' *> $null
            $env:Path=(Join-Path $env:USERPROFILE '.bun\bin')+';'+$env:Path
            $bunCommand=Get-Command bun -CommandType Application -ErrorAction SilentlyContinue
            if ($bunCommand) { $bun=$bunCommand.Source }
            if (-not [IO.File]::Exists($bun)) { Fail 'Bun installation finished, but bun was not found on PATH' }
        }
        Info "using Bun at $bun"; if ((Invoke-Captured $bun '--version').Code -ne 0) { Fail 'Bun verification failed' }
        Info 'installing production dependencies'; Push-Location -LiteralPath $tree
        try { & $bun install --production --frozen-lockfile *> $null; if ($LASTEXITCODE -ne 0) { Fail 'bun install failed' } } finally { Pop-Location }
        $check=Join-Path $script:Stage source-check.cmd; Write-Utf8Bytes $check $script:Utf8.GetBytes((New-Launcher source (Join-Path $tree 'src\cli.ts')))
        if (-not (Test-Payload $check $kind $ver $target)) { Fail "envswitch launcher verification failed: $script:Launcher" }
    }
    $receipt=[ordered]@{schemaVersion=1;kind=$kind;repo=$Repo;appRoot=$AppRoot;installDir=$InstallDir;launcherPath=$script:Launcher;target=$(if ($kind -eq 'native') { $target } else { $null });tag=$script:Tag;payloadPath=$payload}
    $healthy=$null -ne $previous
    foreach ($key in $receipt.Keys) { if ($healthy -and $previous.$key -cne $receipt[$key]) { $healthy=$false } }
    $healthy=$healthy -and [IO.File]::Exists((Join-Path $app '.version')) -and [IO.File]::ReadAllText((Join-Path $app '.version'),$script:Utf8) -ceq "$script:Tag`n" -and [IO.File]::Exists($script:Launcher) -and [IO.File]::ReadAllText($script:Launcher,$script:Utf8) -ceq (New-Launcher $kind $payload) -and [IO.File]::Exists($payload)
    foreach ($n in @('install.sh','install.ps1')) {
        $p=Join-Path $app ('scripts\'+$n)
        if (-not [IO.File]::Exists($p) -or (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLowerInvariant() -cne $script:Sums[$n]) { $healthy=$false }
    }
    if ($kind -eq 'native' -and $healthy -and (Get-FileHash -LiteralPath $payload -Algorithm SHA256).Hash -cne (Get-FileHash -LiteralPath $staged -Algorithm SHA256).Hash) { $healthy=$false }
    if ($healthy -and (Test-Payload $script:Launcher $kind $ver $target)) {
        $script:Committed=$true; Cleanup; $script:Stage=$null; $script:Locked=$false
        [Console]::Out.WriteLine("success: envswitch is already up to date ($script:Tag)"); exit 0
    }
    $oldTag=if ($previous) { $previous.tag } else { $null }
    if (-not $oldTag -and [IO.File]::Exists((Join-Path $app '.version'))) { try { $oldTag=Convert-Tag ([IO.File]::ReadAllText((Join-Path $app '.version')).Trim([char]0xfeff,"`r","`n")) CurrentVersion } catch {} }
    $label=if ($oldTag) { $oldTag } else { $current }; $outcome=if ($label -ceq $script:Tag -and (-not $previous -or ($previous.kind -ceq $kind -and $previous.target -ceq $receipt.target))) { 'repaired' } else { 'installed' }
    Ensure-Directory $app; Ensure-Directory $InstallDir
    if ($kind -eq 'native') {
        Ensure-Directory (Join-Path $app bin); Assert-NoLinks $payload
        if ([IO.File]::Exists($payload)) {
            if ((Get-FileHash -LiteralPath $payload -Algorithm SHA256).Hash -cne (Get-FileHash -LiteralPath $staged -Algorithm SHA256).Hash) { Fail "existing native payload differs from verified release: $payload" 'close envswitch processes and reinstall using the README instructions; preserve the existing file until recovery is complete' }
        } else { Info "installing binary to $payload"; [IO.File]::Move($staged,$payload); $script:NewPayload=$payload }
    } else {
        foreach ($f in Get-ChildItem -LiteralPath $app -Force) { if ($f.Name -cnotin @('src','node_modules','package.json','bun.lock','README.md','tsconfig.json','scripts','bin','.version','install.json')) { Unmanaged $f.FullName } }
        $old=Join-Path $script:Stage old-source; New-PrivateDirectory $old
        foreach ($n in @('src','node_modules','package.json','bun.lock','README.md','tsconfig.json')) { $p=Join-Path $app $n; if (Test-Path -LiteralPath $p) { Assert-NoLinks $p; foreach ($f in Get-ChildItem -LiteralPath $p -Recurse -Force) { Assert-NoLinks $f.FullName; if ($f.Name -eq '.env' -or $f.Name.StartsWith('.env.',[StringComparison]::OrdinalIgnoreCase)) { Unmanaged $f.FullName } }; Copy-Item -LiteralPath $p -Destination (Join-Path $old $n) -Recurse } }
        if ([IO.Directory]::GetFileSystemEntries($old).Length) { Assert-Source $old -Existing }
        foreach ($n in @('src','node_modules','package.json','bun.lock','README.md','tsconfig.json')) {
            $p=Join-Path $app $n; $s=Join-Path $tree $n
            if (Test-Path -LiteralPath $s) { Replace-Managed $p $null $s }
            elseif (Test-Path -LiteralPath $p) { $b=Join-Path $script:Stage ('removed-'+$n); $null=$script:Journal.Add(@($p,$b)); Move-Item -LiteralPath $p -Destination $b }
        }
    }
    Boundary after-payload
    Info "installing launcher to $script:Launcher"; Replace-Managed $script:Launcher $script:Utf8.GetBytes((New-Launcher $kind $payload))
    Info 'verifying envswitch launcher'
    if (-not [IO.File]::Exists($script:Launcher)) { Fail "launcher was not installed: $script:Launcher" }
    if (-not (Test-Payload $script:Launcher $kind $ver $target)) { Fail "envswitch launcher verification failed: $script:Launcher" }
    Boundary after-launcher
    foreach ($n in @('install.sh','install.ps1')) { Replace-Managed (Join-Path $app ('scripts\'+$n)) $null (Join-Path $tree ('scripts\'+$n)) }
    Replace-Managed (Join-Path $app install.json) $script:Utf8.GetBytes((ConvertTo-Json $receipt)+"`n")
    Replace-Managed (Join-Path $app '.version') $script:Utf8.GetBytes("$script:Tag`n")
    $checked=Read-Receipt (Join-Path $app install.json)
    foreach ($key in $receipt.Keys) { if ($checked.$key -cne $receipt[$key]) { Fail 'envswitch installation failed: metadata verification' } }
    if ([IO.File]::ReadAllText((Join-Path $app '.version'),$script:Utf8) -cne "$script:Tag`n") { Fail 'envswitch installation failed: metadata verification' }
    Boundary after-metadata
    $script:OldUserPath=Get-UserPath; $script:OldProcessPath=$env:Path
    if (-not (Test-PathEntry ([string]$script:OldUserPath) $InstallDir)) {
        $newPath=if ($script:OldUserPath) { $script:OldUserPath+';'+$InstallDir } else { $InstallDir }
        $script:PathChanged=$true; Set-UserPath $newPath; Info "added $InstallDir to the user PATH"
    }
    if (-not (Test-PathEntry $env:Path $InstallDir)) { $env:Path=$InstallDir+';'+$env:Path }
    $script:Committed=$true; Cleanup; $script:Stage=$null; $script:Locked=$false
    [Console]::Out.WriteLine("success: $outcome envswitch $script:Tag")
    if ($kind -eq 'native') { Info "binary: $payload" } else { Info "source: $app" }; Info "launcher: $script:Launcher"
    $escaped=$InstallDir.Replace('`','``').Replace('"','`"').Replace('$','`$')
    Info 'restart your terminal, or run this command now:'; Info ('$env:Path = "'+$escaped+';$env:Path"'); Info "then run 'envswitch --help' to verify the installation"
} catch [Management.Automation.PipelineStoppedException] {
    exit 130
} catch {
    if ($_.Exception.Message.StartsWith('error: ')) { [Console]::Error.WriteLine($_.Exception.Message) }
    else { [Console]::Error.WriteLine('error: envswitch installation failed: '+$_.Exception.GetType().Name) }
    exit 1
} finally {
    try { Cleanup } catch { [Console]::Error.WriteLine('error: rollback incomplete; preserve '+$script:Stage+' and reinstall using the README instructions') }
}
