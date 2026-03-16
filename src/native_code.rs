use std::ffi::{c_int, c_long, c_void};

#[repr(C)]
pub(crate) struct ikcpcb {
    _private: [u8; 0],
}

#[derive(Clone)]
pub(crate) struct KcpHandle(pub(crate) *mut ikcpcb);

unsafe impl Send for KcpHandle {}
unsafe impl Sync for KcpHandle {}

impl Drop for KcpHandle {
    fn drop(&mut self) {
        ikcp_release(self.0);
    }
}

pub(crate) type KcpOutput =
    extern "C" fn(buf: *const u8, len: c_int, kcp: *mut ikcpcb, user: *mut c_void) -> c_int;

unsafe extern "C" {
    pub(crate) safe fn ikcp_create(conv: u32, user: *mut c_void) -> *mut ikcpcb;
    pub(crate) safe fn ikcp_release(kcp: *mut ikcpcb);
    pub(crate) safe fn ikcp_setoutput(kcp: *mut ikcpcb, output: KcpOutput);
    pub(crate) safe fn ikcp_recv(kcp: *mut ikcpcb, buffer: *mut u8, len: c_int) -> c_int;
    pub(crate) safe fn ikcp_send(kcp: *mut ikcpcb, buffer: *const u8, len: c_int) -> c_int;
    pub(crate) safe fn ikcp_update(kcp: *mut ikcpcb, current: u32);
    pub(crate) safe fn ikcp_input(kcp: *mut ikcpcb, data: *const u8, size: c_long) -> c_int;
    pub(crate) safe fn ikcp_setmtu(kcp: *mut ikcpcb, mtu: c_int) -> c_int;
}
