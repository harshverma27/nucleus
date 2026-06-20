//! Parser for `[[test]]` assertion strings (M6).
//!
//! This is the *syntactic* layer only: it turns an assertion string from
//! `stm32.toml` (e.g. `"pin PA5 toggles at 1Hz ±5%"`) into a typed
//! [`Assertion`]. It does no hardware validation — `pin` and `instance`
//! values are kept as raw strings; resolving them against the
//! [`nucleus_db`] pin/peripheral tables is the solver's job in a later task.
//!
//! Hand-written string splitting/matching, no parser-combinator crate —
//! consistent with this codebase's preference for avoiding dependencies for
//! small grammars (see [`crate::config`]).

use std::time::Duration;

/// A parsed assertion from a `[[test]]` block's `assertion` field.
#[derive(Debug, Clone, PartialEq)]
pub enum Assertion {
    /// `pin <PIN> toggles at <N>Hz ±<N>%`
    PinToggles {
        pin: String,
        hz: f64,
        tolerance_pct: f64,
    },
    /// `pin <PIN> is <high|low> within <N>ms`
    PinState {
        pin: String,
        level: bool,
        within: Duration,
    },
    /// `<PERIPH> echoes "<text>" within <N>ms`
    UartEcho {
        instance: String,
        payload: Vec<u8>,
        within: Duration,
    },
    /// `trace event "<pattern>" within <N>ms`
    ItmEvent { pattern: String, within: Duration },
}

/// Parse an assertion string into a typed [`Assertion`].
///
/// Returns `Err(String)` with a human-readable reason for anything that
/// doesn't match one of the supported grammar forms.
pub fn parse(s: &str) -> Result<Assertion, String> {
    let s = s.trim();
    let mut words = s.split_whitespace();

    let head = words
        .next()
        .ok_or_else(|| "empty assertion string".to_string())?;

    match head {
        "pin" => parse_pin_assertion(words),
        "trace" => parse_trace_assertion(words),
        _ => parse_uart_echo_assertion(head, words),
    }
}

/// `pin <PIN> toggles at <N>Hz ±<N>%` or `pin <PIN> is <high|low> within <N>ms`
fn parse_pin_assertion<'a>(mut words: impl Iterator<Item = &'a str>) -> Result<Assertion, String> {
    let pin = words
        .next()
        .ok_or_else(|| "expected a pin name after 'pin'".to_string())?
        .to_string();

    let verb = words
        .next()
        .ok_or_else(|| "expected 'toggles' or 'is' after pin name".to_string())?;

    match verb {
        "toggles" => parse_pin_toggles(pin, words),
        "is" => parse_pin_state(pin, words),
        other => Err(format!(
            "unknown verb {other:?} after pin name (expected 'toggles' or 'is')"
        )),
    }
}

/// `at <N>Hz ±<N>%`
fn parse_pin_toggles<'a>(
    pin: String,
    mut words: impl Iterator<Item = &'a str>,
) -> Result<Assertion, String> {
    let at = words
        .next()
        .ok_or_else(|| "expected 'at' after 'toggles'".to_string())?;
    if at != "at" {
        return Err(format!("expected 'at' after 'toggles', got {at:?}"));
    }

    let hz_tok = words
        .next()
        .ok_or_else(|| "expected a frequency like '1Hz' after 'at'".to_string())?;
    let hz_str = hz_tok
        .strip_suffix("Hz")
        .ok_or_else(|| format!("expected frequency to end in 'Hz', got {hz_tok:?}"))?;
    let hz = parse_nonnegative_f64(hz_str, "frequency")?;

    let pct_tok = words
        .next()
        .ok_or_else(|| "expected a tolerance like '±5%' after the frequency".to_string())?;
    let pct_str = pct_tok
        .strip_prefix('±')
        .or_else(|| pct_tok.strip_prefix("+-"))
        .unwrap_or(pct_tok);
    let pct_str = pct_str
        .strip_suffix('%')
        .ok_or_else(|| format!("expected tolerance to end in '%', got {pct_tok:?}"))?;
    let tolerance_pct = parse_nonnegative_f64(pct_str, "tolerance")?;

    if words.next().is_some() {
        return Err("unexpected trailing tokens after tolerance".to_string());
    }

    Ok(Assertion::PinToggles {
        pin,
        hz,
        tolerance_pct,
    })
}

