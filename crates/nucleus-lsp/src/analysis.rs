//! Pure analysis functions behind the language server.
//!
//! Each function maps `stm32.toml` source text (plus a cursor position) to an
//! LSP payload — diagnostics, hover, or completions — with **no I/O and no
//! async**, so the interesting behaviour is fast and deterministic to unit-test.
//! [`crate::server`] is a thin tower-lsp shell that calls these.
//!
//! All hardware knowledge comes from [`nucleus_compiler`] and [`nucleus_db`];
//! this module only translates between source spans and LSP ranges.

use std::collections::HashMap;
use std::ops::Range;
use std::str::FromStr;

use nucleus_compiler::check_family;
use nucleus_compiler::solver::Conflict;
use nucleus_db::{Database, Pin};
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, DiagnosticSeverity, Documentation, Hover,
    HoverContents, MarkupContent, MarkupKind, Position, Range as LspRange,
};

fn db() -> Database {
    Database::f446re()
}

/// Maps byte offsets in the document to LSP [`Position`]s (UTF-16 columns).
struct LineIndex<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    fn new(text: &'a str) -> LineIndex<'a> {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineIndex { text, line_starts }
    }

    /// LSP position for a byte `offset`.
    fn position(&self, offset: usize) -> Position {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(l) => l,
            Err(l) => l - 1,
        };
        let line_start = self.line_starts[line];
        let character = self.text[line_start..offset].encode_utf16().count() as u32;
        Position {
            line: line as u32,
            character,
        }
    }

    /// Byte offset for an LSP `position`.
    fn offset(&self, position: Position) -> usize {
        let line = position.line as usize;
        let Some(&line_start) = self.line_starts.get(line) else {
            return self.text.len();
        };
        let line_end = self
            .line_starts
            .get(line + 1)
            .map(|&s| s - 1)
            .unwrap_or(self.text.len());
        let mut offset = line_start;
        let mut col = 0u32;
        for ch in self.text[line_start..line_end].chars() {
            if col >= position.character {
                break;
            }
            col += ch.len_utf16() as u32;
            offset += ch.len_utf8();
        }
        offset
    }

    fn range(&self, span: Range<usize>) -> LspRange {
        LspRange {
            start: self.position(span.start),
            end: self.position(span.end),
        }
    }
}

/// Diagnostics for `text`: TOML/schema errors and every hardware conflict, each
/// placed at the most relevant source range.
pub fn diagnostics(text: &str) -> Vec<Diagnostic> {
    let li = LineIndex::new(text);

    let (report, _family) = match check_family(text) {
        Ok(result) => result,
        Err(err) => {
            let span = err.span().unwrap_or(0..text.len().min(1));
            return vec![error(li.range(span), err.message().to_string())];
        }
    };

    // Map each peripheral's DB name (e.g. "SPI1") back to the instance key the
    // user wrote (e.g. "spi1"), so we can locate its `[peripherals.…]` table.
    let name_to_key: HashMap<String, String> = report
        .config
        .peripherals
        .keys()
        .map(|k| (k.to_ascii_uppercase(), k.clone()))
        .collect();

    let mut out = Vec::new();
    for conflict in &report.conflicts {
        let message = conflict.to_string();
        for span in conflict_spans(text, conflict, &name_to_key) {
            out.push(error(li.range(span), message.clone()));
        }
    }
    out
}

fn error(range: LspRange, message: String) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("nucleus".to_string()),
        message,
        ..Diagnostic::default()
    }
}

