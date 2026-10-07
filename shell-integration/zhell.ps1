if ($global:__ZhellLoaded) { return }
$global:__ZhellLoaded = $true
$global:__ZhellOrigPrompt = $function:prompt
$global:__ZhellRan = $false
$global:__ZhellLastId = 0

function global:__ZhellEscape([string]$s) {
    $s.Replace('\', '\\').Replace(';', '\x3b').Replace("`n", '\x0a').Replace("`r", '\x0d').Replace("$([char]27)", '\x1b').Replace("$([char]7)", '\x07')
}

function global:prompt {
    $ok = $?
    $last = Get-History -Count 1
    if ($last -and $last.Id -ne $global:__ZhellLastId) { $global:__ZhellRan = $true }
    $global:__ZhellLastId = if ($last) { $last.Id } else { 0 }
    $code = if ($global:__ZhellRan) { if ($ok) { 0 } elseif ($LASTEXITCODE) { $LASTEXITCODE } else { 1 } } else { 0 }
    $e = [char]27; $b = [char]7
    $out = ""
    if ($global:__ZhellRan) { $out += "$e]133;D;$code$b" }
    $global:__ZhellRan = $false
    $loc = $executionContext.SessionState.Path.CurrentLocation
    if ($loc.Provider.Name -eq 'FileSystem') {
        $out += "$e]7;file://$($env:COMPUTERNAME)/$($loc.ProviderPath.Replace('\', '/'))$b"
    }
    $out += "$e]133;A$b" + (& $global:__ZhellOrigPrompt) + "$e]133;B$b"
    $out
}

if (Get-Module PSReadLine) {
    $global:__ZhellReadLine = $function:PSConsoleHostReadLine
    function global:PSConsoleHostReadLine {
        $line = & $global:__ZhellReadLine
        $global:__ZhellRan = $line.Trim().Length -gt 0
        [Console]::Write("$([char]27)]633;E;$(__ZhellEscape $line)$([char]7)$([char]27)]133;C$([char]7)")
        $line
    }
}
