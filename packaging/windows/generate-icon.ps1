# The geometry matches assets/till.svg; render each size separately for crisp scaling.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$sizes = @(16, 20, 24, 32, 40, 48, 64, 128, 256)
$images = @()
foreach ($size in $sizes) {
    $bitmap = [System.Drawing.Bitmap]::new($size, $size)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $path = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $green = [System.Drawing.SolidBrush]::new([System.Drawing.ColorTranslator]::FromHtml('#265746'))
    $cream = [System.Drawing.SolidBrush]::new([System.Drawing.ColorTranslator]::FromHtml('#fff9ed'))
    $accent = [System.Drawing.SolidBrush]::new([System.Drawing.ColorTranslator]::FromHtml('#c6ddce'))
    $stream = [System.IO.MemoryStream]::new()
    try {
        $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $graphics.ScaleTransform($size / 256.0, $size / 256.0)
        $path.AddArc(16, 16, 96, 96, 180, 90)
        $path.AddArc(144, 16, 96, 96, 270, 90)
        $path.AddArc(144, 144, 96, 96, 0, 90)
        $path.AddArc(16, 144, 96, 96, 90, 90)
        $path.CloseFigure()
        $graphics.FillPath($green, $path)
        $graphics.FillRectangle($cream, 64, 64, 128, 32)
        $graphics.FillRectangle($cream, 112, 96, 32, 96)
        $graphics.FillRectangle($accent, 64, 208, 128, 8)
        $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
        $images += ,($stream.ToArray())
        if ($size -eq 256) {
            $bitmap.Save((Join-Path $root 'assets/till.png'), [System.Drawing.Imaging.ImageFormat]::Png)
        }
    }
    finally {
        $stream.Dispose(); $accent.Dispose(); $cream.Dispose(); $green.Dispose()
        $path.Dispose(); $graphics.Dispose(); $bitmap.Dispose()
    }
}
$file = [System.IO.File]::Create((Join-Path $root 'assets/till.ico'))
$writer = [System.IO.BinaryWriter]::new($file)
try {
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($index = 0; $index -lt $sizes.Count; $index++) {
        $dimension = if ($sizes[$index] -eq 256) { 0 } else { $sizes[$index] }
        $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
        $writer.Write([byte]0); $writer.Write([byte]0)
        $writer.Write([uint16]1); $writer.Write([uint16]32)
        $writer.Write([uint32]$images[$index].Length); $writer.Write([uint32]$offset)
        $offset += $images[$index].Length
    }
    foreach ($bytes in $images) { $writer.Write([byte[]]$bytes) }
}
finally { $writer.Dispose(); $file.Dispose() }
