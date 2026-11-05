//! The engine's comment dialog (0x213c) through the system keyboard. Game strings are
//! Shift-JIS, the only encoding cmvs-font renders.
use cmvs_core::platform::{TextDialogRequest, TextDialogResult};
use encoding_rs::SHIFT_JIS;
use std::error::Error;

/// Shows (title, prompt, initial, max_chars, check) and returns the text, or None when
/// cancelled; `check` rejects input with a message while the keyboard stays open.
pub type Keyboard<'a> = dyn FnMut(
        &str,
        &str,
        &str,
        u32,
        &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<Option<String>, &'static str>
    + 'a;

fn decode(bytes: &[u8]) -> Result<String, Box<dyn Error>> {
    Ok(SHIFT_JIS
        .decode_without_bom_handling_and_without_replacement(bytes)
        .ok_or("text dialog string is not valid Shift-JIS")?
        .into_owned())
}

/// Characters of `text` that Shift-JIS cannot represent, in order, without repeats.
fn unmappable(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        let mut buffer = [0; 4];
        if SHIFT_JIS.encode(ch.encode_utf8(&mut buffer)).2 && !result.contains(ch) {
            result.push(ch);
        }
    }
    result
}

pub fn edit(
    request: &TextDialogRequest,
    keyboard: &mut Keyboard<'_>,
) -> Result<TextDialogResult, Box<dyn Error>> {
    let title = decode(&request.title)?;
    let prompt = decode(&request.prompt)?;
    let initial = decode(&request.initial)?;
    // The desktop host's limit: the byte budget in full-width characters, which is
    // also the only kind the engine keeps.
    let limit = if request.max_bytes == 0 {
        127
    } else {
        (request.max_bytes.min(255) / 2).max(1)
    };
    let mut check = |text: &str| match unmappable(text) {
        bad if bad.is_empty() => Ok(()),
        bad => Err(format!(
            "以下字符无法用游戏文字编码（Shift-JIS）保存：{bad}"
        )),
    };
    Ok(
        match keyboard(&title, &prompt, &initial, limit, &mut check)? {
            None => TextDialogResult::Cancelled,
            Some(text) => {
                let (bytes, _, unmappable) = SHIFT_JIS.encode(&text);
                if unmappable {
                    return Err("keyboard text is not representable in Shift-JIS".into());
                }
                TextDialogResult::Accepted(bytes.into_owned())
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(max_bytes: u32) -> TextDialogRequest {
        let sjis = |s: &str| SHIFT_JIS.encode(s).0.into_owned();
        TextDialogRequest {
            title: sjis("コメント"),
            prompt: sjis("入力してください"),
            initial: sjis("前回"),
            max_bytes,
        }
    }

    #[test]
    fn decodes_the_request_and_encodes_the_accepted_text() {
        let mut seen = None;
        let mut keyboard = |title: &str,
                            prompt: &str,
                            initial: &str,
                            limit: u32,
                            check: &mut dyn FnMut(&str) -> Result<(), String>|
         -> Result<Option<String>, &'static str> {
            seen = Some((
                title.to_owned(),
                prompt.to_owned(),
                initial.to_owned(),
                limit,
            ));
            assert!(check("今日の記録").is_ok());
            let rejected = check("記録😀と😀🎮").unwrap_err();
            assert!(rejected.ends_with("：😀🎮"), "{rejected}");
            Ok(Some("今日の記録".into()))
        };
        let TextDialogResult::Accepted(bytes) = edit(&request(100), &mut keyboard).unwrap() else {
            panic!("expected accepted text");
        };
        assert_eq!(bytes, SHIFT_JIS.encode("今日の記録").0.as_ref());
        assert_eq!(
            seen.unwrap(),
            (
                "コメント".into(),
                "入力してください".into(),
                "前回".into(),
                50
            )
        );
    }

    #[test]
    fn limits_cancel_and_invalid_strings() {
        for (max_bytes, expected) in [(0, 127), (1, 1), (1000, 127)] {
            let mut keyboard = |_: &str,
                                _: &str,
                                _: &str,
                                limit: u32,
                                _: &mut dyn FnMut(&str) -> Result<(), String>|
             -> Result<Option<String>, &'static str> {
                assert_eq!(limit, expected);
                Ok(None)
            };
            assert!(matches!(
                edit(&request(max_bytes), &mut keyboard).unwrap(),
                TextDialogResult::Cancelled
            ));
        }
        let mut invalid = request(100);
        invalid.prompt = vec![0x82];
        let mut keyboard = |_: &str,
                            _: &str,
                            _: &str,
                            _: u32,
                            _: &mut dyn FnMut(&str) -> Result<(), String>|
         -> Result<Option<String>, &'static str> { unreachable!() };
        assert!(edit(&invalid, &mut keyboard).is_err());
    }
}
