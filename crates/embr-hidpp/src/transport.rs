//! Talking to a single hidraw node.
//!
//! A `Channel` owns one open `/dev/hidrawN` for as long as it is alive. Requests and
//! replies are matched by feature index, function and a rotating software id, and every
//! request drains pending notifications first — a stale reply matched to the wrong request
//! is how a battery monitor ends up showing another device's percentage.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::report::{FeatureCall, FeatureReply, LONG_LEN, RegisterCall};
use crate::{Error, Result};

/// A device call may need to wake a sleeping wireless device: a receiver round trip can
/// take over a second, and a sleeping device answers late or not at all.
pub const DEVICE_TIMEOUT: Duration = Duration::from_millis(4000);
/// The receiver itself answers quickly when it answers at all.
pub const RECEIVER_TIMEOUT: Duration = Duration::from_millis(900);
/// What the second attempt at the other report size gets (see [`Channel::call_feature`]).
///
/// Short on purpose: a device that answers the other size answers it in milliseconds — the G733
/// headset on this machine replies to a long request in 2 ms and never to a short one — so there
/// is nothing to wait for. A device that is simply not there would otherwise spend a whole second
/// timeout proving it again. Measured: this is what made a pass over three quiet devices take
/// 2.9 s instead of 1.2 s, and a sweep of them 11 s instead of 5 s.
const OTHER_SIZE_TIMEOUT: Duration = Duration::from_millis(500);

/// Which report size each device answers, remembered for the life of the process.
///
/// The size is a property of the device, not of a channel — [`Channel::call_feature`] says why —
/// but discovering it costs a timeout, and a channel is opened fresh for every pass over every
/// device. Without this memory, every sweep of a device that has gone quiet pays twice: once for
/// the size it does not answer, and once for the size it does. The key is the node and the HID++
/// index on it, because one receiver node talks to several devices.
fn preferences() -> &'static Mutex<HashMap<String, bool>> {
    static PREFERENCES: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    PREFERENCES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The size this device last answered at, if it has ever answered.
fn remembered(key: &str) -> Option<bool> {
    preferences().lock().ok()?.get(key).copied()
}

/// Note which size a device answered at.
fn remember(key: &str, long: bool) {
    if let Ok(mut preferences) = preferences().lock() {
        preferences.insert(key.to_string(), long);
    }
}

/// An open hidraw node.
pub struct Channel {
    path: PathBuf,
    file: File,
    software_id: u8,
    /// Whether to address this node with long reports. Set from the first timeout rather than
    /// from a protocol version we would have to ask for, so one channel keeps one answer.
    prefer_long: bool,
    /// `path`, as the first half of a [`preferences`] key.
    key: String,
}

impl Channel {
    /// Open a node read/write. Fails on a node that is not a HID++ interface.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let key = path.display().to_string();
        let file = OpenOptions::new().read(true).write(true).open(&path)?;
        Ok(Self {
            path,
            file,
            software_id: 0x8,
            prefer_long: true,
            key,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Rotate through `0x8..=0xF`.
    ///
    /// HID++ 2.0 wants bit 3 set in the software id so that a reply cannot be mistaken for a
    /// notification; the reference implementation sends one fixed id in that range, this crate
    /// rotates within it so a late reply to an identical earlier request cannot be matched to
    /// the one we just sent.
    fn next_software_id(&mut self) -> u8 {
        let id = self.software_id;
        self.software_id = if self.software_id >= 0xF {
            0x8
        } else {
            self.software_id + 1
        };
        id
    }

    /// Send a hand-built frame and return whatever the node answers with, ignoring matching.
    ///
    /// Diagnostics only: a device that answers nothing to [`Self::call_feature`] needs to be
    /// asked with a different report size or software id, which the matching path does not
    /// expose.
    pub fn call_raw(&mut self, frame: &[u8], timeout: Duration) -> Result<Vec<u8>> {
        self.send(frame)?;
        let deadline = Instant::now() + timeout;
        self.receive(deadline, timeout)
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
        // Which request size a device answers is a property of the device, not of how much we
        // are sending: the reference implementation picks it from the protocol version, and a
        // 2-byte ROOT lookup sent short is simply ignored — the G733 headset on this machine
        // answers long requests in 2 ms and never answers a short one. A device that only
        // answers short reports costs one timeout to discover, and the answer is remembered for
        // the next pass over it, because a fresh channel per pass would otherwise pay again.
        let key = format!("{}#{}", self.key, device);
        let proven = remembered(&key);
        let prefer_long = proven.unwrap_or(self.prefer_long);
        let long = prefer_long && params.len() <= LONG_LEN - 4;
        match self.exchange(&call, long, timeout) {
            Ok(reply) => {
                self.learn(&key, long);
                Ok(reply)
            }
            Err(Error::Timeout { .. }) if proven == Some(long) => {
                // This device has answered at this very size before, so the silence is the
                // device, not the report size: it has stopped answering and asking it the same
                // question in the other size would only buy a second timeout.
                Err(Error::Timeout { waited: timeout })
            }
            Err(Error::Timeout { .. }) if long => {
                // The other size gets one cheap attempt and no more: a device that answers it
                // answers at once (2 ms, measured), and one that is not there should not be
                // asked twice at full price.
                let reply = self.exchange(&call, false, timeout.min(OTHER_SIZE_TIMEOUT))?;
                self.learn(&key, false);
                Ok(reply)
            }
            other => other,
        }
    }

    /// Note which size this device answers at: for the rest of this channel, and for the next
    /// pass over the same node and device index.
    fn learn(&mut self, key: &str, long: bool) {
        self.prefer_long = long;
        remember(key, long);
    }

    /// Send one request frame and wait for the reply that belongs to it.
    fn exchange(
        &mut self,
        call: &FeatureCall,
        long: bool,
        timeout: Duration,
    ) -> Result<FeatureReply> {
        let frame: Vec<u8> = if long {
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
                    if parsed.feature_index != call.feature_index
                        || parsed.function != call.function
                    {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_size_is_remembered_per_node_and_device() {
        // One receiver node talks to several devices, and the same index on another node is a
        // different device: a remembered preference may not leak between them.
        let one = "/dev/hidraw-remember-test-a#255";
        let other_node = "/dev/hidraw-remember-test-b#255";
        let other_device = "/dev/hidraw-remember-test-a#1";
        assert_eq!(
            remembered(one),
            None,
            "nothing is known before anyone answers"
        );
        remember(one, false);
        assert_eq!(
            remembered(one),
            Some(false),
            "a device that answered short is asked short next time"
        );
        assert_eq!(
            remembered(other_node),
            None,
            "another node knows nothing of it"
        );
        assert_eq!(
            remembered(other_device),
            None,
            "nor another device on the same node"
        );
        remember(one, true);
        assert_eq!(
            remembered(one),
            Some(true),
            "the latest answer is what counts"
        );
    }
}
