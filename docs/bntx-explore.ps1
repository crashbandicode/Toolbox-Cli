param(
    [string]$Path = "c:\Users\intpa\Switch-Toolbox\local-assets\info_melee\unpacked\timg\__Combined.bntx"
)

$b = [System.IO.File]::ReadAllBytes($Path)
$len = $b.Length

function ReadU16($off) { return [BitConverter]::ToUInt16($b, $off) }
function ReadU32($off) { return [BitConverter]::ToUInt32($b, $off) }
function ReadU64($off) { return [BitConverter]::ToUInt64($b, $off) }
function ReadAscii($off, $n) { return [System.Text.Encoding]::ASCII.GetString($b, $off, $n) }
function HexAt($off, $n) {
    $end = [Math]::Min($off + $n, $len) - 1
    return ($b[$off..$end] | ForEach-Object { $_.ToString('x2') }) -join ' '
}

Write-Host "File: $Path  ($len bytes)"
Write-Host ""

Write-Host "=== BNTX header (0x00-0x1F) ==="
Write-Host "  magic           = $(ReadAscii 0 4)"
Write-Host "  padding         = 0x$('{0:x8}' -f (ReadU32 0x4))"
Write-Host "  version         = 0x$('{0:x8}' -f (ReadU32 0x8))"
Write-Host "  bom             = 0x$('{0:x4}' -f (ReadU16 0xC))"
Write-Host "  align_shift     = 0x$('{0:x2}' -f $b[0xE])"
Write-Host "  target_addr_sz  = 0x$('{0:x2}' -f $b[0xF])"
Write-Host "  filename_offset = 0x$('{0:x8}' -f (ReadU32 0x10))"
Write-Host "  flag            = 0x$('{0:x4}' -f (ReadU16 0x14))"
Write-Host "  first_blk_off   = 0x$('{0:x4}' -f (ReadU16 0x16))"
Write-Host "  reloc_tbl_off   = 0x$('{0:x8}' -f (ReadU32 0x18))"
Write-Host "  file_size       = 0x$('{0:x8}' -f (ReadU32 0x1C))"
Write-Host ""

Write-Host "=== NX header (0x20-0x47) ==="
Write-Host "  magic           = $(ReadAscii 0x20 4)"
Write-Host "  count           = $(ReadU32 0x24)"
Write-Host "  info_ptrs_off   = 0x$('{0:x16}' -f (ReadU64 0x28))"
Write-Host "  data_blk_ptr    = 0x$('{0:x16}' -f (ReadU64 0x30))"
Write-Host "  dict_ptr        = 0x$('{0:x16}' -f (ReadU64 0x38))"
Write-Host "  str_pool_off    = 0x$('{0:x8}' -f (ReadU32 0x40))"
Write-Host "  str_pool_size   = 0x$('{0:x8}' -f (ReadU32 0x44))"
Write-Host ""

# Memory pool 0x48..0x198 (0x150 bytes)
$mpStart = 0x48
$mpEnd = $mpStart + 0x150
$allZeros = $true
for ($i = $mpStart; $i -lt $mpEnd; $i++) { if ($b[$i] -ne 0) { $allZeros = $false; break } }
Write-Host "=== memory pool (0x$('{0:x4}' -f $mpStart)..0x$('{0:x4}' -f $mpEnd)) — all zeros: $allZeros ==="
Write-Host ""

# Find _STR
$strOff = -1
for ($i = 0; $i -lt $len - 4; $i++) {
    if ($b[$i] -eq 0x5F -and $b[$i+1] -eq 0x53 -and $b[$i+2] -eq 0x54 -and $b[$i+3] -eq 0x52) { $strOff = $i; break }
}
Write-Host "=== _STR section at 0x$('{0:x4}' -f $strOff) ==="
if ($strOff -gt 0) {
    Write-Host "  magic           = $(ReadAscii $strOff 4)"
    Write-Host "  block_offset    = 0x$('{0:x8}' -f (ReadU32 ($strOff+4)))"
    Write-Host "  block_size      = 0x$('{0:x16}' -f (ReadU64 ($strOff+8)))"
    Write-Host "  str_count       = 0x$('{0:x8}' -f (ReadU32 ($strOff+0x10)))"
    Write-Host "  bytes 0x14..0x40:"
    Write-Host "    $(HexAt ($strOff+0x14) 0x40)"
}
Write-Host ""

# Find _DIC
$dicOff = -1
for ($i = $strOff + 0x14; $i -lt $len - 4; $i++) {
    if ($b[$i] -eq 0x5F -and $b[$i+1] -eq 0x44 -and $b[$i+2] -eq 0x49 -and $b[$i+3] -eq 0x43) { $dicOff = $i; break }
}
Write-Host "=== _DIC section at 0x$('{0:x4}' -f $dicOff) ==="
if ($dicOff -gt 0) {
    Write-Host "  magic           = $(ReadAscii $dicOff 4)"
    Write-Host "  bytes 0x4..0x60:"
    Write-Host "    $(HexAt ($dicOff+4) 0x60)"
}
Write-Host ""

# First BRTI
$brtiOff = -1
for ($i = $dicOff + 0x40; $i -lt $len - 4; $i++) {
    if ($b[$i] -eq 0x42 -and $b[$i+1] -eq 0x52 -and $b[$i+2] -eq 0x54 -and $b[$i+3] -eq 0x49) { $brtiOff = $i; break }
}
Write-Host "=== first BRTI at 0x$('{0:x4}' -f $brtiOff) ==="
Write-Host ""

# Find _RLT
$rltOff = -1
for ($i = $len - 0x100; $i -lt $len - 4; $i++) {
    if ($b[$i] -eq 0x5F -and $b[$i+1] -eq 0x52 -and $b[$i+2] -eq 0x4C -and $b[$i+3] -eq 0x54) { $rltOff = $i; break }
}
Write-Host "=== _RLT section at 0x$('{0:x4}' -f $rltOff) ==="
if ($rltOff -gt 0) {
    Write-Host "  magic           = $(ReadAscii $rltOff 4)"
    Write-Host "  rlt_section_pos = 0x$('{0:x8}' -f (ReadU32 ($rltOff+4)))"
    Write-Host "  count           = $(ReadU32 ($rltOff+8))"
    Write-Host "  padding         = 0x$('{0:x8}' -f (ReadU32 ($rltOff+0xC)))"
    $rltLen = $len - $rltOff
    Write-Host "  rlt total size  = $rltLen bytes"
}
Write-Host ""

Write-Host "=== Find BRTD ==="
$brtdOff = -1
for ($i = $brtiOff; $i -lt $len - 4; $i++) {
    if ($b[$i] -eq 0x42 -and $b[$i+1] -eq 0x52 -and $b[$i+2] -eq 0x54 -and $b[$i+3] -eq 0x44) { $brtdOff = $i; break }
}
Write-Host "  BRTD at 0x$('{0:x8}' -f $brtdOff)"
if ($brtdOff -gt 0) {
    Write-Host "  magic           = $(ReadAscii $brtdOff 4)"
    Write-Host "  field@0x4       = 0x$('{0:x8}' -f (ReadU32 ($brtdOff+4)))"
    Write-Host "  block_size      = 0x$('{0:x16}' -f (ReadU64 ($brtdOff+8)))"
}
