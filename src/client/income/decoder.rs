use std::sync::atomic::Ordering;
use bytes::{Buf, BufMut, Bytes};
use crate::binary::BinaryReader;
use crate::client::Client;
use super::tlv_decoder::*;
use crate::client::device::random_string;

#[derive(Debug)]
pub enum QRCodeState {
    QRCodeImageFetch {
        image_data: Bytes,
        sig: Bytes,
    },
    QRCodeWaitingForScan,
    QRCodeWaitingForConfirm,
    QRCodeTimeout,
    QRCodeConfirmed {
        tmp_pwd: Bytes,
        tmp_no_pic_sig: Bytes,
        tgt_qr: Bytes,
    },
    QRCodeCanceled,
}


#[derive(Debug)]
pub enum LoginResponse {
    Success,
    SliderNeededError {
        verify_url: String,
    },
    NeedCaptcha {
        captcha_sign: Bytes,
        captcha_image: Bytes,
    },
    UnknownLoginError {
        error_message: String,
    },
    SMSOrVerifyNeededError {
        verify_url: String,
        sms_phone: String,
        error_message: String,
    },
    SMSNeededError {
        sms_phone: String,
        error_message: String,
    },
    UnsafeDeviceError {
        verify_url: String
    },
    TooManySMSRequestError,
    OtherLoginError {
        error_message: String
    },
}


pub async fn decode_trans_emp_response(cli: &Client, payload: &[u8]) -> Option<QRCodeState> {
    if payload.len() < 48 {
        return None;
    }
    let mut payload = Bytes::from(payload.to_owned());
    payload.advance(5);// trans req head
    payload.get_u8();
    payload.get_u16();
    let cmd = payload.get_u16();
    payload.advance(21);
    payload.get_u8();
    payload.get_u16();
    payload.get_u16();
    payload.get_i32();
    payload.get_i64();
    let len = payload.remaining() - 1;
    let mut body = Bytes::from(payload.copy_to_bytes(len));
    if cmd == 0x31 {
        body.get_u16();
        body.get_i32();
        let code = body.get_u8();
        if code != 0 {
            return None;
        }
        let sig = body.read_bytes_short();
        body.get_u16();
        let mut m = body.read_tlv_map(2);
        if m.contains_key(&0x17) {
            return Some(QRCodeState::QRCodeImageFetch {
                image_data: m.remove(&0x17).unwrap(),
                sig,
            });
        }
    }
    if cmd == 0x12 {
        let mut a_var_len = body.get_u16();
        if a_var_len != 0 {
            a_var_len -= 1; // 阴间的位移操作
            if body.get_u8() == 2 {
                body.get_i64();//uin?
                a_var_len -= 8;
            }
        }
        if a_var_len > 0 {
            body.advance(a_var_len as usize);
        }
        body.get_i32();
        let code = body.get_u8();
        if code != 0 {
            return match code {
                0x30 => Some(QRCodeState::QRCodeWaitingForScan),
                0x35 => Some(QRCodeState::QRCodeWaitingForConfirm),
                0x36 => Some(QRCodeState::QRCodeCanceled),
                0x11 => Some(QRCodeState::QRCodeTimeout),
                _ => None
            };
        }
        cli.uin.store(body.get_i64(), Ordering::SeqCst);
        body.get_i32(); // sig create time
        body.get_u16();
        let mut m = body.read_tlv_map(2);
        if !m.contains_key(&0x18) || !m.contains_key(&0x1e) || !m.contains_key(&0x19) {
            return None;
        }
        {
            let mut device_info = cli.device_info.write().await;
            device_info.tgtgt_key = m.remove(&0x1e).unwrap();
        }
        return Some(QRCodeState::QRCodeConfirmed {
            tmp_pwd: m.remove(&0x18).unwrap(),
            tmp_no_pic_sig: m.remove(&0x19).unwrap(),
            tgt_qr: m.remove(&0x65).unwrap(),
        });
    }
    return None;
}

