//! Minimal std-only argument parsing: positionals, `--key value` / `--key=value` / `-o value`
//! options (given again: the last counts, or all with [`Args::values`]), and boolean `--flag`s.
//! Unknown options are usage errors; `--help` or `-h` is [`CliError::Help`].

use std::collections::BTreeMap;

use crate::{CliError, CliResult};

#[derive(Debug, Default)]
pub struct Args {
    pub positional: Vec<String>,
    values: BTreeMap<String, Vec<String>>,
    flags: Vec<String>,
}

impl Args {
    pub fn parse(argv: &[String], valued: &[&str], flags: &[&str]) -> CliResult<Self> {
        let mut a = Args::default();
        // The first bad option, reported once the rest of the line is checked for `--help`.
        let mut bad: Option<CliError> = None;
        let mut it = argv.iter();
        while let Some(arg) = it.next() {
            let name = if let Some(n) = arg.strip_prefix("--") {
                n
            } else if let Some(n) = arg.strip_prefix('-').filter(|n| n.len() == 1 && !n.starts_with(|c: char| c.is_ascii_digit())) {
                n
            } else {
                a.positional.push(arg.clone());
                continue;
            };
            let (name, inline) = match name.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (name, None),
            };
            // `--help` / `-h` anywhere among the options (not as an option's value) asks for help.
            if inline.is_none() && matches!(name, "help" | "h") {
                return Err(CliError::Help);
            }
            if valued.contains(&name) {
                match inline.or_else(|| it.next().cloned()) {
                    Some(v) => a.values.entry(name.to_string()).or_default().push(v),
                    None => {
                        bad.get_or_insert(CliError::Usage(format!("`{arg}` needs a value")));
                    }
                }
            } else if flags.contains(&name) && inline.is_none() {
                a.flags.push(name.to_string());
            } else {
                bad.get_or_insert(CliError::Usage(format!("unknown option `{arg}`")));
            }
        }
        match bad {
            Some(e) => Err(e),
            None => Ok(a),
        }
    }

    pub fn value(&self, k: &str) -> Option<String> {
        self.values.get(k).and_then(|v| v.last()).cloned()
    }

    /// Every value of an option that may be given more than once, in order.
    pub fn values(&self, k: &str) -> &[String] {
        self.values.get(k).map_or(&[], Vec::as_slice)
    }

    pub fn flag(&self, k: &str) -> bool {
        self.flags.iter().any(|f| f == k)
    }

    pub fn one_positional(&self, usage: &str) -> CliResult<String> {
        match self.positional.as_slice() {
            [one] => Ok(one.clone()),
            _ => Err(CliError::Usage(format!("expected: {usage}"))),
        }
    }

    /// PDF print options: `--bleed LENGTH` (points, or "3mm", "0.125in"), `--crop-marks`,
    /// `--bleed-marks`, `--cmyk`.
    pub fn print(&self) -> CliResult<figcraft_doc::PrintSettings> {
        let bleed = match self.value("bleed") {
            None => 0.0,
            Some(s) => figcraft_doc::PrintSettings::parse_length(&s)
                .filter(|b| *b <= figcraft_doc::PrintSettings::MAX_BLEED)
                .ok_or_else(|| CliError::Usage(format!("--bleed takes points or a length like 3mm or 0.125in (up to 2in), got `{s}`")))?,
        };
        Ok(figcraft_doc::PrintSettings { bleed, crop_marks: self.flag("crop-marks"), bleed_marks: self.flag("bleed-marks"), cmyk: self.flag("cmyk") })
    }

    pub fn scale(&self) -> CliResult<f64> {
        match self.value("scale") {
            None => Ok(1.0),
            Some(s) => s
                .trim_end_matches('x')
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v > 0.0 && *v <= 64.0)
                .ok_or_else(|| CliError::Usage(format!("--scale must be a number in (0, 64], got `{s}`"))),
        }
    }

    /// A comma-separated list of whole numbers (`--layers 1000,10_000`).
    pub fn usizes(&self, k: &str, default: &[usize]) -> CliResult<Vec<usize>> {
        match self.value(k) {
            None => Ok(default.to_vec()),
            Some(s) => s
                .split(',')
                .map(|v| v.trim().replace('_', "").parse::<usize>().map_err(|_| CliError::Usage(format!("--{k} must be whole numbers, got `{s}`"))))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_options_and_flags() {
        let a = Args::parse(&v(&["in.figcraft", "-o", "out", "--scale=2", "--json"]), &["o", "scale"], &["json"]).unwrap();
        assert_eq!(a.positional, ["in.figcraft"]);
        assert_eq!(a.value("o").as_deref(), Some("out"));
        assert_eq!(a.scale().unwrap(), 2.0);
        assert!(a.flag("json"));
        let a = Args::parse(&v(&["--root", "a", "--root=b"]), &["root"], &[]).unwrap();
        assert_eq!(a.values("root"), ["a", "b"]);
        assert_eq!(a.value("root").as_deref(), Some("b"));
        assert!(a.values("none").is_empty());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(Args::parse(&v(&["--nope"]), &[], &[]), Err(CliError::Usage(_))));
        assert!(matches!(Args::parse(&v(&["-o"]), &["o"], &[]), Err(CliError::Usage(_))));
        assert!(matches!(Args::parse(&v(&["--json=1"]), &[], &["json"]), Err(CliError::Usage(_))));
        let a = Args::parse(&v(&["--scale", "abc"]), &["scale"], &[]).unwrap();
        assert!(a.scale().is_err());
        let a = Args::parse(&v(&["--scale", "-3"]), &["scale"], &[]).unwrap();
        assert!(a.scale().is_err());
        // Negative numbers are positionals, not options.
        let a = Args::parse(&v(&["-5"]), &[], &[]).unwrap();
        assert_eq!(a.positional, ["-5"]);
    }

    #[test]
    fn help_is_asked_for_anywhere_but_as_a_value() {
        // Also after a bad option: help wins over the usage error (#1161 review).
        for argv in
            [&["--help"][..], &["-h"], &["in.figcraft", "-o", "out", "--help"], &["--json", "-h"], &["--bogus", "--help"], &["--json=1", "-h"]]
        {
            assert!(matches!(Args::parse(&v(argv), &["o"], &["json"]), Err(CliError::Help)), "{argv:?}");
        }
        let a = Args::parse(&v(&["--filter", "--help", "-o", "-h"]), &["filter", "o"], &[]).unwrap();
        assert_eq!((a.value("filter").as_deref(), a.value("o").as_deref()), (Some("--help"), Some("-h")));
        // `--help=x` is no request for help, just an unknown option.
        assert!(matches!(Args::parse(&v(&["--help=x"]), &[], &[]), Err(CliError::Usage(_))));
    }
}
