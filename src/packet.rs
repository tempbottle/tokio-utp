#![allow(dead_code)]

use std::error::Error;
use std::mem;
use std::fmt;
use std::ops::Deref;
use bit_iterator::BitIterator;
use std::slice::Iter;

pub const HEADER_SIZE: usize = 20;

macro_rules! u8_to_unsigned_be {
    ($src:ident, $start:expr, $end:expr, $t:ty) => ({
        (0 .. $end - $start + 1).rev().fold(0, |acc, i| acc | $src[$start+i] as $t << (i * 8))
    })
}

macro_rules! make_getter {
    ($name:ident, $t:ty, $m:ident) => {
        pub fn $name(&self) -> $t {
            $m::from_be(self.header.$name)
        }
    }
}

macro_rules! make_setter {
    ($fn_name:ident, $field:ident, $t: ty) => {
        pub fn $fn_name(&mut self, new: $t) {
            self.header.$field = new.to_be();
        }
    }
}

/// Attempt to construct `Self` through conversion.
///
/// Waiting for rust-lang/rust#33417 to become stable.
pub trait TryFrom<T>: Sized {
    type Err;
    fn try_from(T) -> Result<Self, Self::Err>;
}

/// A trait for objects that can be represented as a vector of bytes.
pub trait Encodable {
    /// Returns a vector of bytes representing the data structure in a way that can be sent over the
    /// network.
    fn to_bytes(&self) -> Vec<u8>;
}