pub async fn decode_login_response(cli: &Client, payload: &[u8]) -> Option<LoginResponse> {
    let mut reader = Bytes::from(payload.to_owned());
    reader.get_u16(); // sub command
    let t = reader.get_u8();
    reader.get_u16();
    let mut m = reader.read_tlv_map(2);
    if m.contains_key(&0x402) {
        let mut cache_info = cli.cache_info.write().await;
        cache_info.dpwd = random_string(16).into();
        cache_info.t402 = m.remove(&0x402).unwrap();
        let mut v = Vec::new();
        v.put_slice(&cli.device_info.read().await.guid);
        v.put_slice(&cache_info.dpwd);
        v.put_slice(&cache_info.t402);
        cache_info.g = md5::compute(&v).to_vec().into();
    }
    if t == 0 {
        let mut cache_info = cli.cache_info.write().await;
        let mut account_info = cli.account_info.write().await;
        if m.contains_key(&0x150) {
            cache_info.t150 = m.remove(&0x150).unwrap().into();
        }
        if m.contains_key(&0x161) {
            decode_t161(&m.remove(&0x161).unwrap(), &mut cache_info);
        }
        if m.contains_key(&0x403) {
            cache_info.rand_seed = m.remove(&0x403).unwrap().into();
        }
        decode_t119(&m.get(&0x119).unwrap(), &cli.device_info.read().await.tgtgt_key, &mut cache_info, &mut account_info).await;
        return Some(LoginResponse::Success);
    }
    if t == 2 {
        let mut cache_info = cli.cache_info.write().await;
        cache_info.t104 = m.remove(&0x104).unwrap();
        if m.contains_key(&0x192) {
            return Some(LoginResponse::SliderNeededError {
                verify_url: String::from_utf8(m.remove(&0x192).unwrap().to_vec()).unwrap(),
            });
        }
        if m.contains_key(&0x165) {
            let mut img_data = Bytes::from(m.remove(&0x105).unwrap());
            let sign_len = img_data.get_u16();
            img_data.get_u16();
            let sign = img_data.copy_to_bytes(sign_len as usize);
            return Some(LoginResponse::NeedCaptcha {
                captcha_sign: sign,
                captcha_image: img_data,
            });
        } else {
            return Some(LoginResponse::UnknownLoginError {
                error_message: "".to_string()
            });
        }
    } // need captcha

    if t == 40 {
        return Some(LoginResponse::UnknownLoginError {
            error_message: "账号被冻结".to_string(),
        });
    }

    if t == 160 || t == 239 {
        let mut cache_info = cli.cache_info.write().await;
        if m.contains_key(&0x174) {
            cache_info.t174 = m.remove(&0x147).unwrap();
            cache_info.t104 = m.remove(&0x104).unwrap();
            cache_info.rand_seed = m.remove(&0x403).unwrap();
            let phone = {
                let mut r = Bytes::from(m.remove(&0x178).unwrap());
                let len = r.get_i32() as usize;
                r.read_string_limit(len)
            };
            if m.contains_key(&0x204) {
                return Some(LoginResponse::SMSOrVerifyNeededError {
                    verify_url: String::from_utf8(m.remove(&0x204).unwrap().to_vec()).unwrap(),
                    sms_phone: phone,
                    error_message: String::from_utf8(m.remove(&0x17e).unwrap().to_vec()).unwrap(),
                });
            }
            return Some(LoginResponse::SMSNeededError {
                sms_phone: phone,
                error_message: String::from_utf8(m.remove(&0x17e).unwrap().to_vec()).unwrap(),
            });
        }

        if m.contains_key(&0x17b) {
            cache_info.t104 = m.remove(&0x104).unwrap();
            return Some(LoginResponse::SMSNeededError {
                sms_phone: "".to_string(),
                error_message: "".to_string(),
            });
        }

        if m.contains_key(&0x204) {
            return Some(LoginResponse::UnsafeDeviceError {
                verify_url: String::from_utf8(m.remove(&0x204).unwrap().to_vec()).unwrap(),
            });
        }
    }

    if t == 162 {
        return Some(LoginResponse::TooManySMSRequestError);
    }

    if t == 204 {
        {
            let mut cache_info = cli.cache_info.write().await;
            cache_info.t104 = m.remove(&0x104).unwrap();
            cache_info.rand_seed = m.remove(&0x403).unwrap();
        }
        // TODO c.sendAndWait(c.buildDeviceLockLoginPacket())
        let (num, vec) = cli.build_device_lock_login_packet().await;
        println!("{} - {:?}", num, vec);
        return None;
    } // drive lock

    if m.contains_key(&0x149) {
        let mut t149r = Bytes::from(m.remove(&0x149).unwrap());
        t149r.advance(2);
        t149r.read_string_short();//title
        return Some(LoginResponse::OtherLoginError {
            error_message: t149r.read_string_short(),
        });
    }

    if m.contains_key(&0x146) {
        let mut t146r = Bytes::from(m.remove(&0x146).unwrap());
        t146r.advance(4); // ver and code
        t146r.read_string_short(); // title
        return Some(LoginResponse::OtherLoginError {
            error_message: t146r.read_string_short(),
        });
    }
    return None;
}

pub async fn decode_exchange_emp_response(cli: &mut Client, payload: &[u8]) -> Option<()> {
    let mut cache_info = cli.cache_info.write().await;
    let mut account_info = cli.account_info.write().await;
    let mut payload = Bytes::from(payload.to_owned());
    let cmd = payload.get_u16();
    let t = payload.get_u8();
    payload.get_u16();
    let m = payload.read_tlv_map(2);
    if t != 0 {
        return None;
    }
    if cmd == 15 {
        decode_t119r(m.get(&0x119).unwrap(), &cli.device_info.read().await.tgtgt_key, &mut cache_info, &mut account_info);
    }
    if cmd == 11 {
        let h = md5::compute(&cli.cache_info.read().await.sig_info.d2key).to_vec();
        decode_t119(m.get(&0x119).unwrap(), &h, &mut cache_info, &mut account_info).await;
    }
    return None;
}