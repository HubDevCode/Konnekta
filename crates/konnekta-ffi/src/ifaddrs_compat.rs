//! `getifaddrs` / `freeifaddrs` for Android 6.
//!
//! Bionic has them only from Android 7 (API 24), but `if-addrs` (used by the
//! core and by mDNS) calls them directly: without these definitions the
//! library would not even load on Android 6. Defined here, they are the ones
//! the core links to. From Android 7 they forward to the system's; on Android 6
//! they list the IPv4 addresses with the classic `SIOCGIFCONF` ioctls
//! (no IPv6, which discovery does not use).

// On the Linux host only the fallback is built, for its test.
#![cfg_attr(not(target_os = "android"), allow(dead_code, unused_imports))]

use libc::{
    c_char, c_int, c_short, c_uint, c_ulong, c_ushort, c_void, ifaddrs, sockaddr, sockaddr_in,
};
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::sync::OnceLock;

type GetFn = unsafe extern "C" fn(*mut *mut ifaddrs) -> c_int;
type FreeFn = unsafe extern "C" fn(*mut ifaddrs);

/// The system's functions, when libc has them (Android 7+).
#[cfg(target_os = "android")]
fn system() -> Option<(GetFn, FreeFn)> {
    static SYS: OnceLock<Option<(usize, usize)>> = OnceLock::new();
    let (g, f) = (*SYS.get_or_init(|| unsafe {
        let libc = libc::dlopen(c"libc.so".as_ptr(), libc::RTLD_NOW);
        if libc.is_null() {
            return None;
        }
        let g = libc::dlsym(libc, c"getifaddrs".as_ptr());
        let f = libc::dlsym(libc, c"freeifaddrs".as_ptr());
        (!g.is_null() && !f.is_null()).then_some((g as usize, f as usize))
    }))?;
    // SAFETY: the symbols come from libc and have these C signatures.
    unsafe {
        Some((
            std::mem::transmute::<usize, GetFn>(g),
            std::mem::transmute::<usize, FreeFn>(f),
        ))
    }
}

