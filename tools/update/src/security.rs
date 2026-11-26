use std::{
    ffi::c_void,
    path::Path,
    ptr::{self, NonNull},
};

use objc2_core_foundation::{CFDictionary, CFError, CFNumber, CFRetained, CFString, CFType, CFURL};
use objc2_security::{
    SecCSFlags, SecCode, SecCodeSignatureFlags, SecRequirement, SecStaticCode, errSecSuccess,
    kSecCSCheckAllArchitectures, kSecCSCheckNestedCode, kSecCSSigningInformation,
    kSecCSStrictValidate, kSecCodeInfoFlags,
};

use crate::release::Result;

/// Additional code signing requirements accepted during a migration release,
/// such as transitioning from a persistent self-signed certificate to an Apple
/// Developer ID certificate (`identifier "cc.loongw.Gewu" and anchor apple generic and certificate
/// leaf[subject.OU] = "TEAMID"`).
const EXTRA_TRUSTED_REQUIREMENTS: &[&str] = &[];

#[expect(
    unsafe_code,
    reason = "creates a SecStaticCode reference from a file URL"
)]
fn static_code(path: &Path) -> Result<CFRetained<SecStaticCode>> {
    let url = CFURL::from_file_path(path).ok_or("Invalid application bundle path.")?;
    let mut raw: *const SecStaticCode = ptr::null();
    // SAFETY: `url` is a valid CFURL and `raw` points to stack storage for the out-parameter.
    let status = unsafe {
        SecStaticCode::create_with_path(&url, SecCSFlags::DefaultFlags, NonNull::from(&mut raw))
    };
    if status != errSecSuccess {
        return Err(format!(
            "Could not inspect code signature at {} (OSStatus {status}).",
            path.display()
        )
        .into());
    }
    let ptr = NonNull::new(raw.cast_mut()).ok_or("Missing SecStaticCode reference.")?;
    // SAFETY: `SecStaticCodeCreateWithPath` follows the Create Rule and transfers +1 ownership.
    Ok(unsafe { CFRetained::from_raw(ptr) })
}

#[expect(unsafe_code, reason = "reads signing flags from Security.framework")]
fn signature_flags(code: &SecStaticCode) -> Result<SecCodeSignatureFlags> {
    let mut raw_info: *const CFDictionary = ptr::null();
    // SAFETY: `code` is a valid `SecStaticCode` and `raw_info` points to stack out-storage.
    let status = unsafe {
        SecCode::copy_signing_information(
            code,
            SecCSFlags(kSecCSSigningInformation),
            NonNull::from(&mut raw_info),
        )
    };
    if status != errSecSuccess {
        return Err(
            format!("Could not read bundle signing information (OSStatus {status}).").into(),
        );
    }
    let ptr = NonNull::new(raw_info.cast_mut()).ok_or("Missing signing information dictionary.")?;
    // SAFETY: `SecCodeCopySigningInformation` follows the Copy Rule and transfers +1 ownership.
    let info = unsafe { CFRetained::from_raw(ptr) };
    // SAFETY: `kSecCodeInfoFlags` is a constant `CFString` exported by Security.framework.
    let key: *const CFString = unsafe { kSecCodeInfoFlags };
    // SAFETY: `info` is a valid `CFDictionary` and `key` is a valid `CFString`.
    let raw_value = unsafe { info.value(key.cast::<c_void>()) };
    if raw_value.is_null() {
        return Ok(SecCodeSignatureFlags::empty());
    }
    // SAFETY: Values in `CFDictionary` returned by `SecCodeCopySigningInformation` are `CFType`
    // objects alive as long as `info`.
    let cf_value = unsafe { &*raw_value.cast::<CFType>() };
    let number = cf_value
        .downcast_ref::<CFNumber>()
        .ok_or("Unexpected type for kSecCodeInfoFlags.")?;
    let bits = number
        .as_i64()
        .ok_or("Could not decode kSecCodeInfoFlags.")?;
    Ok(SecCodeSignatureFlags::from_bits_retain(bits as u32))
}

