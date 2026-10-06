//! What a mapped file does to the process when something else shortens it.
//!
//! A mapping of a file is memory like any other until the file is truncated by
//! another process: the pages past its new end are gone, and reading one is a
//! `SIGBUS`, which ends the process. A log that is rotated while it is open
//! here is enough. So the mapping is watched: when a read of it faults, the
//! page (and the rest of the mapping after it, which is gone too) is replaced
//! by memory that holds zeros, the read goes on, and the mapping is marked as
//! [damaged](Watch::damaged), so that what was read from it can be told from
//! what can be trusted, and whoever is looking is told to open the file again.
//!
//! Where it is done — a signal handler — only what may be done there is: a few
//! atomics and one system call, no allocation, no locks. A fault of anything
//! else (a bug of ours, a stack overflow) is handed to whatever was there before,
//! as if none of this existed.
//!
//! Windows does not need it: a mapped file cannot be truncated there. Nor does a
//! system that has no `SIGBUS` to catch in this way, which has no guard.

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod imp {
    use std::cell::UnsafeCell;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Once;

    /// How many mappings can be watched at once. (The app has one document open,
    /// and one being loaded to replace it.)
    const SLOTS: usize = 32;

    /// A mapping that is watched: where it is, and whether a read of it faulted.
    struct Slot {
        /// 0 when the slot is free, 1 when it is taken.
        taken: AtomicUsize,
        /// Where the mapping begins, 0 until the slot is ready to be looked at
        /// by the handler.
        start: AtomicUsize,
        /// Just past where it ends.
        end: AtomicUsize,
        damaged: AtomicBool,
    }

    impl Slot {
        const fn new() -> Self {
            Self {
                taken: AtomicUsize::new(0),
                start: AtomicUsize::new(0),
                end: AtomicUsize::new(0),
                damaged: AtomicBool::new(false),
            }
        }
    }

    static WATCHED: [Slot; SLOTS] = [const { Slot::new() }; SLOTS];
    static INSTALL: Once = Once::new();
    static PAGE: AtomicUsize = AtomicUsize::new(0);

    /// The action `SIGBUS` had before ours, for what is not ours to handle.
    struct Previous(UnsafeCell<Option<libc::sigaction>>);

    // SAFETY: written once, inside `INSTALL`, before the handler that reads it is
    // installed.
    unsafe impl Sync for Previous {}

    static PREVIOUS: Previous = Previous(UnsafeCell::new(None));

    /// A mapping that is being watched, for as long as this is held.
    pub(in crate::lazy) struct Watch {
        slot: Option<usize>,
    }

    impl Watch {
        /// A watch of nothing, for bytes that are not a mapping.
        pub(in crate::lazy) fn none() -> Self {
            Self { slot: None }
        }

        /// Watch the `len` bytes at `address`, which have to stay mapped for as
        /// long as this is held. If there are too many watched already, or the
        /// handler cannot be installed, nothing is watched, and a fault is a
        /// crash as it would be without any of this.
        pub(in crate::lazy) fn new(address: usize, len: usize) -> Self {
            if len == 0 || !install() {
                return Self { slot: None };
            }
            let page = PAGE.load(Ordering::Relaxed);
            // The mapping covers whole pages.
            let end = (address + len).div_ceil(page) * page;
            for (at, slot) in WATCHED.iter().enumerate() {
                if slot
                    .taken
                    .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
                {
                    slot.damaged.store(false, Ordering::Relaxed);
                    slot.end.store(end, Ordering::Release);
                    // The handler looks at a slot once it has a start.
                    slot.start.store(address, Ordering::Release);
                    return Self { slot: Some(at) };
                }
            }
            Self { slot: None }
        }

        /// Whether a read of the mapping faulted: some of what it holds is
        /// zeros that the file does not have.
        pub(in crate::lazy) fn damaged(&self) -> bool {
            self.slot
                .is_some_and(|at| WATCHED[at].damaged.load(Ordering::Acquire))
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            if let Some(at) = self.slot {
                let slot = &WATCHED[at];
                slot.start.store(0, Ordering::Release);
                slot.end.store(0, Ordering::Release);
                slot.damaged.store(false, Ordering::Relaxed);
                slot.taken.store(0, Ordering::Release);
            }
        }
    }

    /// Put the handler in place, once. Whether it is.
    fn install() -> bool {
        static INSTALLED: AtomicBool = AtomicBool::new(false);
        INSTALL.call_once(|| {
            // SAFETY: plain calls of the system's, with structures that are
            // zeroed and then filled in as `sigaction` wants them.
            unsafe {
                let page = libc::sysconf(libc::_SC_PAGESIZE);
                if page <= 0 {
                    return;
                }
                let mut previous: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(libc::SIGBUS, std::ptr::null(), &mut previous) != 0 {
                    return;
                }
                *PREVIOUS.0.get() = Some(previous);
                PAGE.store(page as usize, Ordering::Relaxed);

                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_fault as *const () as usize;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                if libc::sigaction(libc::SIGBUS, &action, std::ptr::null_mut()) == 0 {
                    INSTALLED.store(true, Ordering::Release);
                }
            }
        });
        INSTALLED.load(Ordering::Acquire)
    }

    /// The handler: a fault inside a watched mapping is repaired and the read
    /// that made it is made again; any other is passed on.
    extern "C" fn on_fault(
        signal: libc::c_int,
        info: *mut libc::siginfo_t,
        context: *mut libc::c_void,
    ) {
        // SAFETY: the system gives a handler that asked for `SA_SIGINFO` a valid
        // `info`; `si_addr` is the address of the fault, for a `SIGBUS`.
        let address = unsafe { (*info).si_addr() } as usize;
        if repair(address) {
            return;
        }
        pass_on(signal, info, context);
    }

    /// Replace what is left of the mapping that `address` is in, from its page
    /// on, by zeros. (Whatever faulted is past the end of the file, and so is
    /// everything after it.)
    fn repair(address: usize) -> bool {
        let page = PAGE.load(Ordering::Relaxed);
        if page == 0 {
            return false;
        }
        for slot in &WATCHED {
            let start = slot.start.load(Ordering::Acquire);
            if start == 0 {
                continue;
            }
            let end = slot.end.load(Ordering::Acquire);
            if address < start || address >= end {
                continue;
            }
            let from = address & !(page - 1);
            // SAFETY: `from..end` is part of a mapping that is still there (it is
            // unwatched, and so freed, only once it is unmapped); a fixed
            // mapping of anonymous memory over it replaces its pages.
            let replaced = unsafe {
                libc::mmap(
                    from as *mut libc::c_void,
                    end - from,
                    libc::PROT_READ,
                    libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_FIXED,
                    -1,
                    0,
                )
            };
            if replaced == libc::MAP_FAILED {
                return false;
            }
            slot.damaged.store(true, Ordering::Release);
            return true;
        }
        false
    }

    /// A fault that is not of a watched mapping: whatever dealt with it before
    /// does again. If nothing did, the default is put back, and the read that
    /// faulted is made once more, as it would have been, for the system to end
    /// the process.
    fn pass_on(signal: libc::c_int, info: *mut libc::siginfo_t, context: *mut libc::c_void) {
        // SAFETY: `PREVIOUS` was filled in before this handler was installed; what
        // it holds is a handler's address, to be called as the kind of handler
        // its flags say.
        unsafe {
            let previous = (*PREVIOUS.0.get()).as_ref();
            match previous {
                Some(action)
                    if action.sa_sigaction != libc::SIG_DFL
                        && action.sa_sigaction != libc::SIG_IGN =>
                {
                    if action.sa_flags & libc::SA_SIGINFO != 0 {
                        let handler: extern "C" fn(
                            libc::c_int,
                            *mut libc::siginfo_t,
                            *mut libc::c_void,
                        ) = std::mem::transmute(action.sa_sigaction);
                        handler(signal, info, context);
                    } else {
                        let handler: extern "C" fn(libc::c_int) =
                            std::mem::transmute(action.sa_sigaction);
                        handler(signal);
                    }
                }
                _ => {
                    let mut default: libc::sigaction = std::mem::zeroed();
                    default.sa_sigaction = libc::SIG_DFL;
                    libc::sigemptyset(&mut default.sa_mask);
                    libc::sigaction(signal, &default, std::ptr::null_mut());
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod imp {
    /// Nothing to watch for here.
    pub(in crate::lazy) struct Watch;

    impl Watch {
        pub(in crate::lazy) fn none() -> Self {
            Self
        }

        pub(in crate::lazy) fn new(_address: usize, _len: usize) -> Self {
            Self
        }

        pub(in crate::lazy) fn damaged(&self) -> bool {
            false
        }
    }
}

pub(super) use imp::Watch;
