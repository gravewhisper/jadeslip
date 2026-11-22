use std::ffi::{CStr, c_char, c_int, c_void};
use std::fs::OpenOptions;
use std::io::Write;

const RTLD_NEXT: *mut c_void = -1isize as *mut c_void;

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

fn log(line: String) {
    if let Some(path) = std::env::var_os("SS_SYNC_TRACE")
        && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

fn fd_path(fd: c_int) -> String {
    std::fs::read_link(format!("/proc/self/fd/{fd}"))
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

unsafe fn text(path: *const c_char) -> String {
    unsafe { CStr::from_ptr(path) }.to_string_lossy().into_owned()
}

#[unsafe(no_mangle)]
pub extern "C" fn fsync(fd: c_int) -> c_int {
    let real: extern "C" fn(c_int) -> c_int =
        unsafe { std::mem::transmute(dlsym(RTLD_NEXT, c"fsync".as_ptr())) };
    let result = real(fd);
    log(format!("fsync\t{}\t{result}\n", fd_path(fd)));
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn fdatasync(fd: c_int) -> c_int {
    let real: extern "C" fn(c_int) -> c_int =
        unsafe { std::mem::transmute(dlsym(RTLD_NEXT, c"fdatasync".as_ptr())) };
    let result = real(fd);
    log(format!("fdatasync\t{}\t{result}\n", fd_path(fd)));
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rename(from: *const c_char, to: *const c_char) -> c_int {
    let real: extern "C" fn(*const c_char, *const c_char) -> c_int =
        unsafe { std::mem::transmute(dlsym(RTLD_NEXT, c"rename".as_ptr())) };
    let result = real(from, to);
    log(format!(
        "rename\t{}\t{}\t{result}\n",
        unsafe { text(from) },
        unsafe { text(to) }
    ));
    result
}