#[expect(
    unsafe_code,
    reason = "copies the designated requirement from Security.framework"
)]
fn designated_requirement(code: &SecStaticCode) -> Result<CFRetained<SecRequirement>> {
    let mut raw: *mut SecRequirement = ptr::null_mut();
    // SAFETY: `code` is valid and `raw` points to stack out-storage.
    let status = unsafe {
        SecCode::copy_designated_requirement(
            code,
            SecCSFlags::DefaultFlags,
            NonNull::from(&mut raw),
        )
    };
    if status != errSecSuccess {
        return Err(format!(
            "Could not read the installed application's signing requirement (OSStatus {status})."
        )
        .into());
    }
    let ptr = NonNull::new(raw).ok_or("Missing designated SecRequirement.")?;
    // SAFETY: `SecCodeCopyDesignatedRequirement` follows the Copy Rule and transfers +1 ownership.
    Ok(unsafe { CFRetained::from_raw(ptr) })
}

#[expect(
    unsafe_code,
    reason = "compiles a textual SecRequirement string via Security.framework"
)]
pub(crate) fn requirement_from_str(text: &str) -> Result<CFRetained<SecRequirement>> {
    let cf_text = CFString::from_str(text);
    let mut raw_error: *mut CFError = ptr::null_mut();
    let mut raw_req: *mut SecRequirement = ptr::null_mut();
    // SAFETY: `cf_text` is valid and both out-pointers point to stack variables.
    let status = unsafe {
        SecRequirement::create_with_string_and_errors(
            &cf_text,
            SecCSFlags::DefaultFlags,
            &mut raw_error,
            NonNull::from(&mut raw_req),
        )
    };
    let error = NonNull::new(raw_error).map(|ptr| {
        // SAFETY: `SecRequirementCreateWithStringAndErrors` returns an owned `CFError` on failure.
        unsafe { CFRetained::from_raw(ptr) }
    });
    if status != errSecSuccess {
        let detail = error
            .and_then(|err| err.description())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("OSStatus {status}"));
        return Err(format!("Invalid signing requirement: {detail}").into());
    }
    let ptr = NonNull::new(raw_req).ok_or("Missing compiled SecRequirement.")?;
    // SAFETY: `SecRequirementCreateWithStringAndErrors` follows the Create Rule.
    Ok(unsafe { CFRetained::from_raw(ptr) })
}

#[expect(
    unsafe_code,
    reason = "validates a candidate bundle against a SecRequirement"
)]
fn check_validity(
    code: &SecStaticCode,
    requirement: &SecRequirement,
) -> std::result::Result<(), String> {
    let flags =
        SecCSFlags(kSecCSCheckAllArchitectures | kSecCSCheckNestedCode | kSecCSStrictValidate);
    let mut raw_error: *mut CFError = ptr::null_mut();
    // SAFETY: `code` and `requirement` are valid CF objects and `raw_error` is valid stack storage.
    let status =
        unsafe { code.check_validity_with_errors(flags, Some(requirement), &mut raw_error) };
    let error = NonNull::new(raw_error).map(|ptr| {
        // SAFETY: When non-null, `SecStaticCodeCheckValidityWithErrors` returns a +1 owned
        // `CFError`.
        unsafe { CFRetained::from_raw(ptr) }
    });
    if status == errSecSuccess {
        return Ok(());
    }
    let detail = error
        .and_then(|err| err.description())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("OSStatus {status}"));
    Err(format!("Candidate code signature rejected: {detail}"))
}

pub(crate) fn verify_candidate(installed: &Path, candidate: &Path) -> Result<()> {
    let installed_code = static_code(installed)?;
    let flags = signature_flags(&installed_code)?;
    if flags.contains(SecCodeSignatureFlags::Adhoc) {
        return Err(
            "The installed application is ad-hoc signed and cannot authenticate updates.".into(),
        );
    }
    let current_req = designated_requirement(&installed_code)?;
    let candidate_code = static_code(candidate)?;

    if let Err(primary_error) = check_validity(&candidate_code, &current_req) {
        for text in EXTRA_TRUSTED_REQUIREMENTS {
            let extra = requirement_from_str(text)?;
            if check_validity(&candidate_code, &extra).is_ok() {
                return Ok(());
            }
        }
        return Err(primary_error.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;
