param(
    [string]$Path = "c:\Users\intpa\Switch-Toolbox\local-assets\info_melee\unpacked\timg\__Combined.bntx"
)

$b = [System.IO.File]::ReadAllBytes($Path)
$len = $b.Length

function H4($n)  { return [System.Convert]::ToString($n, 16).PadLeft(4, '0') }
function H8($n)  { return [System.Convert]::ToString($n, 16).PadLeft(8, '0') }
function H16($n) { return [System.Convert]::ToString($n, 16).PadLeft(16, '0') }
function ReadU16($off) { return [BitConverter]::ToUInt16($b, $off) }
function ReadU32($off) { return [BitConverter]::ToUInt32($b, $off) }
function ReadU64($off) { return [BitConverter]::ToUInt64($b, $off) }
function ReadAscii($off, $n) { return [System.Text.Encoding]::ASCII.GetString($b, $off, $n) }
function HexAt($off, $n) {
    $end = [Math]::Min($off + $n, $len) - 1
    return ($b[$off..$end] | ForEach-Object { $_.ToString('x2') }) -join ' '
}

$count = ReadU32 0x24
$infoPtrsOff = ReadU64 0x28
$dataBlkPtr = ReadU64 0x30
$dictOff = ReadU64 0x38
$relocOff = ReadU32 0x18
$fileSize = ReadU32 0x1C

Write-Host "count=$count infoPtrsOff=0x$(H8 $infoPtrsOff) dictOff=0x$(H8 $dictOff)"
Write-Host "dataBlkPtr=0x$(H8 $dataBlkPtr) relocOff=0x$(H8 $relocOff) fileSize=0x$(H8 $fileSize)"
Write-Host ""

Write-Host "=== Texture info pointer array @ 0x$(H4 $infoPtrsOff) (count=$count u64s) ==="
Write-Host "  first 5 pointers:"
for ($i = 0; $i -lt 5; $i++) {
    $p = ReadU64 ($infoPtrsOff + $i*8)
    Write-Host ("    [$i] -> 0x" + (H8 $p))
}
Write-Host "  last 3 pointers:"
for ($i = $count - 3; $i -lt $count; $i++) {
    $p = ReadU64 ($infoPtrsOff + $i*8)
    Write-Host ("    [$i] -> 0x" + (H8 $p))
}
$arrayEnd = $infoPtrsOff + $count * 8
Write-Host ("  array end = 0x" + (H8 $arrayEnd))
Write-Host ""

Write-Host "=== bytes 0x$(H8 $arrayEnd) .. 0x808 (gap before _STR) ==="
$gap = 0x808 - $arrayEnd
Write-Host ("  gap size = $gap bytes")
$allZero = $true
for ($i = $arrayEnd; $i -lt 0x808; $i++) { if ($b[$i] -ne 0) { $allZero = $false; break } }
Write-Host ("  all zeros: " + $allZero)
Write-Host ""

Write-Host "=== _STR section breakdown ==="
$strOff = 0x808
Write-Host ("  magic           = " + (ReadAscii $strOff 4))
Write-Host ("  unk1 (u32)      = 0x" + (H8 (ReadU32 ($strOff+4))))
Write-Host ("  block_size (u64) = 0x" + (H16 (ReadU64 ($strOff+8))))
Write-Host ("  str_count (u32) = " + (ReadU32 ($strOff+0x10)))
Write-Host ""
Write-Host "  first 8 strings:"
$cur = $strOff + 0x14
for ($i = 0; $i -lt 8 -and $cur -lt $dictOff; $i++) {
    $strLen = ReadU16 $cur
    $s = if ($strLen -gt 0) { ReadAscii ($cur + 2) $strLen } else { "" }
    Write-Host ("    @0x" + (H8 $cur) + ": len=" + $strLen + " '" + $s + "'")
    $entrySize = 2 + $strLen + 1  # u16 + bytes + null
    # align to 4
    $entrySize = [Math]::Ceiling($entrySize / 4) * 4
    $cur += $entrySize
}
Write-Host ""

Write-Host "=== first few BNTX strings to understand layout ==="
# String at filename_offset = 0x822
$fnOff = 0x822
Write-Host ("  filename @0x" + (H8 $fnOff) + ":")
$strLen = ReadU16 $fnOff
Write-Host ("    len=" + $strLen + " '" + (ReadAscii ($fnOff+2) $strLen) + "'")
Write-Host ""

Write-Host "=== _DIC section first 3 entries ==="
$dicOff = $dictOff
Write-Host ("  magic    = " + (ReadAscii $dicOff 4))
$bnsdsCount = ReadU32 ($dicOff + 4)
Write-Host ("  count_or_ref = 0x" + (H8 $bnsdsCount) + " (= " + $bnsdsCount + ")")
# Each entry is presumably 16 bytes: 4 ref_bit + 2 left + 2 right + 8 name_ptr
Write-Host "  root + first 2 entries (16 bytes each):"
for ($i = 0; $i -lt 3; $i++) {
    $ePos = $dicOff + 4 + ($i * 16)
    $refBit = ReadU32 $ePos
    $left = ReadU16 ($ePos + 4)
    $right = ReadU16 ($ePos + 6)
    $namePtr = ReadU64 ($ePos + 8)
    Write-Host ("    [$i] @0x" + (H8 $ePos) + ":  ref_bit=" + $refBit + "  L=" + $left + "  R=" + $right + "  name_ptr=0x" + (H8 $namePtr))
}
Write-Host ""

