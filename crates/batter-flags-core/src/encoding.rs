//! State encoding 1, decoded strictly.
//!
//! A stored row's state is `jsonb` text: `{"encoding":1,"variant":"on"}` for a
//! release or mode flag and `{"encoding":1,"switch":"engaged"}` or
//! `{"encoding":1,"switch":"released"}` for a kill switch. Encoding 1 admits
//! exactly those two fields: a higher encoding, a missing field, an extra
//! field, a repeated field or a field of the wrong shape is not encoding 1.
//!
//! The bare crate carries no JSON library, and `serde_json` would accept a
//! repeated field (the last wins), so this module scans the one shape it
//! needs. It accepts any member order and any JSON whitespace, because
//! `jsonb` normalizes both, and the escapes JSON defines inside strings.

use std::borrow::Cow;

use crate::decision::SwitchState;

/// What a row's state decoded to.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Decoded<'a> {
    /// A release or mode flag's variant name, not yet checked against its
    /// declaration.
    Flag(Cow<'a, str>),
    /// A kill switch's state.
    Switch(SwitchState),
}

/// The shape a row's declaration expects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Expect {
    Flag,
    Switch,
}

/// The state text is not exactly encoding 1.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Undecodable;

/// Decodes `text` as encoding 1 of the expected shape.
pub(crate) fn decode(text: &str, expect: Expect) -> Result<Decoded<'_>, Undecodable> {
    Scanner { text, at: 0 }.state(expect)
}

