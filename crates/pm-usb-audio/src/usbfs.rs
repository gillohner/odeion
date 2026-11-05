//! The [`Transport`] on Linux's usbfs: the file descriptor Android's
//! `UsbDeviceConnection.getFileDescriptor()` returns is a usbfs device node, and these ioctls are
//! what libusb's own Linux and Android backend sends on it (`linux_usbfs.c`). Isochronous
//! transfers are submitted with `USBDEVFS_SUBMITURB` and collected with `USBDEVFS_REAPURBNDELAY`
//! once `poll` says one finished.
//!
//! It works on its own duplicate of the app's descriptor (`dup`), closed when it is dropped: the
//! app closing its connection (an unplug, a release) never pulls the descriptor from under a
//! transfer, and a descriptor number the app reuses is never this one. Device-unverified: the layouts follow `linux/usbdevice_fs.h` and are checked here
//! against its sizes, but only a phone with a DAC proves the transfers.

#![allow(unsafe_code)]

use crate::driver::{Reaped, Transport, Urb, UsbError};
use crate::requests::ControlRequest;
use crate::schedule::Speed;
use std::collections::HashMap;
use std::os::raw::{c_int, c_uint, c_void};
use std::sync::Mutex;
use std::time::Duration;

/// `struct usbdevfs_ctrltransfer`.
#[repr(C)]
struct CtrlTransfer {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
    timeout: u32,
    data: *mut c_void,
}

/// `struct usbdevfs_setinterface`.
#[repr(C)]
struct SetInterface {
    interface: c_uint,
    altsetting: c_uint,
}

/// `struct usbdevfs_iso_packet_desc`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IsoPacket {
    length: c_uint,
    actual_length: c_uint,
    status: c_uint,
}

/// `struct usbdevfs_urb` (its `iso_frame_desc[]` follows it directly: see [`UrbBox`]).
#[repr(C)]
struct RawUrb {
    kind: u8,
    endpoint: u8,
    status: c_int,
    flags: c_uint,
    buffer: *mut c_void,
    buffer_length: c_int,
    actual_length: c_int,
    start_frame: c_int,
    number_of_packets: c_int,
    error_count: c_int,
    signr: c_uint,
    usercontext: *mut c_void,
}

/// Packets per transfer at most (usbfs takes up to 128 per isochronous URB).
const MAX_PACKETS: usize = 128;

/// One URB as usbfs reads it: the header, then the packet descriptors right after it (the
/// header's size is a multiple of its alignment, which is at least the descriptors', so `repr(C)`
/// leaves no gap: the C flexible array member's place). Boxed: its address is the kernel's
/// handle until it is reaped.
#[repr(C)]
struct UrbBox {
    urb: RawUrb,
    iso: [IsoPacket; MAX_PACKETS],
}

/// `struct usbdevfs_ioctl`.
#[repr(C)]
struct IoctlArgs {
    ifno: c_int,
    ioctl_code: c_int,
    data: *mut c_void,
}

const USBDEVFS_URB_TYPE_ISO: u8 = 0;
const USBDEVFS_URB_ISO_ASAP: c_uint = 0x02;

// _IOC(dir, 'U', nr, size), as Linux encodes it on arm, arm64 and x86
const fn ioc(dir: u32, nr: u32, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((b'U' as u32) << 8) | nr
}
const NONE: u32 = 0;
const WRITE: u32 = 1;
const READ: u32 = 2;
const USBDEVFS_CONTROL: u32 = ioc(READ | WRITE, 0, std::mem::size_of::<CtrlTransfer>());
const USBDEVFS_SETINTERFACE: u32 = ioc(READ, 4, std::mem::size_of::<SetInterface>());
const USBDEVFS_SUBMITURB: u32 = ioc(READ, 10, std::mem::size_of::<RawUrb>());
const USBDEVFS_DISCARDURB: u32 = ioc(NONE, 11, 0);
const USBDEVFS_REAPURBNDELAY: u32 = ioc(WRITE, 13, std::mem::size_of::<*mut c_void>());
const USBDEVFS_CLAIMINTERFACE: u32 = ioc(READ, 15, std::mem::size_of::<c_uint>());
const USBDEVFS_RELEASEINTERFACE: u32 = ioc(READ, 16, std::mem::size_of::<c_uint>());
const USBDEVFS_IOCTL: u32 = ioc(READ | WRITE, 18, std::mem::size_of::<IoctlArgs>());
const USBDEVFS_CONNECT: u32 = ioc(NONE, 23, 0);
const USBDEVFS_GET_SPEED: u32 = ioc(NONE, 31, 0);

