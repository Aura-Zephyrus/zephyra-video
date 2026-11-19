//! `status.rs` 的断言：解码状态码。

use super::super::{ok, DecoderStatus, DecoderStatusCode};

#[test]
fn default_status_is_ok() {
    let status = DecoderStatus::default();
    assert!(status.is_ok());
    assert!(!status.is_error());
    assert_eq!(status.code(), DecoderStatusCode::Ok);
    assert!(status.message().is_empty());
}

#[test]
fn helper_ok_matches_default() {
    assert_eq!(ok(), DecoderStatus::default());
}

#[test]
fn message_is_carried_but_not_part_of_equality_of_codes() {
    let with_message = DecoderStatus::with_message(
        DecoderStatusCode::UnsupportedConfig,
        "no decoder candidates",
    );
    assert_eq!(with_message.code(), DecoderStatusCode::UnsupportedConfig);
    assert_eq!(with_message.message(), "no decoder candidates");
    assert!(with_message.is_error());
}

#[test]
fn display_includes_code_and_message() {
    let status = DecoderStatus::with_message(DecoderStatusCode::Failed, "boom");
    let text = status.to_string();
    assert!(text.contains("Failed"), "{text}");
    assert!(text.contains("boom"), "{text}");
}

#[test]
fn display_omits_empty_message() {
    let text = DecoderStatus::new(DecoderStatusCode::Ok).to_string();
    assert_eq!(text, "Ok");
}

#[test]
fn only_hard_errors_require_reset() {
    assert!(DecoderStatus::new(DecoderStatusCode::Failed).requires_reset());
    assert!(DecoderStatus::new(DecoderStatusCode::PlatformDecodeFailure).requires_reset());
    assert!(DecoderStatus::new(DecoderStatusCode::MalformedBitstream).requires_reset());
    // 这两个**不**要求复位：流结束与「因配置变更省略流结束」都是正常路径。
    assert!(!DecoderStatus::new(DecoderStatusCode::Ok).requires_reset());
    assert!(!DecoderStatus::new(DecoderStatusCode::Aborted).requires_reset());
    assert!(
        !DecoderStatus::new(DecoderStatusCode::ElidedEndOfStreamForConfigChange).requires_reset()
    );
}

#[test]
fn status_codes_keep_chromium_values() {
    // 这些值会进 UMA 直方图，改值等于改历史。
    assert_eq!(DecoderStatusCode::Ok as i32, 0);
    assert_eq!(DecoderStatusCode::Failed as i32, 1);
    assert_eq!(DecoderStatusCode::Aborted as i32, 2);
    assert_eq!(DecoderStatusCode::NotInitialized as i32, 100);
    assert_eq!(DecoderStatusCode::PlatformDecodeFailure as i32, 103);
    assert_eq!(DecoderStatusCode::MalformedBitstream as i32, 104);
    assert_eq!(DecoderStatusCode::KeyFrameRequired as i32, 111);
    assert_eq!(DecoderStatusCode::UnsupportedProfile as i32, 200);
    assert_eq!(DecoderStatusCode::UnsupportedConfig as i32, 202);
    assert_eq!(DecoderStatusCode::FailedToCreateDecoder as i32, 205);
    assert_eq!(DecoderStatusCode::TooManyDecoders as i32, 206);
    assert_eq!(
        DecoderStatusCode::ElidedEndOfStreamForConfigChange as i32,
        300
    );
}
