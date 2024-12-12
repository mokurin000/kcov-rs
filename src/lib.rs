use std::mem;
use std::num::NonZero;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::raw::c_void;
use std::os::unix::io::RawFd;
use std::ptr::NonNull;

use nix::errno::Errno;
use nix::sys::{mman, stat};
use nix::{Result, fcntl, libc, request_code_none, request_code_read, unistd};

pub const KCOV: &str = "/sys/kernel/debug/kcov";
pub const KCOV_BUF_LEN: usize = 1024 * 1024 * 8;

const KCOV_MAGIC: u8 = b'c';
const KCOV_INIT_TRACE: u8 = 1;
const KCOV_ENABLE: u8 = 100;
const KCOV_DISABLE: u8 = 101;

macro_rules! exits {
	( $code:expr ) => {
		::std::process::exit($code)
	};

	( $code :expr, $fmt:expr $( , $arg:expr )* ) => {{
        eprintln!($fmt $( , $arg )*);
		::std::process::exit($code)
	}};
}

unsafe fn kcov_init(fd: RawFd, len: usize) -> Result<libc::c_int> {
    let res = unsafe {
        libc::ioctl(
            fd,
            request_code_read!(KCOV_MAGIC, KCOV_INIT_TRACE, mem::size_of::<usize>()),
            len,
        )
    };

    Errno::result(res)
}

unsafe fn kcov_enable(fd: RawFd) -> Result<libc::c_int> {
    let res = unsafe { libc::ioctl(fd, request_code_none!(KCOV_MAGIC, KCOV_ENABLE), 0) };
    Errno::result(res)
}

unsafe fn kcov_disable(fd: RawFd) -> Result<libc::c_int> {
    let res = unsafe { libc::ioctl(fd, request_code_none!(KCOV_MAGIC, KCOV_DISABLE), 0) };
    Errno::result(res)
}

pub struct CovHandle {
    fd: RawFd,
    pcs: NonNull<c_void>,
    len: NonNull<c_void>,
    mem: NonNull<c_void>,
}

pub fn open() -> CovHandle {
    let fd = fcntl::open(KCOV, fcntl::OFlag::O_RDWR, stat::Mode::empty())
        .unwrap_or_else(|e| exits!(exitcode::OSERR, "Fail to open {}: {}", KCOV, e));

    unsafe {
        use mman::MapFlags;
        use mman::ProtFlags;

        kcov_init(fd, KCOV_BUF_LEN / mem::size_of::<usize>())
            .unwrap_or_else(|e| exits!(exitcode::OSERR, "Fail to init kcov trace: {}", e));

        let mem = mman::mmap(
            None,
            NonZero::new(KCOV_BUF_LEN).unwrap(),
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
            MapFlags::MAP_SHARED,
            OwnedFd::from_raw_fd(fd),
            0,
        )
        .unwrap_or_else(|e| exits!(exitcode::OSERR, "Fail to map kcov: {}", e));

        let cover = mem;
        let len = cover;
        let pcs = cover.add(1);
        CovHandle { fd, pcs, len, mem }
    }
}

impl CovHandle {
    pub fn collect<F: FnMut()>(&mut self, mut call: F) -> &[usize] {
        self.clear();
        let _g = self.enable();
        call();
        self.covers()
    }

    fn clear(&mut self) {
        unsafe {
            *(self.len.as_ptr() as *mut usize) = 0;
        }
    }

    fn enable(&self) -> Guard {
        unsafe {
            kcov_enable(self.fd)
                .unwrap_or_else(|e| exits!(exitcode::OSERR, "Fail to enable kcov trace: {}", e));
        }
        Guard { inner: self }
    }

    fn covers(&self) -> &[usize] {
        unsafe {
            let len = self.len;
            std::slice::from_raw_parts(self.pcs.as_ptr() as _, len.as_ptr() as _)
        }
    }
}

impl Drop for CovHandle {
    fn drop(&mut self) {
        unsafe {
            mman::munmap(self.mem, KCOV_BUF_LEN)
                .unwrap_or_else(|e| exits!(exitcode::OSERR, "Fail to munmap kcov: {}", e));
        }
        unistd::close(self.fd).unwrap_or_else(|e| exits!(exitcode::OSERR, "Fail to close: {}", e));
    }
}

pub struct Guard<'a> {
    inner: &'a CovHandle,
}

impl<'a> Drop for Guard<'a> {
    fn drop(&mut self) {
        unsafe {
            kcov_disable(self.inner.fd)
                .unwrap_or_else(|e| exits!(exitcode::OSERR, "Fail to disable kcov trace: {}", e));
        }
    }
}
