// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use crate::checkers::EndpointState;
use crate::db::history::WindowStats;

/// Parse and evaluate a condition string against a value.
/// Conditions: <N, >N, ==N, !=N, diff(12h)>N, absdiff(12h)>N
///
/// `value` is the current measured value. `window` carries stats over the
/// configured diff window (oldest/min/max), used by diff() and absdiff().
/// - `diff(N)`  compares `current - oldest_in_window` (gap-tolerant: oldest
///   shifts forward instead of becoming None when a sample is missing).
/// - `absdiff(N)` compares `max - min` across the window (volatility, not a
///   point-in-time delta).
pub fn evaluate(
    condition: &str,
    value: Option<&str>,
    window: Option<WindowStats>,
) -> EndpointState {
    let condition = condition.trim();
    if condition.is_empty() {
        return EndpointState::Ok;
    }

    // Parse diff conditions: diff(12h)>N, absdiff(12h)>N, etc.
    if let Some(rest) = condition.strip_prefix("absdiff(") {
        return evaluate_diff(rest, value, window, true);
    }
    if let Some(rest) = condition.strip_prefix("diff(") {
        return evaluate_diff(rest, value, window, false);
    }

    // Simple comparison conditions
    let current = match value.and_then(|v| v.parse::<f64>().ok()) {
        Some(v) => v,
        None => return EndpointState::NoData,
    };

    if let Some(threshold) = condition.strip_prefix("<=") {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if current <= threshold {
            return EndpointState::Warning;
        }
    } else if let Some(threshold) = condition.strip_prefix(">=") {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if current >= threshold {
            return EndpointState::Warning;
        }
    } else if let Some(threshold) = condition.strip_prefix("==") {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if (current - threshold).abs() < f64::EPSILON {
            return EndpointState::Warning;
        }
    } else if let Some(threshold) = condition.strip_prefix("!=") {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if (current - threshold).abs() > f64::EPSILON {
            return EndpointState::Warning;
        }
    } else if let Some(threshold) = condition.strip_prefix('<') {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if current < threshold {
            return EndpointState::Warning;
        }
    } else if let Some(threshold) = condition.strip_prefix('>') {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if current > threshold {
            return EndpointState::Warning;
        }
    }

    EndpointState::Ok
}

fn evaluate_diff(
    rest: &str,
    value: Option<&str>,
    window: Option<WindowStats>,
    absolute: bool,
) -> EndpointState {
    // Parse: 12h)>N or 12h)<N
    let close_paren = match rest.find(')') {
        Some(i) => i,
        None => return EndpointState::NoData,
    };

    let _hours_str = &rest[..close_paren]; // e.g., "12h"
    let comparison = &rest[close_paren + 1..]; // e.g., ">100"

    let current = match value.and_then(|v| v.parse::<f64>().ok()) {
        Some(v) => v,
        None => return EndpointState::NoData,
    };

    let Some(window) = window else {
        return EndpointState::NoData;
    };

    // absdiff = volatility across the whole window (max - min), where the
    // current sample is folded into the min/max even though it hasn't been
    // persisted yet. Without this, a flat history + a brand-new step change
    // would report spread=0 and miss the event.
    // diff = signed change from the oldest sample in the window to now.
    let diff = if absolute {
        let effective_max = current.max(window.max);
        let effective_min = current.min(window.min);
        effective_max - effective_min
    } else {
        current - window.oldest
    };

    if let Some(threshold) = comparison.strip_prefix("<=") {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if diff <= threshold {
            return EndpointState::Warning;
        }
    } else if let Some(threshold) = comparison.strip_prefix(">=") {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if diff >= threshold {
            return EndpointState::Warning;
        }
    } else if let Some(threshold) = comparison.strip_prefix("==") {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if (diff - threshold).abs() < f64::EPSILON {
            return EndpointState::Warning;
        }
    } else if let Some(threshold) = comparison.strip_prefix("!=") {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if (diff - threshold).abs() > f64::EPSILON {
            return EndpointState::Warning;
        }
    } else if let Some(threshold) = comparison.strip_prefix('>') {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if diff > threshold {
            return EndpointState::Warning;
        }
    } else if let Some(threshold) = comparison.strip_prefix('<') {
        let threshold: f64 = match threshold.trim().parse() {
            Ok(t) => t,
            Err(_) => return EndpointState::NoData,
        };
        if diff < threshold {
            return EndpointState::Warning;
        }
    }

    EndpointState::Ok
}

