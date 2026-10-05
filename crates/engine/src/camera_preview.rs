//! Fit missing camera colour from a file's own colour JPEG, never from another vendor's profiles.
//! The JPEG supplies colour correspondences only: all output pixels still come from the RAW mosaic.
//! Use a fixed-size sensor proxy so thumbnails and full exports obtain the same transform.
use lightcraft_color::{Mat3, luminance_2020};
use lightcraft_pipeline::tone::ToneMap;
use lightcraft_raster::{
    Rgb32f,
    resample::{Filter, fit},
};
use lightcraft_raw::{RawImage, color::CameraTransform};

pub(crate) fn fit_preview(raw: &RawImage, bytes: &[u8], transform: &CameraTransform) -> Option<Mat3> {
    if !transform.matrix_is_fallback || raw.format != lightcraft_raw::RawFormat::Arw {
        return None;
    }
    let jpeg = lightcraft_raw::embedded_preview(bytes)?;
    let decoded = lightcraft_codecs::decode(&jpeg, lightcraft_codecs::DecodeOptions { max_size: Some((384, 384)), max_pixels: 64_000_000 }).ok()?;
    let reference = decoded.to_working();
    // Reject reference aspect mismatches; same-aspect crops/warps also face the holdout gate.
    let crop = raw.crop.clipped(raw.active_area.width, raw.active_area.height);
    if crop.width == 0 || crop.height == 0 || reference.width == 0 || reference.height == 0 {
        return None;
    }
    if lightcraft_pipeline::profiling() {
        eprintln!("[profile] camera-preview reference {}x{} raw {}x{}", reference.width, reference.height, crop.width, crop.height);
    }
    let aspect = crop.width as f64 / crop.height as f64;
    if (reference.width as f64 / reference.height as f64 / aspect - 1.0).abs() > 0.02 {
        return None;
    }
    let k = (crop.width.max(crop.height).div_ceil(384).max(2)).div_ceil(2) * 2;
    let sensor = raw.develop_binned(k, 0.99).ok()??;
    let mut sensor = fit(&sensor, 96, 96, Filter::Box);
    let reference = fit(&reference, sensor.width, sensor.height, Filter::Box);
    if (sensor.width, sensor.height) != (reference.width, reference.height) {
        return None;
    }
    let gain = 2f32.powf(transform.baseline_exposure as f32);
    sensor.map_in_place(|p| transform.matrix.apply_f32(std::array::from_fn(|i| p[i] * transform.wb[i] * gain)));
    let matrix = fit_pairs(&sensor, &reference)?;
    if lightcraft_pipeline::profiling() {
        eprintln!("[profile] camera-preview colour fitted: {:?}", matrix.0);
    }
    Some(matrix)
}