struct Scanner<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Scanner<'a> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.peek() == Some(byte);
        if found {
            self.at += 1;
        }
        found
    }

    fn expect(&mut self, byte: u8) -> Result<(), Undecodable> {
        if self.eat(byte) {
            Ok(())
        } else {
            Err(Undecodable)
        }
    }

    /// The whole state object, with nothing after it.
    fn state(mut self, expect: Expect) -> Result<Decoded<'a>, Undecodable> {
        let payload_name = match expect {
            Expect::Flag => "variant",
            Expect::Switch => "switch",
        };
        self.skip_whitespace();
        self.expect(b'{')?;
        let mut encoding_seen = false;
        let mut payload: Option<Cow<'a, str>> = None;
        loop {
            self.skip_whitespace();
            let name = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            if name == "encoding" {
                if encoding_seen || self.number_token()? != "1" {
                    return Err(Undecodable);
                }
                encoding_seen = true;
            } else if name == payload_name {
                if payload.is_some() {
                    return Err(Undecodable);
                }
                payload = Some(self.string()?);
            } else {
                return Err(Undecodable);
            }
            self.skip_whitespace();
            if !self.eat(b',') {
                self.expect(b'}')?;
                break;
            }
        }
        self.skip_whitespace();
        if self.peek().is_some() {
            return Err(Undecodable);
        }
        let payload = payload.filter(|_| encoding_seen).ok_or(Undecodable)?;
        match expect {
            Expect::Flag => Ok(Decoded::Flag(payload)),
            Expect::Switch => match &*payload {
                "engaged" => Ok(Decoded::Switch(SwitchState::Engaged)),
                "released" => Ok(Decoded::Switch(SwitchState::Released)),
                _ => Err(Undecodable),
            },
        }
    }

    /// The characters of a JSON number, left for the caller to compare: the
    /// only number encoding 1 admits is the token `1`.
    fn number_token(&mut self) -> Result<&'a str, Undecodable> {
        let start = self.at;
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        ) {
            self.at += 1;
        }
        if self.at == start {
            Err(Undecodable)
        } else {
            Ok(&self.text[start..self.at])
        }
    }

    /// A JSON string, borrowed when it holds no escape.
    fn string(&mut self) -> Result<Cow<'a, str>, Undecodable> {
        self.expect(b'"')?;
        let start = self.at;
        loop {
            match self.peek() {
                None => return Err(Undecodable),
                Some(b'"') => {
                    let borrowed = &self.text[start..self.at];
                    self.at += 1;
                    return Ok(Cow::Borrowed(borrowed));
                }
                Some(b'\\') => break,
                Some(byte) if byte < 0x20 => return Err(Undecodable),
                Some(_) => self.at += 1,
            }
        }
        let mut owned = String::from(&self.text[start..self.at]);
        loop {
            match self.peek() {
                None => return Err(Undecodable),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(Cow::Owned(owned));
                }
                Some(b'\\') => {
                    self.at += 1;
                    owned.push(self.escape()?);
                }
                Some(byte) if byte < 0x20 => return Err(Undecodable),
                Some(_) => {
                    let character = self.text[self.at..].chars().next().ok_or(Undecodable)?;
                    owned.push(character);
                    self.at += character.len_utf8();
                }
            }
        }
    }

    /// The character after a backslash.
    fn escape(&mut self) -> Result<char, Undecodable> {
        let byte = self.peek().ok_or(Undecodable)?;
        self.at += 1;
        Ok(match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => self.unicode()?,
            _ => return Err(Undecodable),
        })
    }

    /// A `\u` escape, joining a surrogate pair into one character.
    fn unicode(&mut self) -> Result<char, Undecodable> {
        let first = self.hex4()?;
        let code = match first {
            0xD800..=0xDBFF => {
                self.expect(b'\\')?;
                self.expect(b'u')?;
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(Undecodable);
                }
                0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            0xDC00..=0xDFFF => return Err(Undecodable),
            _ => first,
        };
        char::from_u32(code).ok_or(Undecodable)
    }

    fn hex4(&mut self) -> Result<u32, Undecodable> {
        let mut value = 0;
        for _ in 0..4 {
            let digit = self.peek().ok_or(Undecodable)?;
            let digit = char::from(digit).to_digit(16).ok_or(Undecodable)?;
            value = value * 16 + digit;
            self.at += 1;
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(text: &str) -> Result<String, Undecodable> {
        match decode(text, Expect::Flag)? {
            Decoded::Flag(variant) => Ok(variant.into_owned()),
            Decoded::Switch(_) => unreachable!("a flag decode yields a flag"),
        }
    }

    fn switch(text: &str) -> Result<SwitchState, Undecodable> {
        match decode(text, Expect::Switch)? {
            Decoded::Switch(state) => Ok(state),
            Decoded::Flag(_) => unreachable!("a switch decode yields a switch"),
        }
    }

    #[test]
    fn canonical_and_jsonb_spellings_decode_alike() {
        for text in [
            r#"{"encoding":1,"variant":"on"}"#,
            r#"{"variant":"on","encoding":1}"#,
            r#"{"variant": "on", "encoding": 1}"#,
            "  {\n\t\"encoding\" :\r\n 1 ,\n\"variant\"\t:\t\"on\"\n}  \n",
            r#"{"encoding":1,"variant":"\u006fn"}"#,
            r#"{"\u0065ncoding":1,"variant":"on"}"#,
        ] {
            assert_eq!(flag(text).as_deref(), Ok("on"), "{text:?}");
        }
        assert_eq!(
            switch(r#"{"encoding":1,"switch":"engaged"}"#),
            Ok(SwitchState::Engaged)
        );
        assert_eq!(
            switch(r#"{"switch": "released", "encoding": 1}"#),
            Ok(SwitchState::Released)
        );
    }

    #[test]
    fn escapes_decode_to_the_characters_json_defines() {
        let text = r#"{"encoding":1,"variant":"a\"b\\c\/d\b\f\n\r\t\u00e9\ud83d\ude00z"}"#;
        assert_eq!(
            flag(text).as_deref(),
            Ok("a\"b\\c/d\u{8}\u{c}\n\r\t\u{e9}\u{1f600}z")
        );
        assert_eq!(
            flag("{\"encoding\":1,\"variant\":\"caf\u{e9} \u{1f600}\"}").as_deref(),
            Ok("caf\u{e9} \u{1f600}")
        );
    }

    #[test]
    fn anything_else_is_not_encoding_one() {
        let refused = [
            "",
            " ",
            "null",
            "[]",
            "{}",
            r#"{"encoding":1}"#,
            r#"{"variant":"on"}"#,
            r#"{"encoding":2,"variant":"on"}"#,
            r#"{"encoding":0,"variant":"on"}"#,
            r#"{"encoding":"1","variant":"on"}"#,
            r#"{"encoding":1.0,"variant":"on"}"#,
            r#"{"encoding":01,"variant":"on"}"#,
            r#"{"encoding":1e0,"variant":"on"}"#,
            r#"{"encoding":true,"variant":"on"}"#,
            r#"{"encoding":null,"variant":"on"}"#,
            r#"{"encoding":-1,"variant":"on"}"#,
            r#"{"encoding":1,"variant":"on","note":"x"}"#,
            r#"{"encoding":1,"variant":"on","encoding":1}"#,
            r#"{"encoding":1,"variant":"on","variant":"on"}"#,
            r#"{"encoding":1,"variant":1}"#,
            r#"{"encoding":1,"variant":null}"#,
            r#"{"encoding":1,"variant":["on"]}"#,
            r#"{"encoding":1,"switch":"engaged"}"#,
            r#"{"encoding":1,"variant":"on"} x"#,
            r#"{"encoding":1,"variant":"on"}{"#,
            r#"{"encoding":1,"variant":"on",}"#,
            r#"{"encoding":1 "variant":"on"}"#,
            r#"{"encoding":1,"variant":"on""#,
            r#"{"encoding":1,"variant":"on}"#,
            r#"{"encoding":1,"variant":"o"n"}"#,
            "{\"encoding\":1,\"variant\":\"o\nn\"}",
            r#"{"encoding":1,"variant":"o\qn"}"#,
            r#"{"encoding":1,"variant":"\u00"}"#,
            r#"{"encoding":1,"variant":"\u00zz"}"#,
            r#"{"encoding":1,"variant":"\ud83d"}"#,
            r#"{"encoding":1,"variant":"\ud83dx"}"#,
            r#"{"encoding":1,"variant":"\ud83d\u0041"}"#,
            r#"{"encoding":1,"variant":"\ude00"}"#,
            r#"{'encoding':1,'variant':'on'}"#,
            r#"{encoding:1,variant:"on"}"#,
            r#"{"encoding":1,"variant":"on"#,
        ];
        for text in refused {
            assert_eq!(flag(text), Err(Undecodable), "{text:?}");
        }
    }

    #[test]
    fn a_switch_state_is_engaged_or_released_and_nothing_else() {
        for text in [
            r#"{"encoding":1,"switch":"tripped"}"#,
            r#"{"encoding":1,"switch":"Engaged"}"#,
            r#"{"encoding":1,"switch":""}"#,
            r#"{"encoding":1,"switch":true}"#,
            r#"{"encoding":1,"variant":"engaged"}"#,
            r#"{"encoding":1,"switch":"engaged","variant":"on"}"#,
        ] {
            assert_eq!(switch(text), Err(Undecodable), "{text:?}");
        }
    }
}
