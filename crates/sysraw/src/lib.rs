//! Safe, bounded access to macOS's camera-calibrated RAW decoder. No profiles or platform
//! implementation code are copied. Linear floating-point pixels retain scene highlight headroom;
//! a small paired proxy describes the system's starting tone separately from those pixels.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub struct Pixels {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<[f32; 3]>,
}

pub struct LinearRaw {
    pub pixels: Pixels,
    pub linear_proxy: Pixels,
    pub display_proxy: Pixels,
}

pub fn decode(bytes: &[u8], max_edge: usize) -> Result<LinearRaw, String> {
    if bytes.is_empty() || bytes.len() > 256 * 1024 * 1024 || max_edge == 0 {
        return Err("invalid or oversized native RAW request".into());
    }
    #[cfg(target_os = "macos")]
    {
        macos::decode(bytes, max_edge)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("calibrated macOS RAW decoding is unavailable on this platform".into())
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use super::{LinearRaw, Pixels};
    use objc2::{
        encode::{Encoding, RefEncode},
        msg_send,
        rc::{Retained, autoreleasepool},
        runtime::{AnyClass, AnyObject},
    };
    use objc2_core_image::{
        CIContext, CIFilter, CIImage, CIRAWFilter, kCIContextCacheIntermediates, kCIContextWorkingColorSpace, kCIContextWorkingFormat, kCIFormatRGBAf,
    };
    use objc2_foundation::{NSData, NSDictionary, NSNumber, NSPoint, NSRect, NSSize, NSString};
    use std::ffi::c_void;
    static DECODE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        static kCGColorSpaceExtendedLinearITUR_2020: *const c_void;
        fn CGColorSpaceCreateWithName(name: *const c_void) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: *const c_void);
    }

    #[repr(C)]
    struct CGColorSpace {
        _private: [u8; 0],
    }
    // SAFETY: CGColorSpaceRef is an opaque pointer with the documented Objective-C encoding.
    // The pointee is never dereferenced or constructed in Rust.
    unsafe impl RefEncode for CGColorSpace {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColorSpace", &[]));
    }

    struct ColourSpace(*const c_void);
    impl Drop for ColourSpace {
        fn drop(&mut self) {
            // SAFETY: this owns exactly one non-null Create-rule colour-space reference.
            unsafe { CFRelease(self.0) };
        }
    }

    pub fn decode(bytes: &[u8], max_edge: usize) -> Result<LinearRaw, String> {
        // A RAW decoder has large framework-owned intermediates; bound simultaneous decodes.
        let _guard = DECODE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        autoreleasepool(|_| {
            if AnyClass::get("CIRAWFilter").is_none() {
                return Err("macOS 12 or later is required for calibrated RAW decoding".into());
            }
            let data = NSData::with_bytes(bytes);
            // SAFETY: framework objects are local to this thread, retained for every use. NSData
            // owns a bounded copy. All controls use their documented ranges; absent/unsupported
            // images return nil. No pointer to an object or a pixel buffer escapes this module.
            unsafe {
                let hint = NSString::from_str("com.sony.arw-raw-image");
                let filter = CIRAWFilter::filterWithImageData_identifierHint(&data, Some(&hint)).ok_or("macOS does not support this RAW image")?;
                filter.setExposure(0.0);
                filter.setLuminanceNoiseReductionAmount(0.0);
                filter.setColorNoiseReductionAmount(0.0);
                filter.setSharpnessAmount(0.0);
                filter.setContrastAmount(0.0);
                filter.setDetailAmount(0.0);
                filter.setLocalToneMapAmount(0.0);
                filter.setLensCorrectionEnabled(false);
                let space = CGColorSpaceCreateWithName(kCGColorSpaceExtendedLinearITUR_2020);
                if space.is_null() {
                    return Err("linear Rec.2020 colour space is unavailable".into());
                }
                let colour_space = ColourSpace(space);
                let format = NSNumber::new_i32(kCIFormatRGBAf);
                let cache = NSNumber::new_bool(false);
                // SAFETY: NSNumber is an NSObject subclass; CGColorSpace is the documented CF
                // value accepted by CIContext's workingColorSpace option and bridges as an
                // Objective-C object. Owners outlive construction; the dictionary/context retain
                // their own references. Float working precision avoids a half-float bottleneck.
                let mut values = Vec::new();
                for p in [
                    colour_space.0.cast_mut().cast::<AnyObject>(),
                    (&*format as *const NSNumber).cast_mut().cast::<AnyObject>(),
                    (&*cache as *const NSNumber).cast_mut().cast::<AnyObject>(),
                ] {
                    values.push(Retained::retain(p).ok_or("cannot retain RAW context option")?);
                }
                let opts = NSDictionary::<NSString, AnyObject>::from_vec(
                    &[kCIContextWorkingColorSpace, kCIContextWorkingFormat, kCIContextCacheIntermediates],
                    values,
                );
                let context = CIContext::contextWithOptions(Some(&opts));
                let display = filter.outputImage().ok_or("RAW decoder produced no display image")?;
                let display_proxy = render(&context, &display, 96, &colour_space)?;
                filter.setBoostAmount(0.0);
                filter.setBoostShadowAmount(0.0);
                filter.setGamutMappingEnabled(false);
                let linear = filter.outputImage().ok_or("RAW decoder produced no linear image")?;
                let linear_proxy = render(&context, &linear, 96, &colour_space)?;
                let pixels = render(&context, &linear, max_edge, &colour_space)?;
                Ok(LinearRaw { pixels, linear_proxy, display_proxy })
            }
        })
    }

    unsafe fn render(context: &CIContext, image: &CIImage, edge: usize, cs: &ColourSpace) -> Result<Pixels, String> {
        // SAFETY: caller owns retained framework objects and a live colour-space reference.
        let extent = unsafe { image.extent() };
        let (iw, ih) = (extent.size.width, extent.size.height);
        if ![iw, ih, extent.origin.x, extent.origin.y].iter().all(|v| v.is_finite()) || iw < 1.0 || ih < 1.0 || iw * ih > 64_000_000.0 {
            return Err(format!("invalid or oversized RAW dimensions: {extent:?}"));
        }
        let scale = (edge as f64 / iw.max(ih)).min(1.0);
        let (w, h) = ((iw * scale).round().max(1.0) as usize, (ih * scale).round().max(1.0) as usize);
        let count = w.checked_mul(h).and_then(|v| v.checked_mul(4)).ok_or("RAW buffer overflow")?;
        if count > 256_000_000 {
            return Err("RAW buffer exceeds limit".into());
        }
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(count).map_err(|_| "cannot allocate native RAW buffer")?;
        rgba.resize(count, 0.0f32);
        let mut scaled = None;
        if scale < 1.0 {
            // SAFETY: documented Lanczos filter keys, input objects retained by the filter.
            unsafe {
                let f = CIFilter::filterWithName(&NSString::from_str("CILanczosScaleTransform")).ok_or("RAW resampler unavailable")?;
                let _: () = msg_send![&*f, setValue: image, forKey: &*NSString::from_str("inputImage")];
                let n = NSNumber::new_f64(scale);
                let _: () = msg_send![&*f, setValue: &*n, forKey: &*NSString::from_str("inputScale")];
                scaled = Some(f.outputImage().ok_or("RAW resampling failed")?);
            }
        }
        let image = scaled.as_deref().unwrap_or(image);
        let bounds = NSRect::new(NSPoint::new(extent.origin.x * scale, extent.origin.y * scale), NSSize::new(w as f64, h as f64));
        // SAFETY: rowBytes is exactly w * 4 * sizeof(f32); the initialized Vec holds h complete
        // rows. RGBAf writes four native-endian f32 channels. Bounds and allocation are bounded
        // and checked above. Core Image renders synchronously and retains no bitmap pointer.
        unsafe {
            let _: () = msg_send![context, render: image, toBitmap: rgba.as_mut_ptr().cast::<c_void>(), rowBytes: (w * 16) as isize, bounds: bounds, format: kCIFormatRGBAf, colorSpace: cs.0.cast::<CGColorSpace>()];
        }
        if rgba.iter().any(|v| !v.is_finite()) {
            return Err("nonfinite native RAW samples".into());
        }
        let mut rgb = Vec::new();
        rgb.try_reserve_exact(w * h).map_err(|_| "cannot allocate linear RAW pixels")?;
        rgb.extend(rgba.as_chunks::<4>().0.iter().map(|v| [v[0], v[1], v[2]]));
        Ok(Pixels { width: w, height: h, rgb })
    }
}