/// Control transfers wait this long.
const CONTROL_TIMEOUT_MS: u32 = 1000;

/// A submitted URB and its buffer, kept until reaped.
struct InFlight {
    tag: u64,
    urb: Box<UrbBox>,
    data: Vec<u8>,
}

/// usbfs on its own duplicate of the app's file descriptor.
pub struct Usbfs {
    /// Ours (a `dup` of the app's), closed on drop; -1 when it could not be duplicated (every
    /// call then fails as gone).
    fd: c_int,
    /// Submitted URBs, by the address the kernel hands back.
    in_flight: Mutex<HashMap<usize, InFlight>>,
}

impl Usbfs {
    /// usbfs on a duplicate of `fd` (an open `UsbDeviceConnection`'s); `fd` stays the app's.
    pub fn new(fd: c_int) -> Self {
        // SAFETY: dup on any integer is safe; a bad one fails with -1 (then every call is Gone)
        let own = if fd >= 0 { unsafe { libc::dup(fd) } } else { -1 };
        Usbfs {
            fd: own,
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    fn raw_ioctl(&self, request: u32, arg: *mut c_void) -> Result<c_int, std::io::Error> {
        // SAFETY: `fd` is the app's open usbfs node; every `arg` is a live, correctly laid out
        // struct for `request` (or null where the request takes none), valid for the call.
        let r = unsafe { libc::ioctl(self.fd, request as _, arg) };
        if r < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(r)
        }
    }

    fn ioctl(&self, request: u32, arg: *mut c_void) -> Result<c_int, UsbError> {
        self.raw_ioctl(request, arg).map_err(usb_error)
    }

    fn in_flight(&self) -> std::sync::MutexGuard<'_, HashMap<usize, InFlight>> {
        self.in_flight.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Give a released interface back to Android's driver (`USBDEVFS_CONNECT`).
    fn reattach(&self, interface: u8) {
        let mut args = IoctlArgs {
            ifno: c_int::from(interface),
            ioctl_code: USBDEVFS_CONNECT as c_int,
            data: std::ptr::null_mut(),
        };
        let _ = self.ioctl(USBDEVFS_IOCTL, &mut args as *mut IoctlArgs as *mut c_void);
    }
}

fn usb_error(e: std::io::Error) -> UsbError {
    match e.raw_os_error() {
        // Unplugged, or the descriptor is no longer one (closed): gone either way
        Some(libc::ENODEV) | Some(libc::ESHUTDOWN) | Some(libc::EBADF) => UsbError::Gone,
        _ => UsbError::Other(e.to_string()),
    }
}

impl Transport for Usbfs {
    fn speed(&self) -> Option<Speed> {
        match self.ioctl(USBDEVFS_GET_SPEED, std::ptr::null_mut()) {
            // enum usb_device_speed: LOW 1, FULL 2, HIGH 3, WIRELESS 4, SUPER 5, SUPER_PLUS 6
            Ok(1 | 2) => Some(Speed::Full),
            Ok(3..=6) => Some(Speed::High),
            _ => None,
        }
    }

    fn claim(&self, interface: u8) -> Result<(), UsbError> {
        let mut i = c_uint::from(interface);
        self.ioctl(USBDEVFS_CLAIMINTERFACE, &mut i as *mut c_uint as *mut c_void)
            .map(|_| ())
    }

    fn release(&self, interface: u8) {
        let mut i = c_uint::from(interface);
        let _ = self.ioctl(USBDEVFS_RELEASEINTERFACE, &mut i as *mut c_uint as *mut c_void);
        self.reattach(interface);
    }

    fn set_alt(&self, interface: u8, alt: u8) -> Result<(), UsbError> {
        let mut s = SetInterface {
            interface: c_uint::from(interface),
            altsetting: c_uint::from(alt),
        };
        self.ioctl(USBDEVFS_SETINTERFACE, &mut s as *mut SetInterface as *mut c_void)
            .map(|_| ())
    }