fn scene_reference(display: [f32; 3], tone: &ToneMap) -> Option<[f64; 3]> {
    let y = luminance_2020(display);
    if !display.iter().all(|v| v.is_finite() && *v > 0.004 && *v < 0.98) || !(0.015..0.85).contains(&y) {
        return None;
    }
    // Undo the default RAW shoulder before fitting; it is applied exactly once by the normal pipeline.
    let (mut lo, mut hi) = (0.0, 4.0);
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if tone.apply(mid) < y {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let scale = ((lo + hi) * 0.5 / y) as f64;
    Some(display.map(|v| v as f64 * scale))
}

// Validate in linear display light, where a fixed error bound is meaningful regardless of exposure.
fn display_reference(scene: [f64; 3], tone: &ToneMap) -> Option<[f64; 3]> {
    if !scene.iter().all(|v| v.is_finite()) {
        return None;
    }
    let y = luminance_2020(scene.map(|v| v as f32));
    if y <= 0.0 || !y.is_finite() {
        return None;
    }
    Some(scene.map(|v| v * (tone.apply(y) / y) as f64))
}

fn fit_pairs(sensor: &Rgb32f, reference: &Rgb32f) -> Option<Mat3> {
    if sensor.data.len() != reference.data.len() {
        return None;
    }
    let tone = ToneMap::new(0.0, 0.0, 0.0);
    let mut pairs = Vec::new();
    let mut colour = 0;
    for (input, output) in sensor.data.iter().zip(&reference.data) {
        if !input.iter().all(|v| v.is_finite() && *v > 0.001 && *v < 1.5) {
            continue;
        }
        let Some(target) = scene_reference(*output, &tone) else {
            continue;
        };
        let min = output.iter().copied().fold(f32::INFINITY, f32::min);
        let max = output.iter().copied().fold(0.0, f32::max);
        colour += usize::from(max - min > 0.05);
        pairs.push((input.map(f64::from), target));
    }
    if lightcraft_pipeline::profiling() {
        eprintln!("[profile] calibration pairs {} colour {}", pairs.len(), colour);
    }
    if pairs.len() < 256 || colour < pairs.len() / 20 {
        return None;
    }
    let mut gram = [[0.0; 3]; 3];
    let mut cross = [[0.0; 3]; 3];
    for (i, (x, y)) in pairs.iter().enumerate() {
        if i % 3 == 0 {
            continue;
        }
        for row in 0..3 {
            for col in 0..3 {
                gram[row][col] += x[row] * x[col];
                cross[row][col] += y[row] * x[col];
            }
        }
    }
    let trace: f64 = (0..3).map(|i| gram[i][i]).sum();
    // Reject insufficient colour diversity; regularization must not invent a calibration.
    let g = Mat3(gram);
    let inverse = g.inverse()?;
    let condition = trace * (0..3).map(|i| inverse.0[i][i].abs()).sum::<f64>();
    if !inverse.0.iter().flatten().all(|v| v.is_finite()) || trace <= 0.0 || !condition.is_finite() || condition > 1e6 {
        return None;
    }
    let regularization = trace * 1e-3;
    let exposure = (0..3).map(|i| cross[i][i]).sum::<f64>() / trace;
    for i in 0..3 {
        gram[i][i] += regularization;
        cross[i][i] += regularization * exposure;
    }
    let matrix = Mat3(cross).mul(&Mat3(gram).inverse()?);
    if lightcraft_pipeline::profiling() {
        eprintln!("[profile] calibration candidate {:?}", matrix.0);
    }
    if !matrix.0.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0) {
        return None;
    }
    let mut before = 0.0;
    let mut after = 0.0;
    let mut samples = 0;
    for (i, (x, y)) in pairs.iter().enumerate() {
        if i % 3 != 0 {
            continue;
        }
        let corrected = display_reference(matrix.apply(*x), &tone)?;
        let original = display_reference(*x, &tone)?;
        let target = display_reference(*y, &tone)?;
        for c in 0..3 {
            before += (original[c] - target[c]).powi(2);
            after += (corrected[c] - target[c]).powi(2);
            samples += 1;
        }
    }
    // Independent held-out pixels must materially improve, and match within a bounded error.
    if lightcraft_pipeline::profiling() {
        eprintln!("[profile] calibration holdout before {} after {} mean {}", before, after, after / samples as f64);
    }
    if !after.is_finite() || after >= before * 0.7 || after / samples as f64 > 0.075f64.powi(2) {
        return None;
    }
    Some(matrix)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovers_missing_colour_and_exposure_without_replacing_sensor_pixels() {
        let tone = ToneMap::new(0.0, 0.0, 0.0);
        let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
        let mut sensor = Rgb32f::new(32, 32);
        let mut reference = sensor.clone();
        for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
            *src = [0.05 + (i % 11) as f32 * 0.018, 0.05 + (i % 17) as f32 * 0.011, 0.06 + (i % 23) as f32 * 0.008];
            let p = known.apply_f32(*src);
            let y = luminance_2020(p);
            *dst = p.map(|v| v * tone.apply(y) / y);
        }
        let original = sensor.clone();
        let fitted = fit_pairs(&sensor, &reference).unwrap();
        assert_eq!(sensor.data, original.data);
        for (a, b) in fitted.0.iter().flatten().zip(known.0.iter().flatten()) {
            assert!((a - b).abs() < 0.15, "{fitted:?}");
        }
    }
    #[test]
    fn rejects_monochrome_invalid_and_unrelated_previews() {
        let mut sensor = Rgb32f::new(32, 32);
        let mut reference = sensor.clone();
        for (i, p) in sensor.data.iter_mut().enumerate() {
            *p = [0.1 + (i % 13) as f32 * 0.02, 0.15, 0.1];
        }
        reference.data.fill([0.2; 3]);
        assert!(fit_pairs(&sensor, &reference).is_none());
        reference.data.fill([f32::NAN; 3]);
        assert!(fit_pairs(&sensor, &reference).is_none());
        for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
            *src = [0.04 + (i % 11) as f32 * 0.02, 0.05 + (i % 17) as f32 * 0.01, 0.03 + (i % 23) as f32 * 0.01];
            *dst = [0.05 + (i % 7) as f32 * 0.07, 0.05 + (i % 19) as f32 * 0.02, 0.05 + (i % 29) as f32 * 0.01];
        }
        assert!(fit_pairs(&sensor, &reference).is_none());
        // A constant coloured sensor cannot determine a three-dimensional colour transform.
        sensor.data.fill([0.1, 0.15, 0.12]);
        assert!(fit_pairs(&sensor, &reference).is_none());
        reference.data.truncate(8);
        assert!(fit_pairs(&sensor, &reference).is_none());
    }
}
