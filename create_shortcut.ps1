
# AI Desktop Pet - Build production app and create desktop shortcut with cute icon

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$BuildExe    = Join-Path $ProjectRoot "src-tauri\target\release\ai-desktop-pet.exe"
$StableDir   = Join-Path $ProjectRoot "desktop-release"
$AppExe      = Join-Path $StableDir "AI Desktop Pet.exe"
$IconDir     = Join-Path $ProjectRoot "src-tauri\icons"
$IconPath    = Join-Path $IconDir "pet_shortcut.ico"
$Desktop     = [Environment]::GetFolderPath("Desktop")
$ShortcutPath = Join-Path $Desktop "AI-Pet.lnk"

Write-Host "Generating cute pet icon..."

Add-Type -TypeDefinition @"
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.IO;

public class IconMaker {
    public static void MakeIco(string outPath) {
        int size = 256;
        Bitmap bmp = new Bitmap(size, size, PixelFormat.Format32bppArgb);
        Graphics g = Graphics.FromImage(bmp);
        g.SmoothingMode      = SmoothingMode.AntiAlias;
        g.PixelOffsetMode    = PixelOffsetMode.HighQuality;
        g.CompositingQuality = CompositingQuality.HighQuality;
        g.Clear(Color.Transparent);

        // --- Background circle (cozy soft sky-indigo gradient) ---
        using (GraphicsPath bgPath = new GraphicsPath()) {
            bgPath.AddEllipse(8, 8, 240, 240);
            using (PathGradientBrush pgb = new PathGradientBrush(bgPath)) {
                pgb.CenterColor = Color.FromArgb(255, 129, 140, 248);
                pgb.SurroundColors = new Color[] { Color.FromArgb(255, 67, 56, 202) };
                g.FillPath(pgb, bgPath);
            }
        }

        // --- Cat Ears (rounded cute kitty ears) ---
        PointF[] leftEar = { new PointF(62, 100), new PointF(74, 38), new PointF(112, 78) };
        PointF[] rightEar = { new PointF(194, 100), new PointF(182, 38), new PointF(144, 78) };
        using (SolidBrush earBrush = new SolidBrush(Color.FromArgb(255, 255, 255, 255))) {
            g.FillPolygon(earBrush, leftEar);
            g.FillPolygon(earBrush, rightEar);
        }
        using (Pen earPen = new Pen(Color.FromArgb(255, 51, 65, 85), 5.5f)) {
            earPen.LineJoin = LineJoin.Round;
            g.DrawPolygon(earPen, leftEar);
            g.DrawPolygon(earPen, rightEar);
        }
        // Inner ears (sweet sakura pink)
        PointF[] leftInner = { new PointF(70, 92), new PointF(78, 52), new PointF(104, 78) };
        PointF[] rightInner = { new PointF(186, 92), new PointF(178, 52), new PointF(152, 78) };
        using (SolidBrush inEarBrush = new SolidBrush(Color.FromArgb(255, 251, 207, 232))) {
            g.FillPolygon(inEarBrush, leftInner);
            g.FillPolygon(inEarBrush, rightInner);
        }

        // --- Cat Body & Head (Plump marshmallow white) ---
        using (GraphicsPath bodyPath = new GraphicsPath()) {
            bodyPath.AddEllipse(52, 60, 152, 140);
            using (SolidBrush whiteB = new SolidBrush(Color.FromArgb(255, 255, 255, 255)))
                g.FillPath(whiteB, bodyPath);
            using (Pen bodyPen = new Pen(Color.FromArgb(255, 51, 65, 85), 5.5f))
                g.DrawPath(bodyPen, bodyPath);
        }

        // --- Red Ribbon Collar with Gold Bell ---
        using (Pen collar = new Pen(Color.FromArgb(255, 239, 68, 68), 7f)) {
            collar.StartCap = LineCap.Round;
            collar.EndCap = LineCap.Round;
            g.DrawArc(collar, 88, 148, 80, 28, 20, 140);
        }
        // Gold Bell
        using (SolidBrush bell = new SolidBrush(Color.FromArgb(255, 251, 191, 36))) {
            g.FillEllipse(bell, 120, 166, 16, 16);
        }
        using (Pen bellPen = new Pen(Color.FromArgb(255, 180, 83, 9), 1.5f)) {
            g.DrawEllipse(bellPen, 120, 166, 16, 16);
        }

        // --- Cheeks (strawberry blush) ---
        using (SolidBrush cheek = new SolidBrush(Color.FromArgb(170, 253, 164, 175))) {
            g.FillEllipse(cheek, 68, 116, 28, 18);
            g.FillEllipse(cheek, 160, 116, 28, 18);
        }

        // --- Eyes (Big sparkling anime eyes) ---
        using (SolidBrush eb = new SolidBrush(Color.FromArgb(255, 30, 41, 59))) {
            g.FillEllipse(eb, 88, 96, 22, 28);
            g.FillEllipse(eb, 146, 96, 22, 28);
        }
        // Big shine highlight
        using (SolidBrush es1 = new SolidBrush(Color.FromArgb(255, 255, 255, 255))) {
            g.FillEllipse(es1, 98, 98, 9, 10);
            g.FillEllipse(es1, 156, 98, 9, 10);
            g.FillEllipse(es1, 91, 112, 5, 5);
            g.FillEllipse(es1, 149, 112, 5, 5);
        }

        // --- Whiskers ---
        using (Pen whisker = new Pen(Color.FromArgb(200, 148, 163, 184), 2.5f)) {
            whisker.StartCap = LineCap.Round;
            whisker.EndCap = LineCap.Round;
            g.DrawLine(whisker, 72, 122, 38, 118);
            g.DrawLine(whisker, 72, 128, 36, 130);
            g.DrawLine(whisker, 184, 122, 218, 118);
            g.DrawLine(whisker, 184, 128, 220, 130);
        }

        // --- Nose (Tiny pink) ---
        using (SolidBrush noseB = new SolidBrush(Color.FromArgb(255, 244, 114, 182))) {
            g.FillEllipse(noseB, 124, 118, 8, 6);
        }

        // --- Mouth (:3) ---
        using (Pen mouth = new Pen(Color.FromArgb(255, 51, 65, 85), 3.5f)) {
            mouth.StartCap = LineCap.Round;
            mouth.EndCap = LineCap.Round;
            g.DrawArc(mouth, 113, 120, 15, 12, 10, 155);
            g.DrawArc(mouth, 128, 120, 15, 12, 15, 160);
        }

        // --- Cozy Wooden Desk in Foreground ---
        using (GraphicsPath deskPath = new GraphicsPath()) {
            deskPath.AddArc(24, 172, 20, 20, 180, 90);
            deskPath.AddArc(212, 172, 20, 20, 270, 90);
            deskPath.AddLine(232, 224, 24, 224);
            deskPath.CloseFigure();
            using (LinearGradientBrush db = new LinearGradientBrush(new Point(128, 172), new Point(128, 224), Color.FromArgb(255, 254, 243, 199), Color.FromArgb(255, 245, 158, 11))) {
                g.FillPath(db, deskPath);
            }
            using (Pen dp = new Pen(Color.FromArgb(255, 217, 119, 6), 4f)) {
                g.DrawPath(dp, deskPath);
            }
        }

        // --- Miniature Mechanical Keyboard ---
        using (GraphicsPath kbPath = new GraphicsPath()) {
            kbPath.AddArc(54, 182, 8, 8, 180, 90);
            kbPath.AddArc(194, 182, 8, 8, 270, 90);
            kbPath.AddArc(194, 208, 8, 8, 0, 90);
            kbPath.AddArc(54, 208, 8, 8, 90, 90);
            kbPath.CloseFigure();
            using (SolidBrush kbB = new SolidBrush(Color.FromArgb(255, 241, 245, 249)))
                g.FillPath(kbB, kbPath);
            using (Pen kbp = new Pen(Color.FromArgb(255, 148, 163, 184), 2f))
                g.DrawPath(kbp, kbPath);
        }
        // Pastel Keycaps
        Color[] keyCols = { Color.FromArgb(255, 186, 230, 253), Color.FromArgb(255, 187, 247, 208), Color.FromArgb(255, 254, 240, 138), Color.FromArgb(255, 251, 207, 232), Color.FromArgb(255, 233, 213, 255) };
        for (int i = 0; i < 5; i++) {
            using (SolidBrush kc = new SolidBrush(keyCols[i])) {
                g.FillRectangle(kc, 64 + i * 26, 186, 20, 10);
            }
        }
        for (int i = 0; i < 4; i++) {
            using (SolidBrush kc = new SolidBrush(keyCols[(i + 2) % 5])) {
                g.FillRectangle(kc, 68 + i * 32, 200, 24, 10);
            }
        }

        // --- Two Bongo Paws Tapping Keyboard ---
        using (SolidBrush pawBrush = new SolidBrush(Color.FromArgb(255, 255, 255, 255))) {
            g.FillEllipse(pawBrush, 82, 184, 28, 22);
            g.FillEllipse(pawBrush, 146, 184, 28, 22);
        }
        using (Pen pawPen = new Pen(Color.FromArgb(255, 51, 65, 85), 4f)) {
            g.DrawEllipse(pawPen, 82, 184, 28, 22);
            g.DrawEllipse(pawPen, 146, 184, 28, 22);
        }

        // --- Rim shine ---
        using (SolidBrush sheen = new SolidBrush(Color.FromArgb(35, 255, 255, 255))) {
            g.FillEllipse(sheen, 40, 20, 120, 70);
        }

        g.Dispose();

        // --- Write multi-size ICO ---
        int[] sizes = { 256, 64, 48, 32, 16 };
        var pngStreams = new MemoryStream[sizes.Length];
        for (int i = 0; i < sizes.Length; i++) {
            Bitmap rb = new Bitmap(bmp, new Size(sizes[i], sizes[i]));
            pngStreams[i] = new MemoryStream();
            rb.Save(pngStreams[i], ImageFormat.Png);
            pngStreams[i].Position = 0;
            rb.Dispose();
        }

        using (MemoryStream ms = new MemoryStream()) {
            ms.Write(new byte[] { 0, 0, 1, 0 }, 0, 4);
            ms.Write(BitConverter.GetBytes((short)sizes.Length), 0, 2);

            int offset = 6 + 16 * sizes.Length;
            for (int i = 0; i < sizes.Length; i++) {
                int w = sizes[i] >= 256 ? 0 : sizes[i];
                ms.WriteByte((byte)w);
                ms.WriteByte((byte)w);
                ms.WriteByte(0);
                ms.WriteByte(0);
                ms.Write(new byte[] { 1, 0 }, 0, 2);
                ms.Write(new byte[] { 32, 0 }, 0, 2);
                ms.Write(BitConverter.GetBytes((int)pngStreams[i].Length), 0, 4);
                ms.Write(BitConverter.GetBytes(offset), 0, 4);
                offset += (int)pngStreams[i].Length;
            }
            for (int i = 0; i < sizes.Length; i++) {
                byte[] data = pngStreams[i].ToArray();
                ms.Write(data, 0, data.Length);
                pngStreams[i].Dispose();
            }
            File.WriteAllBytes(outPath, ms.ToArray());
        }
        bmp.Dispose();
    }
}
"@ -ReferencedAssemblies "System.Drawing"

