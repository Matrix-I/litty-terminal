//! Audit the current tab inverse mapping against the current rendered spans.
//!
//! Source: src/render/renderer/mod.rs::hit_test_tab_bar and
//! src/render/renderer/ui/tab_bar.rs::build at a89baf70bba9c5695719c7152f64bd8954901de6.
//! This reproduces arithmetic only; it does not claim to exercise the UI.
//!
//! Run: rustc -O docs/plans/0.7.3-geometry-audit.rs -o "$TMPDIR/geometry-audit" && "$TMPDIR/geometry-audit"

fn main() {
    let mut checks = 0u64;
    let mut le_checks = 0u64;
    let mut le_mismatches = 0u64;
    let mut gt_mismatches = 0u64;
    let mut gt_examples: Vec<(usize, usize, usize, usize, usize, usize)> = Vec::new();

    for cols in 1usize..=400 {
        let available = cols.saturating_sub(2).max(1);
        for tab_count in 2usize..=128 {
            let spans: Vec<(usize, usize)> = (0..tab_count)
                .map(|i| (i * available / tab_count, (i + 1) * available / tab_count))
                .collect();
            for col in 0..available {
                let mut index = col * tab_count / available;
                let next_start = (index + 1) * available / tab_count;
                if col >= next_start && index + 1 < tab_count {
                    index += 1;
                }
                index = index.min(tab_count - 1);
                checks += 1;
                let (start, end) = spans[index];
                let contains = start <= col && col < end;
                if tab_count <= available {
                    le_checks += 1;
                    if !contains {
                        le_mismatches += 1;
                    }
                } else if !contains {
                    gt_mismatches += 1;
                    if gt_examples.len() < 3 {
                        gt_examples.push((cols, tab_count, col, index, start, end));
                    }
                }
            }
        }
    }

    println!("total_checks={checks}");
    println!("tabs_le_available_checks={le_checks}");
    println!("tabs_le_available_mismatches={le_mismatches}");
    println!("tabs_gt_available_mismatches={gt_mismatches}");
    println!("tabs_gt_available_examples={gt_examples:?}");
    assert_eq!(le_mismatches, 0);
}
