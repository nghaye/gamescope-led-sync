//! The calibration fit: from (target shown on screen, adjusted sent to the
//! strip) pairs, derive the per-channel gains to enter in WLED's white
//! balance (Settings > LED preferences > "Color white", R/G/B sliders,
//! 0..=255, 255 = no correction).
//!
//! WLED's white balance is a per-channel gain applied to the incoming RGB
//! before output, so the model is `out = clamp(target * gain)` per channel.
//! Each channel is fitted independently by least squares over the samples
//! whose target channel is non-zero (a zero target carries no information
//! about that channel's gain):
//!
//! ```text
//! gain_c = sum(t_c * a_c) / sum(t_c^2)
//! ```
//!
//! Gains are clamped to [0, 1] (WLED can only attenuate, 255 = neutral) and
//! then normalized so the largest becomes 1.0, which keeps the white point
//! at full brightness; the WLED values are `round(gain * 255)`.

/// One validated calibration sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// The color displayed full-screen.
    pub target: [u8; 3],
    /// The slider values that matched it on the strip.
    pub adjusted: [u8; 3],
}

/// The fitted calibration.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    /// Per-channel gains, normalized so the largest is 1.0.
    pub gains: [f64; 3],
    /// The values to enter in WLED's white balance (0..=255, 255 = neutral).
    pub wled_rgb: [u8; 3],
    /// Human-readable caveats about the fit quality.
    pub warnings: Vec<String>,
}

/// Fit the per-channel gains from the samples. Returns None when there are
/// no samples at all.
pub fn fit(samples: &[Sample]) -> Option<Fit> {
    if samples.is_empty() {
        return None;
    }
    let mut gains = [1.0f64; 3];
    let mut no_data = [false; 3];
    let mut saturated = [0usize; 3];
    for c in 0..3 {
        let (mut num, mut den) = (0.0f64, 0.0f64);
        for s in samples {
            let t = s.target[c] as f64;
            let a = s.adjusted[c] as f64;
            if t <= 0.0 {
                continue;
            }
            num += t * a;
            den += t * t;
            if s.adjusted[c] == 255 && s.target[c] < 255 {
                saturated[c] += 1;
            }
        }
        if den <= 0.0 {
            no_data[c] = true;
            continue;
        }
        gains[c] = (num / den).clamp(0.0, 1.0);
    }
    // Normalize so the strongest channel stays at full gain (white point
    // at maximum brightness); WLED cannot amplify, only attenuate.
    let max = gains.iter().copied().fold(0.0f64, f64::max);
    if max > 0.0 {
        for g in &mut gains {
            *g /= max;
        }
    }
    let wled_rgb = gains.map(|g| (g * 255.0).round().clamp(0.0, 255.0) as u8);

    let mut warnings = Vec::new();
    for c in 0..3 {
        let ch = "RGB"[c..c + 1].to_string();
        if no_data[c] {
            warnings.push(format!(
                "canal {ch}: aucune cible non nulle, gain laissé neutre (255)"
            ));
        }
        if saturated[c] > 0 {
            let n = saturated[c];
            warnings.push(format!(
                "canal {ch}: {n} échantillon(s) à 255 (saturation du slider), le gain est peut-être sous-estimé"
            ));
        }
        if gains[c] < 0.05 && !no_data[c] {
            warnings.push(format!(
                "canal {ch} quasi nul ({:.3}): vérifiez le Color Order du ruban dans WLED",
                gains[c]
            ));
        }
    }
    Some(Fit {
        gains,
        wled_rgb,
        warnings,
    })
}