/// `<high|low> within <N>ms`
fn parse_pin_state<'a>(
    pin: String,
    mut words: impl Iterator<Item = &'a str>,
) -> Result<Assertion, String> {
    let level_tok = words
        .next()
        .ok_or_else(|| "expected 'high' or 'low' after 'is'".to_string())?;
    let level = match level_tok {
        "high" => true,
        "low" => false,
        other => return Err(format!("expected 'high' or 'low', got {other:?}")),
    };

    let within = parse_within_clause(&mut words)?;

    if words.next().is_some() {
        return Err("unexpected trailing tokens after duration".to_string());
    }

    Ok(Assertion::PinState { pin, level, within })
}

/// `trace event "<pattern>" within <N>ms`
fn parse_trace_assertion<'a>(
    mut words: impl Iterator<Item = &'a str>,
) -> Result<Assertion, String> {
    let event = words
        .next()
        .ok_or_else(|| "expected 'event' after 'trace'".to_string())?;
    if event != "event" {
        return Err(format!("expected 'event' after 'trace', got {event:?}"));
    }

    let rest: Vec<&str> = words.collect();
    let rest = rest.join(" ");
    let (pattern, remainder) = take_quoted_string(&rest)?;

    let mut remainder_words = remainder.split_whitespace();
    let within = parse_within_clause(&mut remainder_words)?;
    if remainder_words.next().is_some() {
        return Err("unexpected trailing tokens after duration".to_string());
    }

    Ok(Assertion::ItmEvent { pattern, within })
}

/// `<PERIPH> echoes "<text>" within <N>ms`
fn parse_uart_echo_assertion<'a>(
    head: &str,
    mut words: impl Iterator<Item = &'a str>,
) -> Result<Assertion, String> {
    let echoes = words
        .next()
        .ok_or_else(|| format!("unknown leading subject {head:?} (expected 'pin', 'trace', or a peripheral name followed by 'echoes')"))?;
    if echoes != "echoes" {
        return Err(format!(
            "unknown verb {echoes:?} (expected 'echoes' after peripheral name {head:?})"
        ));
    }

    let rest: Vec<&str> = words.collect();
    let rest = rest.join(" ");
    let (text, remainder) = take_quoted_string(&rest)?;

    let mut remainder_words = remainder.split_whitespace();
    let within = parse_within_clause(&mut remainder_words)?;
    if remainder_words.next().is_some() {
        return Err("unexpected trailing tokens after duration".to_string());
    }

    Ok(Assertion::UartEcho {
        instance: head.to_string(),
        payload: text.into_bytes(),
        within,
    })
}

/// `within <N>ms`
fn parse_within_clause<'a>(words: &mut impl Iterator<Item = &'a str>) -> Result<Duration, String> {
    let within = words
        .next()
        .ok_or_else(|| "expected 'within' clause".to_string())?;
    if within != "within" {
        return Err(format!("expected 'within', got {within:?}"));
    }

    let ms_tok = words
        .next()
        .ok_or_else(|| "expected a duration like '10ms' after 'within'".to_string())?;
    let ms_str = ms_tok
        .strip_suffix("ms")
        .ok_or_else(|| format!("expected duration to end in 'ms', got {ms_tok:?}"))?;
    let ms = parse_nonnegative_f64(ms_str, "duration")?;

    Ok(Duration::from_secs_f64(ms / 1000.0))
}

