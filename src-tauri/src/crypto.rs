use crate::error::{AppError, Result};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};
use zeroize::{Zeroize, Zeroizing};

pub fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes)
        .map_err(|_| AppError::new("RANDOM_FAILED", "无法获取系统安全随机数。"))?;
    Ok(bytes)
}

pub fn protect(key: &[u8]) -> Result<Vec<u8>> {
    dpapi(key, false)
}
pub fn unprotect(key: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    Ok(Zeroizing::new(dpapi(key, true)?))
}

fn dpapi(input: &[u8], decrypt: bool) -> Result<Vec<u8>> {
    let input_len = u32::try_from(input.len()).map_err(|_| AppError::invalid())?;
    let blob = CRYPT_INTEGER_BLOB {
        cbData: input_len,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // Windows owns output until LocalFree. Do not use machine-wide protection:
    // the key must remain bound to the logged-in user's DPAPI identity.
    let success = unsafe {
        if decrypt {
            CryptUnprotectData(
                &blob,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &blob,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err(AppError::new(
            "UNLOCK_FAILED",
            "无法使用当前 Windows 身份解锁，请使用创建此密码库的账户和电脑。",
        ));
    }
    let result = unsafe {
        let slice = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result = slice.to_vec();
        if decrypt {
            slice.zeroize();
        }
        LocalFree(output.pbData.cast());
        result
    };
    Ok(result)
}

pub fn encrypt(key: &[u8], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| AppError::corrupt())?;
    let nonce = random::<12>()?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| AppError::corrupt())?;
    let mut packed = nonce.to_vec();
    packed.extend(ciphertext);
    Ok(packed)
}

pub fn decrypt(key: &[u8], aad: &[u8], packed: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if packed.len() < 28 {
        return Err(AppError::corrupt());
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| AppError::corrupt())?;
    Ok(Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&packed[..12]),
                Payload {
                    msg: &packed[12..],
                    aad,
                },
            )
            .map_err(|_| AppError::corrupt())?,
    ))
}
