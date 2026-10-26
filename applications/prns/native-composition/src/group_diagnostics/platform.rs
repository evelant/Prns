//! Controlled native-trial I/O. No general application logs are forwarded.
use std::io::{self, Write};

pub(super) fn opted_in(environment: &str, android_property: &'static std::ffi::CStr) -> bool {
    if super::opted_in(std::env::var(environment).ok().as_deref()) {
        return true;
    }
    #[cfg(target_os = "android")]
    {
        // PROP_VALUE_MAX from the Android NDK's sys/system_properties.h.
        let mut value = [0_u8; 92];
        // SAFETY: the property name is terminated, and the writable buffer has
        // the NDK-required size. The result is inspected only within that buffer.
        let length =
            unsafe { __system_property_get(android_property.as_ptr(), value.as_mut_ptr().cast()) };
        length == 1 && value[0] == b'1'
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = android_property;
        false
    }
}

/// Collect a whole formatted record before sending it to Android's line-based
/// logger. Formatting cannot allocate or accidentally split one event into many.
pub(super) struct Output {
    buffer: [u8; 2048],
    len: usize,
    invalid: bool,
}

pub(super) fn output() -> Output {
    Output {
        buffer: [0; 2048],
        len: 0,
        invalid: false,
    }
}

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // Reserve the terminating NUL required by liblog, and reject embedded
        // NULs so that the native sink cannot silently truncate an event.
        if bytes.contains(&0) || bytes.len() >= self.buffer.len() - self.len {
            self.invalid = true;
            return Err(io::ErrorKind::InvalidData.into());
        }
        self.buffer[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if self.invalid {
            emit(b"[prns.trial] stage=diagnostics category=output_truncated\n\0");
        } else if self.len != 0 {
            emit(&self.buffer[..=self.len]);
        }
    }
}

fn emit(terminated: &[u8]) {
    #[cfg(target_os = "android")]
    {
        let tag = android_tag(terminated);
        // SAFETY: both buffers are NUL-terminated and remain alive for this
        // synchronous NDK call. ANDROID_LOG_INFO is 4 in android/log.h.
        unsafe {
            __android_log_write(4, tag.as_ptr(), terminated.as_ptr().cast());
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = std::io::stderr()
            .lock()
            .write_all(&terminated[..terminated.len() - 1]);
    }
}

#[cfg(any(target_os = "android", test))]
fn android_tag(terminated: &[u8]) -> &'static std::ffi::CStr {
    if terminated.starts_with(b"[prns.bluetooth]") {
        c"prns.bluetooth"
    } else if terminated.starts_with(b"[prns.groups]") {
        c"prns.groups"
    } else if terminated.starts_with(b"[prns.routes]") {
        c"prns.routes"
    } else if terminated.starts_with(b"[prns.links]") {
        c"prns.links"
    } else if terminated.starts_with(b"[prns.schedule]") {
        c"prns.schedule"
    } else {
        c"prns.trial"
    }
}

#[cfg(target_os = "android")]
unsafe extern "C" {
    fn __system_property_get(
        name: *const std::ffi::c_char,
        value: *mut std::ffi::c_char,
    ) -> std::ffi::c_int;
}

#[cfg(target_os = "android")]
#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(
        priority: std::ffi::c_int,
        tag: *const std::ffi::c_char,
        text: *const std::ffi::c_char,
    ) -> std::ffi::c_int;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_schedule_tag_does_not_consume_or_hide_in_other_streams() {
        assert_eq!(
            android_tag(b"[prns.links] stage=handshake\n\0"),
            c"prns.links"
        );
        assert_eq!(
            android_tag(b"[prns.schedule] stage=schedule\n\0"),
            c"prns.schedule"
        );
        assert_eq!(
            android_tag(b"[prns.groups] stage=receipt\n\0"),
            c"prns.groups"
        );
        assert_eq!(
            android_tag(b"[prns.schedule.extra] arbitrary\n\0"),
            c"prns.trial"
        );
    }
    #[test]
    fn sink_retains_complete_records_and_a_native_terminator() {
        let mut sink = output();
        writeln!(sink, "[prns.groups] stage=test count={}", 3).unwrap();
        assert_eq!(
            &sink.buffer[..=sink.len],
            b"[prns.groups] stage=test count=3\n\0"
        );
        // Suppress test output without exercising the real platform sink.
        sink.len = 0;
    }
    #[test]
    fn sink_rejects_overflow_and_interior_nuls_without_partial_success() {
        for bytes in [&[b'x'; 2048][..], &b"bad\0event"[..]] {
            let mut sink = output();
            assert!(sink.write_all(bytes).is_err());
            assert!(sink.invalid);
            assert_eq!(sink.len, 0);
            sink.invalid = false;
        }
    }
}