const IFNAMSIZ: usize = 16;
const SIOCGIFCONF: c_int = 0x8912;
const SIOCGIFFLAGS: c_int = 0x8913;
const SIOCGIFBRDADDR: c_int = 0x8919;
const SIOCGIFNETMASK: c_int = 0x891b;
const MAX_IFS: usize = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct IfMap {
    mem_start: c_ulong,
    mem_end: c_ulong,
    base_addr: c_ushort,
    irq: u8,
    dma: u8,
    port: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
union IfrIfru {
    addr: sockaddr,
    flags: c_short,
    map: IfMap,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IfReq {
    name: [c_char; IFNAMSIZ],
    ifru: IfrIfru,
}

#[repr(C)]
struct IfConf {
    len: c_int,
    buf: *mut c_void,
}

/// One entry of the list: the `ifaddrs` and the memory it points to. The whole
/// list is one `malloc` block, so freeing the first entry frees everything.
#[repr(C)]
struct Node {
    ifa: ifaddrs,
    name: [c_char; IFNAMSIZ + 1],
    addr: sockaddr_in,
    mask: sockaddr_in,
    brd: sockaddr_in,
}

fn as_in(a: &sockaddr) -> sockaddr_in {
    // SAFETY: sockaddr and sockaddr_in have the same size; the family says it is IPv4.
    unsafe { std::mem::transmute_copy(a) }
}

/// Android 6: IPv4 interfaces from SIOCGIFCONF.
unsafe fn fallback(out: *mut *mut ifaddrs) -> c_int {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return -1;
        }
        let mut reqs: [IfReq; MAX_IFS] = zeroed();
        let mut conf = IfConf {
            len: size_of::<[IfReq; MAX_IFS]>() as c_int,
            buf: reqs.as_mut_ptr().cast(),
        };
        if libc::ioctl(fd, SIOCGIFCONF as _, &mut conf) < 0 {
            libc::close(fd);
            return -1;
        }
        let count = (conf.len.max(0) as usize / size_of::<IfReq>()).min(MAX_IFS);
        let nodes = if count == 0 {
            null_mut()
        } else {
            libc::calloc(count, size_of::<Node>()) as *mut Node
        };
        if count > 0 && nodes.is_null() {
            libc::close(fd);
            return -1;
        }
        let mut used = 0;
        for req in &reqs[..count] {
            if req.ifru.addr.sa_family as c_int != libc::AF_INET {
                continue;
            }
            let n = &mut *nodes.add(used);
            n.name[..IFNAMSIZ].copy_from_slice(&req.name);
            n.addr = as_in(&req.ifru.addr);

            let mut q = *req;
            let flags = if libc::ioctl(fd, SIOCGIFFLAGS as _, &mut q) == 0 {
                q.ifru.flags as u16 as c_uint
            } else {
                0
            };
            let mut q = *req;
            let has_mask = libc::ioctl(fd, SIOCGIFNETMASK as _, &mut q) == 0;
            if has_mask {
                n.mask = as_in(&q.ifru.addr);
                n.mask.sin_family = libc::AF_INET as _;
            }
            let mut q = *req;
            let has_brd = flags & libc::IFF_BROADCAST as c_uint != 0
                && libc::ioctl(fd, SIOCGIFBRDADDR as _, &mut q) == 0;
            if has_brd {
                n.brd = as_in(&q.ifru.addr);
                n.brd.sin_family = libc::AF_INET as _;
            }

            n.ifa.ifa_name = n.name.as_mut_ptr();
            n.ifa.ifa_flags = flags;
            n.ifa.ifa_addr = (&mut n.addr as *mut sockaddr_in).cast();
            n.ifa.ifa_netmask = if has_mask {
                (&mut n.mask as *mut sockaddr_in).cast()
            } else {
                null_mut()
            };
            n.ifa.ifa_ifu = if has_brd {
                (&mut n.brd as *mut sockaddr_in).cast()
            } else {
                null_mut()
            };
            if used > 0 {
                (*nodes.add(used - 1)).ifa.ifa_next = &mut n.ifa;
            }
            used += 1;
        }
        libc::close(fd);
        if used == 0 && !nodes.is_null() {
            libc::free(nodes.cast());
            *out = null_mut();
        } else {
            *out = nodes.cast(); // `ifa` is the first field of Node
        }
        0
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getifaddrs(out: *mut *mut ifaddrs) -> c_int {
    unsafe {
        match system() {
            Some((get, _)) => get(out),
            None => fallback(out),
        }
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn freeifaddrs(list: *mut ifaddrs) {
    unsafe {
        match system() {
            Some((_, free)) => free(list),
            None => libc::free(list.cast()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (name, address, netmask, broadcast) of an IPv4 entry.
    type Ipv4Entry = (String, [u8; 4], Option<[u8; 4]>, Option<[u8; 4]>);

    /// The IPv4 entries of a list.
    unsafe fn ipv4(list: *mut ifaddrs) -> Vec<Ipv4Entry> {
        unsafe fn ip(a: *mut sockaddr) -> Option<[u8; 4]> {
            unsafe { (!a.is_null()).then(|| as_in(&*a).sin_addr.s_addr.to_ne_bytes()) }
        }
        let mut v = Vec::new();
        let mut p = list;
        unsafe {
            while !p.is_null() {
                let a = &*p;
                if !a.ifa_addr.is_null() && (*a.ifa_addr).sa_family as c_int == libc::AF_INET {
                    let brd = if a.ifa_flags & libc::IFF_BROADCAST as c_uint != 0 {
                        ip(a.ifa_ifu)
                    } else {
                        None
                    };
                    v.push((
                        std::ffi::CStr::from_ptr(a.ifa_name)
                            .to_string_lossy()
                            .into_owned(),
                        ip(a.ifa_addr).unwrap(),
                        ip(a.ifa_netmask),
                        brd,
                    ));
                }
                p = a.ifa_next;
            }
        }
        v.sort();
        v
    }

    #[test]
    fn fallback_matches_the_system_ipv4_list() {
        unsafe {
            let mut ours = null_mut();
            assert_eq!(fallback(&mut ours), 0);
            let mut sys = null_mut();
            assert_eq!(libc::getifaddrs(&mut sys), 0);
            let (a, b) = (ipv4(ours), ipv4(sys));
            libc::free(ours.cast());
            libc::freeifaddrs(sys);
            assert!(a.iter().any(|i| i.0 == "lo"), "{a:?}");
            assert_eq!(a, b);
        }
    }
}
