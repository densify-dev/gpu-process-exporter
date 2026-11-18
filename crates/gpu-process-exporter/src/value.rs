// SPDX-License-Identifier: Apache-2.0

//! Parsing of string values (environment variables, annotations) with Go `strconv` and `time` semantics, and
//! label fingerprints.

use std::collections::BTreeMap;

/// A source of named string values. A missing value is the empty string.
pub trait Provider {
    fn get_value(&self, name: &str) -> String;
}

pub struct EnvProvider;

impl Provider for EnvProvider {
    fn get_value(&self, name: &str) -> String {
        std::env::var(name).unwrap_or_default()
    }
}

pub struct MapProvider<'a>(pub Option<&'a BTreeMap<String, String>>);

impl Provider for MapProvider<'_> {
    fn get_value(&self, name: &str) -> String {
        self.0
            .and_then(|m| m.get(name))
            .cloned()
            .unwrap_or_default()
    }
}

/// Parses a trimmed, non-empty string the way the Go exporter does.
pub trait Parse: Sized {
    fn parse_go(s: &str) -> Result<Self, String>;
}

pub fn parse<T: Parse>(s: &str) -> Result<T, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("cannot parse an empty (trimmed) string".into());
    }
    T::parse_go(trimmed)
}

pub fn parse_or_default<T: Parse>(s: &str, default: T) -> T {
    parse(s).unwrap_or(default)
}

pub fn parse_value<T: Parse>(p: &dyn Provider, name: &str) -> Result<T, String> {
    parse(&p.get_value(name))
}

pub fn find_parsed_value<T: Parse>(p: &dyn Provider, name: &str) -> Option<T> {
    parse_value(p, name).ok()
}

pub fn parse_value_or_default<T: Parse>(p: &dyn Provider, name: &str, default: T) -> T {
    parse_or_default(&p.get_value(name), default)
}

impl Parse for String {
    fn parse_go(s: &str) -> Result<Self, String> {
        Ok(s.to_string())
    }
}

/// `strconv.ParseBool`.
impl Parse for bool {
    fn parse_go(s: &str) -> Result<Self, String> {
        match s {
            "1" | "t" | "T" | "TRUE" | "true" | "True" => Ok(true),
            "0" | "f" | "F" | "FALSE" | "false" | "False" => Ok(false),
            _ => Err(format!("invalid bool {s:?}")),
        }
    }
}

/// `strconv.ParseInt(s, 10, 64)`.
impl Parse for i64 {
    fn parse_go(s: &str) -> Result<Self, String> {
        s.parse().map_err(|e| format!("invalid int {s:?}: {e}"))
    }
}

/// `strconv.ParseUint(s, 10, 64)`, which rejects a sign.
impl Parse for u64 {
    fn parse_go(s: &str) -> Result<Self, String> {
        if s.starts_with('+') {
            return Err(format!("invalid uint {s:?}"));
        }
        s.parse().map_err(|e| format!("invalid uint {s:?}: {e}"))
    }
}

/// `strconv.ParseFloat(s, 64)`: decimal and hex floats, `inf`/`infinity`/`nan` in any case, and an error on
/// overflow.
impl Parse for f64 {
    fn parse_go(s: &str) -> Result<Self, String> {
        let err = || format!("invalid float {s:?}");
        let (sign, unsigned) = match s.as_bytes().first() {
            Some(b'-') => (-1.0, &s[1..]),
            Some(b'+') => (1.0, &s[1..]),
            _ => (1.0, s),
        };
        let lower = unsigned.to_ascii_lowercase();
        if lower == "inf" || lower == "infinity" {
            return Ok(sign * f64::INFINITY);
        }
        if lower == "nan" {
            return if unsigned.len() == s.len() {
                Ok(f64::NAN)
            } else {
                Err(err())
            };
        }
        let value = if let Some(hex) = lower.strip_prefix("0x") {
            sign * parse_hex_float(hex).ok_or_else(err)?
        } else if unsigned
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        {
            s.parse::<f64>().map_err(|_| err())?
        } else {
            return Err(err());
        };
        if value.is_infinite() {
            Err(format!("float {s:?} out of range"))
        } else {
            Ok(value)
        }
    }
}