Write-Host "=== Dict size calculation ==="
$expectedDictBytes = 4 + 4 + (16 * ($count + 1))  # magic + count + entries
$dicEnd = $dicOff + $expectedDictBytes
Write-Host ("  expected dict end = 0x" + (H8 $dicEnd))
Write-Host ("  bytes between dict_end and first_BRTI:")
Write-Host ("    dict_end          = 0x" + (H8 $dicEnd))
$brtiStart = ReadU64 $infoPtrsOff
Write-Host ("    first BRTI ptr    = 0x" + (H8 $brtiStart))
$gap = $brtiStart - $dicEnd
Write-Host ("    gap = " + $gap + " bytes")
if ($gap -gt 0 -and $gap -le 100) {
    Write-Host ("    bytes: " + (HexAt $dicEnd $gap))
}
Write-Host ""

Write-Host "=== BRTI block structure ==="
# Each BRTI is 0xA0 + 0x200 (per jam1garner SIZE_OF_BRTI = 0xA0, then 0x200 = padding)
Write-Host ("  first BRTI @0x" + (H8 $brtiStart))
Write-Host ("  size = 0xA0 (header)")
Write-Host ("  bytes 0x80..0xA0 (last fields = name_ptr, parent_ptr, texture_offset_ptr):")
Write-Host ("    " + (HexAt ($brtiStart + 0x80) 0x20))
$nameAddr = ReadU64 ($brtiStart + 0x80)  # actually not 0x80; let me re-check offset
# From jam1garner: name_addr is at offset 0x60 in BRTI struct
$nameAddr2 = ReadU64 ($brtiStart + 0x60)
Write-Host ("  name_addr (@+0x60)         = 0x" + (H8 $nameAddr2))
$parentAddr = ReadU64 ($brtiStart + 0x68)
Write-Host ("  parent_addr (@+0x68)        = 0x" + (H8 $parentAddr))
$textureOff = ReadU64 ($brtiStart + 0x70)
Write-Host ("  texture_addr (@+0x70)       = 0x" + (H8 $textureOff))
Write-Host ""

# Distance between consecutive BRTIs
$brti1 = ReadU64 $infoPtrsOff
$brti2 = ReadU64 ($infoPtrsOff + 8)
Write-Host ("  BRTI[0] = 0x" + (H8 $brti1) + ", BRTI[1] = 0x" + (H8 $brti2))
Write-Host ("  spacing  = 0x" + (H8 ($brti2 - $brti1)) + " bytes (= " + ($brti2 - $brti1) + ")")
Write-Host ""

Write-Host "=== BRTD section (data block) ==="
$brtdOff = $dataBlkPtr
Write-Host ("  BRTD @0x" + (H8 $brtdOff))
Write-Host ("  magic       = " + (ReadAscii $brtdOff 4))
Write-Host ("  field@+4    = 0x" + (H8 (ReadU32 ($brtdOff+4))))
Write-Host ("  block_size  = 0x" + (H16 (ReadU64 ($brtdOff+8))))
Write-Host ("  data starts at 0x" + (H8 ($brtdOff + 0x10)))
Write-Host ("  texture[0].texture_offset = 0x" + (H8 $textureOff) + " (vs brtd_data_start = 0x" + (H8 ($brtdOff + 0x10)) + ")")
Write-Host ""

Write-Host "=== _RLT relocation table at 0x$(H8 $relocOff) ==="
Write-Host ("  magic        = " + (ReadAscii $relocOff 4))
Write-Host ("  this_offset  = 0x" + (H8 (ReadU32 ($relocOff+4))))
$rltSecCount = ReadU32 ($relocOff+8)
Write-Host ("  section_cnt  = " + $rltSecCount)
Write-Host ("  padding      = 0x" + (H8 (ReadU32 ($relocOff+0xC))))
Write-Host ""
Write-Host "  Relocation sections (24 bytes each):"
for ($i = 0; $i -lt [Math]::Min($rltSecCount, 4); $i++) {
    $sPos = $relocOff + 0x10 + ($i * 24)
    $ptr = ReadU64 $sPos
    $pos = ReadU32 ($sPos + 8)
    $size = ReadU32 ($sPos + 12)
    $idx = ReadU32 ($sPos + 16)
    $cnt = ReadU32 ($sPos + 20)
    Write-Host ("    [$i] ptr=0x" + (H8 $ptr) + " pos=0x" + (H8 $pos) + " size=0x" + (H8 $size) + " idx=" + $idx + " cnt=" + $cnt)
}
$rltEntriesOff = $relocOff + 0x10 + ($rltSecCount * 24)
Write-Host ("  rlt_entries_off = 0x" + (H8 $rltEntriesOff))
$totalEntryCount = 0
for ($i = 0; $i -lt $rltSecCount; $i++) {
    $totalEntryCount += ReadU32 ($relocOff + 0x10 + ($i * 24) + 20)
}
Write-Host ("  total entry count = " + $totalEntryCount)
Write-Host ("  rlt total bytes = " + ($len - $relocOff))
Write-Host ("  computed:  16 (header) + " + ($rltSecCount * 24) + " (sections) + " + ($totalEntryCount * 8) + " (entries) = " + (16 + $rltSecCount * 24 + $totalEntryCount * 8))