/// Parse a non-negative number, rejecting negative values, NaN, and
/// non-numeric input with a `field`-specific error message.
fn parse_nonnegative_f64(s: &str, field: &str) -> Result<f64, String> {
    let value: f64 = s
        .parse()
        .map_err(|_| format!("expected a numeric {field}, got {s:?}"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{field} must be a non-negative number, got {s:?}"));
    }
    Ok(value)
}

/// Extract the first `"..."` or `'...'` delimited string from `s`, returning
/// the unquoted text and the remainder of `s` after the closing quote.
fn take_quoted_string(s: &str) -> Result<(String, &str), String> {
    let s = s.trim_start();
    let mut chars = s.char_indices();
    let (_, delim) = chars
        .next()
        .ok_or_else(|| "expected a quoted string".to_string())?;
    if delim != '"' && delim != '\'' {
        return Err(format!(
            "expected a quoted string starting with '\"' or '\'', got {s:?}"
        ));
    }

    for (idx, ch) in chars {
        if ch == delim {
            let text = &s[1..idx];
            let remainder = &s[idx + 1..];
            return Ok((text.to_string(), remainder));
        }
    }

    Err(format!("unterminated quoted string in {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pin_toggles() {
        let got = parse("pin PA5 toggles at 1Hz ±5%").unwrap();
        assert_eq!(
            got,
            Assertion::PinToggles {
                pin: "PA5".to_string(),
                hz: 1.0,
                tolerance_pct: 5.0,
            }
        );
    }

    #[test]
    fn parses_pin_toggles_with_ascii_tolerance_prefix() {
        let got = parse("pin PA5 toggles at 1Hz +-5%").unwrap();
        assert_eq!(
            got,
            Assertion::PinToggles {
                pin: "PA5".to_string(),
                hz: 1.0,
                tolerance_pct: 5.0,
            }
        );
    }

    #[test]
    fn parses_pin_state_high() {
        let got = parse("pin PA5 is high within 10ms").unwrap();
        assert_eq!(
            got,
            Assertion::PinState {
                pin: "PA5".to_string(),
                level: true,
                within: Duration::from_millis(10),
            }
        );
    }

    #[test]
    fn parses_pin_state_low() {
        let got = parse("pin PA5 is low within 25ms").unwrap();
        assert_eq!(
            got,
            Assertion::PinState {
                pin: "PA5".to_string(),
                level: false,
                within: Duration::from_millis(25),
            }
        );
    }

    #[test]
    fn parses_uart_echo_double_quoted() {
        let got = parse(r#"UART2 echoes "ping" within 10ms"#).unwrap();
        assert_eq!(
            got,
            Assertion::UartEcho {
                instance: "UART2".to_string(),
                payload: b"ping".to_vec(),
                within: Duration::from_millis(10),
            }
        );
    }

    #[test]
    fn parses_uart_echo_single_quoted() {
        let got = parse("UART2 echoes 'ping' within 10ms").unwrap();
        assert_eq!(
            got,
            Assertion::UartEcho {
                instance: "UART2".to_string(),
                payload: b"ping".to_vec(),
                within: Duration::from_millis(10),
            }
        );
    }

    #[test]
    fn parses_trace_event() {
        let got = parse(r#"trace event "boot_done" within 50ms"#).unwrap();
        assert_eq!(
            got,
            Assertion::ItmEvent {
                pattern: "boot_done".to_string(),
                within: Duration::from_millis(50),
            }
        );
    }

    #[test]
    fn rejects_unknown_verb() {
        let err = parse("frobnicate PA5").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn rejects_missing_hz_unit() {
        let err = parse("pin PA5 toggles at 1 ±5%").unwrap_err();
        assert!(err.contains("Hz"), "got {err:?}");
    }

    #[test]
    fn rejects_missing_percent_unit() {
        let err = parse("pin PA5 toggles at 1Hz ±5").unwrap_err();
        assert!(err.contains('%'), "got {err:?}");
    }

    #[test]
    fn rejects_missing_ms_unit() {
        let err = parse("pin PA5 is high within 10").unwrap_err();
        assert!(err.contains("ms"), "got {err:?}");
    }

    #[test]
    fn rejects_unterminated_quote() {
        let err = parse(r#"UART2 echoes "ping within 10ms"#).unwrap_err();
        assert!(err.contains("unterminated"), "got {err:?}");
    }

    #[test]
    fn rejects_negative_duration() {
        let err = parse("pin PA5 is high within -10ms").unwrap_err();
        assert!(err.contains("non-negative"), "got {err:?}");
    }

    #[test]
    fn rejects_garbage_input() {
        let err = parse("this is not an assertion at all").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn rejects_empty_string() {
        let err = parse("").unwrap_err();
        assert!(!err.is_empty());
    }
}
