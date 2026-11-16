//! NIP-44 v2: encryption of an exchange message to its recipient.
//!
//! Steps as the NIP gives them: the conversation key is HKDF-extract
//! (SHA-256, salt `nip44-v2`) of the unhashed ECDH x coordinate; per message,
//! HKDF-expand of a random 32-byte nonce gives the ChaCha20 key and nonce and
//! the HMAC key; the plaintext is padded behind a big-endian u16 length; the
//! payload is base64 of `0x02 ‖ nonce ‖ ciphertext ‖ HMAC-SHA256(nonce ‖
//! ciphertext)`.
//!
//! Only plaintexts of 1..=65535 bytes, as in the published test vectors.
//! The NIP's later extended prefix (6 bytes, for 65536 bytes and more) is
//! refused both ways: exchange messages are a few kilobytes.

use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use secp256k1::{Parity, SecretKey, XOnlyPublicKey};
use sha2::Sha256;

use crate::error::AppError;

use base64::Engine as _;

const VERSION: u8 = 2;
const MAX_PLAINTEXT: usize = 65535;

fn refuse(why: &str) -> AppError {
    AppError::InvalidInput(format!("NIP-44: {why}"))
}

/// The key two parties share: HKDF-extract of the ECDH x coordinate.
pub fn conversation_key(sec: &SecretKey, pub_x: &[u8; 32]) -> Result<[u8; 32], AppError> {
    let x = XOnlyPublicKey::from_byte_array(*pub_x).map_err(|_| refuse("not a public key"))?;
    let point = x.public_key(Parity::Even);
    let shared = secp256k1::ecdh::shared_secret_point(&point, sec);
    let (prk, _) = Hkdf::<Sha256>::extract(Some(b"nip44-v2"), &shared[..32]);
    Ok(prk.into())
}

/// ChaCha20 key, ChaCha20 nonce and HMAC key of one message.
pub fn message_keys(ck: &[u8; 32], nonce: &[u8; 32]) -> ([u8; 32], [u8; 12], [u8; 32]) {
    let hk = Hkdf::<Sha256>::from_prk(ck).expect("a 32-byte PRK is long enough");
    let mut okm = [0u8; 76];
    hk.expand(nonce, &mut okm)
        .expect("76 bytes is within HKDF-SHA256's limit");
    let mut key = [0u8; 32];
    let mut iv = [0u8; 12];
    let mut mac = [0u8; 32];
    key.copy_from_slice(&okm[..32]);
    iv.copy_from_slice(&okm[32..44]);
    mac.copy_from_slice(&okm[44..76]);
    (key, iv, mac)
}

/// The padded length of an `unpadded`-byte plaintext (without the prefix).
pub fn padded_len(unpadded: usize) -> usize {
    if unpadded <= 32 {
        return 32;
    }
    let next_power = 1usize << (usize::BITS - (unpadded - 1).leading_zeros());
    let chunk = if next_power <= 256 {
        32
    } else {
        next_power / 8
    };
    chunk * ((unpadded - 1) / chunk + 1)
}

fn pad(plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
    let n = plaintext.len();
    if n == 0 || n > MAX_PLAINTEXT {
        return Err(refuse("a message must be 1 to 65535 bytes"));
    }
    let mut out = Vec::with_capacity(2 + padded_len(n));
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(plaintext);
    out.resize(2 + padded_len(n), 0);
    Ok(out)
}

fn unpad(padded: &[u8]) -> Result<Vec<u8>, AppError> {
    if padded.len() < 2 {
        return Err(refuse("invalid padding"));
    }
    let n = u16::from_be_bytes([padded[0], padded[1]]) as usize;
    if n == 0 || padded.len() != 2 + padded_len(n) {
        return Err(refuse("invalid padding"));
    }
    Ok(padded[2..2 + n].to_vec())
}

fn hmac_of(key: &[u8; 32], nonce: &[u8; 32], ciphertext: &[u8]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(nonce);
    mac.update(ciphertext);
    mac
}

/// Encrypt with a given nonce (for test vectors; real messages use
/// [`encrypt`]).
pub fn encrypt_with_nonce(
    plaintext: &str,
    ck: &[u8; 32],
    nonce: &[u8; 32],
) -> Result<String, AppError> {
    let (key, iv, mac_key) = message_keys(ck, nonce);
    let mut data = pad(plaintext.as_bytes())?;
    ChaCha20::new(&key.into(), &iv.into()).apply_keystream(&mut data);
    let mac = hmac_of(&mac_key, nonce, &data).finalize().into_bytes();
    let mut payload = Vec::with_capacity(1 + 32 + data.len() + 32);
    payload.push(VERSION);
    payload.extend_from_slice(nonce);
    payload.extend_from_slice(&data);
    payload.extend_from_slice(&mac);
    Ok(base64::engine::general_purpose::STANDARD.encode(payload))
}

/// Encrypt with a fresh random nonce.
pub fn encrypt(plaintext: &str, ck: &[u8; 32]) -> Result<String, AppError> {
    let nonce: [u8; 32] = rand::random();
    encrypt_with_nonce(plaintext, ck, &nonce)
}

/// Decrypt a payload; any malformed, foreign or altered payload is an error.
pub fn decrypt(payload: &str, ck: &[u8; 32]) -> Result<String, AppError> {
    if payload.is_empty() || payload.starts_with('#') {
        return Err(refuse("unknown encryption version"));
    }
    if !(132..=87472).contains(&payload.len()) {
        return Err(refuse("invalid payload size"));
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|_| refuse("invalid base64"))?;
    if !(99..=65603).contains(&data.len()) {
        return Err(refuse("invalid data size"));
    }
    if data[0] != VERSION {
        return Err(refuse("unknown encryption version"));
    }
    let nonce: [u8; 32] = data[1..33].try_into().expect("checked length");
    let (ciphertext, mac) = data[33..].split_at(data.len() - 33 - 32);
    let (key, iv, mac_key) = message_keys(ck, &nonce);
    hmac_of(&mac_key, &nonce, ciphertext)
        .verify_slice(mac)
        .map_err(|_| refuse("invalid MAC"))?;
    let mut padded = ciphertext.to_vec();
    ChaCha20::new(&key.into(), &iv.into()).apply_keystream(&mut padded);
    String::from_utf8(unpad(&padded)?).map_err(|_| refuse("the message is not UTF-8"))
}
