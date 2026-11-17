<#
.SYNOPSIS
A class-based DSC resource used only by the real-tool tests of the Meridian DSC plugin.

.DESCRIPTION
The tests need a real DSC resource that works on any machine, changes nothing outside a
directory the test owns, and leaves evidence of what was done to it. This is one: a marker
file. Its state is whether the file exists and what it holds.

  Test  the file exists with the wanted content (Present), or does not exist (Absent).
  Set   writes or removes the file, and appends a line to <Name>.sets beside it, so a test
        can count how many times Set really ran.
  Get   reports what is on disk.

It runs through Invoke-DscResource (DSC v2) and, through the Microsoft.DSC/PowerShell
adapter, through the dsc command line (DSC v3).
#>

enum Ensure
{
    Absent
    Present
}

[DscResource()]
class MarkerFile
{
    [DscProperty(Key)]
    [string] $Name

    [DscProperty(Mandatory)]
    [string] $Path

    [DscProperty()]
    [string] $Content = ''

    [DscProperty()]
    [Ensure] $Ensure = [Ensure]::Present

    hidden [string] File()
    {
        return (Join-Path -Path $this.Path -ChildPath ('{0}.marker' -f $this.Name))
    }

    [MarkerFile] Get()
    {
        $current = [MarkerFile]::new()
        $current.Name = $this.Name
        $current.Path = $this.Path
        if (Test-Path -LiteralPath $this.File())
        {
            $current.Ensure = [Ensure]::Present
            $current.Content = [System.IO.File]::ReadAllText($this.File())
        }
        else
        {
            $current.Ensure = [Ensure]::Absent
        }
        return $current
    }

    [bool] Test()
    {
        $current = $this.Get()
        if ($this.Ensure -eq [Ensure]::Absent)
        {
            return ($current.Ensure -eq [Ensure]::Absent)
        }
        return ($current.Ensure -eq [Ensure]::Present -and $current.Content -eq $this.Content)
    }

    [void] Set()
    {
        if (-not (Test-Path -LiteralPath $this.Path))
        {
            $null = New-Item -ItemType Directory -Path $this.Path -Force
        }
        if ($this.Ensure -eq [Ensure]::Present)
        {
            [System.IO.File]::WriteAllText($this.File(), $this.Content)
        }
        elseif (Test-Path -LiteralPath $this.File())
        {
            Remove-Item -LiteralPath $this.File() -Force
        }
        $log = Join-Path -Path $this.Path -ChildPath ('{0}.sets' -f $this.Name)
        [System.IO.File]::AppendAllText($log, ("{0}`n" -f $this.Ensure))
    }
}
