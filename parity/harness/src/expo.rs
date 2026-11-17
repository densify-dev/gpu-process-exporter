//! Minimal parser for the Prometheus text exposition format (0.0.4).
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;

pub type Labels = BTreeMap<String, String>;

#[derive(Debug, Default, Clone)]
pub struct Family {
    pub help: Option<String>,
    pub kind: Option<String>,
    /// Samples keyed by their full sample name plus labels.
    pub samples: BTreeMap<(String, Labels), f64>,
}

#[derive(Debug, Default, Clone)]
pub struct Exposition {
    pub families: BTreeMap<String, Family>,
}

impl Exposition {
    pub fn parse(text: &str) -> Result<Exposition> {
        let mut expo = Exposition::default();
        for (n, line) in text.lines().enumerate() {
            parse_line(&mut expo, line).with_context(|| format!("line {}: {line:?}", n + 1))?;
        }
        Ok(expo)
    }

    /// All samples whose name and labels match `selector` (`name{k="v",...}`, labels are a subset match).
    pub fn select(&self, selector: &Selector) -> Vec<(&Labels, f64)> {
        self.families
            .values()
            .flat_map(|f| f.samples.iter())
            .filter(|((name, labels), _)| {
                *name == selector.name
                    && selector
                        .labels
                        .iter()
                        .all(|(k, v)| labels.get(k) == Some(v))
            })
            .map(|((_, labels), v)| (labels, *v))
            .collect()
    }
}

fn parse_line(expo: &mut Exposition, line: &str) -> Result<()> {
    let line = line.trim_end();
    if line.is_empty() {
        return Ok(());
    }
    if let Some(rest) = line.strip_prefix("# ") {
        let mut parts = rest.splitn(3, ' ');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("HELP"), Some(name), help) => {
                expo.families.entry(name.into()).or_default().help =
                    Some(unescape_help(help.unwrap_or("")));
            }
            (Some("TYPE"), Some(name), Some(kind)) => {
                expo.families.entry(name.into()).or_default().kind = Some(kind.into());
            }
            _ => {}
        }
        return Ok(());
    }
    if line.starts_with('#') {
        return Ok(());
    }
    let (name, labels, rest) = parse_series(line)?;
    let value = rest.split_whitespace().next().context("missing value")?;
    let value = parse_value(value)?;
    let family = family_name(expo, &name);
    expo.families
        .entry(family)
        .or_default()
        .samples
        .insert((name, labels), value);
    Ok(())
}

/// Maps a sample name to its family (`_total`, `_sum`, `_count`, `_bucket` suffixes).
fn family_name(expo: &Exposition, name: &str) -> String {
    if expo.families.contains_key(name) {
        return name.into();
    }
    ["_total", "_sum", "_count", "_bucket"]
        .iter()
        .filter_map(|s| name.strip_suffix(s))
        .find(|base| expo.families.contains_key(*base))
        .unwrap_or(name)
        .into()
}

fn parse_series(s: &str) -> Result<(String, Labels, &str)> {
    let end = s.find(['{', ' ']).context("missing value")?;
    let name = s[..end].to_string();
    let mut labels = Labels::new();
    let Some(mut body) = s[end..].strip_prefix('{') else {
        return Ok((name, labels, s[end..].trim_start()));
    };
    loop {
        body = body.trim_start_matches([',', ' ']);
        if let Some(rest) = body.strip_prefix('}') {
            return Ok((name, labels, rest.trim_start()));
        }
        let (key, after) = body.split_once("=\"").context("label without '=\"'")?;
        let mut value = String::new();
        let mut chars = after.char_indices();
        let close = loop {
            match chars.next() {
                Some((_, '\\')) => match chars.next() {
                    Some((_, 'n')) => value.push('\n'),
                    Some((_, c)) => value.push(c),
                    None => bail!("dangling escape"),
                },
                Some((i, '"')) => break i,
                Some((_, c)) => value.push(c),
                None => bail!("unterminated label value"),
            }
        };
        labels.insert(key.trim().to_string(), value);
        body = &after[close + 1..];
    }
}

fn parse_value(s: &str) -> Result<f64> {
    Ok(match s {
        "NaN" => f64::NAN,
        "+Inf" | "Inf" => f64::INFINITY,
        "-Inf" => f64::NEG_INFINITY,
        s => s.parse().with_context(|| format!("bad value {s:?}"))?,
    })
}

fn unescape_help(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (c, chars.clone().next()) {
            ('\\', Some('n')) => {
                out.push('\n');
                chars.next();
            }
            ('\\', Some('\\')) => {
                out.push('\\');
                chars.next();
            }
            (c, _) => out.push(c),
        }
    }
    out
}

/// `name{k="v",...}` used by checks to pick series.
#[derive(Debug, Clone)]
pub struct Selector {
    pub name: String,
    pub labels: Labels,
}

impl std::str::FromStr for Selector {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Selector> {
        let line = format!("{s} 0");
        let (name, labels, rest) = parse_series(&line)?;
        if rest != "0" {
            bail!("bad selector {s:?}");
        }
        Ok(Selector { name, labels })
    }
}

impl std::fmt::Display for Selector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{{", self.name)?;
        for (i, (k, v)) in self.labels.iter().enumerate() {
            write!(f, "{}{k}={v:?}", if i > 0 { "," } else { "" })?;
        }
        write!(f, "}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_families_and_samples() {
        let text = "# HELP a_total Some \\\\ help\n# TYPE a_total counter\na_total{x=\"1\",y=\"q\\\"z\"} 1.5\n\
                    # HELP g Gauge\n# TYPE g gauge\ng NaN\ng{a=\"b,c\"} 5.8720256e+09\n";
        let e = Exposition::parse(text).unwrap();
        let a = &e.families["a_total"];
        assert_eq!(a.help.as_deref(), Some("Some \\ help"));
        assert_eq!(a.kind.as_deref(), Some("counter"));
        let labels: Labels = [("x".into(), "1".into()), ("y".into(), "q\"z".into())].into();
        assert_eq!(a.samples[&("a_total".into(), labels)], 1.5);
        let g = &e.families["g"];
        assert!(g.samples[&("g".into(), Labels::new())].is_nan());
        let sel: Selector = "g{a=\"b,c\"}".parse().unwrap();
        assert_eq!(e.select(&sel)[0].1, 5872025600.0);
    }
}
