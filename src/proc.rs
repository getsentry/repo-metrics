/// `kill -0` is the portable "does this pid exist" check; it sends no signal.
pub fn pid_alive(pid: i32) -> bool {
    unsafe { c_kill(pid, 0) == 0 }
}

pub fn signal(pid: i32, sig: i32) {
    unsafe {
        c_kill(pid, sig);
    }
}

extern "C" {
    #[link_name = "kill"]
    fn c_kill(pid: i32, sig: i32) -> i32;
}

pub fn uid() -> u32 {
    unsafe { c_getuid() }
}

extern "C" {
    #[link_name = "getuid"]
    fn c_getuid() -> u32;
}
