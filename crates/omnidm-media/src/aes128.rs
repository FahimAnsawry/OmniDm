// SPDX-License-Identifier: GPL-3.0-or-later

//! Unlocking HLS pieces locked with `METHOD=AES-128`: the whole piece is AES-128-CBC with
//! PKCS#7 padding, and the key is a 16-byte file the site hands out (RFC 8216 §5.2).

use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};

use crate::MediaError;

type Decryptor = cbc::Decryptor<aes::Aes128>;

/// Unlocks `data` in place and shortens it to the unlocked length.
pub fn decrypt(key: &[u8], iv: &[u8; 16], data: &mut Vec<u8>) -> Result<(), MediaError> {
    let key: [u8; 16] = key.try_into().map_err(|_| {
        MediaError::Invalid(format!("the key is {} bytes long instead of 16", key.len()))
    })?;
    let len = Decryptor::new(&key.into(), &(*iv).into())
        .decrypt_padded::<Pkcs7>(data)
        .map_err(|_| MediaError::Invalid("a piece couldn't be unlocked with its key".into()))?
        .len();
    data.truncate(len);
    Ok(())
}

/// The IV a piece uses when its key gives none: its media sequence number, as a 128-bit
/// big-endian number.
pub fn iv_for_sequence(sequence: u64) -> [u8; 16] {
    u128::from(sequence).to_be_bytes()
}

/// Reads an `IV=0x…` attribute: up to 32 hex digits.
pub fn parse_iv(text: &str) -> Result<[u8; 16], MediaError> {
    let bad = || MediaError::Invalid(format!("a bad IV: {text}"));
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .ok_or_else(bad)?;
    if digits.is_empty() || digits.len() > 32 {
        return Err(bad());
    }
    u128::from_str_radix(digits, 16)
        .map(u128::to_be_bytes)
        .map_err(|_| bad())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockModeEncrypt;

    fn encrypt(key: &[u8; 16], iv: &[u8; 16], plain: &[u8]) -> Vec<u8> {
        cbc::Encryptor::<aes::Aes128>::new(&(*key).into(), &(*iv).into())
            .encrypt_padded_vec::<Pkcs7>(plain)
    }

    #[test]
    fn unlocks_what_was_locked() {
        let key = [7u8; 16];
        let iv = iv_for_sequence(42);
        for size in [0, 1, 15, 16, 17, 188 * 100] {
            let plain: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let mut data = encrypt(&key, &iv, &plain);
            decrypt(&key, &iv, &mut data).unwrap();
            assert_eq!(data, plain, "size {size}");
        }
    }

    #[test]
    fn a_wrong_key_or_size_is_an_error() {
        let iv = [0u8; 16];
        let mut data = encrypt(&[1; 16], &iv, b"some video bytes");
        assert!(decrypt(&[2; 16], &iv, &mut data.clone()).is_err());
        assert!(decrypt(&[1; 8], &iv, &mut data).is_err());
        let mut odd = vec![0u8; 20];
        assert!(decrypt(&[1; 16], &iv, &mut odd).is_err());
    }

    #[test]
    fn ivs() {
        assert_eq!(iv_for_sequence(1), {
            let mut iv = [0u8; 16];
            iv[15] = 1;
            iv
        });
        assert_eq!(
            parse_iv("0x000102030405060708090A0B0C0D0E0F").unwrap(),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        );
        assert_eq!(parse_iv("0X1F").unwrap(), iv_for_sequence(0x1f));
        for bad in [
            "",
            "0x",
            "1234",
            "0xZZ",
            "0x000102030405060708090A0B0C0D0E0F00",
        ] {
            assert!(parse_iv(bad).is_err(), "{bad}");
        }
    }
}
