param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $CargoArgs = @()
)

Write-Host "Running slow tests..."
$ErrorActionPreference = "Stop"
if ($PSVersionTable.PSVersion -ge [version]"7.3") {
    $PSNativeCommandUseErrorActionPreference = $true
}
$env:RUN_SLOW_TESTS = "1"
$exitCode = 0

try {
    cargo test --workspace @CargoArgs
    $exitCode = $LASTEXITCODE
} catch [System.Management.Automation.NativeCommandExitException] {
    $exitCode = $_.Exception.ExitCode
} catch [System.Exception] {
    if ($null -ne $_.Exception.ExitCode) {
        $exitCode = $_.Exception.ExitCode
    } else {
        $exitCode = 1
    }
}
exit $exitCode
