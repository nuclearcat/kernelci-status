use crate::checkers::EndpointState;

/// Parse and evaluate a condition string against a value.
/// Conditions: <N, >N, ==N, !=N, diff(12h)>N, diff(12h)<N
/// Returns the resulting state (Warning or Critical based on context).
/// `value` is the current measured value, `past_value` is for diff() conditions.
pub fn evaluate(
    condition: &str,
    value: Option<&str>,
    past_value: Option<&str>,
) -> EndpointState {
    let condition = condition.trim();
    if condition.is_empty() {
        return EndpointState::Ok;
    }

    // Parse diff conditions: diff(12h)>N, absdiff(12h)>N, etc.
    if let Some(rest) = condition.strip_prefix("absdiff(") {
        return evaluate_diff(rest, value, past_value, true);
    }
    if let Some(rest) = condition.strip_prefix("diff(") {
        return evaluate_diff(rest, value, past_value, false);
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

fn evaluate_diff(rest: &str, value: Option<&str>, past_value: Option<&str>, absolute: bool) -> EndpointState {
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

    let past = match past_value.and_then(|v| v.parse::<f64>().ok()) {
        Some(v) => v,
        None => return EndpointState::NoData,
    };

    let diff = if absolute {
        (current - past).abs()
    } else {
        current - past
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
        assert_eq!(
            evaluate("diff(12h)>100", Some("500"), Some("300")),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("diff(12h)>100", Some("350"), Some("300")),
            EndpointState::Ok
        );
        // == operator on diff
        assert_eq!(
            evaluate("diff(12h)==0", Some("300"), Some("300")),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("diff(12h)==0", Some("301"), Some("300")),
            EndpointState::Ok
        );
        // != operator on diff
        assert_eq!(
            evaluate("diff(12h)!=0", Some("500"), Some("300")),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("diff(12h)!=0", Some("300"), Some("300")),
            EndpointState::Ok
        );
        // >= and <= on diff
        assert_eq!(
            evaluate("diff(12h)>=100", Some("400"), Some("300")),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("diff(12h)<=0", Some("200"), Some("300")),
            EndpointState::Warning
        );
    }

    #[test]
    fn test_absdiff_conditions() {
        // absdiff uses absolute value: |current - past|
        // 300 - 500 = -200, abs = 200 > 100 → Warning
        assert_eq!(
            evaluate("absdiff(12h)>100", Some("300"), Some("500")),
            EndpointState::Warning
        );
        // 500 - 300 = 200, abs = 200 > 100 → Warning (same either direction)
        assert_eq!(
            evaluate("absdiff(12h)>100", Some("500"), Some("300")),
            EndpointState::Warning
        );
        // 350 - 300 = 50, abs = 50, not > 100 → Ok
        assert_eq!(
            evaluate("absdiff(12h)>100", Some("350"), Some("300")),
            EndpointState::Ok
        );
        // 300 - 350 = -50, abs = 50, not > 100 → Ok
        assert_eq!(
            evaluate("absdiff(12h)>100", Some("300"), Some("350")),
            EndpointState::Ok
        );
        // absdiff with == : |300 - 300| = 0 == 0 → Warning
        assert_eq!(
            evaluate("absdiff(12h)==0", Some("300"), Some("300")),
            EndpointState::Warning
        );
        assert_eq!(
            evaluate("absdiff(12h)==0", Some("301"), Some("300")),
            EndpointState::Ok
        );
    }

    #[test]
    fn test_no_data() {
        assert_eq!(evaluate(">90", None, None), EndpointState::NoData);
        assert_eq!(evaluate("diff(12h)>100", Some("500"), None), EndpointState::NoData);
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
