//! A font's `trak` table: the letter spacing its designer set for each point
//! size. Core Text applies it on macOS, which is why SF Pro sets tighter
//! there at text sizes; DirectWrite ignores it, so we apply it ourselves.
//!
//! https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6trak.html

/// The normal track (track value 0) of a `trak` table: font units of extra
/// advance per glyph, at each of a list of point sizes.
#[derive(Debug, Clone)]
pub(crate) struct Tracking {
    units_per_em: f32,
    /// (point size, font units), sorted by size.
    sizes: Vec<(f32, f32)>,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn i16_at(data: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// A 16.16 fixed-point number.
fn fixed_at(data: &[u8], at: usize) -> Option<f32> {
    Some(u32_at(data, at)? as i32 as f32 / 65536.0)
}

impl Tracking {
    /// Parses the horizontal normal track of `table`, if it has one.
    pub(crate) fn parse(table: &[u8], units_per_em: u16) -> Option<Self> {
        let horizontal = u16_at(table, 6)? as usize;
        if horizontal == 0 {
            return None;
        }
        let tracks = u16_at(table, horizontal)? as usize;
        let size_count = u16_at(table, horizontal + 2)? as usize;
        let size_table = u32_at(table, horizontal + 4)? as usize;
        let normal = (0..tracks).find_map(|i| {
            let entry = horizontal + 8 + i * 8;
            (fixed_at(table, entry)? == 0.0).then(|| u16_at(table, entry + 6)).flatten()
        })? as usize;
        let sizes = (0..size_count)
            .map(|i| Some((fixed_at(table, size_table + i * 4)?, i16_at(table, normal + i * 2)? as f32)))
            .collect::<Option<Vec<_>>>()?;
        (!sizes.is_empty() && sizes.iter().any(|&(_, v)| v != 0.0))
            .then_some(Self { units_per_em: units_per_em.max(1) as f32, sizes })
    }

    /// Extra advance per glyph at `size` (pixels, which DirectWrite treats as
    /// points), interpolated between the table's sizes and held past its ends.
    pub(crate) fn at(&self, size: f32) -> f32 {
        let s = &self.sizes;
        let units = match s.iter().position(|&(at, _)| at >= size) {
            None => s[s.len() - 1].1,
            Some(0) => s[0].1,
            Some(i) => {
                let ((a, va), (b, vb)) = (s[i - 1], s[i]);
                va + (vb - va) * (size - a) / (b - a)
            }
        };
        units / self.units_per_em * size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table with one normal track at 12pt (0) and 13pt (-12 units).
    fn table() -> Vec<u8> {
        let mut t = vec![0, 1, 0, 0, 0, 0, 0, 12, 0, 0, 0, 0];
        // Track data at 12: one track, two sizes, size table at 28.
        t.extend([0, 1, 0, 2, 0, 0, 0, 28]);
        // Track 0.0, name 256, values at 36.
        t.extend([0, 0, 0, 0, 1, 0, 0, 36]);
        // Sizes 12.0 and 13.0.
        t.extend([0, 12, 0, 0, 0, 13, 0, 0]);
        // Values 0 and -12.
        t.extend([0, 0, 0xFF, 0xF4]);
        t
    }

    #[test]
    fn interpolates_and_holds() {
        let tracking = Tracking::parse(&table(), 2048).unwrap();
        assert_eq!(tracking.at(12.0), 0.0);
        assert!((tracking.at(13.0) - (-12.0 / 2048.0 * 13.0)).abs() < 1e-6);
        assert!((tracking.at(12.5) - (-6.0 / 2048.0 * 12.5)).abs() < 1e-6);
        assert!((tracking.at(30.0) - (-12.0 / 2048.0 * 30.0)).abs() < 1e-6);
    }
}
