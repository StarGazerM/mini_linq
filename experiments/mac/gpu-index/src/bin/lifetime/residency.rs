//! Optional early residency preparation; this does not change encoder bindings.
//!
//! Apple documents `requestResidency` as preparatory work that may be deferred
//! under memory pressure, not as a synchronization operation or residency fence:
//! https://developer.apple.com/documentation/metal/mtlresidencyset/requestresidency()
//! Residency sets do not track hazards. This experiment retains its original
//! tracked, directly bound buffers and never attaches this set to a queue or
//! command buffer. Callers must finish synchronous GPU work before dropping it.

use metal::objc::runtime::{BOOL, Class, Object, Sel, YES};
use metal::objc::{Message, msg_send, sel, sel_impl};
use metal::{Buffer, BufferRef, DeviceRef, NSUInteger};
use std::ffi::{CStr, c_char};
use std::ptr::{self, NonNull};

// Owns one +1 Objective-C reference. NonNull keeps this helper and Residency
// !Send and !Sync: MTLResidencySet methods are not thread-safe.
struct OwnedObject(NonNull<Object>);

impl OwnedObject {
    fn new(object: *mut Object, name: &str) -> Result<Self, String> {
        NonNull::new(object)
            .map(Self)
            .ok_or_else(|| format!("{name} returned nil"))
    }

    fn object(&self) -> &Object {
        // Only non-null, owned Objective-C instances enter this wrapper.
        unsafe { self.0.as_ref() }
    }
}

impl Drop for OwnedObject {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![self.object(), release];
        }
    }
}

fn require_selector<T: Message>(object: &T, selector: Sel) -> Result<(), String> {
    let supported: BOOL = unsafe { msg_send![object, respondsToSelector: selector] };
    if supported == YES {
        Ok(())
    } else {
        Err(format!(
            "Metal residency API unavailable: {} (requires supported macOS 15+ device)",
            selector.name()
        ))
    }
}

/// An explicit residency request, retained until synchronous GPU work completes.
///
/// Dropping ends the request before releasing the set and retained buffers.
/// This guard neither waits for GPU work nor substitutes for hazard tracking.
pub struct Residency {
    // Fields drop in declaration order, after Residency::drop.
    set: OwnedObject,
    _buffers: Vec<Buffer>,
}

/// Creates, populates, commits, and requests a real Metal residency set.
///
/// Retains buffer handles without reading, writing, or copying their contents.
/// Unsupported APIs and nil Objective-C results are errors, never a fallback.
pub fn request(device: &DeviceRef, buffers: &[&BufferRef]) -> Result<Residency, String> {
    let class = Class::get("MTLResidencySetDescriptor")
        .ok_or_else(|| "MTLResidencySetDescriptor unavailable (requires macOS 15+)".to_owned())?;
    require_selector(device, sel!(newResidencySetWithDescriptor:error:))?;
    require_selector(class, sel!(new))?;
    let descriptor = OwnedObject::new(
        unsafe { msg_send![class, new] },
        "MTLResidencySetDescriptor.new",
    )?;
    require_selector(descriptor.object(), sel!(setInitialCapacity:))?;
    unsafe {
        let _: () = msg_send![descriptor.object(), setInitialCapacity: buffers.len() as NSUInteger];
    }

    let mut error: *mut Object = ptr::null_mut();
    let raw_set: *mut Object = unsafe {
        msg_send![device,
            newResidencySetWithDescriptor: descriptor.object()
            error: &mut error
        ]
    };
    if raw_set.is_null() {
        // NSError/NSString are longstanding Foundation APIs. Copy the diagnostic
        // before their autoreleased lifetime ends; no resource contents are read.
        if !error.is_null() {
            let description: *mut Object = unsafe { msg_send![error, localizedDescription] };
            if !description.is_null() {
                let text: *const c_char = unsafe { msg_send![description, UTF8String] };
                if !text.is_null() {
                    return Err(format!(
                        "newResidencySetWithDescriptor failed: {}",
                        unsafe { CStr::from_ptr(text) }.to_string_lossy()
                    ));
                }
            }
        }
        return Err("newResidencySetWithDescriptor returned nil without a description".to_owned());
    }
    let set = OwnedObject::new(raw_set, "newResidencySetWithDescriptor")?;
    for selector in [
        sel!(addAllocation:),
        sel!(commit),
        sel!(requestResidency),
        sel!(endResidency),
    ] {
        require_selector(set.object(), selector)?;
    }

    let retained: Vec<Buffer> = buffers.iter().map(|buffer| (*buffer).to_owned()).collect();
    for buffer in &retained {
        unsafe {
            let _: () = msg_send![set.object(), addAllocation: &**buffer];
        }
    }
    unsafe {
        let _: () = msg_send![set.object(), commit];
        let _: () = msg_send![set.object(), requestResidency];
    }
    Ok(Residency {
        set,
        _buffers: retained,
    })
}

impl Drop for Residency {
    fn drop(&mut self) {
        // The caller has waited for completion; ending residency is not a wait.
        unsafe {
            let _: () = msg_send![self.set.object(), endResidency];
        }
    }
}
