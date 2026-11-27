//! The fixed set of test colors shown one at a time during calibration.
//!
//! The goal is to cover the gamut the strip actually reproduces badly:
//! the primaries and secondaries, a few whites (the white point is what
//! WLED's white balance corrects), grays (the neutral axis), black, and a
//! spread of saturated and mid-intensity hues. 24 colors, inside the
//! requested 20-30 range.

/// One calibration target: the name shown in the UI/report, and the RGB
/// value displayed full-screen (and used as the slider starting point).
pub struct TestColor {
    pub name: &'static str,
    pub rgb: [u8; 3],
}

/// The calibration palette. Order matters: whites and grays first (they
/// pin the white balance), then the saturated hues.
pub fn test_colors() -> Vec<TestColor> {
    vec![
        TestColor {
            name: "Blanc 6500K",
            rgb: [255, 255, 255],
        },
        TestColor {
            name: "Blanc chaud",
            rgb: [255, 190, 120],
        },
        TestColor {
            name: "Blanc froid",
            rgb: [180, 210, 255],
        },
        TestColor {
            name: "Gris 75%",
            rgb: [191, 191, 191],
        },
        TestColor {
            name: "Gris 50%",
            rgb: [128, 128, 128],
        },
        TestColor {
            name: "Gris 25%",
            rgb: [64, 64, 64],
        },
        TestColor {
            name: "Noir",
            rgb: [0, 0, 0],
        },
        TestColor {
            name: "Rouge",
            rgb: [255, 0, 0],
        },
        TestColor {
            name: "Vert",
            rgb: [0, 255, 0],
        },
        TestColor {
            name: "Bleu",
            rgb: [0, 0, 255],
        },
        TestColor {
            name: "Jaune",
            rgb: [255, 255, 0],
        },
        TestColor {
            name: "Cyan",
            rgb: [0, 255, 255],
        },
        TestColor {
            name: "Magenta",
            rgb: [255, 0, 255],
        },
        TestColor {
            name: "Orange",
            rgb: [255, 128, 0],
        },
        TestColor {
            name: "Ambre",
            rgb: [255, 191, 0],
        },
        TestColor {
            name: "Rouge orangé",
            rgb: [255, 69, 0],
        },
        TestColor {
            name: "Rose",
            rgb: [255, 105, 180],
        },
        TestColor {
            name: "Violet",
            rgb: [148, 0, 211],
        },
        TestColor {
            name: "Vert pomme",
            rgb: [173, 255, 47],
        },
        TestColor {
            name: "Sarcelle",
            rgb: [0, 128, 128],
        },
        TestColor {
            name: "Bleu outremer",
            rgb: [65, 105, 225],
        },
        TestColor {
            name: "Rouge brique",
            rgb: [178, 34, 34],
        },
        TestColor {
            name: "Gris chaud",
            rgb: [200, 190, 180],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_is_between_20_and_30_colors() {
        let n = test_colors().len();
        assert!((20..=30).contains(&n), "palette has {n} colors");
    }

    #[test]
    fn every_channel_is_a_byte_and_names_are_unique() {
        let colors = test_colors();
        let mut names = std::collections::HashSet::new();
        for c in &colors {
            assert!(names.insert(c.name), "duplicate name {}", c.name);
        }
    }

    #[test]
    fn palette_covers_primaries_secondaries_white_and_black() {
        let colors = test_colors();
        let has = |rgb: [u8; 3]| colors.iter().any(|c| c.rgb == rgb);
        for rgb in [
            [255, 255, 255],
            [0, 0, 0],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
            [0, 255, 255],
            [255, 0, 255],
        ] {
            assert!(has(rgb), "palette misses {rgb:?}");
        }
    }
}
