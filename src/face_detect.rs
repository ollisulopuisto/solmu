//! Face detection for the avatar crop. macOS uses the native Vision
//! framework through the Objective-C runtime; every other platform stays
//! naive and reports no face, so the editor keeps its centre crop. Detection
//! never fails loudly: any error is `None`.
//!
//! This module never touches the network; the picked image bytes stay on
//! device, like the rest of the local avatar feature.

/// A detected face in image coordinates: centre (x, y) as a fraction of the
/// image's width/height (origin top-left) and the bounding square's side as
/// a fraction of the image's short side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub x: f32,
    pub y: f32,
    pub side: f32,
}

/// The largest face in `png`, or `None` on other platforms, when the OS has
/// no Vision framework and when detection simply finds nothing. Runs
/// synchronously; call it off the UI thread.
pub fn detect(png: &[u8], width: u32, height: u32) -> Option<Face> {
    if png.is_empty() || width == 0 || height == 0 {
        return None;
    }
    imp::detect(png, width, height)
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn detect(_png: &[u8], _width: u32, _height: u32) -> Option<super::Face> {
        None
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Face;
    use objc2::Encode;
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use std::ffi::c_void;
    use std::ptr;

    // Linking Vision loads its classes at launch; the calls themselves go
    // through the Objective-C runtime, so no new bindings crates.
    #[link(name = "Vision", kind = "framework")]
    unsafe extern "C" {}

    /// CGRect on 64-bit macOS: origin then size, all doubles.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NormRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    // SAFETY: NormRect is repr(C) with the same layout as CGRect on 64-bit
    // macOS: an origin and a size of two doubles each.
    unsafe impl Encode for NormRect {
        const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
            "CGRect",
            &[
                objc2::Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]),
                objc2::Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]),
            ],
        );
    }

    pub fn detect(png: &[u8], width: u32, height: u32) -> Option<Face> {
        let request_class = AnyClass::get(c"VNDetectFaceRectanglesRequest")?;
        let handler_class = AnyClass::get(c"VNImageRequestHandler")?;
        let data_class = AnyClass::get(c"NSData")?;
        let array_class = AnyClass::get(c"NSArray")?;
        // SAFETY: every receiver is a live object for the call. +0 (borrowed)
        // object returns are used only while their owner (`data`, `request`)
        // is alive and are never released here; `alloc`/`new`/`init` returns
        // are held in Retained and released on drop.
        unsafe {
            let data: *mut AnyObject = msg_send![
                data_class,
                dataWithBytes: png.as_ptr() as *const c_void,
                length: png.len()
            ];
            if data.is_null() {
                return None;
            }
            let alloc: *mut AnyObject = msg_send![handler_class, alloc];
            let handler: Option<Retained<AnyObject>> = msg_send![
                alloc,
                initWithData: data,
                options: ptr::null::<AnyObject>()
            ];
            let handler = handler?;
            let request: Option<Retained<AnyObject>> = msg_send![request_class, new];
            let request = request?;
            let requests: *mut AnyObject =
                msg_send![array_class, arrayWithObject: Retained::as_ptr(&request).as_ptr()];
            if requests.is_null() {
                return None;
            }
            let ok: Bool = msg_send![
                &*handler,
                performRequests: requests,
                error: ptr::null_mut::<*mut AnyObject>()
            ];
            if !ok.as_bool() {
                return None;
            }
            let results: *mut AnyObject = msg_send![&*request, results];
            if results.is_null() {
                return None;
            }
            let count: usize = msg_send![results, count];
            let short = f64::from(width.min(height));
            let mut best: Option<Face> = None;
            for index in 0..count {
                let observation: *mut AnyObject = msg_send![results, objectAtIndex: index];
                if observation.is_null() {
                    continue;
                }
                let rect: NormRect = msg_send![observation, boundingBox];
                if !rect.x.is_finite()
                    || !rect.y.is_finite()
                    || !rect.w.is_finite()
                    || !rect.h.is_finite()
                    || rect.w <= 0.0
                    || rect.h <= 0.0
                {
                    continue;
                }
                // Vision's bounding boxes have a bottom-left origin; flip to
                // the image's top-left origin.
                let side = (rect.w * f64::from(width)).max(rect.h * f64::from(height)) / short;
                let face = Face {
                    x: (rect.x + rect.w / 2.0) as f32,
                    y: (1.0 - rect.y - rect.h / 2.0) as f32,
                    side: side as f32,
                };
                if best.is_none_or(|b| face.side > b.side) {
                    best = Some(face);
                }
            }
            best
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn degenerate_inputs_never_reach_the_platform() {
        assert!(super::detect(&[], 0, 0).is_none());
        assert!(super::detect(b"png", 0, 4).is_none());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_mac_platforms_report_no_face() {
        assert!(super::detect(b"a png", 4, 4).is_none());
    }
}