/// The source span(s) to underline for a conflict. A collision underlines every
/// colliding pin occurrence; everything else underlines one site (the pin value
/// or, lacking one, the peripheral's table header).
fn conflict_spans(
    text: &str,
    conflict: &Conflict,
    name_to_key: &HashMap<String, String>,
) -> Vec<Range<usize>> {
    let region_of = |peripheral: &str| -> Option<(String, Range<usize>)> {
        let key = name_to_key.get(peripheral)?;
        Some((key.clone(), table_region(text, key)?))
    };

    match conflict {
        Conflict::PinCollision { pin, users } => {
            let pin = pin.to_string();
            let mut spans = Vec::new();
            for user in users {
                if let Some((_, region)) = region_of(&user.peripheral) {
                    if let Some(s) = find_quoted(text, region.clone(), &pin) {
                        spans.push(s);
                    } else {
                        spans.push(region);
                    }
                }
            }
            if spans.is_empty() {
                spans.push(whole_first_line(text));
            }
            spans
        }
        Conflict::AfMismatch {
            pin, peripheral, ..
        } => single(
            region_of(peripheral)
                .and_then(|(_, r)| find_quoted(text, r.clone(), &pin.to_string()).or(Some(r))),
            text,
        ),
        Conflict::InvalidPin {
            peripheral, value, ..
        } => single(
            region_of(peripheral)
                .and_then(|(_, r)| find_quoted(text, r.clone(), value).or(Some(r))),
            text,
        ),
        Conflict::MissingPin { peripheral, .. }
        | Conflict::ClockDomainDisabled { peripheral, .. } => single(
            name_to_key
                .get(peripheral)
                .and_then(|key| header_span(text, key)),
            text,
        ),
    }
}

fn single(span: Option<Range<usize>>, text: &str) -> Vec<Range<usize>> {
    vec![span.unwrap_or_else(|| whole_first_line(text))]
}

fn whole_first_line(text: &str) -> Range<usize> {
    0..text.find('\n').unwrap_or(text.len())
}

/// The byte range of a `[peripherals.<key>]` header, if present.
fn header_span(text: &str, key: &str) -> Option<Range<usize>> {
    let header = format!("[peripherals.{key}]");
    let start = text.find(&header)?;
    Some(start..start + header.len())
}

/// The body region of a `[peripherals.<key>]` table: from its header to the next
/// `[section]` header or end of file.
fn table_region(text: &str, key: &str) -> Option<Range<usize>> {
    let header = format!("[peripherals.{key}]");
    let start = text.find(&header)?;
    let body_start = start + header.len();
    let end = text[body_start..]
        .find("\n[")
        .map(|rel| body_start + rel)
        .unwrap_or(text.len());
    Some(start..end)
}

/// The span of `value` where it appears quoted inside `region` (the span covers
/// the value text, not the surrounding quotes).
fn find_quoted(text: &str, region: Range<usize>, value: &str) -> Option<Range<usize>> {
    let hay = &text[region.clone()];
    for quote in ['"', '\''] {
        let pat = format!("{quote}{value}{quote}");
        if let Some(rel) = hay.find(&pat) {
            let start = region.start + rel + 1;
            return Some(start..start + value.len());
        }
    }
    None
}

/// Hover for the pin name under the cursor: its full alternate-function table.
pub fn hover(text: &str, position: Position) -> Option<Hover> {
    let li = LineIndex::new(text);
    let offset = li.offset(position);
    let (token, span) = token_at(text, offset)?;
    let pin = Pin::from_str(&token).ok()?;

    let db = db();
    let mut afs: Vec<_> = db.alt_functions(pin).collect();
    if afs.is_empty() {
        return None;
    }
    afs.sort_by_key(|m| (m.af, m.peripheral, m.signal));

    let mut md = format!(
        "**{pin}** — Port {}, pin {}\n\nAlternate functions:\n",
        pin.port.letter(),
        pin.number
    );
    for m in afs {
        md.push_str(&format!("- `AF{}` — {}_{}\n", m.af, m.peripheral, m.signal));
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: md,
        }),
        range: Some(li.range(span)),
    })
}

/// Pin-name completions, offered when the cursor sits on a value line inside a
/// `[peripherals.…]` table.
pub fn completion(text: &str, position: Position) -> Vec<CompletionItem> {
    let li = LineIndex::new(text);
    let offset = li.offset(position);

    if !in_peripheral_value_position(text, offset) {
        return Vec::new();
    }

    let db = db();
    db.pins()
        .into_iter()
        .map(|pin| {
            let mut afs: Vec<_> = db.alt_functions(pin).collect();
            afs.sort_by_key(|m| (m.af, m.peripheral, m.signal));
            let doc = afs
                .iter()
                .map(|m| format!("AF{} {}_{}", m.af, m.peripheral, m.signal))
                .collect::<Vec<_>>()
                .join(", ");
            CompletionItem {
                label: pin.to_string(),
                kind: Some(CompletionItemKind::VALUE),
                detail: Some(format!("{} alternate function(s)", afs.len())),
                documentation: Some(Documentation::String(doc)),
                ..CompletionItem::default()
            }
        })
        .collect()
}