    fn control(&self, req: &ControlRequest) -> Result<Vec<u8>, UsbError> {
        let mut buf = if req.reads() {
            vec![0u8; usize::from(req.length)]
        } else {
            req.data.clone()
        };
        let mut c = CtrlTransfer {
            request_type: req.request_type,
            request: req.request,
            value: req.value,
            index: req.index,
            length: buf.len() as u16,
            timeout: CONTROL_TIMEOUT_MS,
            data: if buf.is_empty() {
                std::ptr::null_mut()
            } else {
                buf.as_mut_ptr() as *mut c_void
            },
        };
        let n = self.ioctl(USBDEVFS_CONTROL, &mut c as *mut CtrlTransfer as *mut c_void)?;
        if req.reads() {
            buf.truncate(n.max(0) as usize);
            Ok(buf)
        } else {
            Ok(Vec::new())
        }
    }

    fn submit(&self, urb: Urb) -> Result<(), UsbError> {
        let n = urb.packets.len();
        if n == 0 || n > MAX_PACKETS {
            return Err(UsbError::Other(format!("{n} packets in one transfer")));
        }
        let total: usize = urb.packets.iter().map(|p| *p as usize).sum();
        let mut data = urb.data;
        data.resize(total.max(data.len()), 0);
        let mut b = Box::new(UrbBox {
            urb: RawUrb {
                kind: USBDEVFS_URB_TYPE_ISO,
                endpoint: urb.endpoint,
                status: 0,
                flags: USBDEVFS_URB_ISO_ASAP,
                buffer: data.as_mut_ptr() as *mut c_void,
                buffer_length: total as c_int,
                actual_length: 0,
                start_frame: 0,
                number_of_packets: n as c_int,
                error_count: 0,
                signr: 0,
                usercontext: std::ptr::null_mut(),
            },
            iso: [IsoPacket::default(); MAX_PACKETS],
        });
        for (d, len) in b.iso.iter_mut().zip(&urb.packets) {
            d.length = *len;
        }
        let key = &mut b.urb as *mut RawUrb as usize;
        // Held before the kernel has it: a reap on another thread finds it
        let mut map = self.in_flight();
        self.ioctl(USBDEVFS_SUBMITURB, key as *mut c_void)?;
        map.insert(
            key,
            InFlight {
                tag: urb.tag,
                urb: b,
                data,
            },
        );
        Ok(())
    }

    fn reap(&self, timeout: Duration) -> Result<Option<Reaped>, UsbError> {
        let mut p = libc::pollfd {
            fd: self.fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        let ms = timeout.as_millis().min(c_int::MAX as u128) as c_int;
        // SAFETY: one valid pollfd, for the call.
        let r = unsafe { libc::poll(&mut p, 1, ms) };
        if r < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) {
                return Ok(None);
            }
            return Err(usb_error(e));
        }
        if p.revents & libc::POLLNVAL != 0
            || p.revents & (libc::POLLERR | libc::POLLHUP) != 0 && p.revents & libc::POLLOUT == 0
        {
            return Err(UsbError::Gone);
        }
        let mut got: *mut c_void = std::ptr::null_mut();
        match self.raw_ioctl(
            USBDEVFS_REAPURBNDELAY,
            &mut got as *mut *mut c_void as *mut c_void,
        ) {
            Ok(_) => {}
            Err(e) if e.raw_os_error() == Some(libc::EAGAIN) => return Ok(None),
            Err(e) => return Err(usb_error(e)),
        }
        let Some(f) = self.in_flight().remove(&(got as usize)) else {
            return Ok(None);
        };
        let n = (f.urb.urb.number_of_packets.max(0) as usize).min(MAX_PACKETS);
        Ok(Some(Reaped {
            tag: f.tag,
            endpoint: f.urb.urb.endpoint,
            status: f.urb.urb.status,
            actual: f.urb.iso[..n].iter().map(|d| d.actual_length).collect(),
            // The kernel keeps a packet's (negative) errno in an unsigned field
            packet_status: f.urb.iso[..n].iter().map(|d| d.status as i32).collect(),
            data: f.data,
        }))
    }

    fn discard(&self, tag: u64) {
        let key = self
            .in_flight()
            .iter()
            .find(|(_, f)| f.tag == tag)
            .map(|(k, _)| *k);
        if let Some(k) = key {
            let _ = self.ioctl(USBDEVFS_DISCARDURB, k as *mut c_void);
        }
    }
}

impl Drop for Usbfs {
    fn drop(&mut self) {
        // URBs still with the kernel: discard them and wait for them, so their memory isn't freed
        // under it
        let keys: Vec<usize> = self.in_flight().keys().copied().collect();
        for k in keys {
            let _ = self.ioctl(USBDEVFS_DISCARDURB, k as *mut c_void);
        }
        for _ in 0..100 {
            if self.in_flight().is_empty() {
                break;
            }
            if self.reap(Duration::from_millis(10)).is_err() {
                break;
            }
        }
        // Whatever the kernel never gave back is leaked rather than freed under it
        for (_, f) in self.in_flight().drain() {
            std::mem::forget(f);
        }
        if self.fd >= 0 {
            // SAFETY: our own duplicate, closed once
            unsafe { libc::close(self.fd) };
        }
    }
}

