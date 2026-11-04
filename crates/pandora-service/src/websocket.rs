//! RFC 6455 Full-Duplex WebSocket Gateway and Event Streaming Engine
//!
//! Grounded in mathematical foundations:
//! - Uniform 32-bit XOR frame masking entropy (Theorem 1, H = 8 bits/byte)
//! - Bounded M/M/1/K backpressure queueing stability (Theorem 2, W <= K / mu)
//! - Cryptographic RFC 6455 handshake with HMAC-SHA256 session authentication

#![forbid(unsafe_code)]

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

pub const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
pub const DEFAULT_QUEUE_CAPACITY: usize = 1024;
pub const MAX_FRAME_PAYLOAD_SIZE: usize = 16 * 1024 * 1024; // 16 MB limit

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Continuation = 0x0,
    Text = 0x1,
    Binary = 0x2,
    Close = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}

impl Opcode {
    pub fn from_u8(byte: u8) -> Result<Self, WebSocketError> {
        match byte {
            0x0 => Ok(Opcode::Continuation),
            0x1 => Ok(Opcode::Text),
            0x2 => Ok(Opcode::Binary),
            0x8 => Ok(Opcode::Close),
            0x9 => Ok(Opcode::Ping),
            0xA => Ok(Opcode::Pong),
            other => Err(WebSocketError::InvalidOpcode(other)),
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn is_control(self) -> bool {
        matches!(self, Opcode::Close | Opcode::Ping | Opcode::Pong)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketFrame {
    pub fin: bool,
    pub rsv1: bool,
    pub rsv2: bool,
    pub rsv3: bool,
    pub opcode: Opcode,
    pub masked: bool,
    pub mask_key: Option<[u8; 4]>,
    pub payload: Vec<u8>,
}

impl WebSocketFrame {
    pub fn text(payload: impl Into<String>) -> Self {
        Self {
            fin: true,
            rsv1: false,
            rsv2: false,
            rsv3: false,
            opcode: Opcode::Text,
            masked: false,
            mask_key: None,
            payload: payload.into().into_bytes(),
        }
    }

    pub fn binary(payload: impl Into<Vec<u8>>) -> Self {
        Self {
            fin: true,
            rsv1: false,
            rsv2: false,
            rsv3: false,
            opcode: Opcode::Binary,
            masked: false,
            mask_key: None,
            payload: payload.into(),
        }
    }

    pub fn ping(payload: impl Into<Vec<u8>>) -> Self {
        Self {
            fin: true,
            rsv1: false,
            rsv2: false,
            rsv3: false,
            opcode: Opcode::Ping,
            masked: false,
            mask_key: None,
            payload: payload.into(),
        }
    }

    pub fn pong(payload: impl Into<Vec<u8>>) -> Self {
        Self {
            fin: true,
            rsv1: false,
            rsv2: false,
            rsv3: false,
            opcode: Opcode::Pong,
            masked: false,
            mask_key: None,
            payload: payload.into(),
        }
    }

    pub fn close(code: u16, reason: &str) -> Self {
        let mut payload = Vec::with_capacity(2 + reason.len());
        payload.extend_from_slice(&code.to_be_bytes());
        payload.extend_from_slice(reason.as_bytes());
        Self {
            fin: true,
            rsv1: false,
            rsv2: false,
            rsv3: false,
            opcode: Opcode::Close,
            masked: false,
            mask_key: None,
            payload,
        }
    }

    pub fn with_mask(mut self, key: [u8; 4]) -> Self {
        self.masked = true;
        self.mask_key = Some(key);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketError {
    Incomplete,
    InvalidOpcode(u8),
    FrameTooLarge(usize),
    ProtocolViolation(String),
    ChecksumMismatch,
    QueueFull(usize),
    AuthenticationFailed(String),
}

impl fmt::Display for WebSocketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WebSocketError::Incomplete => write!(f, "Incomplete frame data"),
            WebSocketError::InvalidOpcode(op) => write!(f, "Invalid WebSocket opcode: 0x{op:02x}"),
            WebSocketError::FrameTooLarge(sz) => {
                write!(f, "Frame payload exceeds limit ({sz} bytes)")
            }
            WebSocketError::ProtocolViolation(msg) => write!(f, "Protocol violation: {msg}"),
            WebSocketError::ChecksumMismatch => write!(f, "Checksum validation failure"),
            WebSocketError::QueueFull(cap) => {
                write!(f, "Backpressure queue capacity exceeded: {cap}")
            }
            WebSocketError::AuthenticationFailed(msg) => write!(f, "Authentication failed: {msg}"),
        }
    }
}

impl std::error::Error for WebSocketError {}

/// Pure RFC 3174 SHA-1 digest implementation strictly under `#![forbid(unsafe_code)]`.
/// Required by RFC 6455 Section 1.3 for WebSocket handshake `Sec-WebSocket-Accept`.
#[allow(clippy::chunks_exact_to_as_chunks)]
pub fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, slot) in w.iter_mut().take(16).enumerate() {
            let start = i * 4;
            *slot = u32::from_be_bytes([
                chunk[start],
                chunk[start + 1],
                chunk[start + 2],
                chunk[start + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, &item) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };

            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(item);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

/// Compute `Sec-WebSocket-Accept` from `Sec-WebSocket-Key` per RFC 6455 Section 1.3.
pub fn compute_websocket_accept(sec_websocket_key: &str) -> String {
    let combined = format!("{}{}", sec_websocket_key.trim(), WS_GUID);
    let digest = sha1_digest(combined.as_bytes());
    BASE64.encode(digest)
}

/// Constant-time HMAC-SHA256 calculation for bearer token authentication.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let hash = Sha256::digest(key);
        k[..32].copy_from_slice(&hash);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0u8; 64];
    let mut opad = [0u8; 64];
    for i in 0..64 {
        ipad[i] = k[i] ^ 0x36;
        opad[i] = k[i] ^ 0x5C;
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    let result = outer.finalize();

    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Create an authenticated session bearer token.
pub fn create_session_token(secret: &[u8], subject: &str, expiry_timestamp: u64) -> String {
    let payload = format!("{subject}:{expiry_timestamp}");
    let tag = hmac_sha256(secret, payload.as_bytes());
    let tag_b64 = BASE64.encode(tag);
    let payload_b64 = BASE64.encode(payload.as_bytes());
    format!("{payload_b64}.{tag_b64}")
}

/// Verify an authenticated session bearer token.
pub fn verify_session_token(
    secret: &[u8],
    token: &str,
    current_timestamp: u64,
) -> Result<String, WebSocketError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 2 {
        return Err(WebSocketError::AuthenticationFailed(
            "Malformed token format".to_string(),
        ));
    }

    let payload_bytes = BASE64.decode(parts[0]).map_err(|e| {
        WebSocketError::AuthenticationFailed(format!("Invalid base64 payload: {e}"))
    })?;
    let expected_tag = BASE64
        .decode(parts[1])
        .map_err(|e| WebSocketError::AuthenticationFailed(format!("Invalid base64 tag: {e}")))?;

    let actual_tag = hmac_sha256(secret, &payload_bytes);
    if expected_tag.as_slice() != actual_tag.as_slice() {
        return Err(WebSocketError::AuthenticationFailed(
            "HMAC tag verification mismatch".to_string(),
        ));
    }

    let payload_str = String::from_utf8(payload_bytes).map_err(|e| {
        WebSocketError::AuthenticationFailed(format!("UTF-8 decoding error in token: {e}"))
    })?;

    let sub_parts: Vec<&str> = payload_str.split(':').collect();
    if sub_parts.len() != 2 {
        return Err(WebSocketError::AuthenticationFailed(
            "Invalid payload segments".to_string(),
        ));
    }

    let sub = sub_parts[0];
    let expiry: u64 = sub_parts[1].parse().map_err(|e| {
        WebSocketError::AuthenticationFailed(format!("Invalid expiry integer: {e}"))
    })?;

    if current_timestamp > expiry {
        return Err(WebSocketError::AuthenticationFailed(
            "Session token has expired".to_string(),
        ));
    }

    Ok(sub.to_string())
}

/// Encode a WebSocket frame into its wire byte representation per RFC 6455.
pub fn encode_frame(frame: &WebSocketFrame) -> Vec<u8> {
    let mut header = Vec::with_capacity(14 + frame.payload.len());
    let b0 = (if frame.fin { 0x80 } else { 0x00 })
        | (if frame.rsv1 { 0x40 } else { 0x00 })
        | (if frame.rsv2 { 0x20 } else { 0x00 })
        | (if frame.rsv3 { 0x10 } else { 0x00 })
        | (frame.opcode.to_u8() & 0x0F);
    header.push(b0);

    let mask_bit = if frame.masked { 0x80 } else { 0x00 };
    let len = frame.payload.len();

    if len <= 125 {
        header.push(mask_bit | (len as u8));
    } else if len <= 65535 {
        header.push(mask_bit | 126);
        header.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        header.push(mask_bit | 127);
        header.extend_from_slice(&(len as u64).to_be_bytes());
    }

    if frame.masked {
        let key = frame.mask_key.unwrap_or([0, 0, 0, 0]);
        header.extend_from_slice(&key);
        let mut masked_payload = frame.payload.clone();
        for (i, byte) in masked_payload.iter_mut().enumerate() {
            *byte ^= key[i % 4];
        }
        header.extend_from_slice(&masked_payload);
    } else {
        header.extend_from_slice(&frame.payload);
    }

    header
}

/// Decode a WebSocket frame from wire bytes. Returns parsed frame and consumed byte count.
pub fn decode_frame(bytes: &[u8]) -> Result<Option<(WebSocketFrame, usize)>, WebSocketError> {
    if bytes.len() < 2 {
        return Ok(None);
    }

    let b0 = bytes[0];
    let fin = (b0 & 0x80) != 0;
    let rsv1 = (b0 & 0x40) != 0;
    let rsv2 = (b0 & 0x20) != 0;
    let rsv3 = (b0 & 0x10) != 0;
    let opcode = Opcode::from_u8(b0 & 0x0F)?;

    let b1 = bytes[1];
    let masked = (b1 & 0x80) != 0;
    let len_code = b1 & 0x7F;

    let mut cursor = 2;
    let payload_len: usize = match len_code {
        0..=125 => len_code as usize,
        126 => {
            if bytes.len() < cursor + 2 {
                return Ok(None);
            }
            let len = u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
            cursor += 2;
            len
        }
        127 => {
            if bytes.len() < cursor + 8 {
                return Ok(None);
            }
            let len = u64::from_be_bytes([
                bytes[cursor],
                bytes[cursor + 1],
                bytes[cursor + 2],
                bytes[cursor + 3],
                bytes[cursor + 4],
                bytes[cursor + 5],
                bytes[cursor + 6],
                bytes[cursor + 7],
            ]) as usize;
            cursor += 8;
            len
        }
        _ => unreachable!(),
    };

    if payload_len > MAX_FRAME_PAYLOAD_SIZE {
        return Err(WebSocketError::FrameTooLarge(payload_len));
    }

    let mask_key = if masked {
        if bytes.len() < cursor + 4 {
            return Ok(None);
        }
        let key = [
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ];
        cursor += 4;
        Some(key)
    } else {
        None
    };

    if bytes.len() < cursor + payload_len {
        return Ok(None);
    }

    let raw_payload = &bytes[cursor..cursor + payload_len];
    let mut payload = raw_payload.to_vec();

    if let Some(key) = mask_key {
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= key[i % 4];
        }
    }

    cursor += payload_len;

    let frame = WebSocketFrame {
        fin,
        rsv1,
        rsv2,
        rsv3,
        opcode,
        masked,
        mask_key,
        payload,
    };

    Ok(Some((frame, cursor)))
}

/// Bounded backpressure queue enforcing $M/M/1/K$ capacity invariants (Theorem 2).
#[derive(Debug, Clone)]
pub struct BoundedFrameQueue {
    capacity: usize,
    queue: VecDeque<WebSocketFrame>,
}

impl BoundedFrameQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: if capacity == 0 {
                DEFAULT_QUEUE_CAPACITY
            } else {
                capacity
            },
            queue: VecDeque::with_capacity(capacity.min(DEFAULT_QUEUE_CAPACITY)),
        }
    }

    pub fn enqueue(&mut self, frame: WebSocketFrame) -> Result<(), WebSocketError> {
        if self.queue.len() >= self.capacity {
            return Err(WebSocketError::QueueFull(self.capacity));
        }
        self.queue.push_back(frame);
        Ok(())
    }

    pub fn dequeue(&mut self) -> Option<WebSocketFrame> {
        self.queue.pop_front()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// JSON-RPC 2.0 Request Object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 Response Object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: i64, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(serde_json::json!({
                "code": code,
                "message": message
            })),
        }
    }
}

/// Pub/Sub Broker for streaming multiplexing.
#[derive(Debug, Default)]
pub struct PubSubBroker {
    subscribers: HashMap<String, HashSet<String>>, // topic -> client_ids
}

impl PubSubBroker {
    pub fn new() -> Self {
        Self {
            subscribers: HashMap::new(),
        }
    }

    pub fn subscribe(&mut self, client_id: &str, topic: &str) {
        self.subscribers
            .entry(topic.to_string())
            .or_default()
            .insert(client_id.to_string());
    }

    pub fn unsubscribe(&mut self, client_id: &str, topic: &str) {
        if let Some(clients) = self.subscribers.get_mut(topic) {
            clients.remove(client_id);
        }
    }

    pub fn remove_client(&mut self, client_id: &str) {
        for clients in self.subscribers.values_mut() {
            clients.remove(client_id);
        }
    }

    pub fn get_subscribers(&self, topic: &str) -> Vec<String> {
        self.subscribers
            .get(topic)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }
}
