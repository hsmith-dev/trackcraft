//! `cargo xtask icons [--iconset DIR]`: rasterize `assets/app-icon/trackcraft.svg` into every
//! icon file the packaging needs, with no SVG tool installed.
//!
//! The icon is drawn in a deliberately small SVG subset — one vertical `<linearGradient>` and
//! `<rect>` elements with `x`, `y`, `width`, `height`, `rx` and a `#RRGGBB` or `url(#id)` fill —
//! so this file renders it exactly (anti-aliased by supersampling). Anything else in the SVG is an
//! error rather than a silently wrong icon. Outputs (all committed; builds never run this):
//!
//! - `trackcraft-1024.png` (full-bleed), `trackcraft-macos-512.png` (on Apple's 824/1024 grid,
//!   the desktop app's window/Dock icon)
//! - `hicolor/<n>x<n>/apps/<APP_ID>.png` for 16…512 and `hicolor/scalable/apps/<APP_ID>.svg`
//! - `trackcraft.ico` (16…256, PNG-compressed entries; Windows Vista and later)
//! - with `--iconset DIR`: `DIR/icon_<n>x<n>[@2x].png` for `iconutil -c icns` (macOS only;
//!   `packaging/icons.sh` does that step)

use std::path::Path;

/// Reverse-DNS application id (desktop entry, AppStream, Flatpak, macOS bundle).
pub const APP_ID: &str = "io.github.hsmith_dev.trackcraft";

/// Apple's icon grid: an 824-px body on a 1024-px canvas.
const MAC_BODY: f64 = 824.0 / 1024.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rgb(f64, f64, f64);

#[derive(Debug, Clone, PartialEq)]
enum Fill {
    Solid(Rgb),
    /// Vertical gradient over the shape's bounding box: (offset, colour) stops.
    Vertical(Vec<(f64, Rgb)>),
}

#[derive(Debug, Clone, PartialEq)]
struct RoundRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
    fill: Fill,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Icon {
    size: f64,
    rects: Vec<RoundRect>,
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let rest = tag.get(start..)?;
    rest.find('"').and_then(|end| rest.get(..end))
}

fn num(tag: &str, name: &str) -> Result<f64, String> {
    let v = attr(tag, name).ok_or_else(|| format!("missing {name}= in {tag}"))?;
    v.parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| format!("bad {name}=\"{v}\""))
}

fn hex(s: &str) -> Result<Rgb, String> {
    let h = s.strip_prefix('#').filter(|h| h.len() == 6).ok_or_else(|| format!("colour {s} is not #RRGGBB"))?;
    let n = u32::from_str_radix(h, 16).map_err(|_| format!("colour {s} is not #RRGGBB"))?;
    let c = |shift: u32| f64::from((n >> shift) & 0xFF) / 255.0;
    Ok(Rgb(c(16), c(8), c(0)))
}

/// Tags (`<…>`) of the document, comments dropped.
fn tags(svg: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(i) = rest.find('<') {
        let Some(after) = rest.get(i..) else { break };
        if after.starts_with("<!--") {
            match after.find("-->") {
                Some(end) => rest = after.get(end + 3..).unwrap_or_default(),
                None => break,
            }
            continue;
        }
        match after.find('>') {
            Some(end) => {
                out.push(after.get(..=end).unwrap_or_default());
                rest = after.get(end + 1..).unwrap_or_default();
            }
            None => break,
        }
    }
    out
}