/// Parses the part after `0x` of a Go hex float: hex mantissa with optional point, mandatory `p` exponent,
/// and underscores only between digits or after the prefix.
fn parse_hex_float(s: &str) -> Option<f64> {
    let (mantissa, exponent) = s.split_once('p')?;
    let mut prev_ok = true; // the base prefix counts as a digit for underscores
    let mut digits = String::new();
    for (i, c) in mantissa.char_indices() {
        if c == '_' {
            let next_ok = mantissa[i + 1..]
                .chars()
                .next()
                .is_some_and(|n| n.is_ascii_hexdigit());
            if !prev_ok || !next_ok {
                return None;
            }
        } else {
            digits.push(c);
        }
        prev_ok = c.is_ascii_hexdigit();
    }
    let (int, frac) = digits.split_once('.').unwrap_or((&digits, ""));
    if int.is_empty() && frac.is_empty()
        || !int
            .chars()
            .chain(frac.chars())
            .all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    let exponent: i32 = exponent.parse().ok()?;
    let mut value = 0f64;
    for c in int.chars().chain(frac.chars()) {
        value = value * 16.0 + f64::from(c.to_digit(16)?);
    }
    Some(value * 2f64.powi(exponent - 4 * frac.len() as i32))
}

/// A Go `time.Duration`: signed nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GoDuration(pub i64);

impl Parse for GoDuration {
    fn parse_go(s: &str) -> Result<Self, String> {
        parse_duration(s).map(GoDuration)
    }
}

/// Port of Go's `time.ParseDuration`, returning nanoseconds.
pub fn parse_duration(orig: &str) -> Result<i64, String> {
    let invalid = || format!("time: invalid duration {orig:?}");
    let mut s = orig.as_bytes();
    let mut neg = false;
    if let Some(&c @ (b'-' | b'+')) = s.first() {
        neg = c == b'-';
        s = &s[1..];
    }
    if s == b"0" {
        return Ok(0);
    }
    if s.is_empty() {
        return Err(invalid());
    }
    const LIMIT: u64 = 1 << 63;
    let mut d: u64 = 0;
    while !s.is_empty() {
        if !(s[0] == b'.' || s[0].is_ascii_digit()) {
            return Err(invalid());
        }
        let before = s.len();
        let (mut v, rest) = leading_int(s).ok_or_else(invalid)?;
        s = rest;
        let pre = before != s.len();
        let (mut f, mut scale, mut post) = (0u64, 1f64, false);
        if s.first() == Some(&b'.') {
            s = &s[1..];
            let before = s.len();
            (f, scale, s) = leading_fraction(s);
            post = before != s.len();
        }
        if !pre && !post {
            return Err(invalid());
        }
        let i = s
            .iter()
            .position(|&c| c == b'.' || c.is_ascii_digit())
            .unwrap_or(s.len());
        if i == 0 {
            return Err(format!("time: missing unit in duration {orig:?}"));
        }
        let unit = match &s[..i] {
            b"ns" => 1,
            b"us" | b"\xc2\xb5s" | b"\xce\xbcs" => 1_000, // us, U+00B5 µs, U+03BC μs
            b"ms" => 1_000_000,
            b"s" => 1_000_000_000,
            b"m" => 60_000_000_000,
            b"h" => 3_600_000_000_000,
            u => {
                return Err(format!(
                    "time: unknown unit {:?} in duration {orig:?}",
                    String::from_utf8_lossy(u)
                ));
            }
        };
        s = &s[i..];
        if v > LIMIT / unit {
            return Err(invalid());
        }
        v *= unit;
        if f > 0 {
            v += (f as f64 * (unit as f64 / scale)) as u64;
            if v > LIMIT {
                return Err(invalid());
            }
        }
        d += v;
        if d > LIMIT {
            return Err(invalid());
        }
    }
    if neg {
        return Ok((d as i64).wrapping_neg());
    }
    if d > LIMIT - 1 {
        return Err(invalid());
    }
    Ok(d as i64)
}

fn leading_int(s: &[u8]) -> Option<(u64, &[u8])> {
    let mut x: u64 = 0;
    let i = s
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(s.len());
    for &c in &s[..i] {
        if x > (1 << 63) / 10 {
            return None;
        }
        x = x * 10 + u64::from(c - b'0');
        if x > 1 << 63 {
            return None;
        }
    }
    Some((x, &s[i..]))
}

fn leading_fraction(s: &[u8]) -> (u64, f64, &[u8]) {
    let (mut x, mut scale, mut overflow) = (0u64, 1f64, false);
    let i = s
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(s.len());
    for &c in &s[..i] {
        if overflow {
            continue;
        }
        if x > ((1 << 63) - 1) / 10 {
            overflow = true;
            continue;
        }
        let y = x * 10 + u64::from(c - b'0');
        if y > 1 << 63 {
            overflow = true;
            continue;
        }
        x = y;
        scale *= 10.0;
    }
    (x, scale, &s[i..])
}

