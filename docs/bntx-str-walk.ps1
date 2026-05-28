param(
    [string]$Path = "c:\Users\intpa\Switch-Toolbox\local-assets\info_melee\unpacked\timg\__Combined.bntx"
)

$b = [System.IO.File]::ReadAllBytes($Path)

# _STR starts at 0x808 (we verified), strings at 0x81C
$cur = 0x81C
$dictOff = 0x2148
$idx = 0
$totalSize = 0

while ($cur -lt $dictOff) {
    $len = [BitConverter]::ToUInt16($b, $cur)
    $bodyStart = $cur + 2
    $bodyEnd = $bodyStart + $len
    if ($bodyEnd -ge $b.Length) { break }
    
    # Body
    $body = if ($len -gt 0) { [System.Text.Encoding]::UTF8.GetString($b, $bodyStart, $len) } else { "" }
    
    # Find total entry size: look for the next u16 length field by trying alignments
    # Algorithm: chars+null aligned to 2 = (len+1+1) & !1
    $entrySize = 2 + ((($len + 1) + 1) -band -bnot 1)
    
    # Show
    if ($idx -lt 5 -or $cur -gt 0x2100) {
        Write-Host ("  [{0,3}] @0x{1:x4}: len={2,3} size={3,3} body='{4}'" -f $idx, $cur, $len, $entrySize, $body)
    } elseif ($idx -eq 5) {
        Write-Host "  ..."
    }
    
    $cur += $entrySize
    $totalSize += $entrySize
    $idx++
}

Write-Host ""
Write-Host "Total strings parsed: $idx"
Write-Host "Total bytes consumed: $totalSize"
Write-Host "Expected end: 0x$('{0:x4}' -f $dictOff)"
Write-Host "Actual end:   0x$('{0:x4}' -f $cur)"
Write-Host "Diff:         $($dictOff - $cur) bytes"
