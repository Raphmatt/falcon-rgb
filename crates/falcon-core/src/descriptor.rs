//! Minimal HID report-descriptor parser: report lengths per report ID.
//!
//! The descriptor is not in the Windows binaries. The 264-byte feature length is confirmed on
//! hardware; `doctor` reads the descriptor from the device and prints what this parser finds.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Report direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    Input,
    Output,
    Feature,
}

/// One report as declared by the descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportInfo {
    pub kind: ReportKind,
    /// 0 when the descriptor uses no report IDs.
    pub report_id: u8,
    /// Payload size in bits, report ID excluded.
    pub bits: u32,
    /// Usage page in effect at the first main item of this report.
    pub usage_page: u16,
}

impl ReportInfo {
    /// Bytes hidapi transfers for this report: payload plus one report-ID byte when IDs are used.
    pub fn byte_len(&self) -> usize {
        self.bits.div_ceil(8) as usize + usize::from(self.report_id != 0)
    }
}

#[derive(Clone, Copy, Default)]
struct Globals {
    usage_page: u16,
    report_size: u32,
    report_count: u32,
    report_id: u8,
}

/// Sums the main items of a report descriptor into per-(kind, report ID) sizes. Unknown and
/// malformed input never panics; parsing just stops at a truncated item.
pub fn parse_reports(desc: &[u8]) -> Vec<ReportInfo> {
    let mut g = Globals::default();
    let mut stack: Vec<Globals> = Vec::new();
    let mut reports: BTreeMap<(ReportKind, u8), ReportInfo> = BTreeMap::new();
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            // Long item: FE size tag data...
            let Some(&size) = desc.get(i + 1) else { break };
            i += 3 + size as usize;
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        let Some(data) = desc.get(i + 1..i + 1 + size) else {
            break;
        };
        let value = data
            .iter()
            .rev()
            .fold(0u32, |acc, &b| (acc << 8) | u32::from(b));
        let item_type = (prefix >> 2) & 0x03;
        let tag = prefix >> 4;
        match (item_type, tag) {
            // Main items: Input, Output, Feature.
            (0, 0x8) | (0, 0x9) | (0, 0xB) => {
                let kind = match tag {
                    0x8 => ReportKind::Input,
                    0x9 => ReportKind::Output,
                    _ => ReportKind::Feature,
                };
                let entry = reports
                    .entry((kind, g.report_id))
                    .or_insert_with(|| ReportInfo {
                        kind,
                        report_id: g.report_id,
                        bits: 0,
                        usage_page: g.usage_page,
                    });
                entry.bits = entry
                    .bits
                    .saturating_add(g.report_size.saturating_mul(g.report_count));
            }
            // Global items.
            (1, 0x0) => g.usage_page = value as u16,
            (1, 0x7) => g.report_size = value,
            (1, 0x8) => g.report_id = value as u8,
            (1, 0x9) => g.report_count = value,
            (1, 0xA) => stack.push(g),
            (1, 0xB) => {
                if let Some(saved) = stack.pop() {
                    g = saved;
                }
            }
            _ => {}
        }
        i += 1 + size;
    }
    reports.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_feature_report_264() {
        // Usage Page (0xFF01), Usage (1), Collection (App), Report ID (7), Report Size (8),
        // Report Count (263), Usage (1), Feature (Data,Var,Abs), Report ID (4), Report Count (7),
        // Usage (1), Input, End Collection.
        let desc = [
            0x06, 0x01, 0xFF, 0x09, 0x01, 0xA1, 0x01, 0x85, 0x07, 0x75, 0x08, 0x96, 0x07, 0x01,
            0x09, 0x01, 0xB1, 0x02, 0x85, 0x04, 0x95, 0x07, 0x09, 0x01, 0x81, 0x02, 0xC0,
        ];
        let reports = parse_reports(&desc);
        assert_eq!(reports.len(), 2);
        let feature = reports
            .iter()
            .find(|r| r.kind == ReportKind::Feature)
            .unwrap();
        assert_eq!(feature.report_id, 7);
        assert_eq!(feature.byte_len(), 264);
        assert_eq!(feature.usage_page, 0xFF01);
        let input = reports
            .iter()
            .find(|r| r.kind == ReportKind::Input)
            .unwrap();
        assert_eq!((input.report_id, input.byte_len()), (4, 8));
    }

    #[test]
    fn truncated_input_does_not_panic() {
        assert!(parse_reports(&[0x96, 0x07]).is_empty());
        assert!(parse_reports(&[0xFE]).is_empty());
        assert!(parse_reports(&[]).is_empty());
    }
}
