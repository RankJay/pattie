<#
    The driver's ETW provider (src/trace.c), for trace-run.ps1 and
    analyze-trace.ps1 to dot-source. The scripts follow the provider's name:
    its GUID is the standard name hash (as EventSource and TraceLogging derive
    it), computed here rather than copied, so only trace.c holds the literal.
#>
$PattieProviderName = 'Pattie.Driver'

# SHA-1 over a fixed namespace and the upper-cased name in UTF-16BE; the first
# 16 bytes, marked version 5, read as a GUID.
function Get-EtwProviderGuid([string]$Name) {
    [byte[]]$namespace = 0x48, 0x2C, 0x2D, 0xB2, 0xC3, 0x90, 0x47, 0xC8, 0x87, 0xF8, 0x1A, 0x15, 0xBF, 0xC1, 0x30, 0xFB
    $data = $namespace + [System.Text.Encoding]::BigEndianUnicode.GetBytes($Name.ToUpperInvariant())
    $sha1 = [System.Security.Cryptography.SHA1]::Create()
    try { $hash = $sha1.ComputeHash([byte[]]$data) } finally { $sha1.Dispose() }
    [byte[]]$bytes = $hash[0..15]
    $bytes[7] = ($bytes[7] -band 0x0F) -bor 0x50
    [guid]::new($bytes)
}

$PattieProvider = '{' + (Get-EtwProviderGuid $PattieProviderName) + '}'
