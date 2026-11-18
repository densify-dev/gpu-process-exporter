// SPDX-License-Identifier: Apache-2.0

//! Content negotiation ported from Go: `expfmt.NegotiateAccept` over goautoneg, and gddo
//! `NegotiateContentEncoding`, as promhttp uses them.

use std::collections::HashMap;

pub const TEXT_FORMAT: &str = "text/plain; version=0.0.4; charset=utf-8";
pub const PROTO_DELIM_FORMAT: &str =
    "application/vnd.google.protobuf; proto=io.prometheus.client.MetricFamily; encoding=delimited";
const DEFAULT_ESCAPING: &str = "underscores";
const ESCAPING_SCHEMES: [&str; 4] = ["allow-utf-8", "underscores", "dots", "values"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    ProtoDelim,
    Text,
}

impl Format {
    fn content_type(self) -> &'static str {
        match self {
            Format::ProtoDelim => PROTO_DELIM_FORMAT,
            Format::Text => TEXT_FORMAT,
        }
    }

    /// The (type, subtype, params) that `matchFormat` compares an Accept clause against; `charset` is ignored.
    fn target(
        self,
    ) -> (
        &'static str,
        &'static str,
        &'static [(&'static str, &'static str)],
    ) {
        match self {
            Format::ProtoDelim => (
                "application",
                "vnd.google.protobuf",
                &[
                    ("proto", "io.prometheus.client.MetricFamily"),
                    ("encoding", "delimited"),
                ],
            ),
            Format::Text => ("text", "plain", &[("version", "0.0.4")]),
        }
    }
}

/// Go also offers the protobuf text formats; only delimited protobuf and text are served (see CONTRACT.md).
const ACCEPTED_FORMATS: [Format; 2] = [Format::ProtoDelim, Format::Text];

#[derive(Debug)]
struct Accept<'a> {
    kind: &'a str,
    sub_kind: &'a str,
    q: f64,
    params: HashMap<&'a str, &'a str>,
}

fn split_once_or_all(s: &str, sep: char) -> (&str, &str) {
    s.split_once(sep).unwrap_or((s, ""))
}

/// Port of goautoneg `ParseAccept`, including its sort.
fn parse_accept(header: &str) -> Vec<Accept<'_>> {
    let mut accepts = Vec::new();
    let mut remaining = header;
    while !remaining.is_empty() {
        let part;
        (part, remaining) = split_once_or_all(remaining, ',');
        let part = part.trim_matches(' ');
        let (media, mut params_part) = split_once_or_all(part, ';');
        let (kind, rest) = split_once_or_all(media, '/');
        let kind = kind.trim_matches(' ');
        let sub_kind = if rest.is_empty() {
            if kind != "*" {
                continue;
            }
            "*"
        } else {
            let (sub, rest) = split_once_or_all(rest, '/');
            if !rest.is_empty() {
                continue;
            }
            sub.trim_matches(' ')
        };
        let mut accept = Accept {
            kind,
            sub_kind,
            q: 1.0,
            params: HashMap::new(),
        };
        while !params_part.is_empty() {
            let param;
            (param, params_part) = split_once_or_all(params_part, ';');
            let (key, rest) = split_once_or_all(param, '=');
            if rest.is_empty() {
                continue;
            }
            let (value, rest) = split_once_or_all(rest, '=');
            if !rest.is_empty() {
                continue;
            }
            let key = key.trim_matches(' ');
            if key == "q" {
                // strconv.ParseFloat(s, 32), whose error result is 0.
                accept.q = value.parse::<f32>().map_or(0.0, f64::from);
            } else {
                accept.params.insert(key, value.trim_matches(' '));
            }
        }
        accepts.push(accept);
    }
    sort_accepts(&mut accepts);
    accepts
}