#[derive(Debug)]
pub enum ParseError {
    InvalidExtensionLength,
    InvalidPacketLength,
    InvalidPacketType(u8),
    UnsupportedVersion
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl Error for ParseError {
    fn description(&self) -> &str {
        use self::ParseError::*;
        match *self {
            InvalidExtensionLength => "Invalid extension length (must be a non-zero multiple of 4)",
            InvalidPacketLength => "The packet is too small",
            InvalidPacketType(_) => "Invalid packet type",
            UnsupportedVersion => "Unsupported packet version",
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
pub enum PacketType {
    Data,  // packet carries a data payload
    Fin,   // signals the end of a connection
    State, // signals acknowledgment of a packet
    Reset, // forcibly terminates a connection
    Syn,   // initiates a new connection with a peer
}

impl TryFrom<u8> for PacketType {
    type Err = ParseError;
    fn try_from(original: u8) -> Result<Self, Self::Err> {
        match original {
            0 => Ok(PacketType::Data),
            1 => Ok(PacketType::Fin),
            2 => Ok(PacketType::State),
            3 => Ok(PacketType::Reset),
            4 => Ok(PacketType::Syn),
            n => Err(ParseError::InvalidPacketType(n))
        }
    }
}

impl From<PacketType> for u8 {
    fn from(original: PacketType) -> u8 {
        match original {
            PacketType::Data => 0,
            PacketType::Fin => 1,
            PacketType::State => 2,
            PacketType::Reset => 3,
            PacketType::Syn => 4,
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ExtensionType {
    None,
    SelectiveAck,
    Unknown(u8),
}

impl From<u8> for ExtensionType {
    fn from(original: u8) -> Self {
        match original {
            0 => ExtensionType::None,
            1 => ExtensionType::SelectiveAck,
            n => ExtensionType::Unknown(n),
        }
    }
}

impl From<ExtensionType> for u8 {
    fn from(original: ExtensionType) -> u8 {
        match original {
            ExtensionType::None => 0,
            ExtensionType::SelectiveAck => 1,
            ExtensionType::Unknown(n) => n,
        }
    }
}

#[derive(Clone)]
pub struct Extension {
    ty: ExtensionType,
    pub data: Vec<u8>,
}

impl Extension {
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn get_type(&self) -> ExtensionType {
        self.ty
    }

    pub fn iter(&self) -> BitIterator {
        BitIterator::from_bytes(&self.data)
    }
}

#[derive(Clone, Copy)]
struct PacketHeader {
    type_ver: u8, // type: u4, ver: u4
    extension: u8,
    connection_id: u16,
    timestamp_microseconds: u32,
    timestamp_difference_microseconds: u32,
    wnd_size: u32,
    seq_nr: u16,
    ack_nr: u16,
}

impl PacketHeader {
    /// Sets the type of packet to the specified type.
    pub fn set_type(&mut self, t: PacketType) {
        let version = 0x0F & self.type_ver;
        self.type_ver = u8::from(t) << 4 | version;
    }

    /// Returns the packet's type.
    pub fn get_type(&self) -> PacketType {
        PacketType::try_from(self.type_ver >> 4).unwrap()
    }

    /// Returns the packet's version.
    pub fn get_version(&self) -> u8 {
        self.type_ver & 0x0F
    }
}

impl Deref for PacketHeader {
    type Target = [u8];

    /// Returns the packet header as a slice of bytes.
    fn deref(&self) -> &[u8] {
        unsafe {
            mem::transmute::<&PacketHeader, &[u8; HEADER_SIZE]>(self)
        }
    }
}

impl<'a> TryFrom<&'a[u8]> for PacketHeader {
    type Err = ParseError;
    /// Reads a byte buffer and returns the corresponding packet header.
    /// It assumes the fields are in network (big-endian) byte order,
    /// preserving it.
    fn try_from(buf: &[u8]) -> Result<Self, Self::Err> {
        // Check length
        if buf.len() < HEADER_SIZE {
            return Err(ParseError::InvalidPacketLength);
        }

        // Check version
        if buf[0] & 0x0F != 1 {
            return Err(ParseError::UnsupportedVersion);
        }

        // Check packet type
        if let Err(e) = PacketType::try_from(buf[0] >> 4) {
            return Err(e);
        }

        Ok(PacketHeader {
            type_ver: buf[0],
            extension: buf[1],
            connection_id: u8_to_unsigned_be!(buf, 2, 3, u16),
            timestamp_microseconds: u8_to_unsigned_be!(buf, 4, 7, u32),
            timestamp_difference_microseconds: u8_to_unsigned_be!(buf, 8, 11, u32),
            wnd_size: u8_to_unsigned_be!(buf, 12, 15, u32),
            seq_nr: u8_to_unsigned_be!(buf, 16, 17, u16),
            ack_nr: u8_to_unsigned_be!(buf, 18, 19, u16),
        })
    }
}

impl Default for PacketHeader {
    fn default() -> PacketHeader {
        PacketHeader {
            type_ver: u8::from(PacketType::Data) << 4 | 1,
            extension: 0,
            connection_id: 0,
            timestamp_microseconds: 0,
            timestamp_difference_microseconds: 0,
            wnd_size: 0,
            seq_nr: 0,
            ack_nr: 0,
        }
    }
}

pub struct Packet {
    header: PacketHeader,
    extensions: Vec<Extension>,
    pub payload: Vec<u8>,
}

impl Packet {
    /// Constructs a new, empty packet.
    pub fn new() -> Packet {
        Packet {
            header: PacketHeader::default(),
            extensions: Vec::new(),
            payload: Vec::new(),
        }
    }

    /// Constructs a new data packet with the given payload.
    pub fn with_payload(payload: &[u8]) -> Packet {
        let mut header = PacketHeader::default();
        header.set_type(PacketType::Data);

        let mut p = vec![0; payload.len()];
        p.copy_from_slice(payload);

        Packet {
            header: header,
            extensions: Vec::new(),
            payload: p,
        }
    }

    #[inline]
    pub fn set_type(&mut self, t: PacketType) {
        self.header.set_type(t);
    }

    #[inline]
    pub fn get_type(&self) -> PacketType {
        self.header.get_type()
    }

    pub fn extensions(&self) -> Iter<Extension> {
        self.extensions.iter()
    }

    make_getter!(seq_nr, u16, u16);
    make_getter!(ack_nr, u16, u16);
    make_getter!(connection_id, u16, u16);
    make_getter!(wnd_size, u32, u32);
    make_getter!(timestamp_microseconds, u32, u32);
    make_getter!(timestamp_difference_microseconds, u32, u32);

    make_setter!(set_seq_nr, seq_nr, u16);
    make_setter!(set_ack_nr, ack_nr, u16);
    make_setter!(set_connection_id, connection_id, u16);
    make_setter!(set_wnd_size, wnd_size, u32);
    make_setter!(set_timestamp_microseconds, timestamp_microseconds, u32);
    make_setter!(set_timestamp_difference_microseconds, timestamp_difference_microseconds, u32);

    /// Sets Selective ACK field in packet header and adds appropriate data.
    ///
    /// The length of the SACK extension is expressed in bytes, which
    /// must be a multiple of 4 and at least 4.
    pub fn set_sack(&mut self, bv: Vec<u8>) {
        // The length of the SACK extension is expressed in bytes, which
        // must be a multiple of 4 and at least 4.
        assert!(bv.len() >= 4);
        assert_eq!(bv.len() % 4, 0);

        let extension = Extension {
            ty: ExtensionType::SelectiveAck,
            data: bv,
        };
        self.extensions.push(extension);
        self.header.extension |= u8::from(ExtensionType::SelectiveAck);
    }

    pub fn len(&self) -> usize {
        let ext_len = self.extensions.iter().fold(0, |acc, ext| acc + ext.len() + 2);
        HEADER_SIZE + self.payload.len() + ext_len
    }
}

impl Encodable for Packet {
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::with_capacity(self.len());

        // Copy header
        buf.extend_from_slice(&self.header);

        // Copy extensions
        let mut extensions = self.extensions.iter().peekable();
        while let Some(extension) = extensions.next() {
            // Extensions are a linked list in which each entry contains:
            // - a byte with the type of the next extension or 0 to end the list,
            // - a byte with the length in bytes of this extension,
            // - the content of this extension.
            buf.push(extensions.peek().map_or(0, |next| u8::from(next.ty)));
            buf.push(extension.len() as u8);
            buf.extend_from_slice(&extension.data);
        }

        // Copy payload
        buf.extend_from_slice(&self.payload);

        return buf;
    }
}

impl<'a> TryFrom<&'a [u8]> for Packet {
    type Err = ParseError;

    /// Decodes a byte slice and construct the equivalent Packet.
    ///
    /// Note that this method makes no attempt to guess the payload size, saving
    /// all except the initial 20 bytes corresponding to the header as payload.
    /// It's the caller's responsibility to use an appropriately sized buffer.
    fn try_from(buf: &[u8]) -> Result<Self, Self::Err> {
        let header = try!(PacketHeader::try_from(buf));

        let mut extensions = Vec::new();
        let mut index = HEADER_SIZE;
        let mut extension_type = ExtensionType::from(header.extension);

        if buf.len() == HEADER_SIZE && extension_type != ExtensionType::None {
            return Err(ParseError::InvalidExtensionLength);
        }

        // Consume known extensions and skip over unknown ones
        while index < buf.len() && extension_type != ExtensionType::None {
            if buf.len() < index + 2 {
                return Err(ParseError::InvalidPacketLength);
            }
            let len = buf[index + 1] as usize;
            let extension_start = index + 2;
            let payload_start = extension_start + len;

            // Check validity of extension length:
            // - non-zero,
            // - multiple of 4,
            // - does not exceed packet length
            if len == 0 || len % 4 != 0 || payload_start > buf.len() {
                return Err(ParseError::InvalidExtensionLength);
            }

            if extension_type != ExtensionType::None {
                let extension = Extension {
                    ty: extension_type,
                    data: buf[extension_start..payload_start].to_vec(),
                };
                extensions.push(extension);
            }

            extension_type = ExtensionType::from(buf[index]);
            index += len + 2;
        }
        // Check for pending extensions (early exit of previous loop)
        if extension_type != ExtensionType::None {
            return Err(ParseError::InvalidPacketLength);
        }

        let payload_length = buf.len() - index;
        let mut payload = Vec::with_capacity(payload_length);
        if payload_length > 0 {
            payload.extend_from_slice(&buf[index..]);
        }

        Ok(Packet {
            header: header,
            extensions: extensions,
            payload: payload,
        })
    }
}

impl Clone for Packet {
    fn clone(&self) -> Packet {
        Packet {
            header: self.header,
            extensions: self.extensions.clone(),
            payload: self.payload.clone(),
        }
    }
}

impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Packet")
            .field("type", &self.get_type())
            .field("version", &self.header.get_version())
            .field("extension", &self.header.extension)
            .field("connection_id", &self.connection_id())
            .field("timestamp_microseconds", &self.timestamp_microseconds())
            .field("timestamp_difference_microseconds", &self.timestamp_difference_microseconds())
            .field("wnd_size", &self.wnd_size())
            .field("seq_nr", &self.seq_nr())
            .field("ack_nr", &self.ack_nr())
            .finish()
    }
}

/// Validate correctness of packet extensions, if any, in byte slice
fn check_extensions(data: &[u8]) -> Result<(), ParseError> {
    if data.len() < HEADER_SIZE {
        return Err(ParseError::InvalidPacketLength);
    }

    let mut index = HEADER_SIZE;
    let mut extension_type = ExtensionType::from(data[1]);

    if data.len() == HEADER_SIZE && extension_type != ExtensionType::None {
        return Err(ParseError::InvalidExtensionLength);
    }

    // Consume known extensions and skip over unknown ones
    while index < data.len() && extension_type != ExtensionType::None {
        if data.len() < index + 2 {
            return Err(ParseError::InvalidPacketLength);
        }
        let len = data[index + 1] as usize;
        let extension_start = index + 2;
        let payload_start = extension_start + len;

        // Check validity of extension length:
        // - non-zero,
        // - multiple of 4,
        // - does not exceed packet length
        if len == 0 || len % 4 != 0 || payload_start > data.len() {
            return Err(ParseError::InvalidExtensionLength);
        }

        extension_type = ExtensionType::from(data[index]);
        index += len + 2;
    }
    // Check for pending extensions (early exit of previous loop)
    if extension_type != ExtensionType::None {
        return Err(ParseError::InvalidPacketLength);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{PacketHeader, check_extensions};
    use super::PacketType::{State, Data};
    use quickcheck::{QuickCheck, TestResult};

    #[test]
    fn test_packet_decode() {
        let buf = [0x21, 0x00, 0x41, 0xa8, 0x99, 0x2f, 0xd0, 0x2a, 0x9f, 0x4a,
                   0x26, 0x21, 0x00, 0x10, 0x00, 0x00, 0x3a, 0xf2, 0x6c, 0x79];
        let pkt = Packet::try_from(&buf);
        assert!(pkt.is_ok());
        let pkt = pkt.unwrap();
        assert_eq!(pkt.header.get_version(), 1);
        assert_eq!(pkt.header.get_type(), State);
        assert_eq!(pkt.header.extension, 0);
        assert_eq!(pkt.connection_id(), 16808);
        assert_eq!(pkt.timestamp_microseconds(), 2570047530);
        assert_eq!(pkt.timestamp_difference_microseconds(), 2672436769);
        assert_eq!(pkt.wnd_size(), 2u32.pow(20));
        assert_eq!(pkt.seq_nr(), 15090);
        assert_eq!(pkt.ack_nr(), 27769);
        assert_eq!(pkt.len(), buf.len());
        assert!(pkt.payload.is_empty());
    }

    #[test]
    fn test_decode_packet_with_extension() {
        let buf = [0x21, 0x01, 0x41, 0xa7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                   0x00, 0x00, 0x00, 0x00, 0x05, 0xdc, 0xab, 0x53, 0x3a, 0xf5,
                   0x00, 0x04, 0x00, 0x00, 0x00, 0x00];
        let packet = Packet::try_from(&buf);
        assert!(packet.is_ok());
        let packet = packet.unwrap();
        assert_eq!(packet.header.get_version(), 1);
        assert_eq!(packet.header.get_type(), State);
        assert_eq!(packet.header.extension, 1);
        assert_eq!(packet.connection_id(), 16807);
        assert_eq!(packet.timestamp_microseconds(), 0);
        assert_eq!(packet.timestamp_difference_microseconds(), 0);
        assert_eq!(packet.wnd_size(), 1500);
        assert_eq!(packet.seq_nr(), 43859);
        assert_eq!(packet.ack_nr(), 15093);
        assert_eq!(packet.len(), buf.len());
        assert!(packet.payload.is_empty());
        assert_eq!(packet.extensions.len(), 1);
        assert_eq!(packet.extensions[0].ty, ExtensionType::SelectiveAck);
        assert_eq!(packet.extensions[0].data, vec!(0, 0, 0, 0));
        assert_eq!(packet.extensions[0].len(), packet.extensions[0].data.len());
        assert_eq!(packet.extensions[0].len(), 4);
        // Reversible
        assert_eq!(packet.to_bytes(), &buf);
    }

    #[test]
    fn test_packet_decode_with_missing_extension() {
        let buf = [0x21, 0x01, 0x41, 0xa8, 0x99, 0x2f, 0xd0, 0x2a, 0x9f, 0x4a,
                   0x26, 0x21, 0x00, 0x10, 0x00, 0x00, 0x3a, 0xf2, 0x6c, 0x79];
        let pkt = Packet::try_from(&buf);
        assert!(pkt.is_err());
    }

    #[test]
    fn test_packet_decode_with_malformed_extension() {
        let buf = [0x21, 0x01, 0x41, 0xa8, 0x99, 0x2f, 0xd0, 0x2a, 0x9f, 0x4a,
                   0x26, 0x21, 0x00, 0x10, 0x00, 0x00, 0x3a, 0xf2, 0x6c, 0x79,
                   0x00, 0x04, 0x00];
        let pkt = Packet::try_from(&buf);
        assert!(pkt.is_err());
    }

    #[test]
    fn test_decode_packet_with_unknown_extensions() {
        let buf = [0x21, 0x01, 0x41, 0xa7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                   0x00, 0x00, 0x00, 0x00, 0x05, 0xdc, 0xab, 0x53, 0x3a, 0xf5,
                   0xff, 0x04, 0x00, 0x00, 0x00, 0x00, // Imaginary extension
                   0x00, 0x04, 0x00, 0x00, 0x00, 0x00];
        match Packet::try_from(&buf) {
            Ok(packet) => {
                assert_eq!(packet.header.get_version(), 1);
                assert_eq!(packet.header.get_type(), State);
                assert_eq!(packet.header.extension, 1);
                assert_eq!(packet.connection_id(), 16807);
                assert_eq!(packet.timestamp_microseconds(), 0);
                assert_eq!(packet.timestamp_difference_microseconds(), 0);
                assert_eq!(packet.wnd_size(), 1500);
                assert_eq!(packet.seq_nr(), 43859);
                assert_eq!(packet.ack_nr(), 15093);
                assert!(packet.payload.is_empty());
                // The invalid extension is discarded
                assert_eq!(packet.extensions.len(), 2);
                assert_eq!(packet.extensions[0].ty, ExtensionType::SelectiveAck);
                assert_eq!(packet.extensions[0].data, vec!(0, 0, 0, 0));
                assert_eq!(packet.extensions[0].len(), packet.extensions[0].data.len());
                assert_eq!(packet.extensions[0].len(), 4);
            }
            Err(ref e) => panic!("{}", e)
        }
    }

    #[test]
    fn test_packet_encode() {
        let payload = b"Hello\n".to_vec();
        let (timestamp, timestamp_diff): (u32, u32) = (15270793, 1707040186);
        let (connection_id, seq_nr, ack_nr): (u16, u16, u16) = (16808, 15090, 17096);
        let window_size: u32 = 1048576;
        let mut pkt = Packet::new();
        pkt.set_type(Data);
        pkt.header.timestamp_microseconds = timestamp.to_be();
        pkt.header.timestamp_difference_microseconds = timestamp_diff.to_be();
        pkt.header.connection_id = connection_id.to_be();
        pkt.header.seq_nr = seq_nr.to_be();
        pkt.header.ack_nr = ack_nr.to_be();
        pkt.header.wnd_size = window_size.to_be();
        pkt.payload = payload.clone();
        let header = pkt.header;
        let buf = [0x01, 0x00, 0x41, 0xa8, 0x00, 0xe9, 0x03, 0x89,
                   0x65, 0xbf, 0x5d, 0xba, 0x00, 0x10, 0x00, 0x00,
                   0x3a, 0xf2, 0x42, 0xc8, 0x48, 0x65, 0x6c, 0x6c,
                   0x6f, 0x0a];

        assert_eq!(pkt.len(), buf.len());
        assert_eq!(pkt.len(), HEADER_SIZE + payload.len());
        assert_eq!(pkt.payload, payload);
        assert_eq!(header.get_version(), 1);
        assert_eq!(header.get_type(), Data);
        assert_eq!(header.extension, 0);
        assert_eq!(pkt.connection_id(), connection_id);
        assert_eq!(pkt.seq_nr(), seq_nr);
        assert_eq!(pkt.ack_nr(), ack_nr);
        assert_eq!(pkt.wnd_size(), window_size);
        assert_eq!(pkt.timestamp_microseconds(), timestamp);
        assert_eq!(pkt.timestamp_difference_microseconds(), timestamp_diff);
        assert_eq!(pkt.to_bytes(), buf.to_vec());
    }

    #[test]
    fn test_packet_encode_with_payload() {
        let payload = b"Hello\n".to_vec();
        let (timestamp, timestamp_diff): (u32, u32) = (15270793, 1707040186);
        let (connection_id, seq_nr, ack_nr): (u16, u16, u16) = (16808, 15090, 17096);
        let window_size: u32 = 1048576;
        let mut pkt = Packet::with_payload(&payload[..]);
        pkt.header.timestamp_microseconds = timestamp.to_be();
        pkt.header.timestamp_difference_microseconds = timestamp_diff.to_be();
        pkt.header.connection_id = connection_id.to_be();
        pkt.header.seq_nr = seq_nr.to_be();
        pkt.header.ack_nr = ack_nr.to_be();
        pkt.header.wnd_size = window_size.to_be();
        pkt.payload = payload.clone();
        let header = pkt.header;
        let buf = [0x01, 0x00, 0x41, 0xa8, 0x00, 0xe9, 0x03, 0x89,
                   0x65, 0xbf, 0x5d, 0xba, 0x00, 0x10, 0x00, 0x00,
                   0x3a, 0xf2, 0x42, 0xc8, 0x48, 0x65, 0x6c, 0x6c,
                   0x6f, 0x0a];

        assert_eq!(pkt.len(), buf.len());
        assert_eq!(pkt.len(), HEADER_SIZE + payload.len());
        assert_eq!(pkt.payload, payload);
        assert_eq!(header.get_version(), 1);
        assert_eq!(header.get_type(), Data);
        assert_eq!(header.extension, 0);
        assert_eq!(pkt.connection_id(), connection_id);
        assert_eq!(pkt.seq_nr(), seq_nr);
        assert_eq!(pkt.ack_nr(), ack_nr);
        assert_eq!(pkt.wnd_size(), window_size);
        assert_eq!(pkt.timestamp_microseconds(), timestamp);
        assert_eq!(pkt.timestamp_difference_microseconds(), timestamp_diff);
        assert_eq!(pkt.to_bytes(), buf.to_vec());
    }

    #[test]
    fn test_packet_encode_with_multiple_extensions() {
        let mut packet = Packet::new();
        let extension = Extension { ty: ExtensionType::SelectiveAck, data: vec!(1, 2, 3, 4) };
        packet.header.extension = u8::from(extension.ty);
        packet.extensions.push(extension.clone());
        packet.extensions.push(extension.clone());
        let bytes = packet.to_bytes();
        assert_eq!(bytes.len(), HEADER_SIZE + (extension.len() + 2) * 2);

        // Type of the first extension
        assert_eq!(bytes[1], u8::from(extension.ty));

        // Type of the next (second) extension
        assert_eq!(bytes[HEADER_SIZE], u8::from(extension.ty));
        // Length of the first extension
        assert_eq!(bytes[HEADER_SIZE + 1], extension.data.len() as u8);

        // Type of the next (third, non-existent) extension
        assert_eq!(bytes[HEADER_SIZE + 2 + extension.len()], 0);
        // Length of the second extension
        assert_eq!(bytes[HEADER_SIZE + 2 + extension.len() + 1], extension.data.len() as u8);
    }

    #[test]
    fn test_reversible() {
        let buf = [0x01, 0x00, 0x41, 0xa8, 0x00, 0xe9, 0x03, 0x89,
                   0x65, 0xbf, 0x5d, 0xba, 0x00, 0x10, 0x00, 0x00,
                   0x3a, 0xf2, 0x42, 0xc8, 0x48, 0x65, 0x6c, 0x6c,
                   0x6f, 0x0a];
        assert_eq!(&Packet::try_from(&buf).unwrap().to_bytes()[..], &buf[..]);
    }

    #[test]
    fn test_decode_evil_sequence() {
        let buf = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let packet = Packet::try_from(&buf);
        assert!(packet.is_err());
    }

    #[test]
    fn test_decode_empty_packet() {
        let packet = Packet::try_from(&[]);
        assert!(packet.is_err());
    }

    // Use quickcheck to simulate a malicious attacker sending malformed packets
    #[test]
    fn quicktest() {
        fn run(x: Vec<u8>) -> TestResult {
            let packet = Packet::try_from(&x);

            if PacketHeader::try_from(&x).and(check_extensions(&x)).is_err() {
                TestResult::from_bool(packet.is_err())
            } else if let Ok(bytes) = packet.map(|p| p.to_bytes()) {
                TestResult::from_bool(bytes == x)
            } else {
                TestResult::from_bool(false)
            }
        }
        QuickCheck::new().tests(10000).quickcheck(run as fn(Vec<u8>) -> TestResult)
    }
}
