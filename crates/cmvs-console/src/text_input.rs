//! The engine's comment dialog (0x213c) through the system keyboard. Game strings use
//! the selected game encoding in both directions.
use cmvs_core::{
    TextEncoding,
    platform::{TextDialogRequest, TextDialogResult},
};
use cmvs_font::text_codec;
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

/// The encoding as the player reads it in the rejection message.
fn label(encoding: TextEncoding) -> &'static str {
    match encoding {
        TextEncoding::Gbk => "GBK",
        TextEncoding::Utf8 => "UTF-8",
        _ => "Shift-JIS",
    }
}

pub fn edit(
    request: &TextDialogRequest,
    encoding: TextEncoding,
    keyboard: &mut Keyboard<'_>,
) -> Result<TextDialogResult, Box<dyn Error>> {
    let decode = |bytes: &[u8]| text_codec::decode(encoding, bytes);
    let title = decode(&request.title)?;
    let prompt = decode(&request.prompt)?;
    let initial = decode(&request.initial)?;
    let budget = if request.max_bytes == 0 {
        255
    } else {
        request.max_bytes.min(255)
    };
    // UTF-8 has variable-width characters; the callback enforces the byte budget.
    let limit = if encoding == TextEncoding::Utf8 {
        budget.min(127)
    } else {
        (budget / 2).max(1)
    };
    let mut check = |text: &str| match text_codec::unmappable(encoding, text) {
        _ if text.contains('\0') => Err("Input cannot contain NUL characters".into()),
        Ok(bad) if bad.is_empty() => {
            let bytes = text_codec::encode(encoding, text).map_err(|error| error.to_string())?;
            if bytes.len() > budget as usize {
                Err(format!(
                    "Input exceeds the {budget}-byte limit in the game encoding"
                ))
            } else {
                Ok(())
            }
        }
        Ok(bad) => Err(format!(
            "These characters cannot be saved in the game encoding ({}): {bad}",
            label(encoding)
        )),
        Err(error) => Err(error.to_string()),
    };
    Ok(
        match keyboard(&title, &prompt, &initial, limit, &mut check)? {
            None => TextDialogResult::Cancelled,
            Some(text) => TextDialogResult::Accepted(text_codec::encode(encoding, &text)?),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::SHIFT_JIS;

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
            assert_eq!(
                rejected,
                "These characters cannot be saved in the game encoding (Shift-JIS): 😀🎮"
            );
            Ok(Some("今日の記録".into()))
        };
        let TextDialogResult::Accepted(bytes) =
            edit(&request(100), TextEncoding::ShiftJis, &mut keyboard).unwrap()
        else {
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
                edit(&request(max_bytes), TextEncoding::ShiftJis, &mut keyboard).unwrap(),
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
        assert!(edit(&invalid, TextEncoding::ShiftJis, &mut keyboard).is_err());
    }

    #[test]
    fn gbk_games_round_trip_simplified_chinese() {
        let gbk = |s: &str| text_codec::encode(TextEncoding::Gbk, s).unwrap();
        let request = TextDialogRequest {
            title: gbk("注释"),
            prompt: gbk("请输入"),
            initial: gbk("上次"),
            max_bytes: 100,
        };
        let mut keyboard = |title: &str,
                            _: &str,
                            initial: &str,
                            _: u32,
                            check: &mut dyn FnMut(&str) -> Result<(), String>|
         -> Result<Option<String>, &'static str> {
            assert_eq!((title, initial), ("注释", "上次"));
            let rejected = check("这是记录😀").unwrap_err();
            assert!(
                rejected.contains("(GBK)") && rejected.ends_with(": 😀"),
                "{rejected}"
            );
            Ok(Some("这是记录".into()))
        };
        let TextDialogResult::Accepted(bytes) =
            edit(&request, TextEncoding::Gbk, &mut keyboard).unwrap()
        else {
            panic!("expected accepted text");
        };
        assert_eq!(bytes, gbk("这是记录"));
    }

    #[test]
    fn legacy_input_checks_encoded_bytes_instead_of_utf8_length() {
        for encoding in [TextEncoding::ShiftJis, TextEncoding::Gbk] {
            let request = TextDialogRequest {
                title: vec![],
                prompt: vec![],
                initial: vec![],
                max_bytes: 3,
            };
            let mut keyboard = |_: &str,
                                _: &str,
                                _: &str,
                                _: u32,
                                check: &mut dyn FnMut(&str) -> Result<(), String>|
             -> Result<Option<String>, &'static str> {
                assert!(check("中a").is_ok());
                assert!(check("中中").is_err());
                Ok(None)
            };
            edit(&request, encoding, &mut keyboard).unwrap();
        }
    }

    #[test]
    fn utf8_input_checks_the_encoded_byte_budget() {
        for (max_bytes, budget) in [(0, 255), (1, 1), (100, 100), (1000, 255)] {
            let request = TextDialogRequest {
                title: vec![],
                prompt: vec![],
                initial: vec![],
                max_bytes,
            };
            let text = format!("{}{}", "中".repeat(budget / 3), "a".repeat(budget % 3));
            let mut keyboard = |_: &str,
                                _: &str,
                                _: &str,
                                limit: u32,
                                check: &mut dyn FnMut(&str) -> Result<(), String>|
             -> Result<Option<String>, &'static str> {
                assert_eq!(limit as usize, budget.min(127));
                assert!(check(&text).is_ok());
                assert!(check("\0").is_err());
                assert!(check(&format!("{text}a")).is_err());
                assert!(check(&"中".repeat(budget / 3 + 1)).is_err());
                assert!(check(&"😀".repeat(budget / 4 + 1)).is_err());
                Ok(Some(text.clone()))
            };
            let TextDialogResult::Accepted(bytes) =
                edit(&request, TextEncoding::Utf8, &mut keyboard).unwrap()
            else {
                panic!("expected accepted text");
            };
            assert_eq!(bytes, text.as_bytes());
        }
    }
}
