//! Talking to a single hidraw node.
//!
//! A `Channel` owns one open `/dev/hidrawN` for as long as it is alive. Requests and
//! replies are matched by feature index, function and a rotating software id, and every
//! request drains pending notifications first — a stale reply matched to the wrong request
//! is how a battery monitor ends up showing another device's percentage.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::report::{FeatureCall, FeatureReply, LONG_LEN, RegisterCall};
use crate::{Error, Result};

/// A device call may need to wake a sleeping wireless device: a receiver round trip can
/// take over a second, and a sleeping device answers late or not at all.
pub const DEVICE_TIMEOUT: Duration = Duration::from_millis(4000);
/// The receiver itself answers quickly when it answers at all.
pub const RECEIVER_TIMEOUT: Duration = Duration::from_millis(900);

/// An open hidraw node.
pub struct Channel {
    path: PathBuf,
    file: File,
    software_id: u8,
}

impl Channel {
    /// Open a node read/write. Fails on a node that is not a HID++ interface.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().read(true).write(true).open(&path)?;
        Ok(Self {
            path,
            file,
            software_id: 0x1,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Rotate through `1..=7`. Kept below 8 so a reply can never be confused with a
    /// notification, and rotated per call so a late reply to an identical previous request
    /// cannot be matched to this one.
    fn next_software_id(&mut self) -> u8 {
        let id = self.software_id;
        self.software_id = if self.software_id >= 7 {
            1
        } else {
            self.software_id + 1
        };
        id
    }

    /// Call a HID++ 2.0 feature function and return the decoded reply.
    pub fn call_feature(
        &mut self,
        device: u8,
        feature_index: u8,
        function: u8,
        params: &[u8],
        timeout: Duration,
    ) -> Result<FeatureReply> {
        let mut call = FeatureCall::new(device, feature_index, function, params);
        call.software_id = self.next_software_id();
        let frame: Vec<u8> = if params.len() > 3 {
            call.to_long().to_vec()
        } else {
            call.to_short().to_vec()
        };
        self.send(&frame)?;

        let deadline = Instant::now() + timeout;
        // If a device answers without echoing our software id (they are not all well
        // behaved), keep the best looser match and use it rather than timing out.
        let mut loose: Option<FeatureReply> = None;
        loop {
            match self.receive(deadline, timeout) {
                Ok(reply) => {
                    let Some(parsed) = FeatureReply::parse(&reply) else {
                        continue;
                    };
                    if parsed.feature_index != feature_index || parsed.function != function {
                        continue;
                    }
                    if parsed.software_id == call.software_id {
                        return Ok(parsed);
                    }
                    loose = Some(parsed);
                }
                Err(Error::Timeout { .. }) => {
                    return match loose {
                        Some(parsed) => Ok(parsed),
                        None => Err(Error::Timeout { waited: timeout }),
                    };
                }
                Err(other) => return Err(other),
            }
        }
    }

    /// Read a HID++ 1.0 register and return the payload after the echoed request id.
    ///
    /// The reply is *not* interpreted here: some devices answer an unsupported register
    /// with a short error payload instead of a marker, so callers validate the shape of
    /// what they expect (which is also how a wrong register id shows itself).
    pub fn read_register(
        &mut self,
        device: u8,
        register: u16,
        params: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let call = RegisterCall::new(device, register, params);
        self.send(&call.to_short())?;
        let deadline = Instant::now() + timeout;
        loop {
            let reply = self.receive(deadline, timeout)?;
            if call.matches(&reply) {
                return Ok(reply[4..].to_vec());
            }
        }
    }

    fn send(&mut self, frame: &[u8]) -> Result<()> {
        self.drain();
        self.file.write_all(frame)?;
        self.file.flush()?;
        Ok(())
    }

    /// Discard anything already queued (notifications, replies to earlier requests).
    fn drain(&mut self) {
        let mut buf = [0u8; 64];
        while let Ok(true) = poll_readable(self.file.as_raw_fd(), Duration::ZERO) {
            match self.file.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
    }

    fn receive(&mut self, deadline: Instant, budget: Duration) -> Result<Vec<u8>> {
        let mut buf = [0u8; 64];
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(Error::Timeout { waited: budget });
            }
            if !poll_readable(self.file.as_raw_fd(), deadline - now)? {
                return Err(Error::Timeout { waited: budget });
            }
            match self.file.read(&mut buf) {
                Ok(0) => continue,
                Ok(n) => {
                    let mut frame = buf[..n].to_vec();
                    // Kernel may hand back more than one report per read.
                    frame.truncate(LONG_LEN);
                    return Ok(frame);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(Error::Io(e)),
            }
        }
    }
}

/// `poll(2)` on a raw fd, retrying on `EINTR`.
pub fn poll_readable(fd: i32, timeout: Duration) -> Result<bool> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
    loop {
        let rc = unsafe { libc::poll(&mut pfd, 1, millis) };
        if rc == -1 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(Error::Io(err));
        }
        return Ok(rc > 0 && pfd.revents & libc::POLLIN != 0);
    }
}
