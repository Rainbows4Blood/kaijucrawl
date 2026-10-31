//! Collation compatibility only, so historical Search schemas can be opened and retired.
use std::cmp::Ordering;
// Compare decimal strings exactly, including exponents, without expanding powers
// or converting money to floating point. Invalid values have a stable total order.
fn decimal_parts(input: &str) -> Option<(bool, i128, String)> {
    if !matches!(
        serde_json::from_str::<serde_json::Value>(input).ok()?,
        serde_json::Value::Number(_)
    ) {
        return None;
    }
    let negative = input.starts_with('-');
    let s = input.strip_prefix('-').unwrap_or(input);
    let (mantissa, exponent) = s
        .split_once(['e', 'E'])
        .map_or(Some((s, 0i128)), |(m, e)| {
            Some((m, e.parse::<i128>().ok()?))
        })?;
    let whole = mantissa.find('.').unwrap_or(mantissa.len());
    let mut digits = mantissa.replace('.', "");
    let leading = digits.len() - digits.trim_start_matches('0').len();
    digits = digits
        .trim_start_matches('0')
        .trim_end_matches('0')
        .to_owned();
    if digits.is_empty() {
        return Some((false, 0, String::new()));
    }
    Some((
        negative,
        exponent
            .checked_add(whole as i128)?
            .checked_sub(leading as i128)?,
        digits,
    ))
}
pub fn decimal_order(a: &str, b: &str) -> Ordering {
    match (decimal_parts(a), decimal_parts(b)) {
        (Some((an, ae, ad)), Some((bn, be, bd))) => {
            if an != bn {
                return bn.cmp(&an);
            }
            let order = if ad.is_empty() || bd.is_empty() {
                (!ad.is_empty()).cmp(&(!bd.is_empty()))
            } else {
                ae.cmp(&be).then_with(|| ad.cmp(&bd))
            };
            if an { order.reverse() } else { order }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}
