use std::{collections::HashMap, sync::OnceLock};

use regex::Regex;

use crate::events::SignatureEntry;

pub fn parse_signature_snapshot(input: &str) -> Result<Vec<SignatureEntry>, SignatureParseError> {
    let mut entries = Vec::new();

    for (idx, raw_line) in input.lines().enumerate() {
        let line_number = idx + 1;
        let line = raw_line.trim();

        if line.is_empty() {
            continue;
        }

        let columns: Vec<&str> = raw_line.split('\t').map(str::trim).collect();
        if columns.len() < 5 {
            return Err(SignatureParseError::InvalidColumnCount {
                line: line_number,
                expected_at_least: 5,
                actual: columns.len(),
            });
        }

        let signature_id = columns[0];
        if !is_valid_signature_id(signature_id) {
            return Err(SignatureParseError::InvalidSignatureId {
                line: line_number,
                value: signature_id.to_owned(),
            });
        }

        let group = columns[1];
        if group.is_empty() {
            return Err(SignatureParseError::MissingGroup { line: line_number });
        }

        let site_type = to_optional(columns.get(2).copied().unwrap_or_default());
        let name = to_optional(columns.get(3).copied().unwrap_or_default());
        let scan_percent = parse_scan_percent(columns[4], line_number)?;

        entries.push(SignatureEntry {
            signature_id: signature_id.to_owned(),
            group: group.to_owned(),
            site_type: site_type.map(ToOwned::to_owned),
            name: name.map(ToOwned::to_owned),
            scan_percent,
        });
    }

    Ok(entries)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedSignature {
    pub signature_id: String,
    pub group: String,
    pub site_type: Option<String>,
    pub name: Option<String>,
    pub latest_scan_percent: Option<f32>,
    pub highest_scan_percent_seen: Option<f32>,
    pub missing_from_latest_snapshot: bool,
}

pub fn merge_signature_snapshot(
    signatures_by_id: &mut HashMap<String, ProjectedSignature>,
    incoming_entries: &[SignatureEntry],
) {
    for signature in signatures_by_id.values_mut() {
        signature.missing_from_latest_snapshot = true;
    }

    for entry in incoming_entries {
        let signature = signatures_by_id
            .entry(entry.signature_id.clone())
            .or_insert_with(|| ProjectedSignature {
                signature_id: entry.signature_id.clone(),
                group: entry.group.clone(),
                site_type: None,
                name: None,
                latest_scan_percent: None,
                highest_scan_percent_seen: None,
                missing_from_latest_snapshot: false,
            });

        if !entry.group.is_empty() {
            signature.group = entry.group.clone();
        }

        if entry.site_type.is_some() {
            signature.site_type = entry.site_type.clone();
        }

        if entry.name.is_some() {
            signature.name = entry.name.clone();
        }

        if let Some(percent) = entry.scan_percent {
            signature.latest_scan_percent = Some(percent);
            signature.highest_scan_percent_seen = Some(
                signature
                    .highest_scan_percent_seen
                    .unwrap_or(percent)
                    .max(percent),
            );
        }

        signature.missing_from_latest_snapshot = false;
    }
}

fn to_optional(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

fn parse_scan_percent(raw: &str, line: usize) -> Result<Option<f32>, SignatureParseError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let captures = scan_percent_regex().captures(trimmed).ok_or_else(|| {
        SignatureParseError::InvalidScanPercent {
            line,
            value: trimmed.to_owned(),
            reason: "expected numeric value with '%' suffix".to_owned(),
        }
    })?;

    let numeric = captures.get(1).map(|m| m.as_str()).unwrap_or_default();

    let parsed = numeric
        .parse::<f32>()
        .map_err(|_| SignatureParseError::InvalidScanPercent {
            line,
            value: trimmed.to_owned(),
            reason: "not a valid number".to_owned(),
        })?;

    if !(0.0..=100.0).contains(&parsed) {
        return Err(SignatureParseError::InvalidScanPercent {
            line,
            value: trimmed.to_owned(),
            reason: "value must be between 0 and 100".to_owned(),
        });
    }

    Ok(Some(parsed))
}

pub fn is_valid_signature_id(value: &str) -> bool {
    signature_id_regex().is_match(value)
}

fn signature_id_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^[A-Z]{3}-[0-9]{3}$").expect("valid regex"))
}

