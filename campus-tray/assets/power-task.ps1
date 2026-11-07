# Input is a base64 JSON object supplied by the native application, never executable text.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
try {
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $name = 'SHUNet.Power.' + $sid + '.' + $request.id
    if ($request.id -notmatch '^[a-f0-9]{32}$') { throw 'Invalid SHUNet plan identifier.' }
    $marker = 'SHUNet.PowerPlan/1/' + $request.id
    function Test-Owner($task) {
        $owner = [string]$task.Principal.UserId
        if ($owner -notmatch '^S-1-') {
            $owner = ([Security.Principal.NTAccount]::new($owner)).Translate([Security.Principal.SecurityIdentifier]).Value
        }
        return $task.Description -eq $marker -and $owner -eq $sid
    }
    switch ($request.operation) {
        'create' {
            $xml = $request.xml.Replace('__SHUNET_SID__', $sid)
            # No -Force: an existing task must never silently be overwritten.
            Register-ScheduledTask -TaskName $name -Xml $xml | Out-Null
            @{ok=$true; state='Ready'} | ConvertTo-Json -Compress
        }
        'inspect' {
            $task = Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
            if ($null -eq $task) { @{ok=$true; state='Missing'} | ConvertTo-Json -Compress; break }
            if (-not (Test-Owner $task)) { throw 'Task ownership does not match SHUNet.' }
            $info = Get-ScheduledTaskInfo -InputObject $task
            @{ok=$true; state=[string]$task.State; result=[long]$info.LastTaskResult} | ConvertTo-Json -Compress
        }
        'cancel' {
            $task = Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
            if ($null -ne $task) {
                if (-not (Test-Owner $task)) { throw 'Task ownership does not match SHUNet.' }
                Stop-ScheduledTask -InputObject $task -ErrorAction SilentlyContinue
                Unregister-ScheduledTask -InputObject $task -Confirm:$false
            }
            @{ok=$true; state='Removed'} | ConvertTo-Json -Compress
        }
        default { throw 'Unknown SHUNet scheduler operation.' }
    }
} catch {
    @{ok=$false; error=$_.Exception.Message} | ConvertTo-Json -Compress
    exit 1
}
