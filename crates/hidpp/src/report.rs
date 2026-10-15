//! HID++ frame layouts.
//!
//! A short report is 7 bytes and a long report is 20; on the wire the first byte is the
//! report id (`0x10` short, `0x11` long), the second is the device index, and then either a
//! feature call or a register access follows:
//!
//! ```text
//! feature call : 10 <dev> <feature-index> <function<<4 | software-id> <params...>
//! register I/O : 10 <dev> <0x81|0x83> <register-low-byte>                <params...>
//! reply        : 11 <dev> <feature-index> <function<<4 | software-id> <payload...>
//! ```
//!
//! Device index `0xFF` addresses the node itself; a device paired to a receiver answers
//! under its slot number (`0x01`..`0x06`), and — this is the part that is easy to get
//! wrong — it answers under that slot number *even when the request goes to the device's
//! own hidraw node*, because the node is only a view onto the receiver.

/// Report id of a 7-byte short report.
pub const SHORT_REPORT_ID: u8 = 0x10;
/// Report id of a 20-byte long report.
pub const LONG_REPORT_ID: u8 = 0x11;

pub const SHORT_LEN: usize = 7;
pub const LONG_LEN: usize = 20;

/// Device index meaning "the node itself" rather than a paired device.
pub const DEVICE_SELF: u8 = 0xFF;
/// Highest receiver slot we probe for paired devices.
pub const MAX_DEVICE_SLOTS: u8 = 6;

/// Marker byte that starts a HID++ 2.0 error payload.
pub const ERROR_MARKER: u8 = 0x8F;

/// HID++ 2.0 feature ids we use. Ids are protocol facts; the code around them is ours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub enum Feature {
    /// `0x0000` — root feature, used to look up every other feature's index.
    Root = 0x0000,
    /// `0x0001` — feature set: the list of features a device supports.
    FeatureSet = 0x0001,
    /// `0x0003` — firmware/unit/model ids.
    DeviceFirmwareVersion = 0x0003,
    /// `0x0005` — the device's own idea of its name and kind.
    DeviceName = 0x0005,
    /// `0x0020` — configuration change counter.
    ConfigChange = 0x0020,
    /// `0x1000` — battery level as a percentage plus a charge status.
    BatteryStatus = 0x1000,
    /// `0x1001` — battery voltage in millivolts plus a charge status.
    BatteryVoltage = 0x1001,
    /// `0x1004` — unified battery (level, voltage and flags in one call).
    UnifiedBattery = 0x1004,
    /// `0x1F20` — analogue measurement, which is how headsets report their battery:
    /// the same `millivolts flags` shape as `BATTERY_VOLTAGE`, but different flag bits.
    AdcMeasurement = 0x1F20,
    /// `0x1D4B` — wireless connection status (encryption, RSSI, …).
    WirelessDeviceStatus = 0x1D4B,
    /// `0x1814` — change host / connection state.
    ChangeHost = 0x1814,
    /// `0x2201` — adjustable DPI.
    AdjustableDpi = 0x2201,
    /// `0x8100` — onboard profiles.
    OnboardProfiles = 0x8100,
    /// `0x8070`/`0x8080`/`0x8071` — RGB effects.
    RgbEffects = 0x8071,
    /// `0x1B04` — special/multiplatform F-key behaviour.
    SpecialKeys = 0x1B04,
}

impl Feature {
    pub fn id(self) -> u16 {
        self as u16
    }