fn scan_percent_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^([0-9]+(?:\.[0-9]+)?)%$").expect("valid regex"))
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignatureParseError {
    #[error(
        "line {line}: expected at least {expected_at_least} tab-delimited columns, got {actual}"
    )]
    InvalidColumnCount {
        line: usize,
        expected_at_least: usize,
        actual: usize,
    },
    #[error("line {line}: invalid signature id '{value}'")]
    InvalidSignatureId { line: usize, value: String },
    #[error("line {line}: missing group column")]
    MissingGroup { line: usize },
    #[error("line {line}: invalid scan percent '{value}': {reason}")]
    InvalidScanPercent {
        line: usize,
        value: String,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{is_valid_signature_id, merge_signature_snapshot, parse_signature_snapshot};
    use crate::events::SignatureEntry;

    #[test]
    fn validates_signature_ids() {
        assert!(is_valid_signature_id("ABC-123"));
        assert!(!is_valid_signature_id("abc-123"));
        assert!(!is_valid_signature_id("ABCD-123"));
        assert!(!is_valid_signature_id("ABC123"));
    }

    #[test]
    fn parses_scan_percentages_and_ignores_distance_column() {
        let input = "ABC-123\tCosmic Signature\tGas Site\t\t28.6%\t21.93 AU\n";
        let entries = parse_signature_snapshot(input).expect("parse snapshot");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].signature_id, "ABC-123");
        assert_eq!(entries[0].scan_percent, Some(28.6));
        assert_eq!(entries[0].site_type.as_deref(), Some("Gas Site"));
    }

    #[test]
    fn reports_invalid_percent_with_line_context() {
        let input = "ABC-123\tCosmic Signature\tGas Site\t\tabs%\t21.93 AU\n";
        let err = parse_signature_snapshot(input).expect_err("parse should fail");
        assert!(err.to_string().contains("line 1"));
    }

    #[test]
    fn reports_out_of_range_percent() {
        let input = "ABC-123\tCosmic Signature\tGas Site\t\t101.0%\t21.93 AU\n";
        let err = parse_signature_snapshot(input).expect_err("parse should fail");
        assert!(err.to_string().contains("between 0 and 100"));
    }

    #[test]
    fn merge_keeps_highest_percent_seen_and_updates_latest() {
        let mut projected = HashMap::new();

        merge_signature_snapshot(
            &mut projected,
            &[SignatureEntry {
                signature_id: "ABC-123".to_owned(),
                group: "Cosmic Signature".to_owned(),
                site_type: Some("Relic Site".to_owned()),
                name: None,
                scan_percent: Some(70.0),
            }],
        );

        merge_signature_snapshot(
            &mut projected,
            &[SignatureEntry {
                signature_id: "ABC-123".to_owned(),
                group: "Cosmic Signature".to_owned(),
                site_type: Some("Relic Site".to_owned()),
                name: Some("Relic Training Site".to_owned()),
                scan_percent: Some(0.0),
            }],
        );

        let signature = projected.get("ABC-123").expect("signature should exist");
        assert_eq!(signature.latest_scan_percent, Some(0.0));
        assert_eq!(signature.highest_scan_percent_seen, Some(70.0));
        assert_eq!(
            signature.name.as_deref(),
            Some("Relic Training Site"),
            "name should be refined by newer snapshots"
        );
        assert!(!signature.missing_from_latest_snapshot);
    }

    #[test]
    fn merge_marks_absent_signatures_as_missing_from_latest_snapshot() {
        let mut projected = HashMap::new();

        merge_signature_snapshot(
            &mut projected,
            &[
                SignatureEntry {
                    signature_id: "ABC-123".to_owned(),
                    group: "Cosmic Signature".to_owned(),
                    site_type: None,
                    name: None,
                    scan_percent: Some(10.0),
                },
                SignatureEntry {
                    signature_id: "DEF-456".to_owned(),
                    group: "Cosmic Signature".to_owned(),
                    site_type: None,
                    name: None,
                    scan_percent: Some(25.0),
                },
            ],
        );

        merge_signature_snapshot(
            &mut projected,
            &[SignatureEntry {
                signature_id: "ABC-123".to_owned(),
                group: "Cosmic Signature".to_owned(),
                site_type: None,
                name: None,
                scan_percent: Some(50.0),
            }],
        );

        let abc = projected.get("ABC-123").expect("ABC should exist");
        let def = projected.get("DEF-456").expect("DEF should exist");
        assert!(!abc.missing_from_latest_snapshot);
        assert!(def.missing_from_latest_snapshot);
    }
}