/// Parses the icon subset described in the module docs.
pub fn parse(svg: &str) -> Result<Icon, String> {
    let mut size = None;
    let mut gradients: Vec<(String, Vec<(f64, Rgb)>)> = Vec::new();
    let mut rects = Vec::new();
    for tag in tags(svg) {
        let name = tag.trim_start_matches('<').split([' ', '>', '\n']).next().unwrap_or_default();
        let name = if name.len() > 1 { name.trim_end_matches('/') } else { name };
        match name {
            "svg" => {
                let vb = attr(tag, "viewBox").ok_or("the <svg> needs a viewBox")?;
                let parts: Vec<f64> = vb.split_whitespace().filter_map(|p| p.parse().ok()).collect();
                match parts.as_slice() {
                    [0.0, 0.0, w, h] if w == h && *w > 0.0 => size = Some(*w),
                    _ => return Err(format!("viewBox must be \"0 0 N N\", got \"{vb}\"")),
                }
            }
            "linearGradient" => {
                let id = attr(tag, "id").ok_or("a linearGradient needs an id")?;
                let vertical = [("x1", "0"), ("y1", "0"), ("x2", "0"), ("y2", "1")].iter().all(|(k, v)| attr(tag, k) == Some(*v));
                if !vertical {
                    return Err(format!("gradient {id} must be vertical (x1=0 y1=0 x2=0 y2=1)"));
                }
                gradients.push((id.to_string(), Vec::new()));
            }
            "stop" => {
                let (_, stops) = gradients.last_mut().ok_or("a <stop> outside a linearGradient")?;
                let offset = num(tag, "offset")?.clamp(0.0, 1.0);
                stops.push((offset, hex(attr(tag, "stop-color").ok_or("a <stop> needs stop-color")?)?));
            }
            "rect" => {
                let fill = match attr(tag, "fill").ok_or_else(|| format!("missing fill= in {tag}"))? {
                    f if f.starts_with("url(#") => {
                        let id = f.trim_start_matches("url(#").trim_end_matches(')');
                        let stops = gradients.iter().find(|(g, _)| g == id).map(|(_, s)| s.clone()).ok_or_else(|| format!("no gradient {id}"))?;
                        if stops.is_empty() {
                            return Err(format!("gradient {id} has no stops"));
                        }
                        Fill::Vertical(stops)
                    }
                    f => Fill::Solid(hex(f)?),
                };
                let (w, h) = (num(tag, "width")?, num(tag, "height")?);
                let r = attr(tag, "rx").map_or(Ok(0.0), |_| num(tag, "rx"))?.clamp(0.0, w.min(h) / 2.0);
                rects.push(RoundRect { x: num(tag, "x")?, y: num(tag, "y")?, w, h, r, fill });
            }
            "title" | "/title" | "defs" | "/defs" | "/linearGradient" | "/svg" | "?xml" => {}
            other => return Err(format!("unsupported SVG element <{other}> (see xtask/src/icons.rs)")),
        }
    }
    let size = size.ok_or("no <svg> element")?;
    if rects.is_empty() {
        return Err("the icon has no <rect>".into());
    }
    Ok(Icon { size, rects })
}

impl RoundRect {
    fn contains(&self, x: f64, y: f64) -> bool {
        let (x0, y0, x1, y1, r) = (self.x, self.y, self.x + self.w, self.y + self.h, self.r);
        if x < x0 || x > x1 || y < y0 || y > y1 {
            return false;
        }
        let dx = (x0 + r - x).max(x - (x1 - r)).max(0.0);
        let dy = (y0 + r - y).max(y - (y1 - r)).max(0.0);
        dx * dx + dy * dy <= r * r
    }

    fn colour(&self, y: f64) -> Rgb {
        match &self.fill {
            Fill::Solid(c) => *c,
            Fill::Vertical(stops) => {
                let t = if self.h > 0.0 { ((y - self.y) / self.h).clamp(0.0, 1.0) } else { 0.0 };
                let mut prev = stops.first().copied().unwrap_or((0.0, Rgb(0.0, 0.0, 0.0)));
                for &(o, c) in stops {
                    if t <= o {
                        let span = o - prev.0;
                        let k = if span > 0.0 { (t - prev.0) / span } else { 1.0 };
                        let (a, b) = (prev.1, c);
                        return Rgb(a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k);
                    }
                    prev = (o, c);
                }
                prev.1
            }
        }
    }
}

