//! Device presence, from the kernel rather than from asking the hardware over and over.
//!
//! Linux publishes a uevent for every device that appears or disappears, on a netlink multicast
//! group any user may bind to. That is a better answer than polling: a paired device waking up,
//! a receiver being plugged in, a headset being unplugged — the kernel has already seen it, and a
//! listener costs nothing until it happens. Asking the devices instead costs a wake-up per device
//! per pass, which on a wireless mouse is its battery.
//!
//! This module is deliberately dumb: it reports *that* something HID-shaped changed, never which
//! device it was. The sweep that follows turns it into a device list, and the device list is the
//! only thing that knows how to read a device — the same reason [`crate::discover`] exists.
//!
//! HID++ itself can say more than this (a receiver announces a device connecting on its own
//! hidraw node) but reading those reports means holding the receiver open, and this crate's other
//! half is built on the opposite rule: open what a query needs, close it, hold nothing.
//! `logitech-djreceiver` in the kernel turns those same announcements into real HID devices, so
//! the events below arrive anyway, without competing with the kernel driver that owns the node.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// A listening socket on the kernel's uevent stream.
#[derive(Debug)]
pub struct Uevents {
    socket: OwnedFd,
}

impl Uevents {
    /// Listen for HID devices appearing and disappearing.
    ///
    /// A failure here is not a failure of the application: without it the app behaves exactly as
    /// it did before — sweeps on a cadence — so the caller reports it and carries on.
    pub fn open() -> io::Result<Self> {
        // NETLINK_KOBJECT_UEVENT has no `libc` constant on every target, but the protocol number
        // is fixed in the kernel's uapi headers.
        const NETLINK_KOBJECT_UEVENT: libc::c_int = 15;

        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                NETLINK_KOBJECT_UEVENT,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // Take the descriptor before anything else can fail, so it is closed on the way out.
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };

        // Group 1 is the kernel's uevent group. Binding to it needs no privilege; *sending* on
        // this protocol does, which is why this can only ever listen.
        //
        // `sockaddr_nl`'s padding field is private, so the struct is zeroed and then filled in.
        let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        address.nl_groups = 1;
        let bound = unsafe {
            libc::bind(
                socket.as_raw_fd(),
                std::ptr::from_ref(&address).cast::<libc::sockaddr>(),
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if bound < 0 {
            let problem = io::Error::last_os_error();
            return Err(problem);
        }
        Ok(Self { socket })
    }

    /// Everything that has arrived since the last call: `true` when a HID device came or went.
    ///
    /// Drained every time it is called, so the answer is about the last tick's worth of events
    /// and one change does not leave a queue behind that would trigger sweeps for a while after.
    pub fn hid_changed(&self) -> bool {
        let mut changed = false;
        let mut buffer = [0u8; 4096];
        loop {
            let read = unsafe {
                libc::recv(
                    self.socket.as_raw_fd(),
                    buffer.as_mut_ptr().cast::<libc::c_void>(),
                    buffer.len(),
                    0,
                )
            };
            // -1 is "nothing left" (the socket is non-blocking) as well as a real error: either
            // way there is nothing more to read now.
            if read <= 0 {
                break;
            }
            if is_hid_change(&buffer[..read as usize]) {
                changed = true;
            }
        }
        changed
    }
}

/// Whether a single uevent is a HID device appearing or going.
///
/// A uevent is `action@devpath` and then NUL-separated `KEY=VALUE` fields; the same fields are
/// what `udevadm monitor` prints. Only `hid` and `hidraw` matter here: the first is the device
/// itself (`0003:046D:4086.0009`, a device a receiver has just announced) and the second its
/// node. `input` deliberately does not count — a keyboard's own input node exists for a
/// completely different client, and `net`/`usb`/`power_supply` are none of this app's business.
fn is_hid_change(message: &[u8]) -> bool {
    let text = String::from_utf8_lossy(message);
    let mut subsystem = None;
    let mut action = None;
    for field in text.split('\0') {
        if let Some(value) = field.strip_prefix("SUBSYSTEM=") {
            subsystem = Some(value);
        } else if let Some(value) = field.strip_prefix("ACTION=") {
            action = Some(value);
        }
    }
    // `bind`/`unbind` are left out on purpose: they are a driver taking a device it already has,
    // which changes nothing about whether it can be read.
    matches!(subsystem, Some("hid" | "hidraw")) && matches!(action, Some("add" | "remove"))
}

#[cfg(test)]
mod tests {
    use super::is_hid_change;

    /// Real messages have this shape: the action line first, then the fields, NUL-separated.
    fn uevent(action: &str, devpath: &str, subsystem: &str, extra: &str) -> Vec<u8> {
        format!(
            "{action}@{devpath}\0ACTION={action}\0DEVPATH={devpath}\0SUBSYSTEM={subsystem}\0{extra}\0"
        )
        .into_bytes()
    }

    #[test]
    fn a_device_a_receiver_just_announced_is_a_change() {
        // The G703 behind the Lightspeed receiver: the kernel adds a `hid` device for it and, in
        // turn, a `hidraw` node — which is exactly what "my mouse woke up" looks like.
        assert!(is_hid_change(&uevent(
            "add",
            "/devices/pci0000:00/0000:00:14.0/usb1/1-10/1-10:1.2/0003:046D:C539.0007/0003:046D:4086.0009",
            "hid",
            "HID_ID=0003:0000046D:00004086\0HID_NAME=Logitech G703",
        )));
        assert!(is_hid_change(&uevent(
            "remove",
            "/devices/pci0000:00/0000:00:14.0/usb1/1-10/1-10:1.2/0003:046D:C539.0007/0003:046D:4086.0009/hidraw/hidraw8",
            "hidraw",
            "DEVNAME=hidraw8",
        )));
    }

    #[test]
    fn the_kernel_says_plenty_that_is_none_of_our_business() {
        assert!(!is_hid_change(&uevent(
            "add",
            "/devices/virtual/net/wlan0",
            "net",
            "INTERFACE=wlan0",
        )));
        assert!(!is_hid_change(&uevent(
            "add",
            "/devices/pci0000:00/0000:00:14.0/usb1/1-3/1-3:1.0/0003:046D:C31C.0001/input/input20",
            "input",
            "PRODUCT=3/46d/c31c/110",
        )));
        // A driver taking a device it already has does not change whether it can be read.
        assert!(!is_hid_change(&uevent(
            "bind",
            "/devices/pci0000:00/0000:00:14.0/usb1/1-3/1-3:1.2/0003:046D:C52B.0004",
            "hid",
            "DRIVER=logitech-djreceiver",
        )));
        // A read of a uevent file is a `change`, not a device arriving.
        assert!(!is_hid_change(&uevent(
            "change",
            "/devices/pci0000:00/0000:00:14.0/usb1/1-3/1-3:1.2/0003:046D:C52B.0004/hidraw/hidraw3",
            "hidraw",
            "",
        )));
    }

    #[test]
    fn a_truncated_or_alien_message_is_not_a_change() {
        assert!(!is_hid_change(b""));
        assert!(!is_hid_change(b"\0\0\0"));
        assert!(!is_hid_change(b"add@/devices/x\0ACTION=add\0"));
    }
}
