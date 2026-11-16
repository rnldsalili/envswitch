//! Keep the parent alive until the installer has rolled back and closed stdout.

#[cfg(windows)]
static HANDOFFS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(windows)]
pub(crate) fn handoff_active() -> bool {
    HANDOFFS.load(std::sync::atomic::Ordering::SeqCst) != 0
}

#[cfg(windows)]
pub(crate) struct Handoff;
#[cfg(windows)]
impl Handoff {
    pub fn new() -> std::io::Result<Self> {
        HANDOFFS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Self)
    }
    pub fn prepare(&self, _command: &mut std::process::Command) {}
    pub fn started(&self, _pid: u32) {}
    pub fn finished(&self) {}
}
#[cfg(windows)]
impl Drop for Handoff {
    fn drop(&mut self) {
        HANDOFFS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(unix)]
mod unix {
    use std::io;
    use std::sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicI32, Ordering},
    };
    static LOCK: Mutex<()> = Mutex::new(());
    static CHILD: AtomicI32 = AtomicI32::new(0);
    static TERMINATED: AtomicBool = AtomicBool::new(false);
    unsafe extern "C" {
        fn signal(number: i32, handler: usize) -> usize;
        fn kill(pid: i32, number: i32) -> i32;
    }
    extern "C" fn terminate(number: i32) {
        TERMINATED.store(true, Ordering::SeqCst);
        let pid = CHILD.load(Ordering::SeqCst);
        if pid > 0 {
            // SAFETY: kill is async-signal-safe; pid belongs to the waiting child.
            unsafe {
                kill(pid, number);
            }
        }
    }
    pub(crate) struct Handoff {
        handlers: Vec<(i32, usize)>,
        _lock: MutexGuard<'static, ()>,
    }
    impl Handoff {
        pub fn new() -> io::Result<Self> {
            let mut guard = Self {
                handlers: Vec::new(),
                _lock: LOCK.lock().unwrap_or_else(|p| p.into_inner()),
            };
            TERMINATED.store(false, Ordering::SeqCst);
            for (number, handler) in [(2, 1), (3, 1), (15, terminate as *const () as usize)] {
                // SAFETY: SIG_IGN (1) and the C handler have valid signal dispositions.
                let previous = unsafe { signal(number, handler) };
                if previous == usize::MAX {
                    return Err(io::Error::last_os_error());
                }
                guard.handlers.push((number, previous));
            }
            Ok(guard)
        }
        pub fn prepare(&self, command: &mut std::process::Command) {
            use std::os::unix::process::CommandExt;
            // SAFETY: only async-signal-safe signal calls run between fork and exec.
            unsafe {
                command.pre_exec(|| {
                    for number in [2, 3, 15] {
                        if signal(number, 0) == usize::MAX {
                            return Err(io::Error::last_os_error());
                        }
                    }
                    Ok(())
                });
            }
        }
        pub fn started(&self, pid: u32) {
            CHILD.store(pid as i32, Ordering::SeqCst);
            if TERMINATED.load(Ordering::SeqCst) {
                // SAFETY: this is the child just spawned by the guarded handoff.
                unsafe {
                    kill(pid as i32, 15);
                }
            }
        }
        pub fn finished(&self) {
            CHILD.store(0, Ordering::SeqCst);
        }
    }
    impl Drop for Handoff {
        fn drop(&mut self) {
            CHILD.store(0, Ordering::SeqCst);
            for &(number, handler) in self.handlers.iter().rev() {
                // SAFETY: restore the dispositions returned by signal above.
                unsafe {
                    signal(number, handler);
                }
            }
        }
    }
}
#[cfg(unix)]
pub(crate) use unix::Handoff;