/// goautoneg's `Less` is not a strict weak order, so the result depends on the algorithm. Go's `sort.Sort` uses
/// insertion sort for up to 12 elements, which covers any realistic Accept header.
fn sort_accepts(accepts: &mut [Accept<'_>]) {
    let less = |a: &Accept<'_>, b: &Accept<'_>| {
        a.q > b.q || (a.kind != "*" && b.kind == "*") || (a.sub_kind != "*" && b.sub_kind == "*")
    };
    for i in 1..accepts.len() {
        let mut j = i;
        while j > 0 && less(&accepts[j], &accepts[j - 1]) {
            accepts.swap(j, j - 1);
            j -= 1;
        }
    }
}

fn match_format(accept: &Accept<'_>, format: Format) -> bool {
    let (kind, sub_kind, params) = format.target();
    if accept.kind != "*" && accept.kind != kind
        || accept.sub_kind != "*" && accept.sub_kind != sub_kind
    {
        return false;
    }
    let text_plain = accept.kind == "text" && accept.sub_kind == "plain";
    params.iter().all(|(key, want)| {
        let got = accept.params.get(key).copied().unwrap_or_default();
        let got = if *key == "version" && got.is_empty() && text_plain {
            "0.0.4"
        } else {
            got
        };
        got == *want
    })
}

/// Returns the chosen format and the full Content-Type, including the escaping scheme.
pub fn negotiate_format(accept_header: Option<&str>) -> (Format, String) {
    for accept in parse_accept(accept_header.unwrap_or_default()) {
        if let Some(format) = ACCEPTED_FORMATS
            .into_iter()
            .find(|f| match_format(&accept, *f))
        {
            let requested = accept
                .params
                .get("escaping")
                .copied()
                .filter(|e| ESCAPING_SCHEMES.contains(e));
            return (
                format,
                content_type(format, requested.unwrap_or(DEFAULT_ESCAPING)),
            );
        }
    }
    (Format::Text, content_type(Format::Text, DEFAULT_ESCAPING))
}

fn content_type(format: Format, escaping: &str) -> String {
    format!("{}; escaping={escaping}", format.content_type())
}

fn is_token(b: u8) -> bool {
    b.is_ascii() && !b.is_ascii_control() && !b" \t\"(),/:;<=>?@[]\\{}".contains(&b)
}

fn skip_space(s: &str) -> &str {
    s.trim_start_matches([' ', '\t', '\r', '\n'])
}

fn expect_quality(s: &str) -> Option<(f64, &str)> {
    let q = match s.as_bytes().first()? {
        b'0' => 0.0,
        b'1' => 1.0,
        _ => return None,
    };
    let Some(s) = s[1..].strip_prefix('.') else {
        return Some((q, &s[1..]));
    };
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let (n, d) = s[..digits].bytes().fold((0i64, 1i64), |(n, d), b| {
        (n * 10 + i64::from(b - b'0'), d * 10)
    });
    Some((q + n as f64 / d as f64, &s[digits..]))
}

/// Port of gddo `header.ParseAccept`: (value, q) pairs from every header line.
fn parse_accept_encoding<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<(&'a str, f64)> {
    let mut specs = Vec::new();
    for mut s in lines {
        loop {
            let len = s.bytes().take_while(|&b| is_token(b) || b == b'/').count();
            let value = &s[..len];
            if value.is_empty() {
                break;
            }
            s = skip_space(&s[len..]);
            let mut q = 1.0;
            if let Some(rest) = s.strip_prefix(';') {
                let Some(rest) = skip_space(rest).strip_prefix("q=") else {
                    break;
                };
                let Some((quality, rest)) = expect_quality(rest) else {
                    break;
                };
                q = quality;
                s = rest;
            }
            specs.push((value, q));
            let Some(rest) = skip_space(s).strip_prefix(',') else {
                break;
            };
            s = skip_space(rest);
        }
    }
    specs
}

/// Port of gddo `NegotiateContentEncoding` with promhttp's offers. True means gzip; promhttp falls back to
/// identity when nothing is acceptable.
pub fn negotiate_gzip<'a>(accept_encoding_lines: impl Iterator<Item = &'a str>) -> bool {
    let specs = parse_accept_encoding(accept_encoding_lines);
    let (mut best_offer, mut best_q) = ("identity", -1.0);
    for offer in ["identity", "gzip"] {
        for &(value, q) in &specs {
            if q > best_q && (value == "*" || value == offer) {
                (best_offer, best_q) = (offer, q);
            }
        }
    }
    best_q != 0.0 && best_offer == "gzip"
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_UNDERSCORES: &str = "text/plain; version=0.0.4; charset=utf-8; escaping=underscores";

    #[test]
    fn negotiate_format_cases() {
        let cases = [
            (None, TEXT_UNDERSCORES),
            (Some(""), TEXT_UNDERSCORES),
            (Some("*/*"), TEXT_UNDERSCORES),
            (
                Some("application/openmetrics-text;version=1.0.0"),
                TEXT_UNDERSCORES,
            ),
            (
                Some(
                    "application/openmetrics-text;version=1.0.0;escaping=allow-utf-8;q=0.6,\
                     application/openmetrics-text;version=0.0.1;q=0.5,\
                     text/plain;version=1.0.0;escaping=allow-utf-8;q=0.4,text/plain;version=0.0.4;q=0.3,*/*;q=0.2",
                ),
                TEXT_UNDERSCORES,
            ),
            (
                Some("text/plain;escaping=allow-utf-8"),
                "text/plain; version=0.0.4; charset=utf-8; escaping=allow-utf-8",
            ),
            (Some("text/plain;escaping=bogus"), TEXT_UNDERSCORES),
            (
                Some(
                    "application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited",
                ),
                "application/vnd.google.protobuf; proto=io.prometheus.client.MetricFamily; encoding=delimited; \
                 escaping=underscores",
            ),
            (
                Some(
                    "application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited;q=0.5,\
                     text/plain;version=0.0.4",
                ),
                TEXT_UNDERSCORES,
            ),
            (
                Some(
                    "text/*, */*;q=0.1, \
                     application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited;q=0.9",
                ),
                "application/vnd.google.protobuf; proto=io.prometheus.client.MetricFamily; encoding=delimited; \
                 escaping=underscores",
            ),
        ];
        for (header, want) in cases {
            assert_eq!(negotiate_format(header).1, want, "Accept: {header:?}");
        }
    }

    #[test]
    fn negotiate_gzip_cases() {
        let cases: [(&[&str], bool); 8] = [
            (&[], false),
            (&["gzip"], true),
            (&["gzip, deflate, br"], true),
            (&["identity;q=0.5, gzip;q=1.0"], true),
            (&["gzip;q=0"], false),
            (&["*"], false),
            (&["identity;q=0, *;q=0.5"], false),
            (&["br", "gzip;q=0.8"], true),
        ];
        for (lines, want) in cases {
            assert_eq!(
                negotiate_gzip(lines.iter().copied()),
                want,
                "Accept-Encoding: {lines:?}"
            );
        }
    }
}
