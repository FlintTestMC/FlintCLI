pub(super) fn numbers_after_colon(message: &str) -> Vec<f64> {
    let value = message.split_once(':').map_or(message, |(_, value)| value);
    value
        .split(|character: char| {
            !(character.is_ascii_digit() || matches!(character, '-' | '+' | '.' | 'e' | 'E'))
        })
        .filter_map(|part| match part {
            "" | "-" | "+" | "." => None,
            number => number.parse().ok(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::numbers_after_colon;

    #[test]
    fn parses_signed_decimal_and_exponent_values() {
        assert_eq!(
            numbers_after_colon("Position: [1.5d, -2, +3e2]"),
            vec![1.5, -2.0, 300.0]
        );
    }
}