/// The full report shown on the final page and written to the report file.
pub fn report(samples: &[Sample], fit: &Fit, names: &[&'static str], path: &str) -> String {
    let mut out = String::new();
    out.push_str("gamescope-led-sync — rapport de calibration\n");
    out.push_str("=========================================\n\n");
    out.push_str(&format!(
        "{} couleurs calibrées, rapport sauvegardé dans:\n  {path}\n\n",
        samples.len()
    ));
    out.push_str("Échantillons (cible écran -> ajusté ruban):\n");
    for (i, s) in samples.iter().enumerate() {
        let name = names.get(i).copied().unwrap_or("?");
        out.push_str(&format!(
            "  {:>2}. {:<14} cible rgb({:>3},{:>3},{:>3})  ->  ajusté rgb({:>3},{:>3},{:>3})\n",
            i + 1,
            name,
            s.target[0],
            s.target[1],
            s.target[2],
            s.adjusted[0],
            s.adjusted[1],
            s.adjusted[2],
        ));
    }
    out.push_str(&format!(
        "\nGains ajustés (normalisés, max = 1.0): R={:.4} G={:.4} B={:.4}\n",
        fit.gains[0], fit.gains[1], fit.gains[2]
    ));
    out.push_str(&format!(
        "\nÀ entrer dans WLED (Settings > LED preferences > Color white / White balance):\n  Red   = {}\n  Green = {}\n  Blue  = {}\n",
        fit.wled_rgb[0], fit.wled_rgb[1], fit.wled_rgb[2]
    ));
    if fit.warnings.is_empty() {
        out.push_str("\nAucun avertissement.\n");
    } else {
        out.push_str("\nAvertissements:\n");
        for w in &fit.warnings {
            out.push_str(&format!("  - {w}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(t: [u8; 3], g: [f64; 3]) -> Sample {
        let adjusted = [
            (t[0] as f64 * g[0]).round().clamp(0.0, 255.0) as u8,
            (t[1] as f64 * g[1]).round().clamp(0.0, 255.0) as u8,
            (t[2] as f64 * g[2]).round().clamp(0.0, 255.0) as u8,
        ];
        Sample {
            target: t,
            adjusted,
        }
    }

    #[test]
    fn fit_recovers_known_gains() {
        // A strip whose real gains are [0.8, 1.0, 0.6] (i.e. WLED white
        // balance would be R=204, G=255, B=153).
        let real = [0.8f64, 1.0, 0.6];
        let samples: Vec<Sample> = [
            [255, 255, 255],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
            [200, 100, 50],
            [64, 64, 64],
        ]
        .iter()
        .map(|&t| sample(t, real))
        .collect();
        let f = fit(&samples).unwrap();
        assert!((f.gains[0] - 0.8).abs() < 0.01, "{:?}", f.gains);
        assert!((f.gains[1] - 1.0).abs() < 0.01);
        assert!((f.gains[2] - 0.6).abs() < 0.01);
        assert_eq!(f.wled_rgb, [204, 255, 153]);
    }

    #[test]
    fn fit_normalizes_so_the_max_gain_is_255() {
        // All gains below 1: the strongest channel must still map to 255.
        let real = [0.5f64, 0.25, 1.0];
        let samples: Vec<Sample> = [[255, 255, 255], [120, 200, 40], [10, 250, 90]]
            .iter()
            .map(|&t| sample(t, real))
            .collect();
        let f = fit(&samples).unwrap();
        assert_eq!(f.wled_rgb[2], 255);
        assert!((f.gains[0] - 0.5).abs() < 0.02);
    }

    #[test]
    fn zero_target_channels_carry_no_information() {
        // Only red targets: G and B have no data and must stay neutral.
        let samples = vec![
            sample([255, 0, 0], [0.5, 1.0, 1.0]),
            sample([100, 0, 0], [0.5, 1.0, 1.0]),
        ];
        let f = fit(&samples).unwrap();
        // R is fitted at ~0.5; G and B stay neutral and are the max, so the
        // normalization leaves R at 0.5 -> WLED R = 128.
        assert!((f.gains[0] - 0.5).abs() < 0.01, "{:?}", f.gains);
        assert_eq!(f.wled_rgb, [128, 255, 255]);
        assert_eq!(f.wled_rgb[1], 255, "G stays neutral");
        assert_eq!(f.wled_rgb[2], 255, "B stays neutral");
        assert!(f.warnings.iter().any(|w| w.contains("canal G")));
        assert!(f.warnings.iter().any(|w| w.contains("canal B")));
    }

    #[test]
    fn empty_samples_give_no_fit() {
        assert!(fit(&[]).is_none());
    }

    #[test]
    fn saturation_produces_a_warning() {
        // Target 200 on R but the slider was pinned at 255: the fit sees
        // saturation and warns the gain may be underestimated.
        let samples = vec![
            Sample {
                target: [200, 100, 50],
                adjusted: [255, 100, 50],
            },
            Sample {
                target: [255, 128, 64],
                adjusted: [255, 128, 64],
            },
        ];
        let f = fit(&samples).unwrap();
        assert!(f.warnings.iter().any(|w| w.contains("saturation")));
    }

    #[test]
    fn report_lists_every_sample_and_the_wled_values() {
        let samples = vec![Sample {
            target: [255, 0, 0],
            adjusted: [204, 0, 0],
        }];
        let f = fit(&samples).unwrap();
        let r = report(&samples, &f, &["Rouge"], "/tmp/x.txt");
        assert!(r.contains("Rouge"));
        assert!(r.contains("rgb(255,  0,  0)"));
        assert!(r.contains("Red   = 204"), "{r}");
        assert!(r.contains("/tmp/x.txt"));
    }
}
