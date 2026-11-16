//! Windows PowerShell 5.1 uses the console code page even with redirected stdout.

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::io;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleOutputCP() -> u32;
        fn SetConsoleOutputCP(code_page: u32) -> i32;
        fn GetOEMCP() -> u32;
        fn GetStdHandle(kind: u32) -> *mut c_void;
        fn WriteFile(
            handle: *mut c_void,
            data: *const u8,
            length: u32,
            written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn MultiByteToWideChar(
            code_page: u32,
            flags: u32,
            data: *const u8,
            length: i32,
            wide: *mut u16,
            capacity: i32,
        ) -> i32;
    }
    pub(crate) struct Encoding {
        previous: u32,
        code_page: u32,
    }
    impl Encoding {
        pub fn new() -> Self {
            // SAFETY: these Win32 calls accept scalar code-page IDs only.
            unsafe {
                let previous = GetConsoleOutputCP();
                let changed = previous != 0 && SetConsoleOutputCP(65001) != 0;
                Self {
                    previous: if changed { previous } else { 0 },
                    code_page: if changed {
                        65001
                    } else if previous != 0 {
                        previous
                    } else {
                        GetOEMCP()
                    },
                }
            }
        }
        pub fn decode(&self, bytes: &[u8]) -> String {
            if self.code_page != 65001
                && !bytes.is_empty()
                && let Ok(length) = i32::try_from(bytes.len())
            {
                // SAFETY: byte buffer and then allocated UTF-16 buffer have explicit valid lengths.
                unsafe {
                    let size = MultiByteToWideChar(
                        self.code_page,
                        0,
                        bytes.as_ptr(),
                        length,
                        std::ptr::null_mut(),
                        0,
                    );
                    if size > 0 {
                        let mut wide = vec![0; size as usize];
                        if MultiByteToWideChar(
                            self.code_page,
                            0,
                            bytes.as_ptr(),
                            length,
                            wide.as_mut_ptr(),
                            size,
                        ) == size
                        {
                            return String::from_utf16_lossy(&wide);
                        }
                    }
                }
            }
            String::from_utf8_lossy(bytes).into_owned()
        }
    }
    impl Drop for Encoding {
        fn drop(&mut self) {
            if self.previous != 0 {
                // SAFETY: restore the code page obtained before this handoff.
                unsafe {
                    SetConsoleOutputCP(self.previous);
                }
            }
        }
    }
    pub(crate) fn write(bytes: &[u8]) -> io::Result<()> {
        // SAFETY: GetStdHandle returns this process's borrowed stdout handle.
        let handle = unsafe { GetStdHandle((-11_i32) as u32) };
        let mut rest = bytes;
        while !rest.is_empty() {
            let length = rest.len().min(u32::MAX as usize) as u32;
            let mut written = 0;
            // SAFETY: synchronous write with a live byte buffer; no overlapped I/O.
            if unsafe {
                WriteFile(
                    handle,
                    rest.as_ptr(),
                    length,
                    &mut written,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if written == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            rest = &rest[written as usize..];
        }
        Ok(())
    }
}
#[cfg(windows)]
pub(crate) use windows::{Encoding, write};