#[cfg(test)]
mod tests {
    /// Private fixtures are supplied locally; none are committed or uploaded.
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "requires LIGHTCRAFT_TEST_ARW and a supported camera on macOS"]
    fn real_arw_retains_linear_headroom_and_stable_proxies() {
        let path = std::env::var("LIGHTCRAFT_TEST_ARW").unwrap();
        let bytes = std::fs::read(path).unwrap();
        let small = super::decode(&bytes, 400).unwrap();
        let larger = super::decode(&bytes, 1200).unwrap();
        assert_eq!(small.pixels.width.max(small.pixels.height), 400);
        assert_eq!(larger.pixels.width.max(larger.pixels.height), 1200);
        assert_eq!(small.linear_proxy.rgb, larger.linear_proxy.rgb);
        assert_eq!(small.display_proxy.rgb, larger.display_proxy.rgb);
        let maximum = larger.pixels.rgb.iter().flatten().copied().fold(0.0f32, f32::max);
        eprintln!("native scene-linear maximum: {maximum}");
        assert!(maximum > 1.0, "linear highlights must not be baked/clipped to display white");
        assert!(larger.pixels.rgb.iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn rejects_invalid_requests_before_platform_calls() {
        assert!(super::decode(&[], 100).is_err());
        assert!(super::decode(&[0; 8], 0).is_err());
        assert!(super::decode(&[0; 8], 100).is_err());
    }
}
