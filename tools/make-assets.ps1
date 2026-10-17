# Generates the embedded target assets for lab-core from a source portrait.
#
# Run once by a maintainer and commit the output. The generated PNGs are shipped
# in the binary via include_bytes!, so nothing at build or run time reads the
# source portrait - this script exists so the provenance of those bytes is
# documented and reproducible instead of being a silent rename.
#
#   powershell -File tools/make-assets.ps1 -Source path\to\portrait.jpg
#
# Outputs, both 256x256, overwriting crates/lab-core/assets:
#   lebron-target-256.png    the portrait, cropped to the head and resized
#   lebron-weights-256.png   per-cell importance mask, single channel in RGB
#
# The mask is derived from the target, not copied from upstream. It is
# deliberately single-channel-in-RGB so that lab-core's red-channel read (see
# crates/lab-core/src/target.rs) sees exactly the value written here.

param(
  [Parameter(Mandatory = $true)][string]$Source,
  [int]$Side = 256,
  # Square crop around the head, in source pixels.
  [int]$CropX = 45,
  [int]$CropY = 25,
  [int]$CropSide = 470
)

Add-Type -AssemblyName System.Drawing

$here = Split-Path -Parent $PSScriptRoot
$assets = Join-Path $here 'crates\lab-core\assets'

if (-not (Test-Path $Source)) { throw "source not found: $Source" }
if (-not (Test-Path $assets)) { throw "assets dir not found: $assets" }

$src = [System.Drawing.Bitmap]::FromFile((Resolve-Path $Source).Path)
"source: $($src.Width)x$($src.Height)"

if (($CropX + $CropSide) -gt $src.Width -or ($CropY + $CropSide) -gt $src.Height) {
  throw "crop $CropX,$CropY,$CropSide does not fit inside $($src.Width)x$($src.Height)"
}

# Crop and resample in one step: the destination rectangle is the whole output,
# the source rectangle is the chosen crop.
$face = New-Object System.Drawing.Bitmap $Side, $Side, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
$g = [System.Drawing.Graphics]::FromImage($face)
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
$dest = New-Object System.Drawing.Rectangle 0, 0, $Side, $Side
$srcRect = New-Object System.Drawing.Rectangle $CropX, $CropY, $CropSide, $CropSide
$g.DrawImage($src, $dest, $srcRect, [System.Drawing.GraphicsUnit]::Pixel)
$g.Dispose()
$src.Dispose()

$targetPath = Join-Path $assets 'lebron-target-256.png'
$face.Save($targetPath, [System.Drawing.Imaging.ImageFormat]::Png)
"wrote $targetPath"

# The importance mask.
#
# Two factors, both soft by construction, so the mask has no hard cutout:
#   subjectness  distance from the background colour sampled at the source
#                corner, ramped over a wide band rather than thresholded
#   centrality   a gentle radial falloff, so the face outranks the frame edges
#
# The result is intentionally not normalized to 0..1: lab-core reads this as
# 0..255 and the solver's whole notion of "important" is relative to that range.
$bg = $face.GetPixel(0, 0)
"background sample for the mask: R$($bg.R) G$($bg.G) B$($bg.B)"

$half = ($Side - 1) / 2.0
$maxR = [math]::Sqrt(2 * $half * $half)
$mask = New-Object System.Drawing.Bitmap $Side, $Side, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)

$mid = 0
for ($y = 0; $y -lt $Side; $y++) {
  for ($x = 0; $x -lt $Side; $x++) {
    $c = $face.GetPixel($x, $y)
    $d = [math]::Abs($c.R - $bg.R) + [math]::Abs($c.G - $bg.G) + [math]::Abs($c.B - $bg.B)

    # 0 at the background, 1 well away from it, smooth ramp between.
    $subj = ($d - 30.0) / 120.0
    if ($subj -lt 0) { $subj = 0 } elseif ($subj -gt 1) { $subj = 1 }

    $dx = $x - $half
    $dy = $y - $half
    $r = [math]::Sqrt($dx * $dx + $dy * $dy) / $maxR
    $rad = 1.0 - [math]::Pow($r, 1.2)
    # Floor the falloff so the frame edges still carry some weight: a target
    # whose corners cost nothing makes the solver ignore the outer rows.
    $rad = 0.15 + 0.85 * $rad

    $v = [int](255.0 * $rad * (0.25 + 0.75 * $subj))
    if ($v -lt 0) { $v = 0 } elseif ($v -gt 255) { $v = 255 }
    if ($v -gt 20 -and $v -lt 235) { $mid++ }

    $mask.SetPixel($x, $y, [System.Drawing.Color]::FromArgb($v, $v, $v))
  }
}

$maskPath = Join-Path $assets 'lebron-weights-256.png'
$mask.Save($maskPath, [System.Drawing.Imaging.ImageFormat]::Png)
"wrote $maskPath"
"mask mid-range pixels (20..235): $mid of $($Side * $Side)"

$mask.Dispose()
$face.Dispose()