// SAFETY: the raw pointers inside point into boxes and vectors this value owns; the kernel holds
// them only between submit and reap, and every access goes through the mutex.
unsafe impl Send for Usbfs {}
unsafe impl std::marker::Sync for Usbfs {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layouts and request numbers `linux/usbdevice_fs.h` gives on 64-bit Linux.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn the_kernels_layouts() {
        assert_eq!(std::mem::size_of::<CtrlTransfer>(), 24);
        assert_eq!(std::mem::size_of::<RawUrb>(), 56);
        assert_eq!(std::mem::offset_of!(UrbBox, iso), 56);
        assert_eq!(std::mem::size_of::<IsoPacket>(), 12);
        assert_eq!(USBDEVFS_CONTROL, 0xc018_5500);
        assert_eq!(USBDEVFS_SETINTERFACE, 0x8008_5504);
        assert_eq!(USBDEVFS_SUBMITURB, 0x8038_550a);
        assert_eq!(USBDEVFS_DISCARDURB, 0x0000_550b);
        assert_eq!(USBDEVFS_REAPURBNDELAY, 0x4008_550d);
        assert_eq!(USBDEVFS_CLAIMINTERFACE, 0x8004_550f);
        assert_eq!(USBDEVFS_RELEASEINTERFACE, 0x8004_5510);
        assert_eq!(USBDEVFS_IOCTL, 0xc010_5512);
        assert_eq!(USBDEVFS_CONNECT, 0x0000_5517);
        assert_eq!(USBDEVFS_GET_SPEED, 0x0000_551f);
    }

    /// The layouts and request numbers on 32-bit ARM (Claude L, round 1 of #306).
    #[test]
    #[cfg(target_pointer_width = "32")]
    fn the_kernels_layouts_32() {
        assert_eq!(std::mem::size_of::<CtrlTransfer>(), 16);
        assert_eq!(std::mem::size_of::<RawUrb>(), 44);
        assert_eq!(std::mem::offset_of!(UrbBox, iso), 44);
        assert_eq!(USBDEVFS_CONTROL, 0xc010_5500);
        assert_eq!(USBDEVFS_SUBMITURB, 0x802c_550a);
        assert_eq!(USBDEVFS_REAPURBNDELAY, 0x4004_550d);
        assert_eq!(USBDEVFS_IOCTL, 0xc00c_5512);
    }

    /// Codex M / Claude M6 (round 1 of #306): its own duplicate. The app closing its descriptor
    /// leaves this one working (no reuse of the number), and a closed one is Gone, not an error.
    #[test]
    fn it_works_on_its_own_descriptor() {
        use std::os::fd::IntoRawFd;
        let app = std::fs::File::open("/dev/null").unwrap().into_raw_fd();
        let u = Usbfs::new(app);
        assert_ne!(u.fd, app);
        // SAFETY: the test's own descriptor, closed once
        unsafe { libc::close(app) };
        // Ours is still open (/dev/null answers ENOTTY, not EBADF)
        assert!(matches!(u.claim(0), Err(UsbError::Other(_))));
        let closed = Usbfs::new(-1);
        assert_eq!(closed.claim(0), Err(UsbError::Gone));
        assert!(matches!(
            closed.reap(Duration::from_millis(1)),
            Err(UsbError::Gone) | Ok(None)
        ));
    }

    /// Not a USB device: every call fails cleanly, nothing is leaked or freed under the kernel.
    #[test]
    fn a_file_that_isnt_usbfs_fails_cleanly() {
        let f = std::fs::File::open("/dev/null").unwrap();
        use std::os::fd::AsRawFd;
        let u = Usbfs::new(f.as_raw_fd());
        assert!(u.claim(0).is_err());
        assert!(u.set_alt(1, 0).is_err());
        assert!(u.control(&crate::requests::uac2_get_rate(0x29, 0)).is_err());
        let r = u.submit(Urb {
            tag: 1,
            endpoint: 1,
            data: vec![0; 8],
            packets: vec![8],
        });
        assert!(r.is_err());
        assert!(u.in_flight().is_empty());
        assert_eq!(u.speed(), None);
    }
}