    /// Human-readable feature name, from the HID++ feature catalogue.
    pub fn name(self) -> &'static str {
        match self {
            Feature::Root => "ROOT",
            Feature::FeatureSet => "FEATURE SET",
            Feature::DeviceFirmwareVersion => "DEVICE FW VERSION",
            Feature::DeviceName => "DEVICE NAME",
            Feature::ConfigChange => "CONFIG CHANGE",
            Feature::BatteryStatus => "BATTERY STATUS",
            Feature::BatteryVoltage => "BATTERY VOLTAGE",
            Feature::UnifiedBattery => "UNIFIED BATTERY",
            Feature::AdcMeasurement => "ADC MEASUREMENT",
            Feature::WirelessDeviceStatus => "WIRELESS DEVICE STATUS",
            Feature::ChangeHost => "CHANGE HOST",
            Feature::AdjustableDpi => "ADJUSTABLE DPI",
            Feature::OnboardProfiles => "ONBOARD PROFILES",
            Feature::RgbEffects => "RGB EFFECTS",
            Feature::SpecialKeys => "SPECIAL KEYS",
        }
    }

    /// Name for an arbitrary id, including ones this crate does not use yet.
    pub fn name_of(id: u16) -> &'static str {
        match id {
            0x0000 => "ROOT",
            0x0001 => "FEATURE SET",
            0x0003 => "DEVICE FW VERSION",
            0x0005 => "DEVICE NAME",
            0x0020 => "CONFIG CHANGE",
            0x1000 => "BATTERY STATUS",
            0x1001 => "BATTERY VOLTAGE",
            0x1004 => "UNIFIED BATTERY",
            0x1F20 => "ADC MEASUREMENT",
            0x1D4B => "WIRELESS DEVICE STATUS",
            0x1814 => "CHANGE HOST",
            0x1B04 => "SPECIAL KEYS",
            0x2111 => "SMART SHIFT",
            0x2121 => "SCROLL INVERSION",
            0x2201 => "ADJUSTABLE DPI",
            0x8070 => "RGB EFFECTS (old)",
            0x8071 => "RGB EFFECTS",
            0x8080 => "RGB EFFECTS (keyboard)",
            0x8100 => "ONBOARD PROFILES",
            _ => "unknown feature",
        }
    }
}

/// A feature call to send: `<feature-index> <function<<4 | software-id> <params>`.
#[derive(Clone, Debug)]
pub struct FeatureCall {
    pub device: u8,
    pub feature_index: u8,
    pub function: u8,
    pub software_id: u8,
    pub params: Vec<u8>,
}

impl FeatureCall {
    /// A call with the software id fixed at 0; the function byte is what matters here.
    pub fn new(device: u8, feature_index: u8, function: u8, params: &[u8]) -> Self {
        Self {
            device,
            feature_index,
            function,
            software_id: 0,
            params: params.to_vec(),
        }
    }

    fn function_byte(&self) -> u8 {
        (self.function << 4) | (self.software_id & 0x0F)
    }

    /// The device index a matching reply must carry.
    pub fn expect_device(&self) -> u8 {
        self.device
    }

    /// Encode as a short report. Panics if the parameters do not fit in 3 bytes.
    pub fn to_short(&self) -> [u8; SHORT_LEN] {
        assert!(
            self.params.len() <= 3,
            "short report carries at most 3 parameter bytes"
        );
        let mut out = [0u8; SHORT_LEN];
        out[0] = SHORT_REPORT_ID;
        out[1] = self.device;
        out[2] = self.feature_index;
        out[3] = self.function_byte();
        out[4..4 + self.params.len()].copy_from_slice(&self.params);
        out
    }

    /// Encode as a long report. Panics if the parameters do not fit in 16 bytes.
    pub fn to_long(&self) -> [u8; LONG_LEN] {
        assert!(
            self.params.len() <= 16,
            "long report carries at most 16 parameter bytes"
        );
        let mut out = [0u8; LONG_LEN];
        out[0] = LONG_REPORT_ID;
        out[1] = self.device;
        out[2] = self.feature_index;
        out[3] = self.function_byte();
        out[4..4 + self.params.len()].copy_from_slice(&self.params);
        out
    }
}

/// A register access: `<request-id-high> <request-id-low> <params>`.
///
/// The request id adds `0x200` for anything above `0x7F`, which is why register `0x2B5`
/// goes out as `0x83B5` and not `0x81B5`; sending the wrong id earns HID++ 1.0 error
/// `0x01` and nothing else.
#[derive(Clone, Debug)]
pub struct RegisterCall {
    pub device: u8,
    pub register: u16,
    pub params: Vec<u8>,
}