/// RGBA pixels of the icon at `px` square. `body` < 1 shrinks the artwork into a centred body
/// with a transparent margin (macOS grid).
pub fn render(icon: &Icon, px: u32, body: f64) -> Vec<u8> {
    let n = if px <= 64 { 8 } else { 4 };
    let span = icon.size / body;
    let offset = (span - icon.size) / 2.0;
    let unit = span / f64::from(px);
    let mut out = Vec::with_capacity((px as usize).saturating_mul(px as usize).saturating_mul(4));
    for row in 0..px {
        for col in 0..px {
            // Premultiplied accumulation of source-over composites at each sample.
            let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
            for sy in 0..n {
                for sx in 0..n {
                    let x = (f64::from(col) + (f64::from(sx) + 0.5) / f64::from(n)) * unit - offset;
                    let y = (f64::from(row) + (f64::from(sy) + 0.5) / f64::from(n)) * unit - offset;
                    let (mut pr, mut pg, mut pb, mut pa) = (0.0, 0.0, 0.0, 0.0);
                    for rect in icon.rects.iter().filter(|rect| rect.contains(x, y)) {
                        let c = rect.colour(y);
                        (pr, pg, pb, pa) = (c.0, c.1, c.2, 1.0);
                    }
                    r += pr * pa;
                    g += pg * pa;
                    b += pb * pa;
                    a += pa;
                }
            }
            let samples = f64::from(n * n);
            let alpha = a / samples;
            let un = |v: f64| if a > 0.0 { (v / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
            out.extend_from_slice(&[un(r), un(g), un(b), (alpha * 255.0).round().clamp(0.0, 255.0) as u8]);
        }
    }
    out
}

pub fn png(rgba: &[u8], px: u32) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut enc = png::Encoder::new(&mut buf, px, px);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    let mut w = enc.write_header().map_err(|e| format!("png: {e}"))?;
    w.write_image_data(rgba).map_err(|e| format!("png: {e}"))?;
    w.finish().map_err(|e| format!("png: {e}"))?;
    Ok(buf)
}

/// An ICO file holding square PNGs of at most 256 px (sizes given alongside).
pub fn ico(pngs: &[(u32, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let count = u16::try_from(pngs.len()).map_err(|_| "too many icon images")?;
    if count == 0 {
        return Err("need at least one PNG".into());
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]); // reserved, type 1 = icon
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * pngs.len();
    for (px, data) in pngs {
        let dim = match *px {
            256 => 0,
            1..=255 => *px as u8,
            other => return Err(format!("ICO images are at most 256 px, got {other}")),
        };
        let len = u32::try_from(data.len()).map_err(|_| "icon image too large")?;
        let off = u32::try_from(offset).map_err(|_| "icon too large")?;
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&off.to_le_bytes());
        offset += data.len();
    }
    for (_, data) in pngs {
        out.extend_from_slice(data);
    }
    Ok(out)
}

fn write(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, data).map_err(|e| format!("write {}: {e}", path.display()))
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let iconset = match args {
        [] => None,
        ["--iconset", dir] => Some(Path::new(dir).to_path_buf()),
        _ => return Err("usage: cargo xtask icons [--iconset DIR]".into()),
    };
    let dir = root.join("assets/app-icon");
    let svg_path = dir.join("trackcraft.svg");
    let svg = std::fs::read_to_string(&svg_path).map_err(|e| format!("read {}: {e}", svg_path.display()))?;
    let icon = parse(&svg)?;
    let full = |px: u32| png(&render(&icon, px, 1.0), px);
    let mac = |px: u32| png(&render(&icon, px, MAC_BODY), px);

    write(&dir.join("trackcraft-1024.png"), &full(1024)?)?;
    write(&dir.join("trackcraft-macos-512.png"), &mac(512)?)?;
    for px in [16, 24, 32, 48, 64, 128, 256, 512] {
        write(&dir.join(format!("hicolor/{px}x{px}/apps/{APP_ID}.png")), &full(px)?)?;
    }
    write(&dir.join(format!("hicolor/scalable/apps/{APP_ID}.svg")), svg.as_bytes())?;
    let ico_pngs = [16, 20, 24, 32, 40, 48, 64, 128, 256].into_iter().map(|px| full(px).map(|p| (px, p))).collect::<Result<Vec<_>, _>>()?;
    write(&dir.join("trackcraft.ico"), &ico(&ico_pngs)?)?;
    if let Some(set) = iconset {
        for px in [16u32, 32, 128, 256, 512] {
            write(&set.join(format!("icon_{px}x{px}.png")), &mac(px)?)?;
            write(&set.join(format!("icon_{px}x{px}@2x.png")), &mac(px * 2)?)?;
        }
        println!("icons: iconset in {}", set.display());
    }
    println!("icons: written to {}", dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed() -> Icon {
        parse(&std::fs::read_to_string(crate::root().join("assets/app-icon/trackcraft.svg")).unwrap()).unwrap()
    }

    #[test]
    fn parses_the_committed_icon() {
        let icon = committed();
        assert_eq!(icon.size, 512.0);
        assert_eq!(icon.rects.len(), 4);
        assert!(matches!(icon.rects[0].fill, Fill::Vertical(ref s) if s.len() == 2));
    }

    #[test]
    fn rejects_what_it_cannot_draw() {
        assert!(parse(r##"<svg viewBox="0 0 10 10"><circle r="1"/></svg>"##).is_err());
        assert!(parse(r##"<svg viewBox="0 0 10 20"><rect x="0" y="0" width="1" height="1" fill="#000000"/></svg>"##).is_err());
        assert!(parse(r##"<svg viewBox="0 0 10 10"><rect x="0" y="0" width="1" height="1" fill="red"/></svg>"##).is_err());
        assert!(parse(r##"<svg viewBox="0 0 10 10"><rect x="0" y="0" width="1" height="1" fill="url(#nope)"/></svg>"##).is_err());
    }

    #[test]
    fn renders_corners_transparent_and_centre_opaque() {
        let icon = committed();
        let px = 64u32;
        let rgba = render(&icon, px, 1.0);
        assert_eq!(rgba.len(), (px * px * 4) as usize);
        let at = |x: u32, y: u32| &rgba[((y * px + x) * 4) as usize..((y * px + x) * 4 + 4) as usize];
        assert_eq!(at(0, 0)[3], 0, "rounded corner is transparent");
        assert_eq!(at(32, 60)[3], 255, "tile is opaque");
        assert_eq!(at(16, 30), &[255, 255, 255, 255], "first column is white");
        // macOS variant keeps a transparent margin on every side.
        let mac = render(&icon, px, MAC_BODY);
        assert_eq!(mac[((32 * px + 1) * 4 + 3) as usize], 0);
    }

    #[test]
    fn packs_an_ico_directory() {
        let a = png(&render(&committed(), 16, 1.0), 16).unwrap();
        let ico = ico(&[(16, a.clone()), (256, a.clone())]).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(&ico[6..8], &[16, 16]);
        assert_eq!(&ico[22..24], &[0, 0], "256 px is stored as 0");
        assert_eq!(u32::from_le_bytes(ico[18..22].try_into().unwrap()), 38);
        assert!(super::ico(&[(512, a)]).is_err());
        assert!(super::ico(&[]).is_err());
    }

    #[test]
    fn committed_outputs_exist() {
        let dir = crate::root().join("assets/app-icon");
        for f in ["trackcraft.ico", "trackcraft.icns", "trackcraft-1024.png", "trackcraft-macos-512.png"] {
            assert!(dir.join(f).is_file(), "{f} missing: run packaging/icons.sh");
        }
        assert!(dir.join(format!("hicolor/256x256/apps/{APP_ID}.png")).is_file());
    }
}