/// Whether `offset` is inside a `[peripherals.…]` table, on a line that already
/// has a `key =` (i.e. a value position worth completing pins into).
fn in_peripheral_value_position(text: &str, offset: usize) -> bool {
    let before = &text[..offset.min(text.len())];

    // Nearest section header at column 0 above the cursor.
    let in_peripherals = before
        .rfind("\n[")
        .map(|p| p + 1)
        .or_else(|| before.starts_with('[').then_some(0))
        .map(|h| text[h..].starts_with("[peripherals."))
        .unwrap_or(false);
    if !in_peripherals {
        return false;
    }

    // The current line must contain an '=' before the cursor.
    let line_start = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
    text[line_start..offset.min(text.len())].contains('=')
}

/// The identifier-like token (letters/digits) surrounding `offset`, with its span.
fn token_at(text: &str, offset: usize) -> Option<(String, Range<usize>)> {
    let bytes = text.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric();
    let offset = offset.min(text.len());

    let mut start = offset;
    while start > 0 && is_word(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = offset;
    while end < bytes.len() && is_word(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some((text[start..end].to_string(), start..end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    const CONFLICT: &str = "[device]\nfamily = \"STM32F446RE\"\n\n[peripherals.spi1]\nmosi = \"PA7\"\nmiso = \"PA6\"\nsck = \"PA5\"\n\n[peripherals.tim2]\nchannel1 = \"PA5\"\n";

    #[test]
    fn clean_config_has_no_diagnostics() {
        let text = "[peripherals.usart2]\ntx = \"PA2\"\nrx = \"PA3\"\n";
        assert!(diagnostics(text).is_empty());
    }

    #[test]
    fn collision_underlines_both_pin_sites() {
        let diags = diagnostics(CONFLICT);
        assert_eq!(diags.len(), 2, "expected one squiggle per colliding pin");
        for d in &diags {
            assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
            assert!(d.message.contains("pin collision"));
            // Each range should sit on a `"PA5"` value.
            let line = d.range.start.line;
            let src_line = CONFLICT.lines().nth(line as usize).unwrap();
            assert!(
                src_line.contains("PA5"),
                "diagnostic not on a PA5 line: {src_line}"
            );
        }
    }

    #[test]
    fn af_mismatch_points_at_the_bad_pin() {
        let text = "[peripherals.usart2]\ntx = \"PB0\"\nrx = \"PA3\"\n";
        let diags = diagnostics(text);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("AF mismatch"));
        assert_eq!(diags[0].range.start.line, 1); // the `tx = "PB0"` line
    }

    #[test]
    fn missing_pin_points_at_the_table_header() {
        let text = "[peripherals.spi1]\nmiso = \"PA6\"\nsck = \"PA5\"\n";
        let diags = diagnostics(text);
        assert!(diags
            .iter()
            .any(|d| d.message.contains("missing required pin")));
        let d = diags
            .iter()
            .find(|d| d.message.contains("missing"))
            .unwrap();
        assert_eq!(d.range.start.line, 0); // header line
    }

    #[test]
    fn syntax_error_is_a_single_diagnostic() {
        let diags = diagnostics("this is = = not valid");
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn hover_on_pin_lists_alternate_functions() {
        // Cursor on the "PA5" in `sck = "PA5"` (line 6, inside the quotes).
        let h = hover(CONFLICT, pos(6, 8)).expect("expected hover on PA5");
        let HoverContents::Markup(m) = h.contents else {
            panic!("expected markup hover");
        };
        assert!(m.value.contains("**PA5**"));
        assert!(m.value.contains("SPI1_SCK"));
        assert!(m.value.contains("TIM2_CH1"));
    }

    #[test]
    fn hover_off_a_pin_is_none() {
        // Cursor on "family" key.
        assert!(hover(CONFLICT, pos(1, 2)).is_none());
    }

    #[test]
    fn completion_offers_pins_in_a_value_position() {
        let text = "[peripherals.spi1]\nmosi = \"\"\n";
        let items = completion(text, pos(1, 8)); // inside the value quotes
        assert!(!items.is_empty());
        assert!(items.iter().any(|i| i.label == "PA7"));
    }

    #[test]
    fn no_completion_outside_a_peripheral_table() {
        let text = "[device]\nfamily = \"\"\n";
        assert!(completion(text, pos(1, 10)).is_empty());
    }
}