impl RegisterCall {
    pub fn new(device: u8, register: u16, params: &[u8]) -> Self {
        Self {
            device,
            register,
            params: params.to_vec(),
        }
    }

    /// The two-byte request id, as it appears in the frame.
    pub fn request_id(&self) -> u16 {
        0x8100 | (self.register & 0x2FF)
    }

    /// Register reads go out as SHORT reports even for registers above `0xFF`: these
    /// receivers do not answer a long-form register read.
    pub fn to_short(&self) -> [u8; SHORT_LEN] {
        assert!(
            self.params.len() <= 3,
            "short report carries at most 3 parameter bytes"
        );
        let id = self.request_id().to_be_bytes();
        let mut out = [0u8; SHORT_LEN];
        out[0] = SHORT_REPORT_ID;
        out[1] = self.device;
        out[2] = id[0];
        out[3] = id[1];
        out[4..4 + self.params.len()].copy_from_slice(&self.params);
        out
    }

    /// Whether a reply frame belongs to this call.
    pub fn matches(&self, reply: &[u8]) -> bool {
        let id = self.request_id().to_be_bytes();
        reply.len() >= 6 && reply[2] == id[0] && reply[3] == id[1]
    }
}

/// A decoded reply to a feature call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureReply {
    pub device: u8,
    pub feature_index: u8,
    pub function: u8,
    pub software_id: u8,
    pub payload: Vec<u8>,
}

impl FeatureReply {
    /// Decode a frame as a feature reply, or `None` if it is not shaped like one.
    pub fn parse(frame: &[u8]) -> Option<Self> {
        if frame.len() < SHORT_LEN {
            return None;
        }
        Some(Self {
            device: frame[1],
            feature_index: frame[2],
            function: frame[3] >> 4,
            software_id: frame[3] & 0x0F,
            payload: frame[4..].to_vec(),
        })
    }

    /// The HID++ 2.0 error code, if this reply is an error.
    ///
    /// Only the documented marker is accepted here. Some devices answer an unsupported
    /// call with the HID++ 1.0 style payload instead, which is indistinguishable from a
    /// legitimate payload by shape alone — [`looks_like_error_payload`] handles that case
    /// where the caller knows what it expected.
    pub fn error_code(&self) -> Option<u8> {
        match self.payload.first() {
            Some(&ERROR_MARKER) => self.payload.get(1).copied(),
            _ => None,
        }
    }

    /// The reply payload, or the protocol error as an [`crate::Error`].
    pub fn payload_or_error(&self) -> crate::Result<&[u8]> {
        match self.error_code() {
            Some(code) => Err(crate::Error::Protocol2 {
                code,
                name: crate::protocol2_error_name(code),
            }),
            None => Ok(&self.payload),
        }
    }

    /// Read a big-endian `u16` from the payload.
    pub fn u16_at(&self, offset: usize) -> Option<u16> {
        let bytes = self.payload.get(offset..offset + 2)?;
        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }
}

