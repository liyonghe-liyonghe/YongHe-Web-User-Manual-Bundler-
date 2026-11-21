Add-Type -AssemblyName System.Drawing

$iconsDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cyan = [System.Drawing.Color]::FromArgb(0xFF, 0x4e, 0xc9, 0xc9)

function New-SolidIcon {
    param([int]$Size)
    $bmp = New-Object System.Drawing.Bitmap($Size, $Size)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear($cyan)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Dispose()
    return $bmp
}

# 1) 32x32.png
$bmp32 = New-SolidIcon -Size 32
$bmp32.Save((Join-Path $iconsDir "32x32.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp32.Dispose()
Write-Host "Generated 32x32.png"

# 2) 128x128.png
$bmp128 = New-SolidIcon -Size 128
$bmp128.Save((Join-Path $iconsDir "128x128.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp128.Dispose()
Write-Host "Generated 128x128.png"

# 3) 128x128@2x.png -> 256x256
$bmp256 = New-SolidIcon -Size 256
$bmp256.Save((Join-Path $iconsDir "128x128@2x.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp256.Dispose()
Write-Host "Generated 128x128@2x.png (actual 256x256)"

# 4) icon.ico - 包含 16,32,48,256 尺寸
$sizes = @(16, 32, 48, 256)
$tempPngs = @()
foreach ($sz in $sizes) {
    $bmp = New-SolidIcon -Size $sz
    $tempPath = Join-Path $iconsDir "tmp_$sz.png"
    $bmp.Save($tempPath, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $tempPngs += $tempPath
}

# 使用 System.Drawing.Icon 保存 icon.ico
# Icon 需要从 Bitmap 构造，但可以通过 MemoryStream 用 png 数据创建多尺寸 ico
$icoPath = Join-Path $iconsDir "icon.ico"

# 方法: 读取每个 png，用 Icon.FromHandle + Handle 方式不行，改用二进制构造
# 最简单方式：用最大的 png 转换为 .ico (Windows ICO 格式支持 PNG 压缩)
$bigPng = Join-Path $iconsDir "tmp_256.png"
$pngBytes = [System.IO.File]::ReadAllBytes($bigPng)

# 写 ICO header + directory entry + PNG data
$fs = [System.IO.File]::Open($icoPath, [System.IO.FileMode]::Create)
$bw = New-Object System.IO.BinaryWriter($fs)

# ICO Header: reserved(2), type(2=1), count(2)
$bw.Write([UInt16]0)
$bw.Write([UInt16]1)
$bw.Write([UInt16]1)  # 1 entry (PNG inside ICO is flexible)

# ICO Directory Entry: width(1), height(1), colors(1), reserved(1), planes(2), bpp(2), size(4), offset(4)
$bw.Write([Byte]0)   # width 0 means 256
$bw.Write([Byte]0)   # height 0 means 256
$bw.Write([Byte]0)   # colors
$bw.Write([Byte]0)   # reserved
$bw.Write([UInt16]1) # planes
$bw.Write([UInt16]32) # bpp
$bw.Write([UInt32]$pngBytes.Length)
$bw.Write([UInt32]22) # offset = header(6) + entry(16) = 22

# PNG data
$bw.Write($pngBytes)
$bw.Close()
$fs.Close()

Write-Host "Generated icon.ico"

# 清理临时 png
foreach ($t in $tempPngs) { Remove-Item $t -Force }

# 5) icon.icns - 创建空文件让 tauri build 跳过（Windows 构建不需要 macOS 的 icon）
$icnsPath = Join-Path $iconsDir "icon.icns"
# 先试着把 256x256 png 复制过去做占位（即使格式不对，tauri 在 Windows 构建时也不需要）
Copy-Item (Join-Path $iconsDir "128x128@2x.png") $icnsPath -Force
Write-Host "Created icon.icns (placeholder copy of PNG, will be skipped on Windows build)"

Write-Host "`nAll files generated:"
Get-ChildItem $iconsDir | Format-Table Name, Length
