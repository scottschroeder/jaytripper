use std::{fs, path::PathBuf};

use jaytripper_core::{SignatureParseError, parse_signature_snapshot};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("signatures")
        .join(name)
}

fn read_fixture(name: &str) -> String {
    fs::read_to_string(fixture_path(name)).expect("fixture should be readable")
}

fn split_snapshot_blocks(input: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();

    for line in input.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
            }
            continue;
        }
        current.push(line);
    }

    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }

    blocks
}

#[test]
fn parses_curated_snapshot_fixtures() {
    let snapshots = [
        ("snapshot_01.txt", 11_usize, "CWT-368"),
        ("snapshot_02.txt", 11_usize, "VMJ-105"),
        ("snapshot_03.txt", 12_usize, "ZZO-660"),
    ];

    for (fixture_name, expected_count, expected_id) in snapshots {
        let fixture = read_fixture(fixture_name);
        let entries = parse_signature_snapshot(&fixture).expect("snapshot should parse");
        assert_eq!(entries.len(), expected_count, "fixture {fixture_name}");
        assert!(
            entries
                .iter()
                .any(|entry| entry.signature_id == expected_id),
            "fixture {fixture_name} should contain signature id {expected_id}"
        );
    }
}

#[test]
fn malformed_missing_id_fixture_fails_with_line_context() {
    let fixture = read_fixture("bad_missing_id.txt");
    let err = parse_signature_snapshot(&fixture).expect_err("fixture should fail");

    match err {
        SignatureParseError::InvalidSignatureId { line, .. } => assert_eq!(line, 1),
        other => panic!("expected invalid signature id error, got {other:?}"),
    }
}

#[test]
fn malformed_percent_fixture_fails_with_line_context() {
    let fixture = read_fixture("bad_percent.txt");
    let err = parse_signature_snapshot(&fixture).expect_err("fixture should fail");

    match err {
        SignatureParseError::InvalidScanPercent { line, .. } => assert_eq!(line, 1),
        other => panic!("expected invalid scan percent error, got {other:?}"),
    }
}

#[test]
fn malformed_column_fixture_fails_with_line_context() {
    let fixture = read_fixture("bad_columns.txt");
    let err = parse_signature_snapshot(&fixture).expect_err("fixture should fail");

    match err {
        SignatureParseError::InvalidColumnCount { line, .. } => assert_eq!(line, 1),
        other => panic!("expected invalid column count error, got {other:?}"),
    }
}

#[test]
fn parses_all_blocks_in_multi_snapshot_fixture() {
    let fixture = read_fixture("multi_snapshot.txt");
    let blocks = split_snapshot_blocks(&fixture);

    assert!(
        !blocks.is_empty(),
        "multi_snapshot fixture should contain at least one snapshot block"
    );

    for (idx, block) in blocks.iter().enumerate() {
        let entries = parse_signature_snapshot(block)
            .unwrap_or_else(|err| panic!("block {} failed to parse: {err}", idx + 1));
        assert!(
            !entries.is_empty(),
            "block {} should contain at least one entry",
            idx + 1
        );
    }
}