/// Whether a payload looks like an error rather than data.
///
/// Two shapes are seen on real hardware: the documented `8F <code>` marker, and a shorter
/// HID++ 1.0 style `<code> 00` reply that some devices and firmwares use instead. The
/// second form is only a *heuristic* — a battery voltage payload can legitimately start
/// with the same byte — so it belongs at call sites that know what a valid answer looks
/// like (a feature index, a register record) and never in a decoder.
pub fn looks_like_error_payload(payload: &[u8]) -> Option<u8> {
    match payload {
        [ERROR_MARKER, code, ..] => Some(*code),
        // 0x00 is "no error", so a payload shaped like a valid `index type version`
        // reply — for example `[0x0F, 0x00, 0x00]` — is not treated as an error.
        [first, code, 0x00, ..] if *first == 0x0F && (1..=0x0F).contains(code) => Some(*code),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_feature_call_is_encoded_as_measured() {
        // ROOT.getFeature(0x1001) as it appears on the wire:
        //   11 01 00 0A 10 01 …  (long, device 1, feature 0, function 0, params 10 01)
        let call = FeatureCall::new(0x01, 0x00, 0x00, &[0x10, 0x01]);
        assert_eq!(call.to_long()[..6], [0x11, 0x01, 0x00, 0x00, 0x10, 0x01]);
        assert_eq!(call.to_short(), [0x10, 0x01, 0x00, 0x00, 0x10, 0x01, 0x00]);
    }

    #[test]
    fn register_request_id_carries_the_high_bit() {
        // register 0x2B5 -> request id 0x83B5, never 0x81B5
        assert_eq!(RegisterCall::new(0xFF, 0x2B5, &[0x20]).request_id(), 0x83B5);
        assert_eq!(RegisterCall::new(0xFF, 0x07, &[]).request_id(), 0x8107);
        let frame = RegisterCall::new(0xFF, 0x2B5, &[0x20, 0x00, 0x00]).to_short();
        assert_eq!(frame, [0x10, 0xFF, 0x83, 0xB5, 0x20, 0x00, 0x00]);
    }

    #[test]
    fn replies_are_matched_on_the_request_id() {
        let call = RegisterCall::new(0xFF, 0x2B5, &[0x20]);
        // verbatim wire capture, G703 LIGHTSPEED pairing info
        let reply = [
            0x11, 0xFF, 0x83, 0xB5, 0x20, 0x07, 0x08, 0x40, 0x86, 0x0C, 0x01, 0x02, 0xB3,
        ];
        assert!(call.matches(&reply));
        assert!(!call.matches(&[0x11, 0xFF, 0x81, 0xB5, 0x20, 0x00]));
    }

    #[test]
    fn battery_voltage_reply_decodes_to_millivolts() {
        // verbatim wire capture: G703 LIGHTSPEED, 0x1001 at feature index 6, "0F D2 00"
        let reply = FeatureReply::parse(&[0x11, 0x01, 0x06, 0x09, 0x0F, 0xD2, 0x00]).unwrap();
        assert_eq!(reply.payload_or_error().unwrap(), &[0x0F, 0xD2, 0x00]);
        assert_eq!(reply.u16_at(0), Some(0x0FD2)); // 4050 mV, big endian
        assert_eq!(reply.error_code(), None);
    }

    #[test]
    fn error_replies_are_recognised_in_both_shapes() {
        use crate::report::looks_like_error_payload;
        // the documented marker form
        let marked = FeatureReply::parse(&[0x11, 0x01, 0x08, 0x0F, 0x8F, 0x05, 0x00]).unwrap();
        assert_eq!(marked.error_code(), Some(0x05));
        assert_eq!(looks_like_error_payload(&marked.payload), Some(0x05));
        assert!(marked.payload_or_error().is_err());
        // the HID++ 1.0 style form, as captured from a G733 answering an unsupported call
        let unmarked = FeatureReply::parse(&[0x11, 0xFF, 0xFF, 0x08, 0x0F, 0x05, 0x00]).unwrap();
        assert_eq!(
            unmarked.error_code(),
            None,
            "only the marker is authoritative"
        );
        assert_eq!(looks_like_error_payload(&unmarked.payload), Some(0x05));
        // …and the heuristic must not swallow a real payload: this is a G703's 4050 mV
        // voltage reply, which also starts with 0x0F
        let payload = FeatureReply::parse(&[0x11, 0x01, 0x06, 0x09, 0x0F, 0xD2, 0x00]).unwrap();
        assert_eq!(payload.error_code(), None);
        assert_eq!(looks_like_error_payload(&payload.payload), None);
        assert_eq!(payload.u16_at(0), Some(0x0FD2));
        // a genuine `index type version` reply with index 15 and version 0 is not an error
        assert_eq!(looks_like_error_payload(&[0x0F, 0x00, 0x00]), None);
    }
}