/// Parse the hours from a diff() or absdiff() condition, e.g., "diff(12h)>100" returns Some(12.0)
pub fn parse_diff_hours(condition: &str) -> Option<f64> {
    let rest = condition
        .strip_prefix("absdiff(")
        .or_else(|| condition.strip_prefix("diff("))?;
    let close = rest.find(')')?;
    let hours_str = &rest[..close];
    let hours_str = hours_str.strip_suffix('h').unwrap_or(hours_str);
    hours_str.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(oldest: f64, min: f64, max: f64) -> Option<WindowStats> {
        Some(WindowStats { oldest, min, max })
    }

    #[test]
    fn test_simple_conditions() {
        assert_eq!(evaluate(">90", Some("95"), None), EndpointState::Warning);
        assert_eq!(evaluate(">90", Some("85"), None), EndpointState::Ok);
        assert_eq!(evaluate("<7", Some("5"), None), EndpointState::Warning);
        assert_eq!(evaluate("<7", Some("10"), None), EndpointState::Ok);
        assert_eq!(evaluate("==0", Some("0"), None), EndpointState::Warning);
        assert_eq!(evaluate("!=0", Some("5"), None), EndpointState::Warning);
        assert_eq!(evaluate("!=0", Some("0"), None), EndpointState::Ok);
    }

    #[test]
    fn test_diff_conditions() {
        // diff = current - oldest_in_window
        assert_eq!(
            evaluate("diff(12h)>100", Some("500"), win(300.0, 300.0, 500.0)),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("diff(12h)>100", Some("350"), win(300.0, 300.0, 350.0)),
            EndpointState::Ok
        );
        assert_eq!(
            evaluate("diff(12h)==0", Some("300"), win(300.0, 300.0, 300.0)),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("diff(12h)==0", Some("301"), win(300.0, 300.0, 301.0)),
            EndpointState::Ok
        );
        assert_eq!(
            evaluate("diff(12h)!=0", Some("500"), win(300.0, 300.0, 500.0)),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("diff(12h)!=0", Some("300"), win(300.0, 300.0, 300.0)),
            EndpointState::Ok
        );
        assert_eq!(
            evaluate("diff(12h)>=100", Some("400"), win(300.0, 300.0, 400.0)),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("diff(12h)<=0", Some("200"), win(300.0, 200.0, 300.0)),
            EndpointState::Warning
        );
    }

    #[test]
    fn test_absdiff_conditions() {
        // absdiff = max - min across the window (volatility, not point-in-time).
        // Window saw values from 300 to 500, so max-min = 200 > 100 → Warning.
        assert_eq!(
            evaluate("absdiff(12h)>100", Some("300"), win(500.0, 300.0, 500.0)),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("absdiff(12h)>100", Some("500"), win(300.0, 300.0, 500.0)),
            EndpointState::Warning
        );
        // Window 300..350, spread = 50 → Ok.
        assert_eq!(
            evaluate("absdiff(12h)>100", Some("350"), win(300.0, 300.0, 350.0)),
            EndpointState::Ok
        );
        // Flat window: max-min = 0 → ==0 fires.
        assert_eq!(
            evaluate("absdiff(12h)==0", Some("300"), win(300.0, 300.0, 300.0)),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("absdiff(12h)==0", Some("301"), win(300.0, 300.0, 301.0)),
            EndpointState::Ok
        );
    }

    // The current observation is folded into absdiff's min/max even though
    // it hasn't been persisted to history yet. Below, `win(...)` carries the
    // PAST samples only; the current sample is the `Some("…")` argument.
    #[test]
    fn test_absdiff_eq_zero_detects_flat_series() {
        // condition: absdiff(N)==0 → "value has not changed in the window"
        //   past=[1,1], current=2     → spread {1,2}=1     → OK
        assert_eq!(
            evaluate("absdiff(12h)==0", Some("2"), win(1.0, 1.0, 1.0)),
            EndpointState::Ok
        );
        //   past=[1,1,2], current=None → NO_DATA (current missing)
        assert_eq!(
            evaluate("absdiff(12h)==0", None, win(1.0, 1.0, 2.0)),
            EndpointState::NoData
        );
        //   past=[1,1,1,1], current=1 → spread=0 → FAIL (warning fires)
        assert_eq!(
            evaluate("absdiff(12h)==0", Some("1"), win(1.0, 1.0, 1.0)),
            EndpointState::Warning
        );
    }

    #[test]
    fn test_absdiff_lt_threshold_detects_low_volatility() {
        // condition: absdiff(N)<2 → "spread is too small"
        //   1,1,1,1,2 → past=[1,1,1,1] current=2, spread=1 < 2 → FAIL
        assert_eq!(
            evaluate("absdiff(12h)<2", Some("2"), win(1.0, 1.0, 1.0)),
            EndpointState::Warning
        );
        //   1,1,1,2,2 → past=[1,1,1,2] current=2, spread=1 < 2 → FAIL
        assert_eq!(
            evaluate("absdiff(12h)<2", Some("2"), win(1.0, 1.0, 2.0)),
            EndpointState::Warning
        );
        //   1,1,2,2,3 → past=[1,1,2,2] current=3, spread=2 NOT < 2 → OK
        assert_eq!(
            evaluate("absdiff(12h)<2", Some("3"), win(1.0, 1.0, 2.0)),
            EndpointState::Ok
        );
    }

    #[test]
    fn test_absdiff_with_negative_values() {
        // Past entirely negative, current pulls max up.
        //   past=[-3,-3,-3], current=-1 → spread=|-1 - -3|=2 > 1 → FAIL
        assert_eq!(
            evaluate("absdiff(12h)>1", Some("-1"), win(-3.0, -3.0, -3.0)),
            EndpointState::Warning
        );
        // Monotonically decreasing across the window.
        //   past=[5,4,3,2], current=1 → spread = 5 - 1 = 4 > 3 → FAIL
        assert_eq!(
            evaluate("absdiff(12h)>3", Some("1"), win(5.0, 2.0, 5.0)),
            EndpointState::Warning
        );
        // Crossing zero: past mixed signs, current positive.
        //   past=[-2,-1,0,1], current=2 → spread = 2 - -2 = 4 > 3 → FAIL
        assert_eq!(
            evaluate("absdiff(12h)>3", Some("2"), win(-2.0, -2.0, 1.0)),
            EndpointState::Warning
        );
        // Tiny dip into negatives.
        //   past=[0,0,0,0], current=-1 → spread = 1 < 2 → FAIL on <2
        assert_eq!(
            evaluate("absdiff(12h)<2", Some("-1"), win(0.0, 0.0, 0.0)),
            EndpointState::Warning
        );
        // Flat-negative history, current matches → spread=0
        //   past=[-5,-5,-5], current=-5 → spread=0 → ==0 fires
        assert_eq!(
            evaluate("absdiff(12h)==0", Some("-5"), win(-5.0, -5.0, -5.0)),
            EndpointState::Warning
        );
    }

    #[test]
    fn test_diff_is_gap_tolerant() {
        // Even if no sample exists at exactly 12h ago, as long as there are
        // samples in the window we still get a usable diff. Prior implementation
        // returned NoData here.
        assert_eq!(
            evaluate("diff(12h)>0", Some("5"), win(0.0, 0.0, 5.0)),
            EndpointState::Warning
        );
    }

    #[test]
    fn test_no_data() {
        assert_eq!(evaluate(">90", None, None), EndpointState::NoData);
        // diff/absdiff with no window data → NoData (preserves prior behavior
        // for the truly-empty-history case).
        assert_eq!(
            evaluate("diff(12h)>100", Some("500"), None),
            EndpointState::NoData
        );
        assert_eq!(
            evaluate("absdiff(12h)>100", Some("500"), None),
            EndpointState::NoData
        );
    }

    #[test]
    fn test_parse_diff_hours() {
        assert_eq!(parse_diff_hours("diff(12h)>100"), Some(12.0));
        assert_eq!(parse_diff_hours("diff(24h)<50"), Some(24.0));
        assert_eq!(parse_diff_hours("absdiff(6h)>50"), Some(6.0));
        assert_eq!(parse_diff_hours("absdiff(24h)>10"), Some(24.0));
        assert_eq!(parse_diff_hours(">90"), None);
    }
}