/// FNV-1a 64 over the strings, each followed by a zero byte.
pub fn fingerprint<S: AsRef<str>>(strs: &[S]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    strs.iter()
        .flat_map(|s| s.as_ref().bytes().chain([0]))
        .fold(OFFSET, |h, b| (h ^ u64::from(b)).wrapping_mul(PRIME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn parse_supported_types() {
        assert_eq!(parse::<String>("  hello  ").unwrap(), "hello");
        assert!(parse::<bool>(" true ").unwrap());
        assert_eq!(parse::<i64>(" 42 ").unwrap(), 42);
        assert_eq!(
            parse::<GoDuration>("1500ms").unwrap(),
            GoDuration(1_500_000_000)
        );
        assert_eq!(parse::<u64>(" 99 ").unwrap(), 99);
        assert_eq!(parse::<f64>(" 1.25 ").unwrap(), 1.25);
    }

    #[test]
    fn parse_rejects_empty_and_invalid_values() {
        assert!(parse::<String>("   ").unwrap_err().contains("empty"));
        assert!(parse::<i64>("not-an-int").is_err());
    }

    #[test]
    fn parse_or_default_and_provider_helpers() {
        let m: BTreeMap<String, String> = [("bool", "true"), ("number", "7"), ("bad", "oops")]
            .map(|(k, v)| (k.into(), v.into()))
            .into();
        let provider = MapProvider(Some(&m));
        assert_eq!(parse_or_default("bad", 123u64), 123);
        assert!(parse_value::<bool>(&provider, "bool").unwrap());
        assert_eq!(find_parsed_value::<i64>(&provider, "number"), Some(7));
        assert_eq!(find_parsed_value::<i64>(&provider, "bad"), None);
        assert_eq!(parse_value_or_default(&provider, "missing", 55u64), 55);
    }

    #[test]
    fn env_provider_and_map_provider() {
        // SAFETY: no other test reads or writes this variable.
        unsafe { std::env::set_var("VALUE_TEST_ENV", "env-value") };
        assert_eq!(EnvProvider.get_value("VALUE_TEST_ENV"), "env-value");
        let m: BTreeMap<String, String> = [("k".into(), "v".into())].into();
        assert_eq!(MapProvider(Some(&m)).get_value("k"), "v");
        assert_eq!(MapProvider(None).get_value("missing"), "");
    }

    #[test]
    fn env_provider_reflects_process_environment() {
        const KEY: &str = "VALUE_TEST_PROCESS_ENV";
        // SAFETY: no other test reads or writes this variable.
        unsafe { std::env::set_var(KEY, "setenv-value") };
        assert_eq!(EnvProvider.get_value(KEY), "setenv-value");
        unsafe { std::env::remove_var(KEY) };
    }

    #[test]
    fn parse_go_number_syntax() {
        assert_eq!(parse::<f64>("0x1p-2").unwrap(), 0.25);
        assert_eq!(parse::<f64>("0X1.8P1").unwrap(), 3.0);
        assert_eq!(parse::<f64>("0x_1p0").unwrap(), 1.0);
        assert!(parse::<f64>("0x10").is_err());
        assert!(parse::<f64>("1e400").is_err());
        assert!(parse::<f64>("+nan").is_err());
        assert!(parse::<f64>("NaN").unwrap().is_nan());
        assert_eq!(parse::<f64>("-Inf").unwrap(), f64::NEG_INFINITY);
        assert!(parse::<f64>("1_0").is_err());
        assert!(parse::<u64>("+1024").is_err());
        assert_eq!(parse::<i64>("+1024").unwrap(), 1024);
        assert!(parse::<bool>("yes").is_err());
    }

    #[derive(Deserialize)]
    struct DurationCase {
        #[serde(rename = "in")]
        input: String,
        ns: Option<i64>,
        #[serde(default)]
        error: bool,
    }

    #[test]
    fn parse_duration_fixtures() {
        let cases: Vec<DurationCase> =
            serde_json::from_str(include_str!("../../../testdata/durations.json")).unwrap();
        for c in cases {
            let got = parse_duration(&c.input);
            if c.error {
                assert!(
                    got.is_err(),
                    "parse_duration({:?}) = {got:?}, want error",
                    c.input
                );
            } else {
                assert_eq!(got, Ok(c.ns.unwrap()), "parse_duration({:?})", c.input);
            }
        }
    }

    #[test]
    fn fingerprint_uses_label_boundaries() {
        assert_ne!(fingerprint(&["ab", "c"]), fingerprint(&["a", "bc"]));
        // Go: hash/fnv New64a over "node-a\x00ns-a\x00".
        assert_eq!(fingerprint(&["node-a", "ns-a"]), 0xcb91_e6c8_d594_3f9c);
    }
}