[IconMaker]::MakeIco($IconPath)
Write-Host "Icon created: $IconPath"

Write-Host "Building production app..."
Push-Location $ProjectRoot
try {
    & npm run tauri build
    if ($LASTEXITCODE -ne 0) {
        throw "Production build failed with exit code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}

if (!(Test-Path -LiteralPath $BuildExe)) {
    throw "Build output not found: $BuildExe"
}

$BuildText = [System.Text.Encoding]::UTF8.GetString([System.IO.File]::ReadAllBytes($BuildExe))
if (!$BuildText.Contains("/assets/index-")) {
    throw "Production asset marker was not found in $BuildExe. The shortcut would still depend on the dev server."
}

New-Item -ItemType Directory -Force -Path $StableDir | Out-Null
Copy-Item -LiteralPath $BuildExe -Destination $AppExe -Force
Write-Host "Stable app copied to: $AppExe"

# Create shortcut
$WshShell  = New-Object -ComObject WScript.Shell
$Shortcut  = $WshShell.CreateShortcut($ShortcutPath)
$Shortcut.TargetPath       = $AppExe
$Shortcut.IconLocation     = "$IconPath,0"
$Shortcut.WorkingDirectory = $StableDir
$Shortcut.Description      = "AI Desktop Pet - Your cute AI companion"
$Shortcut.WindowStyle      = 1
$Shortcut.Save()

Write-Host "=== ALL DONE ==="
Write-Host "Shortcut: $ShortcutPath"
