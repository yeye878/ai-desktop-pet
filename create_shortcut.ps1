
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

        // --- Background circle (purple gradient) ---
        using (GraphicsPath bgPath = new GraphicsPath()) {
            bgPath.AddEllipse(8, 8, 240, 240);
            using (PathGradientBrush pgb = new PathGradientBrush(bgPath)) {
                pgb.CenterColor = Color.FromArgb(255, 192, 132, 252);
                pgb.SurroundColors = new Color[] { Color.FromArgb(255, 109, 40, 217) };
                g.FillPath(pgb, bgPath);
            }
        }

        // --- Ears ---
        using (SolidBrush ear = new SolidBrush(Color.FromArgb(255, 216, 180, 254))) {
            g.FillEllipse(ear, 56, 76, 52, 40);
            g.FillEllipse(ear, 148, 76, 52, 40);
        }
        using (SolidBrush innerEar = new SolidBrush(Color.FromArgb(255, 245, 208, 254))) {
            g.FillEllipse(innerEar, 64, 82, 28, 22);
            g.FillEllipse(innerEar, 164, 82, 28, 22);
        }

        // --- Body ---
        using (GraphicsPath bodyPath = new GraphicsPath()) {
            bodyPath.AddEllipse(56, 88, 144, 136);
            using (PathGradientBrush pgb = new PathGradientBrush(bodyPath)) {
                pgb.CenterPoint = new PointF(100, 110);
                pgb.CenterColor = Color.FromArgb(255, 253, 232, 255);
                pgb.SurroundColors = new Color[] { Color.FromArgb(255, 216, 180, 254) };
                g.FillPath(pgb, bodyPath);
            }
        }

        // --- Face ---
        using (SolidBrush fb = new SolidBrush(Color.FromArgb(255, 255, 248, 255)))
            g.FillEllipse(fb, 72, 90, 112, 104);

        // --- Eyes outer ---
        using (SolidBrush eb = new SolidBrush(Color.FromArgb(255, 30, 27, 75))) {
            g.FillEllipse(eb, 92, 112, 30, 34);
            g.FillEllipse(eb, 134, 112, 30, 34);
        }
        // --- Eyes iris ---
        using (SolidBrush ep = new SolidBrush(Color.FromArgb(255, 109, 40, 217))) {
            g.FillEllipse(ep, 98, 118, 18, 22);
            g.FillEllipse(ep, 140, 118, 18, 22);
        }
        // --- Eye shine ---
        using (SolidBrush es = new SolidBrush(Color.FromArgb(230, 255, 255, 255))) {
            g.FillEllipse(es, 101, 114, 10, 10);
            g.FillEllipse(es, 143, 114, 10, 10);
            g.FillEllipse(es, 96, 126, 5, 5);
            g.FillEllipse(es, 138, 126, 5, 5);
        }

        // --- Cheeks ---
        using (SolidBrush ck = new SolidBrush(Color.FromArgb(110, 249, 168, 212))) {
            g.FillEllipse(ck, 76, 140, 32, 20);
            g.FillEllipse(ck, 148, 140, 32, 20);
        }

        // --- Nose ---
        using (SolidBrush ns = new SolidBrush(Color.FromArgb(180, 192, 132, 252)))
            g.FillEllipse(ns, 120, 139, 16, 11);

        // --- Smile ---
        using (Pen sm = new Pen(Color.FromArgb(255, 109, 40, 217), 4f)) {
            sm.StartCap = LineCap.Round;
            sm.EndCap   = LineCap.Round;
            g.DrawArc(sm, 104, 148, 48, 22, 0, 180);
        }

        // --- Antenna stick ---
        using (Pen ant = new Pen(Color.FromArgb(255, 192, 132, 252), 4.5f))
            g.DrawLine(ant, 128, 90, 128, 56);

        // --- Antenna star ---
        PointF[] star = {
            new PointF(128, 28),
            new PointF(133, 42),
            new PointF(148, 42),
            new PointF(136, 50),
            new PointF(140, 64),
            new PointF(128, 56),
            new PointF(116, 64),
            new PointF(120, 50),
            new PointF(108, 42),
            new PointF(123, 42)
        };
        using (SolidBrush sb2 = new SolidBrush(Color.FromArgb(255, 253, 224, 71)))
            g.FillPolygon(sb2, star);
        using (Pen sp = new Pen(Color.FromArgb(255, 251, 191, 36), 1.5f))
            g.DrawPolygon(sp, star);

        // --- Heart ---
        GraphicsPath heart = new GraphicsPath();
        float hx = 114, hy = 176, hw = 28;
        heart.AddBezier(hx + hw/2, hy + 8,   hx + hw/2, hy,     hx + hw, hy,     hx + hw, hy + 10);
        heart.AddBezier(hx + hw,   hy + 10,   hx + hw,   hy + 20, hx + hw/2, hy + 26, hx + hw/2, hy + 30);
        heart.AddBezier(hx + hw/2, hy + 30,   hx + hw/2, hy + 26, hx,       hy + 20, hx,         hy + 10);
        heart.AddBezier(hx,        hy + 10,   hx,        hy,      hx + hw/2, hy,     hx + hw/2,  hy + 8);
        using (SolidBrush hb = new SolidBrush(Color.FromArgb(255, 244, 114, 182)))
            g.FillPath(hb, heart);
        using (Pen hp = new Pen(Color.FromArgb(255, 236, 72, 153), 1.5f))
            g.DrawPath(hp, heart);

        // --- Paws ---
        using (SolidBrush pw = new SolidBrush(Color.FromArgb(255, 233, 213, 255))) {
            g.FillEllipse(pw, 60, 180, 44, 28);
            g.FillEllipse(pw, 152, 180, 44, 28);
        }
        using (Pen pp = new Pen(Color.FromArgb(200, 192, 132, 252), 1.8f)) {
            g.DrawEllipse(pp, 60, 180, 44, 28);
            g.DrawEllipse(pp, 152, 180, 44, 28);
        }
        // Paw toe dots
        using (SolidBrush pd = new SolidBrush(Color.FromArgb(160, 167, 100, 220))) {
            g.FillEllipse(pd, 68, 178, 8, 8);
            g.FillEllipse(pd, 78, 174, 8, 8);
            g.FillEllipse(pd, 88, 178, 8, 8);
            g.FillEllipse(pd, 160, 178, 8, 8);
            g.FillEllipse(pd, 170, 174, 8, 8);
            g.FillEllipse(pd, 180, 178, 8, 8);
        }

        // --- Sheen highlight ---
        using (SolidBrush sh = new SolidBrush(Color.FromArgb(28, 255, 255, 255)))
            g.FillEllipse(sh, 40, 20, 120, 70);

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
