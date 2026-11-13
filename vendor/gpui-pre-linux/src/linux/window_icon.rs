// PowerCommander patch: a window's icon in the sizes panels show it at, each
// shrunk here by averaging, so the desktop need not scale one large image
// (Plasma's taskbar does that roughly).

/// The sizes besides the image's own, those smaller than it.
const SIZES: [u32; 11] = [16, 20, 22, 24, 28, 32, 40, 48, 64, 96, 128];

/// `rgba`, `side` pixels square, and its smaller sizes: (side, RGBA) each.
pub(crate) fn sizes(rgba: &[u8], side: u32) -> Vec<(u32, Vec<u8>)> {
    let mut out = vec![(side, rgba.to_vec())];
    out.extend(SIZES.iter().filter(|&&to| to < side).map(|&to| (to, shrink(rgba, side, to))));
    out
}

/// Each new pixel the average of the old ones it covers, in part at its
/// edges; colours weighted by their alpha, so clear pixels add none.
fn shrink(rgba: &[u8], side: u32, to: u32) -> Vec<u8> {
    let ratio = side as f32 / to as f32;
    let mut out = Vec::with_capacity((to * to * 4) as usize);
    // The old pixels a new one covers along an axis, with how much of each.
    let spans: Vec<Vec<(usize, f32)>> = (0..to)
        .map(|n| {
            let (start, end) = (n as f32 * ratio, (n + 1) as f32 * ratio);
            (start.floor() as usize..(end.ceil() as usize).min(side as usize))
                .map(|old| (old, (end.min(old as f32 + 1.) - start.max(old as f32)).max(0.)))
                .collect()
        })
        .collect();
    for rows in &spans {
        for columns in &spans {
            let mut sum = [0f32; 4];
            let mut weight = 0.;
            for &(y, wy) in rows {
                for &(x, wx) in columns {
                    let at = (y * side as usize + x) * 4;
                    let (w, alpha) = (wx * wy, rgba[at + 3] as f32);
                    for c in 0..3 {
                        sum[c] += rgba[at + c] as f32 * alpha * w;
                    }
                    sum[3] += alpha * w;
                    weight += w;
                }
            }
            let alpha = sum[3] / weight.max(f32::EPSILON);
            let colour = |c: f32| match sum[3] > 0. {
                true => (c / sum[3]).round().clamp(0., 255.) as u8,
                false => 0,
            };
            out.extend([colour(sum[0]), colour(sum[1]), colour(sum[2]), alpha.round().clamp(0., 255.) as u8]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn shrinks_by_averaging() {
        // 4×4: the left half opaque red, the right half clear.
        let px = |x: usize| if x < 2 { [255, 0, 0, 255] } else { [0, 0, 255, 0] };
        let rgba: Vec<u8> = (0..16).flat_map(|i| px(i % 4)).collect();
        let sizes = super::sizes(&rgba, 4);
        assert_eq!(sizes.len(), 1, "none smaller than 16 but itself");
        let half = super::shrink(&rgba, 4, 2);
        assert_eq!(&half[..8], &[255, 0, 0, 255, 0, 0, 0, 0], "red stays red, clear stays clear");
        let one = super::shrink(&rgba, 4, 1);
        assert_eq!(one, [255, 0, 0, 128], "the clear half adds no colour, half the cover");
    }
}
