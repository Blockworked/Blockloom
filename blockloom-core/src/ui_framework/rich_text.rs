//! The small tag language rich text labels are written in.

#[derive(Debug, Clone, PartialEq)]
pub struct UiTextRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub color: Option<String>,
}

/// Small, literal-safe markup: [b], [i], [color=#rrggbb] and closing tags.
pub fn rich_text(text: &str) -> Vec<UiTextRun> {
    let mut result = Vec::new();
    let mut bold = 0usize;
    let mut italic = 0usize;
    let mut colors = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with("[b]") {
            bold += 1;
            rest = &rest[3..];
            continue;
        }
        if rest.starts_with("[/b]") {
            bold = bold.saturating_sub(1);
            rest = &rest[4..];
            continue;
        }
        if rest.starts_with("[i]") {
            italic += 1;
            rest = &rest[3..];
            continue;
        }
        if rest.starts_with("[/i]") {
            italic = italic.saturating_sub(1);
            rest = &rest[4..];
            continue;
        }
        if rest.starts_with("[/color]") {
            colors.pop();
            rest = &rest[8..];
            continue;
        }
        if rest.starts_with("[color=#")
            && let Some(end) = rest.find(']')
        {
            let color = &rest[7..end];
            if matches!(color.len(), 7 | 9) && color[1..].chars().all(|c| c.is_ascii_hexdigit()) {
                colors.push(color.to_string());
                rest = &rest[end + 1..];
                continue;
            }
        }
        let len = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| *c == '[')
            .map_or(rest.len(), |(i, _)| i);
        result.push(UiTextRun {
            text: rest[..len].into(),
            bold: bold > 0,
            italic: italic > 0,
            color: colors.last().cloned(),
        });
        rest = &rest[len..];
    }
    result
}
